//! Bounded, cancellable Wayland clipboard observation for a single session.
use std::{
    io::Read,
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use wl_clipboard_rs::{
    paste::Seat,
    watch::{ClipboardEvent, ClipboardType, Watcher},
};

use mac_deck_receiver::MAX_CLIPBOARD_TEXT_BYTES;

// A compositor roundtrip can stall inside the clipboard library. Keep at most
// one explicit read alive across reconnects, even if that happens.
static MANUAL_READ_ACTIVE: AtomicBool = AtomicBool::new(false);
static WATCH_ACTIVE: AtomicBool = AtomicBool::new(false);

struct WatchPermit;
impl Drop for WatchPermit {
    fn drop(&mut self) {
        WATCH_ACTIVE.store(false, Ordering::Release);
    }
}

pub struct ClipboardWatcher {
    pub events: Receiver<WatchEvent>,
    stop: Arc<AtomicBool>,
    cancel: Arc<Mutex<Option<wl_clipboard_rs::watch::CancelHandle>>>,
    sender: SyncSender<WatchEvent>,
    pending_text: Arc<Mutex<Option<(String, bool)>>>,
    terminated: Arc<AtomicBool>,
}

pub enum WatchEvent {
    Ready(bool),
    Failed,
}

impl ClipboardWatcher {
    pub fn start() -> Self {
        let (sender, events) = mpsc::sync_channel(2);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_sender = sender.clone();
        let pending_text = Arc::new(Mutex::new(None));
        let worker_pending = Arc::clone(&pending_text);
        let terminated = Arc::new(AtomicBool::new(false));
        let worker_terminated = Arc::clone(&terminated);
        let cancel = Arc::new(Mutex::new(None));
        let worker_cancel = Arc::clone(&cancel);
        thread::spawn(move || {
            if WATCH_ACTIVE.swap(true, Ordering::AcqRel) {
                let _ = worker_sender.try_send(WatchEvent::Ready(false));
                return;
            }
            let _permit = WatchPermit;
            let Ok(mut watcher) = Watcher::new(ClipboardType::Regular, Seat::Unspecified) else {
                let _ = worker_sender.try_send(WatchEvent::Ready(false));
                return;
            };
            if let Ok(mut handle) = worker_cancel.lock() {
                if worker_stop.load(Ordering::Acquire) {
                    return;
                }
                *handle = Some(watcher.cancel_handle());
            } else {
                return;
            }
            let _ = worker_sender.try_send(WatchEvent::Ready(true));
            let mut baseline = true;
            loop {
                let event = match watcher.next_event() {
                    Ok(Some(event)) => event,
                    _ => break,
                };
                if baseline {
                    baseline = false;
                    continue;
                }
                if let ClipboardEvent::Changed {
                    mime_types,
                    mut offer,
                    ..
                } = event
                {
                    let mime = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                        .iter()
                        .find(|candidate| mime_types.iter().any(|item| item == **candidate));
                    if let Some(mime) = mime {
                        if let Ok(mut reader) = offer.receive(mime) {
                            if let Some(text) = read_bounded(&mut reader, &worker_stop) {
                                if let Ok(mut pending) = worker_pending.lock() {
                                    *pending = Some((text, false));
                                }
                            }
                        }
                    }
                }
            }
            if !worker_stop.load(Ordering::Acquire) {
                worker_terminated.store(true, Ordering::Release);
            }
        });
        Self {
            events,
            stop,
            cancel,
            sender,
            pending_text,
            terminated,
        }
    }

    pub fn send_current(&self) {
        if MANUAL_READ_ACTIVE.swap(true, Ordering::AcqRel) {
            let _ = self.sender.try_send(WatchEvent::Failed);
            return;
        }
        let sender = self.sender.clone();
        let stop = Arc::clone(&self.stop);
        let pending = Arc::clone(&self.pending_text);
        thread::spawn(move || {
            use wl_clipboard_rs::paste::{get_contents, ClipboardType, MimeType, Seat};
            let text = get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text)
                .ok()
                .and_then(|(mut reader, _)| read_bounded(&mut reader, &stop));
            if !stop.load(Ordering::Acquire) {
                if let Some(text) = text {
                    if let Ok(mut slot) = pending.lock() {
                        *slot = Some((text, true));
                    }
                } else {
                    let _ = sender.try_send(WatchEvent::Failed);
                }
            }
            MANUAL_READ_ACTIVE.store(false, Ordering::Release);
        });
    }

    pub fn take_text(&self) -> Option<(String, bool)> {
        self.pending_text.lock().ok()?.take()
    }

    pub fn terminated(&self) -> bool {
        self.terminated.load(Ordering::Acquire)
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut handle) = self.cancel.lock() {
            if let Some(cancel) = handle.take() {
                cancel.cancel();
            }
        }
    }
}

fn read_bounded(reader: &mut impl ReadFd, stop: &AtomicBool) -> Option<String> {
    let fd = reader.raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return None;
        }
        match reader.read(&mut buffer) {
            Ok(0) => return String::from_utf8(bytes).ok(),
            Ok(n) => {
                if bytes.len() + n > MAX_CLIPBOARD_TEXT_BYTES {
                    return None;
                }
                bytes.extend_from_slice(&buffer[..n]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let mut pollfd = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut pollfd, 1, 100) } < 0 {
                    return None;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

trait ReadFd: Read {
    fn raw_fd(&self) -> i32;
}
impl<T: Read + AsRawFd> ReadFd for T {
    fn raw_fd(&self) -> i32 {
        self.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, os::unix::net::UnixStream};

    #[test]
    fn reads_text_and_rejects_oversized_or_invalid_utf8() {
        for (data, expected) in [
            (b"hello".to_vec(), Some("hello".to_owned())),
            (vec![0xff], None),
            (vec![b'x'; MAX_CLIPBOARD_TEXT_BYTES + 1], None),
        ] {
            let (mut reader, mut writer) = UnixStream::pair().unwrap();
            let thread = std::thread::spawn(move || {
                let _ = writer.write_all(&data);
            });
            assert_eq!(read_bounded(&mut reader, &AtomicBool::new(false)), expected);
            thread.join().unwrap();
        }
    }

    #[test]
    fn cancellation_stops_read_even_when_owner_keeps_pipe_open() {
        let (mut reader, _writer) = UnixStream::pair().unwrap();
        assert_eq!(read_bounded(&mut reader, &AtomicBool::new(true)), None);
    }
}
