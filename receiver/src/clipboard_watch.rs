//! Bounded, cancellable Wayland clipboard observation for a single session.
use std::{
    io::Read,
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
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
static WATCH_SLOT: WatchSlot = WatchSlot::new();

// Bound how long a clipboard owner can hold a transfer open. Polling makes
// cancellation responsive without busy-waiting while the owner produces data.
const CLIPBOARD_READ_TIMEOUT: Duration = Duration::from_secs(2);
const CLIPBOARD_CANCEL_POLL_INTERVAL_MS: libc::c_int = 100;

struct WatchSlot {
    active: Mutex<bool>,
    changed: Condvar,
}

impl WatchSlot {
    const fn new() -> Self {
        Self {
            active: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    fn acquire(&self, stop: &AtomicBool) -> Option<WatchPermit<'_>> {
        let mut active = self.active.lock().ok()?;
        while *active && !stop.load(Ordering::Acquire) {
            active = self.changed.wait(active).ok()?;
        }
        if stop.load(Ordering::Acquire) {
            return None;
        }
        *active = true;
        Some(WatchPermit(self))
    }

    fn wake_waiters(&self) {
        // Locking synchronizes cancellation with the check before wait.
        if let Ok(_active) = self.active.lock() {
            self.changed.notify_all();
        }
    }
}

struct WatchPermit<'a>(&'a WatchSlot);
impl Drop for WatchPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.active.lock() {
            *active = false;
            self.0.changed.notify_all();
        }
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
            // A previous session may still be stopping. Wait for its permit
            // rather than permanently reporting an unsupported clipboard.
            let Some(_permit) = WATCH_SLOT.acquire(&worker_stop) else {
                return;
            };
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

    pub fn take_text(&self, ready: bool) -> Option<(String, bool)> {
        take_ready_text(&self.pending_text, ready)
    }

    pub fn terminated(&self) -> bool {
        self.terminated.load(Ordering::Acquire)
    }
}

fn take_ready_text(pending: &Mutex<Option<(String, bool)>>, ready: bool) -> Option<(String, bool)> {
    if !ready {
        return None;
    }
    pending.lock().ok()?.take()
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        WATCH_SLOT.wake_waiters();
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
    let deadline = Instant::now() + CLIPBOARD_READ_TIMEOUT;
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
                if unsafe { libc::poll(&mut pollfd, 1, CLIPBOARD_CANCEL_POLL_INTERVAL_MS) } < 0 {
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
    use std::{io::Write, os::unix::net::UnixStream, sync::mpsc};

    #[test]
    fn pending_text_waits_for_advertised_readiness() {
        let pending = Mutex::new(Some(("new clipboard".to_owned(), false)));
        assert_eq!(take_ready_text(&pending, false), None);
        assert_eq!(
            take_ready_text(&pending, true),
            Some(("new clipboard".to_owned(), false))
        );
        assert_eq!(take_ready_text(&pending, true), None);
    }

    #[test]
    fn reconnect_waits_for_previous_watcher_and_can_cancel() {
        let slot = Arc::new(WatchSlot::new());
        let stop = Arc::new(AtomicBool::new(false));
        let permit = slot.acquire(&stop).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker_slot = Arc::clone(&slot);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let acquired = worker_slot.acquire(&worker_stop).is_some();
            result_tx.send(acquired).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(result_rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(permit);
        assert!(result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        worker.join().unwrap();

        let permit = slot.acquire(&stop).unwrap();
        let worker_slot = Arc::clone(&slot);
        let worker_stop = Arc::clone(&stop);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let acquired = worker_slot.acquire(&worker_stop).is_some();
            result_tx.send(acquired).unwrap();
        });
        started_rx.recv().unwrap();
        stop.store(true, Ordering::Release);
        slot.wake_waiters();
        assert!(!result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        drop(permit);
        worker.join().unwrap();
    }

    struct SignalBlockedRead {
        stream: UnixStream,
        blocked: Option<mpsc::Sender<()>>,
    }

    impl Read for SignalBlockedRead {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let result = self.stream.read(buffer);
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::WouldBlock)
            {
                if let Some(blocked) = self.blocked.take() {
                    let _ = blocked.send(());
                }
            }
            result
        }
    }

    impl AsRawFd for SignalBlockedRead {
        fn as_raw_fd(&self) -> i32 {
            self.stream.as_raw_fd()
        }
    }

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
    fn cancellation_stops_an_active_read_when_owner_keeps_pipe_open() {
        let (stream, _writer) = UnixStream::pair().unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut reader = SignalBlockedRead {
                stream,
                blocked: Some(blocked_tx),
            };
            let _ = finished_tx.send(read_bounded(&mut reader, &worker_stop));
        });
        blocked_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        stop.store(true, Ordering::Release);
        assert_eq!(
            finished_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            None
        );
        worker.join().unwrap();
    }

    #[test]
    fn stalled_clipboard_owner_hits_read_deadline() {
        let (mut reader, _writer) = UnixStream::pair().unwrap();
        let started = Instant::now();
        assert_eq!(read_bounded(&mut reader, &AtomicBool::new(false)), None);
        assert!(started.elapsed() >= CLIPBOARD_READ_TIMEOUT);
        assert!(started.elapsed() < CLIPBOARD_READ_TIMEOUT + Duration::from_secs(3));
    }
}
