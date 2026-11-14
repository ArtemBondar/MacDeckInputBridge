use clap::{ArgAction, Parser, Subcommand};
use mac_deck_receiver::{
    derive_session_secret, generate_challenge, write_server_hello, BridgeError, ClipboardDelivery,
    ClipboardOrigin, Packet, PairingSecret, SessionFramer, MAX_CLIPBOARD_CHUNK_BYTES,
    MAX_CLIPBOARD_TEXT_BYTES,
};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    ffi::CString,
    io::{self, BufRead, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, Output, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use wl_clipboard_rs::copy::{MimeType, Options as ClipboardOptions, Source as ClipboardSource};
mod clipboard_watch;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(200);
// Once a frame starts, do not let a stalled or trickling peer hold keys forever.
const SESSION_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PENDING_HANDSHAKES: usize = 4;

#[derive(Parser)]
#[command(about = "Encrypted Mac keyboard receiver for SteamOS Desktop Mode")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate a 32-byte pairing secret. Transfer it over a trusted channel.
    Pair,
    /// Listen for one paired macOS sender. This process needs access to /dev/uinput, not root.
    Serve {
        #[arg(long, default_value = "0.0.0.0:4242")]
        bind: String,
        #[arg(long, env = "MAC_DECK_PAIRING_SECRET")]
        pairing_secret: String,
        /// Allow the paired Mac to select a mapped Plasma keyboard layout.
        #[arg(long)]
        sync_layout_from_mac: bool,
        /// One mapping in the form com.apple.keylayout.US=0. Repeat for each layout.
        #[arg(long = "layout-mapping", action = ArgAction::Append)]
        layout_mappings: Vec<String>,
        /// Allow the paired Mac to write plain text to this Plasma clipboard.
        #[arg(long)]
        sync_clipboard_from_mac: bool,
        /// Allow sending new Desktop Mode clipboard text to the paired Mac.
        #[arg(long)]
        sync_clipboard_to_mac: bool,
        #[arg(long)]
        control_stdin: bool,
    },
    /// Check whether this Plasma session can switch configured keyboard layouts.
    LayoutStatus {
        /// One mapping in the form com.apple.keylayout.US=0. Repeat for each layout.
        #[arg(long = "layout-mapping", action = ArgAction::Append)]
        layout_mappings: Vec<String>,
    },
}

trait KeySink {
    fn emit(&mut self, code: u16, value: i32) -> io::Result<()>;
    fn release_all(&mut self) -> io::Result<()>;
}

trait ClipboardSink {
    fn set_text(&mut self, text: &str) -> Result<(), String>;
}

#[derive(Default)]
struct ClipboardEchoSuppression(Option<String>);

impl ClipboardEchoSuppression {
    fn record_applied(&mut self, text: &str) {
        self.0 = Some(text.to_owned());
    }
    fn should_skip(&mut self, text: &str, explicit: bool) -> bool {
        !explicit && self.0.take().as_deref() == Some(text)
    }
}

fn gaming_session(current: Option<&str>, session: Option<&str>) -> bool {
    current.is_some_and(|value| value.to_ascii_lowercase().contains("gamescope"))
        || session.is_some_and(|value| value.to_ascii_lowercase().contains("gamescope"))
}

struct WaylandClipboardSink;

impl WaylandClipboardSink {
    fn new() -> Result<Self, String> {
        if gaming_session(
            env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
            env::var("XDG_SESSION_DESKTOP").ok().as_deref(),
        ) {
            return Err("clipboard sync requires Steam Deck Desktop Mode".to_owned());
        }
        let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            "no active Wayland runtime; open MacDeck in Steam Deck Desktop Mode".to_owned()
        })?;
        let display = env::var_os("WAYLAND_DISPLAY").ok_or_else(|| {
            "no active Wayland display; open MacDeck in Steam Deck Desktop Mode".to_owned()
        })?;
        if runtime.is_empty() || display.is_empty() {
            return Err(
                "no active Wayland session; open MacDeck in Steam Deck Desktop Mode".to_owned(),
            );
        }
        Ok(Self)
    }
}

impl ClipboardSink for WaylandClipboardSink {
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        ClipboardOptions::new()
            .copy(
                ClipboardSource::Bytes(text.as_bytes().to_vec().into()),
                MimeType::Text,
            )
            .map_err(|error| match error {
                wl_clipboard_rs::copy::Error::SocketOpenError(_)
                | wl_clipboard_rs::copy::Error::WaylandConnection(_) => {
                    "Cannot connect to Wayland; open MacDeck in Desktop Mode".to_owned()
                }
                wl_clipboard_rs::copy::Error::MissingProtocol { .. } => {
                    "The Wayland compositor lacks a required clipboard protocol".to_owned()
                }
                _ => "Could not write to the Wayland clipboard; check the Desktop Mode session"
                    .to_owned(),
            })
    }
}

struct ClipboardTransfer {
    transfer_id: u64,
    total_bytes: usize,
    chunk_count: u16,
    next_index: u16,
    bytes: Vec<u8>,
}

impl ClipboardTransfer {
    fn begin(transfer_id: u64, total_bytes: u32, chunk_count: u16) -> Result<Self, BridgeError> {
        let total_bytes = usize::try_from(total_bytes).map_err(|_| BridgeError::InvalidPacket)?;
        if total_bytes > MAX_CLIPBOARD_TEXT_BYTES {
            return Err(BridgeError::InvalidPacket);
        }
        Ok(Self {
            transfer_id,
            total_bytes,
            chunk_count,
            next_index: 0,
            bytes: Vec::with_capacity(total_bytes),
        })
    }

    fn append(&mut self, transfer_id: u64, index: u16, bytes: &[u8]) -> Result<(), BridgeError> {
        if transfer_id != self.transfer_id || index != self.next_index || index >= self.chunk_count
        {
            return Err(BridgeError::InvalidPacket);
        }
        let next_length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(BridgeError::InvalidPacket)?;
        if next_length > self.total_bytes {
            return Err(BridgeError::InvalidPacket);
        }
        self.bytes.extend_from_slice(bytes);
        self.next_index = self
            .next_index
            .checked_add(1)
            .ok_or(BridgeError::InvalidPacket)?;
        Ok(())
    }

    fn finish(self, transfer_id: u64) -> Result<String, BridgeError> {
        if transfer_id != self.transfer_id
            || self.next_index != self.chunk_count
            || self.bytes.len() != self.total_bytes
        {
            return Err(BridgeError::InvalidPacket);
        }
        String::from_utf8(self.bytes).map_err(|_| BridgeError::InvalidPacket)
    }
}

#[cfg(test)]
fn handle_client(
    stream: &mut TcpStream,
    pairing_secret: &PairingSecret,
    sink: &mut impl KeySink,
) -> Result<(), BridgeError> {
    // A peer must prove possession of the pairing secret within a bounded time
    // before it can occupy the receiver. The challenge derives a fresh session
    // key, so an authenticated frame captured on an old connection cannot replay.
    let session = receive_client_ready(stream, pairing_secret)?;
    session.write_packet(stream, 0, &Packet::ServerReady)?;
    let shutdown = AtomicBool::new(false);
    handle_authenticated_client(
        stream,
        &session,
        sink,
        &shutdown,
        None,
        None,
        (false, &AtomicBool::new(false)),
    )
}

// Apply a single deadline to the entire handshake, including partial reads and
// writes. Per-operation socket timeouts alone restart when a peer trickles data.
struct HandshakeStream<'a> {
    stream: &'a mut TcpStream,
    deadline: Instant,
}

impl HandshakeStream<'_> {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "handshake timed out"))
    }
}

impl Read for HandshakeStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(buf)
    }
}

impl Write for HandshakeStream<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.remaining()?;
        self.stream.flush()
    }
}

fn receive_client_ready(
    stream: &mut TcpStream,
    pairing_secret: &PairingSecret,
) -> Result<SessionFramer, BridgeError> {
    receive_client_ready_before(stream, pairing_secret, HANDSHAKE_TIMEOUT)
}

