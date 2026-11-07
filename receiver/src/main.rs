use clap::{ArgAction, Parser, Subcommand};
use mac_deck_receiver::{
    derive_session_secret, generate_challenge, write_server_hello, BridgeError, Packet,
    PairingSecret, SessionFramer,
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    net::{Shutdown, TcpListener, TcpStream},
    path::PathBuf,
    process::{Command as ProcessCommand, ExitCode},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(200);
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
    },
}

trait KeySink {
    fn emit(&mut self, code: u16, value: i32) -> io::Result<()>;
    fn release_all(&mut self) -> io::Result<()>;
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
    handle_authenticated_client(stream, &session, sink, &shutdown, None)
}

fn receive_client_ready(
    stream: &mut TcpStream,
    pairing_secret: &PairingSecret,
) -> Result<SessionFramer, BridgeError> {
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let challenge = generate_challenge();
    write_server_hello(stream, &challenge)?;
    let session = SessionFramer::new(derive_session_secret(pairing_secret, &challenge));
    let (sequence, packet) = session.read_packet(stream)?;
    if sequence != 0 || packet != Packet::ClientReady {
        return Err(BridgeError::Handshake);
    }
    Ok(session)
}

fn handle_authenticated_client(
    stream: &mut TcpStream,
    session: &SessionFramer,
    sink: &mut impl KeySink,
    shutdown: &AtomicBool,
    layout_switcher: Option<&PlasmaLayoutSwitcher>,
) -> Result<(), BridgeError> {
    // A bounded timeout makes shutdown deterministic without treating an idle
    // keyboard as a failed session. If a partial frame spans the timeout, the
    // connection is rejected safely by framing/authentication on its next read.
    stream.set_read_timeout(Some(SESSION_POLL_INTERVAL))?;
    stream.set_write_timeout(None)?;
    let mut next_sequence = 1u64;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        let (sequence, packet) = match session.read_packet(stream) {
            Ok(packet) => packet,
            Err(BridgeError::Io(error))
                if matches!(
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
                    if let Err(error) = layout_switcher.switch_to(&identifier) {
                        eprintln!("Could not synchronize the Deck keyboard layout: {error}");
                    }
                }
            }
            Packet::Key { .. }
            | Packet::ClientReady
            | Packet::ServerReady
            | Packet::ServerReadyWithLayoutSync => return Err(BridgeError::InvalidPacket),
            Packet::ReleaseAll => sink.release_all()?,
            Packet::Ping => {}
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

const PLASMA_KEYBOARD_LAYOUT_SERVICE: &str = "org.kde.keyboard";
const PLASMA_KEYBOARD_LAYOUT_PATH: &str = "/Layouts";
const PLASMA_KEYBOARD_LAYOUT_METHOD: &str = "org.kde.KeyboardLayouts.setLayout";

impl PlasmaLayoutSwitcher {
    fn new(mappings: &[String]) -> Result<Self, String> {
        let qdbus = ["/usr/bin/qdbus6", "/usr/bin/qdbus"]
            .iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .ok_or_else(|| "qdbus was not found in this Plasma session".to_owned())?;
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
        let automatic_layouts = configured_plasma_layouts();
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

    fn switch_to(&self, mac_input_source: &str) -> io::Result<()> {
        let Some(index) = self
            .mappings
            .get(mac_input_source)
            .copied()
            .or_else(|| automatic_layout_index(mac_input_source, &self.automatic_layouts))
        else {
            // An unmapped layout is not an error: ordinary physical key
            // forwarding still works using the Deck's current layout.
            return Ok(());
        };
        let index = index.to_string();
        let status = ProcessCommand::new(&self.qdbus)
            .args([
                PLASMA_KEYBOARD_LAYOUT_SERVICE,
                PLASMA_KEYBOARD_LAYOUT_PATH,
                PLASMA_KEYBOARD_LAYOUT_METHOD,
                &index,
            ])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("Plasma rejected the layout switch"))
        }
    }
}

fn configured_plasma_layouts() -> Vec<ConfiguredLayout> {
    let Some(layout_list) = read_kxkbrc_layout_list() else {
        return Vec::new();
    };
    let descriptions = read_xkb_layout_descriptions();
    layout_list
        .into_iter()
        .enumerate()
        .filter_map(|(index, code)| {
            let index = u8::try_from(index).ok()?;
            let description = descriptions.get(&code)?.clone();
            Some(ConfiguredLayout {
                index,
                xkb_code: code,
                description,
            })
        })
        .collect()
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
    let rules_files = [
        "/usr/share/X11/xkb/rules/base.lst",
        "/usr/share/X11/xkb/rules/evdev.lst",
    ];
    for rules_file in rules_files {
        let Ok(contents) = std::fs::read_to_string(rules_file) else {
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
    BTreeMap::new()
}

fn automatic_layout_index(
    mac_input_source: &str,
    configured_layouts: &[ConfiguredLayout],
) -> Option<u8> {
    let source_name = mac_input_source.rsplit('.').next()?;
    let normalized_source = normalize_layout_name(source_name);
    if normalized_source.is_empty() {
        return None;
    }
    let candidates: Vec<_> = configured_layouts
        .iter()
        .filter(|layout| {
            normalize_layout_name(&layout.xkb_code) == normalized_source
                || normalize_layout_name(&layout.description) == normalized_source
                || layout
                    .description
                    .split(|character: char| !character.is_alphanumeric())
                    .any(|word| normalize_layout_name(word) == normalized_source)
        })
        .map(|layout| layout.index)
        .collect();
    if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    }
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
) -> io::Result<()> {
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
                let layout_switcher = layout_switcher.clone();
                workers.push(thread::spawn(move || {
                    let session = receive_client_ready(&mut stream, &pairing_secret);
                    if let Ok(session) = session {
                        // A receiver has exactly one uinput keyboard. Do not confirm
                        // REMOTE for a second peer while a first peer owns it.
                        if active_session
                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                            .is_ok()
                        {
                            let ready_packet = if layout_switcher.is_some() {
                                Packet::ServerReadyWithLayoutSync
                            } else {
                                Packet::ServerReady
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
                    pending_handshakes.fetch_sub(1, Ordering::AcqRel);
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
        } => {
            let pairing_secret = PairingSecret::parse(&pairing_secret)?;
            let listener = TcpListener::bind(&bind)?;
            let sink = uinput::UinputSink::open()?;
            let shutdown = Arc::new(AtomicBool::new(false));
            let signal_shutdown = Arc::clone(&shutdown);
            ctrlc::set_handler(move || signal_shutdown.store(true, Ordering::Release))?;
            eprintln!("Listening on {bind}; keyboard event contents are not logged.");
            serve(
                listener,
                pairing_secret,
                sink,
                shutdown,
                sync_layout_from_mac,
                layout_mappings,
            )?;
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
            automatic_layout_index("com.apple.keylayout.Ukrainian", &layouts),
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