fn receive_client_ready_before(
    stream: &mut TcpStream,
    pairing_secret: &PairingSecret,
    timeout: Duration,
) -> Result<SessionFramer, BridgeError> {
    let mut handshake = HandshakeStream {
        stream,
        deadline: Instant::now() + timeout,
    };
    let challenge = generate_challenge();
    write_server_hello(&mut handshake, &challenge)?;
    let session = SessionFramer::new(derive_session_secret(pairing_secret, &challenge));
    let (sequence, packet) = session.read_packet(&mut handshake)?;
    handshake.remaining()?;
    if sequence != 0 || packet != Packet::ClientReady {
        return Err(BridgeError::Handshake);
    }
    Ok(session)
}

// A pending slot protects only the unauthenticated phase, not the lifetime of
// an accepted session. Release it on both success and failure.
fn complete_pending_handshake(
    stream: &mut TcpStream,
    pairing_secret: &PairingSecret,
    pending_handshakes: &AtomicUsize,
) -> Result<SessionFramer, BridgeError> {
    let result = receive_client_ready(stream, pairing_secret);
    pending_handshakes.fetch_sub(1, Ordering::AcqRel);
    result
}

// An idle session can wait indefinitely, but a started frame has a bounded
// assembly time. Polling during a partial frame still observes shutdown.
struct SessionFrameReader<'a> {
    stream: &'a mut TcpStream,
    shutdown: &'a AtomicBool,
    deadline: Option<Instant>,
    timeout: Duration,
    bytes_read: usize,
}

impl Read for SessionFrameReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "receiver shutting down",
                ));
            }
            let wait = if let Some(deadline) = self.deadline {
                deadline
                    .checked_duration_since(Instant::now())
                    .filter(|remaining| !remaining.is_zero())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "incomplete frame timed out")
                    })?
                    .min(SESSION_POLL_INTERVAL)
            } else {
                SESSION_POLL_INTERVAL
            };
            self.stream.set_read_timeout(Some(wait))?;
            match self.stream.read(buffer) {
                Ok(count) => {
                    if count > 0 {
                        self.bytes_read += count;
                        self.deadline
                            .get_or_insert_with(|| Instant::now() + self.timeout);
                    }
                    return Ok(count);
                }
                Err(error)
                    if self.deadline.is_some()
                        && matches!(
                            error.kind(),
                            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                        ) =>
                {
                    continue
                }
                result => return result,
            }
        }
    }
}

fn handle_authenticated_client(
    stream: &mut TcpStream,
    session: &SessionFramer,
    sink: &mut impl KeySink,
    shutdown: &AtomicBool,
    layout_switcher: Option<&PlasmaLayoutSwitcher>,
    clipboard_sink: Option<&mut dyn ClipboardSink>,
    reverse_clipboard: (bool, &AtomicBool),
) -> Result<(), BridgeError> {
    let (allow_clipboard_to_mac, manual_clipboard_request) = reverse_clipboard;
    // A stalled peer must not indefinitely hold virtual keys while a clipboard
    // acknowledgement or transfer is being sent.
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let mut next_sequence = 1u64;
    // The TCP connection is full-duplex. Use a separate cloned handle for
    // acknowledgements so the receiver can keep accepting keyboard input while
    // reporting the result of a completed clipboard transfer.
    let mut response_stream = stream.try_clone()?;
    let mut next_response_sequence = 1u64;
    let mut clipboard_transfer: Option<ClipboardTransfer> = None;
    let mut clipboard_sink = clipboard_sink;
    let mut watcher: Option<clipboard_watch::ClipboardWatcher> = None;
    let mut suppression = ClipboardEchoSuppression::default();
    let mut transfer_id = 0u64;
    manual_clipboard_request.store(false, Ordering::Release);
    let mut watcher_ready = false;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        if manual_clipboard_request.swap(false, Ordering::AcqRel) {
            if watcher_ready {
                if let Some(watcher) = watcher.as_ref() {
                    watcher.send_current();
                }
            } else {
                eprintln!("Steam Deck clipboard sharing is not connected.");
            }
        }
        if let Some(event) = watcher
            .as_ref()
            .and_then(|watcher| watcher.events.try_recv().ok())
        {
            if let clipboard_watch::WatchEvent::Ready(ready) = event {
                watcher_ready = ready;
                session.write_packet(
                    &mut response_stream,
                    next_response_sequence,
                    &Packet::ClipboardCapabilities { deck_to_mac: ready },
                )?;
                next_response_sequence = next_response_sequence
                    .checked_add(1)
                    .ok_or(BridgeError::Handshake)?;
                continue;
            }
            eprintln!("Could not read plain text from the Steam Deck clipboard.");
        }
        if watcher_ready && watcher.as_ref().is_some_and(|watcher| watcher.terminated()) {
            watcher_ready = false;
            watcher = None;
            eprintln!(
                "Steam Deck clipboard monitoring stopped. Reconnect to restore clipboard sharing."
            );
            session.write_packet(
                &mut response_stream,
                next_response_sequence,
                &Packet::ClipboardCapabilities { deck_to_mac: false },
            )?;
            next_response_sequence = next_response_sequence
                .checked_add(1)
                .ok_or(BridgeError::Handshake)?;
        }
        if let Some((text, explicit)) = watcher
            .as_ref()
            .and_then(|watcher| watcher.take_text(watcher_ready))
        {
            if !suppression.should_skip(&text, explicit) {
                transfer_id = transfer_id.checked_add(1).ok_or(BridgeError::Handshake)?;
                let bytes = text.as_bytes();
                let count = bytes.len().div_ceil(MAX_CLIPBOARD_CHUNK_BYTES);
                let mut packets = Vec::with_capacity(count + 2);
                packets.push(Packet::ClipboardStart {
                    origin: ClipboardOrigin::SteamDeck,
                    transfer_id,
                    total_bytes: bytes.len() as u32,
                    chunk_count: count as u16,
                });
                for (index, chunk) in bytes.chunks(MAX_CLIPBOARD_CHUNK_BYTES).enumerate() {
                    packets.push(Packet::ClipboardChunk {
                        origin: ClipboardOrigin::SteamDeck,
                        transfer_id,
                        index: index as u16,
                        bytes: chunk.to_vec(),
                    });
                }
                packets.push(Packet::ClipboardEnd {
                    origin: ClipboardOrigin::SteamDeck,
                    transfer_id,
                });
                for packet in packets {
                    session.write_packet(&mut response_stream, next_response_sequence, &packet)?;
                    next_response_sequence = next_response_sequence
                        .checked_add(1)
                        .ok_or(BridgeError::Handshake)?;
                }
            }
        }
        let mut frame = SessionFrameReader {
            stream,
            shutdown,
            deadline: None,
            timeout: SESSION_FRAME_TIMEOUT,
            bytes_read: 0,
        };
        let (sequence, packet) = match session.read_packet(&mut frame) {
            Ok(packet) => packet,
            Err(BridgeError::Io(error))
                if frame.bytes_read == 0
                    && matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        expect_next_sequence(&mut next_sequence, sequence)?;
        match packet {
            Packet::Key { code, value } if is_supported_key(code) => {
                sink.emit(code, value as i32)?
            }
            Packet::KeyboardLayout(identifier) => {
                if let Some(layout_switcher) = layout_switcher {
                    match layout_switcher.switch_to(&identifier) {
                        Ok(outcome) => eprintln!(
                            "Deck layout sync for {identifier}: {}",
                            outcome.description()
                        ),
                        Err(error) => {
                            eprintln!("Could not synchronize the Deck keyboard layout: {error}");
                        }
                    }
                }
            }
            Packet::ClipboardStart {
                origin: ClipboardOrigin::Mac,
                transfer_id,
                total_bytes,
                chunk_count,
            } => {
                if clipboard_sink.is_none() || clipboard_transfer.is_some() {
                    return Err(BridgeError::InvalidPacket);
                }
                clipboard_transfer = Some(ClipboardTransfer::begin(
                    transfer_id,
                    total_bytes,
                    chunk_count,
                )?);
            }
            Packet::ClipboardChunk {
                origin: ClipboardOrigin::Mac,
                transfer_id,
                index,
                bytes,
            } => {
                clipboard_transfer
                    .as_mut()
                    .ok_or(BridgeError::InvalidPacket)?
                    .append(transfer_id, index, &bytes)?;
            }
            Packet::ClipboardEnd {
                origin: ClipboardOrigin::Mac,
                transfer_id,
            } => {
                let text = clipboard_transfer
                    .take()
                    .ok_or(BridgeError::InvalidPacket)?
                    .finish(transfer_id)?;
                let delivery = match clipboard_sink.as_deref_mut() {
                    Some(sink) => match sink.set_text(&text) {
                        Ok(()) => {
                            suppression.record_applied(&text);
                            eprintln!("Steam Deck clipboard updated from Mac.");
                            ClipboardDelivery::Written
                        }
                        Err(error) => {
                            // Clipboard support is optional. A compositor failure must not
                            // terminate keyboard forwarding or expose copied content.
                            eprintln!("Could not update the Steam Deck clipboard: {error}");
                            ClipboardDelivery::Failed
                        }
                    },
                    None => ClipboardDelivery::Unavailable,
                };
                session.write_packet(
                    &mut response_stream,
                    next_response_sequence,
                    &Packet::ClipboardDelivery {
                        origin: ClipboardOrigin::Mac,
                        transfer_id,
                        delivery,
                    },
                )?;
                next_response_sequence = next_response_sequence
                    .checked_add(1)
                    .ok_or(BridgeError::Handshake)?;
            }
            Packet::Key { .. }
            | Packet::ClientReady
            | Packet::ServerReady
            | Packet::ServerReadyWithLayoutSync
            | Packet::ServerReadyWithClipboardSync
            | Packet::ServerReadyWithLayoutAndClipboardSync
            | Packet::ClipboardStart { .. }
            | Packet::ClipboardChunk { .. }
            | Packet::ClipboardEnd { .. }
            | Packet::ClipboardCapabilities { .. }
            | Packet::ClipboardDelivery {
                origin: ClipboardOrigin::Mac,
                ..
            } => return Err(BridgeError::InvalidPacket),
            Packet::ClipboardDelivery {
                origin: ClipboardOrigin::SteamDeck,
                delivery,
                ..
            } => match delivery {
                ClipboardDelivery::Written => eprintln!("Clipboard received by Mac."),
                ClipboardDelivery::Unavailable | ClipboardDelivery::Failed => {
                    eprintln!("Mac could not accept the clipboard.")
                }
            },
            Packet::ReleaseAll => sink.release_all()?,
            Packet::Ping => {
                let allowed = allow_clipboard_to_mac && WaylandClipboardSink::new().is_ok();
                if allowed && watcher.is_none() {
                    watcher = Some(clipboard_watch::ClipboardWatcher::start());
                } else if !allowed {
                    session.write_packet(
                        &mut response_stream,
                        next_response_sequence,
                        &Packet::ClipboardCapabilities { deck_to_mac: false },
                    )?;
                    next_response_sequence = next_response_sequence
                        .checked_add(1)
                        .ok_or(BridgeError::Handshake)?;
                }
            }
        }
    }
}

#[derive(Clone)]
struct PlasmaLayoutSwitcher {
    qdbus: PathBuf,
    mappings: HashMap<String, u8>,
    automatic_layouts: Vec<ConfiguredLayout>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfiguredLayout {
    index: u8,
    xkb_code: String,
    description: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutSelection {
    Override(u8),
    Automatic(u8),
    UnchangedNoMatch,
    UnchangedAmbiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutSwitchOutcome {
    Override(u8),
    Automatic(u8),
    UnchangedNoMatch,
    UnchangedAmbiguous,
}

impl LayoutSwitchOutcome {
    fn description(self) -> String {
        match self {
            Self::Override(index) => format!("manual override selected Deck layout {index}"),
            Self::Automatic(index) => format!("automatic match selected Deck layout {index}"),
            Self::UnchangedNoMatch => {
                "no configured Deck layout matched; left unchanged".to_owned()
            }
            Self::UnchangedAmbiguous => {
                "multiple configured Deck layouts matched; left unchanged".to_owned()
            }
        }
    }
}

const PLASMA_KEYBOARD_LAYOUT_SERVICE: &str = "org.kde.keyboard";
const PLASMA_KEYBOARD_LAYOUT_PATH: &str = "/Layouts";
const PLASMA_KEYBOARD_LAYOUT_METHOD: &str = "org.kde.KeyboardLayouts.setLayout";

impl PlasmaLayoutSwitcher {
    fn new(mappings: &[String]) -> Result<Self, String> {
        let qdbus = find_qdbus()?;
        let mut parsed = HashMap::new();
        for mapping in mappings {
            let Some((source, raw_index)) = mapping.split_once('=') else {
                return Err("invalid layout mapping".to_owned());
            };
            let index = raw_index
                .parse::<u8>()
                .map_err(|_| "invalid Deck layout number")?;
            parsed.insert(source.to_owned(), index);
        }
        let automatic_layouts = configured_plasma_layouts(&qdbus);
        if parsed.is_empty() && automatic_layouts.is_empty() {
            return Err(
                "no Plasma keyboard layouts were found; add layouts in Plasma Settings first"
                    .to_owned(),
            );
        }
        Ok(Self {
            qdbus,
            mappings: parsed,
            automatic_layouts,
        })
    }

    fn switch_to(&self, mac_input_source: &str) -> io::Result<LayoutSwitchOutcome> {
        let (index, outcome) = match self.layout_selection(mac_input_source) {
            LayoutSelection::Override(index) => (index, LayoutSwitchOutcome::Override(index)),
            LayoutSelection::Automatic(index) => (index, LayoutSwitchOutcome::Automatic(index)),
            // An unmatched or ambiguous layout is not an error: ordinary
            // physical-key forwarding continues with the Deck's current layout.
            LayoutSelection::UnchangedNoMatch => return Ok(LayoutSwitchOutcome::UnchangedNoMatch),
            LayoutSelection::UnchangedAmbiguous => {
                return Ok(LayoutSwitchOutcome::UnchangedAmbiguous)
            }
        };
        let index = index.to_string();
        let output = qdbus_call(
            &self.qdbus,
            &[
                PLASMA_KEYBOARD_LAYOUT_SERVICE,
                PLASMA_KEYBOARD_LAYOUT_PATH,
                PLASMA_KEYBOARD_LAYOUT_METHOD,
                &index,
            ],
        )?;
        if output.status.success() && qdbus_boolean_reply_accepted(&output.stdout) {
            Ok(outcome)
        } else {
            Err(io::Error::other(qdbus_failure_description(
                "Plasma rejected the layout switch",
                &output,
            )))
        }
    }

    fn layout_selection(&self, mac_input_source: &str) -> LayoutSelection {
        if let Some(index) = self.mappings.get(mac_input_source).copied() {
            return LayoutSelection::Override(index);
        }
        match automatic_layout_candidates(mac_input_source, &self.automatic_layouts).as_slice() {
            [index] => LayoutSelection::Automatic(*index),
            [] => LayoutSelection::UnchangedNoMatch,
            _ => LayoutSelection::UnchangedAmbiguous,
        }
    }

    fn status_report(&self) -> Result<String, String> {
        let output = qdbus_call(
            &self.qdbus,
            &[
                PLASMA_KEYBOARD_LAYOUT_SERVICE,
                PLASMA_KEYBOARD_LAYOUT_PATH,
                "org.kde.KeyboardLayouts.getLayout",
            ],
        )
        .map_err(|error| format!("could not query Plasma's active layout: {error}"))?;
        if !output.status.success() {
            return Err(qdbus_failure_description(
                "Plasma did not answer the active-layout query",
                &output,
            ));
        }
        let active = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let layouts = self
            .automatic_layouts
            .iter()
            .map(|layout| {
                format!(
                    "{}: {} ({})",
                    layout.index, layout.xkb_code, layout.description
                )
            })
            .collect::<Vec<_>>();
        if layouts.is_empty() {
            return Err("Plasma has no readable configured keyboard layouts".to_owned());
        }
        Ok(format!(
            "Ready. qdbus: {}. Active Deck layout: {}. Configured layouts: {}.",
            self.qdbus.display(),
            if active.is_empty() {
                "unavailable"
            } else {
                &active
            },
            layouts.join(", ")
        ))
    }
}

fn find_qdbus() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("MAC_DECK_QDBUS") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(paths) = env::var_os("PATH") {
        for directory in env::split_paths(&paths) {
            for name in ["qdbus6", "qdbus-qt6", "qdbus"] {
                candidates.push(directory.join(name));
            }
        }
    }
    candidates.extend([
        PathBuf::from("/usr/bin/qdbus6"),
        PathBuf::from("/usr/bin/qdbus-qt6"),
        PathBuf::from("/usr/bin/qdbus"),
        PathBuf::from("/usr/lib/qt6/bin/qdbus6"),
        PathBuf::from("/usr/lib/qt6/bin/qdbus"),
    ]);
    select_qdbus(candidates).ok_or_else(|| {
        "No executable qdbus found. Install qdbus6/qdbus-qt6 or set MAC_DECK_QDBUS to an executable path"
            .to_owned()
    })
}

fn select_qdbus(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|path| {
        if !path.is_file() {
            return false;
        }
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // Match the effective credentials used by the receiver; a non-executable
        // file earlier in PATH must not hide a usable qdbus later in the search.
        unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::X_OK, libc::AT_EACCESS) == 0 }
    })
}

// qdbus output is normally small. Bound it so a malfunctioning command cannot
// exhaust receiver memory while avoiding pipe-buffer deadlocks on larger lists.
const QDBUS_OUTPUT_LIMIT: usize = 1024 * 1024;
const QDBUS_TIMEOUT: Duration = Duration::from_secs(1);
const QDBUS_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn nonblocking_pipe(pipe: &impl AsRawFd) -> io::Result<()> {
    let fd = pipe.as_raw_fd();
    // These descriptors are owned by the child-output handles, and remain open
    // until the handles are dropped below.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain_qdbus_pipe(pipe: &mut impl Read, output: &mut Vec<u8>) -> io::Result<()> {
    let mut buffer = [0; 4096];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if output.len() + count > QDBUS_OUTPUT_LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "qdbus output exceeded the size limit",
                    ));
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn qdbus_call(qdbus: &Path, arguments: &[&str]) -> io::Result<Output> {
    let mut child = ProcessCommand::new(qdbus)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let result = (|| {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("qdbus stdout unavailable"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("qdbus stderr unavailable"))?;
        nonblocking_pipe(&stdout)?;
        nonblocking_pipe(&stderr)?;
        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let deadline = Instant::now() + QDBUS_TIMEOUT;
        loop {
            drain_qdbus_pipe(&mut stdout, &mut stdout_bytes)?;
            drain_qdbus_pipe(&mut stderr, &mut stderr_bytes)?;
            if let Some(status) = child.try_wait()? {
                // Read bytes written between the last drain and process exit.
                drain_qdbus_pipe(&mut stdout, &mut stdout_bytes)?;
                drain_qdbus_pipe(&mut stderr, &mut stderr_bytes)?;
                return Ok(Output {
                    status,
                    stdout: stdout_bytes,
                    stderr: stderr_bytes,
                });
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Plasma D-Bus did not answer within one second",
                ));
            }
            thread::sleep(QDBUS_POLL_INTERVAL);
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn qdbus_boolean_reply_accepted(stdout: &[u8]) -> bool {
    !matches!(
        String::from_utf8_lossy(stdout)
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "false" | "0"
    )
}

fn qdbus_failure_description(prefix: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    if detail.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}: {detail}")
    }
}

fn configured_plasma_layouts(qdbus: &Path) -> Vec<ConfiguredLayout> {
    let runtime = runtime_plasma_layouts(qdbus);
    if !runtime.is_empty() {
        return runtime;
    }
    let Some(layout_list) = read_kxkbrc_layout_list() else {
        return Vec::new();
    };
    let descriptions = read_xkb_layout_descriptions();
    layout_list
        .into_iter()
        .enumerate()
        .filter_map(|(index, code)| {
            let index = u8::try_from(index).ok()?;
            // A minimal SteamOS installation can expose kxkbrc before the XKB
            // rule-description file is available. Keep the layout visible to
            // diagnostics instead of silently discarding it.
            let description = descriptions
                .get(&code)
                .cloned()
                .unwrap_or_else(|| code.clone());
            Some(ConfiguredLayout {
                index,
                xkb_code: code,
                description,
            })
        })
        .collect()
}

fn runtime_plasma_layouts(qdbus: &Path) -> Vec<ConfiguredLayout> {
    let Ok(output) = qdbus_call(
        qdbus,
        &[
            "--literal",
            PLASMA_KEYBOARD_LAYOUT_SERVICE,
            PLASMA_KEYBOARD_LAYOUT_PATH,
            "org.kde.KeyboardLayouts.getLayoutsList",
        ],
    ) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_qdbus_layout_list(&String::from_utf8_lossy(&output.stdout))
}

/// Plasma 6's `qdbus --literal` represents `a(sss)` entries as
/// `(sss) "layout", "variant", "display name"`. The display name is the
/// session's own human-readable name, so it matches macOS source names without
/// a language table and still reflects user-configured variants.
fn parse_qdbus_layout_list(output: &str) -> Vec<ConfiguredLayout> {
    let mut layouts = Vec::new();
    let mut remainder = output;
    while let Some(marker) = remainder.find("(sss)") {
        remainder = &remainder[marker + "(sss)".len()..];
        let Some((layout, after_layout)) = next_qdbus_quoted_string(remainder) else {
            break;
        };
        let Some((_variant, after_variant)) = next_qdbus_quoted_string(after_layout) else {
            break;
        };
        let Some((description, after_description)) = next_qdbus_quoted_string(after_variant) else {
            break;
        };
        let Ok(index) = u8::try_from(layouts.len()) else {
            break;
        };
        layouts.push(ConfiguredLayout {
            index,
            xkb_code: layout,
            description,
        });
        remainder = after_description;
    }
    layouts
}

fn next_qdbus_quoted_string(value: &str) -> Option<(String, &str)> {
    let start = value.find('"')? + 1;
    let bytes = value.as_bytes();
    let mut escaped = false;
    for index in start..bytes.len() {
        match bytes[index] {
            b'\\' if !escaped => escaped = true,
            b'"' if !escaped => {
                let text = value[start..index].replace(r#"\""#, "\"");
                return Some((text, &value[index + 1..]));
            }
            _ => escaped = false,
        }
    }
    None
}

fn read_kxkbrc_layout_list() -> Option<Vec<String>> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let contents = std::fs::read_to_string(root.join("kxkbrc")).ok()?;
    let mut in_layout_section = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_layout_section = line == "[Layout]";
            continue;
        }
        if in_layout_section {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() == "LayoutList" {
                return Some(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|code| !code.is_empty())
                        .map(str::to_owned)
                        .collect(),
                );
            }
        }
    }
    None
}

fn read_xkb_layout_descriptions() -> BTreeMap<String, String> {
    let mut directories = Vec::new();
    if let Some(directory) = env::var_os("MAC_DECK_XKB_RULES_DIR") {
        let directory = PathBuf::from(directory);
        // A launcher can have a different working directory from a terminal.
        if directory.is_absolute() {
            directories.push(directory);
        }
    }
    directories.push(PathBuf::from("/usr/share/X11/xkb/rules"));
    read_xkb_layout_descriptions_from(&directories)
}

fn read_xkb_layout_descriptions_from(directories: &[PathBuf]) -> BTreeMap<String, String> {
    for directory in directories {
        for name in ["base.lst", "evdev.lst"] {
            let Ok(contents) = std::fs::read_to_string(directory.join(name)) else {
                continue;
            };
            let mut descriptions = BTreeMap::new();
            let mut in_layout_section = false;
            for line in contents.lines() {
                let line = line.trim();
                if line.starts_with('!') {
                    in_layout_section = line == "! layout";
                    continue;
                }
                if !in_layout_section || line.is_empty() {
                    continue;
                }
                let mut fields = line.split_whitespace();
                let Some(code) = fields.next() else { continue };
                let description = fields.collect::<Vec<_>>().join(" ");
                if !description.is_empty() {
                    descriptions.insert(code.to_owned(), description);
                }
            }
            if !descriptions.is_empty() {
                return descriptions;
            }
        }
    }
    BTreeMap::new()
}

#[cfg(test)]
fn automatic_layout_index(
    mac_input_source: &str,
    configured_layouts: &[ConfiguredLayout],
) -> Option<u8> {
    match automatic_layout_candidates(mac_input_source, configured_layouts).as_slice() {
        [index] => Some(*index),
        _ => None,
    }
}

fn automatic_layout_candidates(
    mac_input_source: &str,
    configured_layouts: &[ConfiguredLayout],
) -> Vec<u8> {
    let Some(source_name) = mac_input_source.rsplit('.').next() else {
        return Vec::new();
    };
    let mut source_terms = layout_match_terms(source_name);
    // macOS calls its standard US physical keyboard source "ABC", whereas
    // XKB/Plasma calls the same layout "us". Treat this as a platform alias,
    // not a user-specific language mapping, so no manual rule is required.
    if source_terms.iter().any(|term| term == "abc")
        && !source_terms.iter().any(|term| term == "us")
    {
        source_terms.push("us".to_owned());
    }
    if source_terms.is_empty() {
        return Vec::new();
    }
    configured_layouts
        .iter()
        .filter(|layout| {
            let mut layout_terms = layout_match_terms(&layout.xkb_code);
            layout_terms.extend(layout_match_terms(&layout.description));
            source_terms.iter().any(|source_term| {
                // A Mac source ID often adds a hardware or variant suffix,
                // for example `Ukrainian-PC`. Match its meaningful pieces to
                // the configured XKB code/description without enumerating
                // languages. Multiple candidate layouts remain intentionally
                // ambiguous and therefore do not trigger a switch.
                layout_terms
                    .iter()
                    .any(|layout_term| layout_term == source_term)
            })
        })
        .map(|layout| layout.index)
        .collect()
}

fn layout_match_terms(value: &str) -> Vec<String> {
    let whole = normalize_layout_name(value);
    let mut terms = Vec::new();
    if !whole.is_empty() {
        terms.push(whole);
    }
    for word in value.split(|character: char| !character.is_alphanumeric()) {
        let normalized = normalize_layout_name(word);
        if !normalized.is_empty() && !terms.contains(&normalized) {
            terms.push(normalized);
        }
    }
    terms
}

fn normalize_layout_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn expect_next_sequence(expected: &mut u64, received: u64) -> Result<(), BridgeError> {
    if received != *expected {
        return Err(BridgeError::Handshake);
    }
    *expected = expected.checked_add(1).ok_or(BridgeError::Handshake)?;
    Ok(())
}

/// The receiver accepts only codes that its uinput device explicitly advertises.
/// This keeps an authenticated sender within the least-privilege key surface.
fn is_supported_key(code: u16) -> bool {
    (1..=69).contains(&code)
        || (71..=83).contains(&code)
        || matches!(
            code,
            86 | 87
                | 88
                | 96
                | 97
                | 98
                | 100
                | 102
                | 103
                | 104
                | 105
                | 106
                | 107
                | 108
                | 109
                | 110
                | 111
                | 113
                | 114
                | 115
                | 117
                | 125
                | 126
                | 127
                | 183..=190
        )
}

#[cfg(target_os = "linux")]
mod uinput {
    use super::KeySink;
    use std::{
        collections::BTreeSet,
        fs::OpenOptions,
        io::{self, Write},
        mem::size_of,
        os::fd::AsRawFd,
    };

    const EV_SYN: u16 = 0;
    const EV_KEY: u16 = 1;
    const SYN_REPORT: u16 = 0;
    const UI_DEV_CREATE: libc::c_ulong = 0x5501;
    const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
    const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
    const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
    const BUS_USB: u16 = 0x03;
    const DEVICE_NAME: &[u8] = b"Mac Deck Input Bridge";

    #[repr(C)]
    struct InputId {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
    }
    #[repr(C)]
    struct UinputUserDev {
        name: [u8; 80],
        id: InputId,
        ff_effects_max: u32,
        absmax: [i32; 64],
        absmin: [i32; 64],
        absfuzz: [i32; 64],
        absflat: [i32; 64],
    }
    #[repr(C)]
    struct InputEvent {
        time: libc::timeval,
        type_: u16,
        code: u16,
        value: i32,
    }

    pub struct UinputSink {
        device: std::fs::File,
        held: BTreeSet<u16>,
    }

    impl UinputSink {
        pub fn open() -> io::Result<Self> {
            let mut device = OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/uinput")?;
            unsafe {
                ioctl(device.as_raw_fd(), UI_SET_EVBIT, EV_KEY as libc::c_ulong)?;
                ioctl(device.as_raw_fd(), UI_SET_EVBIT, EV_SYN as libc::c_ulong)?;
                // Sender maps only ordinary keyboard, modifier, navigation and function keys.
                for key in supported_keys() {
                    ioctl(device.as_raw_fd(), UI_SET_KEYBIT, key as libc::c_ulong)?;
                }
            }
            let mut setup: UinputUserDev = unsafe { std::mem::zeroed() };
            setup.name[..DEVICE_NAME.len()].copy_from_slice(DEVICE_NAME);
            setup.id = InputId {
                bustype: BUS_USB,
                vendor: 0x1d6b,
                product: 0x0104,
                version: 1,
            };
            device.write_all(as_bytes(&setup))?;
            unsafe {
                ioctl(device.as_raw_fd(), UI_DEV_CREATE, 0)?;
            }
            Ok(Self {
                device,
                held: BTreeSet::new(),
            })
        }

        fn write_event(&mut self, type_: u16, code: u16, value: i32) -> io::Result<()> {
            let event = InputEvent {
                time: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_,
                code,
                value,
            };
            self.device.write_all(as_bytes(&event))?;
            self.device.write_all(as_bytes(&InputEvent {
                time: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: EV_SYN,
                code: SYN_REPORT,
                value: 0,
            }))?;
            self.device.flush()
        }
    }

    impl KeySink for UinputSink {
        fn emit(&mut self, code: u16, value: i32) -> io::Result<()> {
            if value == 1 {
                self.held.insert(code);
            }
            if value == 0 {
                self.held.remove(&code);
            }
            self.write_event(EV_KEY, code, value)
        }
        fn release_all(&mut self) -> io::Result<()> {
            for code in std::mem::take(&mut self.held) {
                self.write_event(EV_KEY, code, 0)?;
            }
            Ok(())
        }
    }

    impl Drop for UinputSink {
        fn drop(&mut self) {
            let _ = self.release_all();
            unsafe {
                let _ = libc::ioctl(self.device.as_raw_fd(), UI_DEV_DESTROY);
            }
        }
    }

    unsafe fn ioctl(fd: i32, request: libc::c_ulong, argument: libc::c_ulong) -> io::Result<()> {
        if libc::ioctl(fd, request, argument) == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    fn as_bytes<T>(value: &T) -> &[u8] {
        unsafe { std::slice::from_raw_parts((value as *const T).cast(), size_of::<T>()) }
    }
    fn supported_keys() -> Vec<u16> {
        // Matches Sources/BridgeCore/KeyMapping.swift and intentionally omits
        // capabilities that the sender cannot emit.
        (1..=190)
            .filter(|code| super::is_supported_key(*code))
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::supported_keys;

        #[test]
        fn supports_every_extended_key_the_sender_can_emit() {
            let keys = supported_keys();
            for key in [83, 98, 110, 113, 114, 115, 117, 125, 126, 127, 183, 190] {
                assert!(keys.contains(&key), "missing evdev key {key}");
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod uinput {
    use super::KeySink;
    use std::io;
    pub struct UinputSink;
    impl UinputSink {
        pub fn open() -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "uinput is available only on Linux",
            ))
        }
    }
    impl KeySink for UinputSink {
        fn emit(&mut self, _: u16, _: i32) -> io::Result<()> {
            unreachable!()
        }
        fn release_all(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

fn serve(
    listener: TcpListener,
    pairing_secret: PairingSecret,
    sink: uinput::UinputSink,
    shutdown: Arc<AtomicBool>,
    sync_layout_from_mac: bool,
    layout_mappings: Vec<String>,
    clipboard_config: ((bool, bool), Arc<AtomicBool>),
) -> io::Result<()> {
    let (clipboard_directions, manual_clipboard_request) = clipboard_config;
    let (sync_clipboard_from_mac, sync_clipboard_to_mac) = clipboard_directions;
    listener.set_nonblocking(true)?;
    let layout_switcher = if sync_layout_from_mac {
        match PlasmaLayoutSwitcher::new(&layout_mappings) {
            Ok(switcher) => Some(Arc::new(switcher)),
            Err(error) => {
                eprintln!("Mac layout sync is disabled for this session: {error}");
                None
            }
        }
    } else {
        None
    };
    // Do the non-mutating session preflight before advertising clipboard
    // support. A Mac therefore never starts sending clipboard bytes when the
    // Deck is in Gaming Mode or has no active Wayland session.
    let clipboard_available = sync_clipboard_from_mac && WaylandClipboardSink::new().is_ok();
    let sink = Arc::new(Mutex::new(sink));
    let active_session = Arc::new(AtomicBool::new(false));
    let pending_handshakes = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();

    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let accepted = pending_handshakes
                    .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                        (count < MAX_PENDING_HANDSHAKES).then_some(count + 1)
                    })
                    .is_ok();
                if !accepted {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }

                let pairing_secret = pairing_secret.clone();
                let sink = Arc::clone(&sink);
                let active_session = Arc::clone(&active_session);
                let pending_handshakes = Arc::clone(&pending_handshakes);
                let shutdown = Arc::clone(&shutdown);
                let manual_clipboard_request = Arc::clone(&manual_clipboard_request);
                let layout_switcher = layout_switcher.clone();
                workers.push(thread::spawn(move || {
                    let session = complete_pending_handshake(
                        &mut stream,
                        &pairing_secret,
                        &pending_handshakes,
                    );
                    if let Ok(session) = session {
                        // A receiver has exactly one uinput keyboard. Do not confirm
                        // REMOTE for a second peer while a first peer owns it.
                        if active_session
                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                            .is_ok()
                        {
                            let ready_packet =
                                match (layout_switcher.is_some(), clipboard_available) {
                                    (false, false) => Packet::ServerReady,
                                    (true, false) => Packet::ServerReadyWithLayoutSync,
                                    (false, true) => Packet::ServerReadyWithClipboardSync,
                                    (true, true) => Packet::ServerReadyWithLayoutAndClipboardSync,
                                };
                            let mut clipboard_sink = if clipboard_available {
                                WaylandClipboardSink::new().ok()
                            } else {
                                None
                            };
                            let result = session
                                .write_packet(&mut stream, 0, &ready_packet)
                                .and_then(|()| {
                                    sink.lock().map_err(|_| BridgeError::Handshake).and_then(
                                        |mut sink| {
                                            handle_authenticated_client(
                                                &mut stream,
                                                &session,
                                                &mut *sink,
                                                &shutdown,
                                                layout_switcher.as_deref(),
                                                clipboard_sink
                                                    .as_mut()
                                                    .map(|sink| sink as &mut dyn ClipboardSink),
                                                (sync_clipboard_to_mac, &manual_clipboard_request),
                                            )
                                        },
                                    )
                                });
                            if let Ok(mut sink) = sink.lock() {
                                let _ = sink.release_all();
                            }
                            let _ = result;
                            active_session.store(false, Ordering::Release);
                        }
                    }
                    let _ = stream.shutdown(Shutdown::Both);
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                // Keep shutdown responsive without spinning the CPU fifty
                // times per second while the receiver is idle.
                thread::sleep(Duration::from_millis(200));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
        workers.retain(|worker| !worker.is_finished());
    }

    for worker in workers {
        let _ = worker.join();
    }
    if let Ok(mut sink) = sink.lock() {
        let _ = sink.release_all();
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Pair => println!("{}", PairingSecret::generate().to_base64()),
        Command::Serve {
            bind,
            pairing_secret,
            sync_layout_from_mac,
            layout_mappings,
            sync_clipboard_from_mac,
            sync_clipboard_to_mac,
            control_stdin,
        } => {
            let pairing_secret = PairingSecret::parse(&pairing_secret)?;
            let listener = TcpListener::bind(&bind)?;
            let sink = uinput::UinputSink::open()?;
            let shutdown = Arc::new(AtomicBool::new(false));
            let signal_shutdown = Arc::clone(&shutdown);
            let manual_clipboard_request = Arc::new(AtomicBool::new(false));
            if control_stdin {
                let request = Arc::clone(&manual_clipboard_request);
                thread::spawn(move || {
                    for line in io::stdin().lock().lines().map_while(Result::ok) {
                        if line == "send-clipboard" {
                            request.store(true, Ordering::Release);
                        }
                    }
                });
            }
            ctrlc::set_handler(move || signal_shutdown.store(true, Ordering::Release))?;
            eprintln!("Listening on {bind}; keyboard event contents are not logged.");
            serve(
                listener,
                pairing_secret,
                sink,
                shutdown,
                sync_layout_from_mac,
                layout_mappings,
                (
                    (sync_clipboard_from_mac, sync_clipboard_to_mac),
                    manual_clipboard_request,
                ),
            )?;
        }
        Command::LayoutStatus { layout_mappings } => {
            let switcher = PlasmaLayoutSwitcher::new(&layout_mappings).map_err(io::Error::other)?;
            println!("{}", switcher.status_report().map_err(io::Error::other)?);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mac-deck-receiver: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod receiver_tests {
    use super::*;
    use std::collections::BTreeSet;
    #[derive(Default)]
    struct TestSink {
        held: BTreeSet<u16>,
    }
    impl KeySink for TestSink {
        fn emit(&mut self, code: u16, value: i32) -> io::Result<()> {
            if value == 0 {
                self.held.remove(&code);
            } else {
                self.held.insert(code);
            };
            Ok(())
        }
        fn release_all(&mut self) -> io::Result<()> {
            self.held.clear();
            Ok(())
        }
    }

    struct TestClipboardSink {
        result: Result<(), String>,
    }

    impl ClipboardSink for TestClipboardSink {
        fn set_text(&mut self, _: &str) -> Result<(), String> {
            self.result.clone()
        }
    }

    #[test]
    fn clipboard_capability_rejects_gaming_sessions() {
        assert!(gaming_session(Some("gamescope"), None));
        assert!(gaming_session(None, Some("Gamescope")));
        // Steam shortcut IDs describe the application, not the desktop session.
        assert!(!gaming_session(Some("KDE"), Some("plasma")));
        assert!(!gaming_session(None, None));
    }

    #[test]
    fn remote_clipboard_echo_is_suppressed_without_suppressing_local_changes() {
        let mut suppression = ClipboardEchoSuppression::default();
        suppression.record_applied("remote");
        assert!(suppression.should_skip("remote", false));
        assert!(!suppression.should_skip("remote", false));
        suppression.record_applied("remote");
        assert!(!suppression.should_skip("local", false));
        assert!(!suppression.should_skip("remote", false));
        suppression.record_applied("remote");
        assert!(!suppression.should_skip("remote", true));
        assert!(suppression.should_skip("remote", false));
    }

    #[test]
    fn clipboard_transfer_requires_order_and_exact_utf8_length() {
        let mut transfer = ClipboardTransfer::begin(7, 5, 2).unwrap();
        assert!(transfer.append(7, 1, b"he").is_err());
        transfer.append(7, 0, b"hel").unwrap();
        transfer.append(7, 1, b"lo").unwrap();
        assert_eq!(transfer.finish(7).unwrap(), "hello");
    }

    #[test]
    fn clipboard_transfer_rejects_malformed_and_incomplete_data() {
        assert!(ClipboardTransfer::begin(1, (MAX_CLIPBOARD_TEXT_BYTES + 1) as u32, 1).is_err());
        let transfer = ClipboardTransfer::begin(1, 1, 1).unwrap();
        assert!(transfer.finish(1).is_err());
        let mut invalid_utf8 = ClipboardTransfer::begin(2, 1, 1).unwrap();
        invalid_utf8.append(2, 0, &[0xff]).unwrap();
        assert!(invalid_utf8.finish(2).is_err());
    }
    #[test]
    fn release_all_clears_modifier_state() {
        let mut sink = TestSink::default();
        sink.emit(29, 1).unwrap();
        sink.emit(42, 1).unwrap();
        sink.release_all().unwrap();
        assert!(sink.held.is_empty());
    }

    #[test]
    fn receiver_whitelist_rejects_unadvertised_codes() {
        assert!(is_supported_key(30));
        assert!(is_supported_key(190));
        assert!(!is_supported_key(0));
        assert!(!is_supported_key(70));
        assert!(!is_supported_key(191));
    }

    #[test]
    fn application_sequence_rejects_replay_and_reordering() {
        let mut expected = 1;
        expect_next_sequence(&mut expected, 1).unwrap();
        assert_eq!(expected, 2);
        assert!(matches!(
            expect_next_sequence(&mut expected, 1),
            Err(BridgeError::Handshake)
        ));
        assert!(matches!(
            expect_next_sequence(&mut expected, 3),
            Err(BridgeError::Handshake)
        ));
    }

    #[test]
    fn partial_session_frame_times_out_without_becoming_an_idle_read() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let shutdown = AtomicBool::new(false);
            let mut reader = SessionFrameReader {
                stream: &mut stream,
                shutdown: &shutdown,
                deadline: None,
                timeout: Duration::from_millis(200),
                bytes_read: 0,
            };
            let mut data = [0; 4];
            let error = reader.read_exact(&mut data).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert_eq!(reader.bytes_read, 1);
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(&[0]).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn session_frame_deadline_is_not_extended_by_trickled_bytes() {
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let shutdown = AtomicBool::new(false);
            let mut reader = SessionFrameReader {
                stream: &mut stream,
                shutdown: &shutdown,
                deadline: None,
                timeout: Duration::from_millis(200),
                bytes_read: 0,
            };
            let result = reader.read_exact(&mut [0; 4096]);
            done_tx.send(result.unwrap_err().kind()).unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        let (stop_tx, stop_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            while stop_rx.recv_timeout(Duration::from_millis(20)).is_err() {
                if client.write_all(&[0]).is_err() {
                    break;
                }
            }
        });
        let completed = done_rx.recv_timeout(Duration::from_secs(2));
        let _ = stop_tx.send(());
        writer.join().unwrap();
        server.join().unwrap();
        assert_eq!(completed.unwrap(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn partial_frame_failure_releases_an_held_key() {
        let pairing = PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_pairing = pairing.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let session = SessionFramer::new(server_pairing);
            let mut sink = TestSink::default();
            let shutdown = AtomicBool::new(false);
            let result = handle_authenticated_client(
                &mut stream,
                &session,
                &mut sink,
                &shutdown,
                None,
                None,
                (false, &AtomicBool::new(false)),
            );
            assert!(
                matches!(result, Err(BridgeError::Io(error)) if error.kind() == io::ErrorKind::TimedOut)
            );
            assert!(sink.held.contains(&30));
            // Match the receiver worker's disconnect cleanup.
            sink.release_all().unwrap();
            assert!(sink.held.is_empty());
        });
        let mut client = TcpStream::connect(address).unwrap();
        let session = SessionFramer::new(pairing);
        session
            .write_packet(&mut client, 1, &Packet::Key { code: 30, value: 1 })
            .unwrap();
        client.write_all(&[0]).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn idle_session_read_remains_distinct_from_a_partial_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let shutdown = AtomicBool::new(false);
            let mut reader = SessionFrameReader {
                stream: &mut stream,
                shutdown: &shutdown,
                deadline: None,
                timeout: SESSION_FRAME_TIMEOUT,
                bytes_read: 0,
            };
            let error = reader.read(&mut [0; 4]).unwrap_err();
            assert!(matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ));
            assert_eq!(reader.bytes_read, 0);
        });
        let _client = TcpStream::connect(address).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn pending_handshake_slot_released_before_session_ends_and_on_failure() {
        let pairing = PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        for should_authenticate in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let pending = Arc::new(AtomicUsize::new(1));
            let pending_server = Arc::clone(&pending);
            let server_pairing = pairing.clone();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let result =
                    complete_pending_handshake(&mut stream, &server_pairing, &pending_server);
                assert_eq!(result.is_ok(), should_authenticate);
                assert_eq!(pending_server.load(Ordering::Acquire), 0);
                // Keep an authenticated connection open after its slot is released.
                if should_authenticate {
                    let mut signal = [0];
                    stream.read_exact(&mut signal).unwrap();
                }
            });
            let mut client = TcpStream::connect(address).unwrap();
            let challenge = mac_deck_receiver::read_server_hello(&mut client).unwrap();
            let session = SessionFramer::new(derive_session_secret(&pairing, &challenge));
            let packet = if should_authenticate {
                Packet::ClientReady
            } else {
                Packet::Ping
            };
            session.write_packet(&mut client, 0, &packet).unwrap();
            if should_authenticate {
                // This byte cannot be read until the server releases its slot.
                client.write_all(&[1]).unwrap();
            }
            server.join().unwrap();
            assert_eq!(pending.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn handshake_deadline_cannot_be_extended_by_partial_data() {
        use std::sync::mpsc;
        let pairing = PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let result =
                receive_client_ready_before(&mut stream, &pairing, Duration::from_millis(200));
            done_tx.send(result.is_err()).unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        mac_deck_receiver::read_server_hello(&mut client).unwrap();
        // Advertise a large frame and repeatedly write partial payload bytes;
        // each arrives within the per-read timeout, but the total exceeds it.
        client.write_all(&4096u32.to_be_bytes()).unwrap();
        let (stop_tx, stop_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            while stop_rx.recv_timeout(Duration::from_millis(20)).is_err() {
                if client.write_all(&[0]).is_err() {
                    break;
                }
            }
        });
        let completed = done_rx.recv_timeout(Duration::from_secs(2));
        let _ = stop_tx.send(());
        writer.join().unwrap();
        server.join().unwrap();
        assert!(completed.unwrap());
    }

    #[test]
    fn handshake_requires_ready_and_rejects_replayed_application_record() {
        let pairing = PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_pairing = pairing.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut sink = TestSink::default();
            handle_client(&mut stream, &server_pairing, &mut sink)
        });

        let mut client = TcpStream::connect(address).unwrap();
        let challenge = mac_deck_receiver::read_server_hello(&mut client).unwrap();
        let session = SessionFramer::new(derive_session_secret(&pairing, &challenge));
        session
            .write_packet(&mut client, 0, &Packet::ClientReady)
            .unwrap();
        assert_eq!(
            session.read_packet(&mut client).unwrap(),
            (0, Packet::ServerReady)
        );
        session
            .write_packet(&mut client, 1, &Packet::Key { code: 30, value: 1 })
            .unwrap();
        session
            .write_packet(&mut client, 1, &Packet::Key { code: 30, value: 1 })
            .unwrap();
        assert!(matches!(
            server.join().unwrap(),
            Err(BridgeError::Handshake)
        ));
    }

    #[test]
    fn capability_probe_reports_disabled_without_disrupting_keyboard_session() {
        let pairing = PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_pairing = pairing.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut sink = TestSink::default();
            handle_client(&mut stream, &server_pairing, &mut sink)
        });
        let mut client = TcpStream::connect(address).unwrap();
        let challenge = mac_deck_receiver::read_server_hello(&mut client).unwrap();
        let session = SessionFramer::new(derive_session_secret(&pairing, &challenge));
        session
            .write_packet(&mut client, 0, &Packet::ClientReady)
            .unwrap();
        assert_eq!(
            session.read_packet(&mut client).unwrap(),
            (0, Packet::ServerReady)
        );
        session.write_packet(&mut client, 1, &Packet::Ping).unwrap();
        assert_eq!(
            session.read_packet(&mut client).unwrap(),
            (1, Packet::ClipboardCapabilities { deck_to_mac: false })
        );
        session
            .write_packet(&mut client, 2, &Packet::Key { code: 30, value: 1 })
            .unwrap();
        drop(client);
        assert!(server.join().unwrap().is_err());
    }

    #[test]
    fn clipboard_transfer_reports_a_content_free_delivery_result() {
        let pairing = PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_pairing = pairing.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let session = receive_client_ready(&mut stream, &server_pairing).unwrap();
            session
                .write_packet(&mut stream, 0, &Packet::ServerReadyWithClipboardSync)
                .unwrap();
            let shutdown = AtomicBool::new(false);
            let mut keys = TestSink::default();
            let mut clipboard = TestClipboardSink { result: Ok(()) };
            let result = handle_authenticated_client(
                &mut stream,
                &session,
                &mut keys,
                &shutdown,
                None,
                Some(&mut clipboard),
                (false, &AtomicBool::new(false)),
            );
            (result, keys)
        });

        let mut client = TcpStream::connect(address).unwrap();
        let challenge = mac_deck_receiver::read_server_hello(&mut client).unwrap();
        let session = SessionFramer::new(derive_session_secret(&pairing, &challenge));
        session
            .write_packet(&mut client, 0, &Packet::ClientReady)
            .unwrap();
        assert_eq!(
            session.read_packet(&mut client).unwrap(),
            (0, Packet::ServerReadyWithClipboardSync)
        );
        session
            .write_packet(
                &mut client,
                1,
                &Packet::ClipboardStart {
                    origin: ClipboardOrigin::Mac,
                    transfer_id: 9,
                    total_bytes: 2,
                    chunk_count: 1,
                },
            )
            .unwrap();
        session
            .write_packet(
                &mut client,
                2,
                &Packet::ClipboardChunk {
                    origin: ClipboardOrigin::Mac,
                    transfer_id: 9,
                    index: 0,
                    bytes: b"ok".to_vec(),
                },
            )
            .unwrap();
        session
            .write_packet(
                &mut client,
                3,
                &Packet::ClipboardEnd {
                    origin: ClipboardOrigin::Mac,
                    transfer_id: 9,
                },
            )
            .unwrap();
        assert_eq!(
            session.read_packet(&mut client).unwrap(),
            (
                1,
                Packet::ClipboardDelivery {
                    origin: ClipboardOrigin::Mac,
                    transfer_id: 9,
                    delivery: ClipboardDelivery::Written,
                }
            )
        );
        // Clipboard traffic and the capability probe must leave the same
        // authenticated session available for normal keyboard events.
        session
            .write_packet(&mut client, 4, &Packet::Key { code: 30, value: 1 })
            .unwrap();
        session.write_packet(&mut client, 5, &Packet::Ping).unwrap();
        assert_eq!(
            session.read_packet(&mut client).unwrap(),
            (2, Packet::ClipboardCapabilities { deck_to_mac: false })
        );
        drop(client);
        let (result, keys) = server.join().unwrap();
        assert!(result.is_err());
        assert!(keys.held.contains(&30));
    }

    #[test]
    fn automatic_layout_matching_is_generic_and_rejects_ambiguity() {
        let layouts = vec![
            ConfiguredLayout {
                index: 0,
                xkb_code: "us".to_owned(),
                description: "English (US)".to_owned(),
            },
            ConfiguredLayout {
                index: 1,
                xkb_code: "ua".to_owned(),
                description: "Ukrainian".to_owned(),
            },
            ConfiguredLayout {
                index: 2,
                xkb_code: "de".to_owned(),
                description: "German".to_owned(),
            },
        ];
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.US", &layouts),
            Some(0)
        );
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.ABC", &layouts),
            Some(0)
        );
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.Ukrainian", &layouts),
            Some(1)
        );
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.Ukrainian-PC", &layouts),
            Some(1)
        );
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.German", &layouts),
            Some(2)
        );
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.Unknown", &layouts),
            None
        );
        assert_eq!(
            automatic_layout_candidates("com.apple.keylayout.Ukrainian", &layouts),
            vec![1]
        );

        let ambiguous = vec![
            ConfiguredLayout {
                index: 0,
                xkb_code: "us".to_owned(),
                description: "English (US)".to_owned(),
            },
            ConfiguredLayout {
                index: 1,
                xkb_code: "us-intl".to_owned(),
                description: "English (US, international)".to_owned(),
            },
        ];
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.US", &ambiguous),
            None
        );
        assert_eq!(
            automatic_layout_candidates("com.apple.keylayout.US", &ambiguous),
            vec![0, 1]
        );
        assert_eq!(
            LayoutSwitchOutcome::UnchangedAmbiguous.description(),
            "multiple configured Deck layouts matched; left unchanged"
        );
    }

    #[test]
    fn plasma_runtime_layout_names_enable_automatic_matching_without_xkb_files() {
        let layouts = parse_qdbus_layout_list(
            r#"[Argument: a(sss) {[Argument: (sss) "us", "", "English (US)"], [Argument: (sss) "ua", "", "Ukrainian"]}]"#,
        );
        assert_eq!(
            layouts,
            vec![
                ConfiguredLayout {
                    index: 0,
                    xkb_code: "us".to_owned(),
                    description: "English (US)".to_owned(),
                },
                ConfiguredLayout {
                    index: 1,
                    xkb_code: "ua".to_owned(),
                    description: "Ukrainian".to_owned(),
                },
            ]
        );
        assert_eq!(
            automatic_layout_index("com.apple.keylayout.Ukrainian", &layouts),
            Some(1)
        );
    }

    #[test]
    fn xkb_rules_fall_back_to_the_next_usable_file_and_directory() {
        let directory = std::env::temp_dir().join(format!(
            "macdeck-xkb-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let missing = directory.join("missing");
        let custom = directory.join("custom");
        let fallback = directory.join("fallback");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::create_dir_all(&fallback).unwrap();
        std::fs::write(custom.join("base.lst"), "! model\n  pc105 Generic\n").unwrap();
        std::fs::write(
            custom.join("evdev.lst"),
            "! layout\n  ua Ukrainian\n! variant\n  custom Ignore\n",
        )
        .unwrap();
        std::fs::write(fallback.join("base.lst"), "! layout\n  us English (US)\n").unwrap();
        assert_eq!(
            read_xkb_layout_descriptions_from(&[missing.clone(), custom.clone(), fallback.clone()]),
            BTreeMap::from([("ua".to_owned(), "Ukrainian".to_owned())]),
        );
        assert_eq!(
            read_xkb_layout_descriptions_from(&[missing.clone(), fallback]),
            BTreeMap::from([("us".to_owned(), "English (US)".to_owned())]),
        );
        assert!(read_xkb_layout_descriptions_from(&[missing]).is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn qdbus_discovery_skips_non_executable_files_and_reports_no_candidate() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "macdeck-qdbus-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let blocked = directory.join("qdbus6");
        let usable = directory.join("qdbus");
        std::fs::write(&blocked, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(&usable, b"#!/bin/sh\nprintf 'true\\n'\n").unwrap();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&usable, std::fs::Permissions::from_mode(0o755)).unwrap();

        let selected = select_qdbus([blocked.clone(), usable.clone()]).unwrap();
        assert_eq!(selected, usable);
        let output = qdbus_call(&selected, &[]).unwrap();
        assert!(output.status.success());
        assert!(qdbus_boolean_reply_accepted(&output.stdout));
        assert_eq!(select_qdbus([directory.clone(), blocked]), None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn qdbus_drains_output_larger_than_a_pipe_buffer() {
        let output = qdbus_call(
            Path::new("/bin/sh"),
            &[
                "-c",
                "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2",
            ],
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 131072);
        assert_eq!(output.stderr.len(), 131072);
    }

    #[test]
    fn qdbus_rejects_excess_output_and_reaps_timed_out_child() {
        let excess = qdbus_call(Path::new("/bin/sh"), &["-c", "head -c 1100000 /dev/zero"]);
        assert_eq!(excess.unwrap_err().kind(), io::ErrorKind::InvalidData);
        let timeout = qdbus_call(Path::new("/bin/sh"), &["-c", "exec sleep 2"]);
        assert_eq!(timeout.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn plasma_false_reply_is_treated_as_a_rejected_switch() {
        assert!(!qdbus_boolean_reply_accepted(b"false\n"));
        assert!(!qdbus_boolean_reply_accepted(b"0"));
        assert!(qdbus_boolean_reply_accepted(b"true\n"));
    }

    #[test]
    fn plasma_layout_switch_uses_the_keyboard_layouts_dbus_service() {
        assert_eq!(PLASMA_KEYBOARD_LAYOUT_SERVICE, "org.kde.keyboard");
        assert_eq!(PLASMA_KEYBOARD_LAYOUT_PATH, "/Layouts");
        assert_eq!(
            PLASMA_KEYBOARD_LAYOUT_METHOD,
            "org.kde.KeyboardLayouts.setLayout"
        );
    }
}
