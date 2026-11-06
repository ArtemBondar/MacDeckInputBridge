use clap::{Parser, Subcommand};
use mac_deck_receiver::{BridgeError, Packet, PairingSecret, SecureFramer};
use std::{collections::BTreeSet, io, net::{Shutdown, TcpListener, TcpStream}, process::ExitCode};

#[derive(Parser)]
#[command(about = "Encrypted Mac keyboard receiver for SteamOS Desktop Mode")]
struct Cli {
    #[command(subcommand)] command: Command,
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
    },
}

trait KeySink {
    fn emit(&mut self, code: u16, value: i32) -> io::Result<()>;
    fn release_all(&mut self) -> io::Result<()>;
}

fn handle_client(stream: &mut TcpStream, framer: &SecureFramer, sink: &mut impl KeySink) -> Result<(), BridgeError> {
    loop {
        match framer.read_packet(stream) {
            Ok(Packet::Key { code, value }) => sink.emit(code, value as i32)?,
            Ok(Packet::ReleaseAll) => sink.release_all()?,
            Ok(Packet::Ping) => {}
            // The peer may vanish mid-frame. The caller always releases held keys.
            Err(error) => return Err(error),
        }
    }
}

#[cfg(target_os = "linux")]
mod uinput {
    use super::KeySink;
    use std::{collections::BTreeSet, fs::OpenOptions, io::{self, Write}, mem::size_of, os::fd::AsRawFd};

    const EV_SYN: u16 = 0;
    const EV_KEY: u16 = 1;
    const SYN_REPORT: u16 = 0;
    const UI_DEV_CREATE: libc::c_ulong = 0x5501;
    const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
    const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
    const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
    const BUS_USB: u16 = 0x03;

    #[repr(C)]
    struct InputId { bustype: u16, vendor: u16, product: u16, version: u16 }
    #[repr(C)]
    struct UinputUserDev {
        name: [u8; 80],
        id: InputId,
        ff_effects_max: u32,
        absmax: [i32; 64], absmin: [i32; 64], absfuzz: [i32; 64], absflat: [i32; 64],
    }
    #[repr(C)]
    struct InputEvent { time: libc::timeval, type_: u16, code: u16, value: i32 }

    pub struct UinputSink { device: std::fs::File, held: BTreeSet<u16> }

    impl UinputSink {
        pub fn open() -> io::Result<Self> {
            let mut device = OpenOptions::new().read(true).write(true).open("/dev/uinput")?;
            unsafe {
                ioctl(device.as_raw_fd(), UI_SET_EVBIT, EV_KEY as libc::c_ulong)?;
                ioctl(device.as_raw_fd(), UI_SET_EVBIT, EV_SYN as libc::c_ulong)?;
                // Sender maps only ordinary keyboard, modifier, navigation and function keys.
                for key in supported_keys() { ioctl(device.as_raw_fd(), UI_SET_KEYBIT, key as libc::c_ulong)?; }
            }
            let mut setup: UinputUserDev = unsafe { std::mem::zeroed() };
            setup.name[..20].copy_from_slice(b"Mac Deck Input Bridge");
            setup.id = InputId { bustype: BUS_USB, vendor: 0x1d6b, product: 0x0104, version: 1 };
            device.write_all(as_bytes(&setup))?;
            unsafe { ioctl(device.as_raw_fd(), UI_DEV_CREATE, 0)?; }
            Ok(Self { device, held: BTreeSet::new() })
        }

        fn write_event(&mut self, type_: u16, code: u16, value: i32) -> io::Result<()> {
            let event = InputEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, type_, code, value };
            self.device.write_all(as_bytes(&event))?;
            self.device.write_all(as_bytes(&InputEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, type_: EV_SYN, code: SYN_REPORT, value: 0 }))?;
            self.device.flush()
        }
    }

    impl KeySink for UinputSink {
        fn emit(&mut self, code: u16, value: i32) -> io::Result<()> {
            if value == 1 { self.held.insert(code); }
            if value == 0 { self.held.remove(&code); }
            self.write_event(EV_KEY, code, value)
        }
        fn release_all(&mut self) -> io::Result<()> {
            for code in std::mem::take(&mut self.held) { self.write_event(EV_KEY, code, 0)?; }
            Ok(())
        }
    }

    impl Drop for UinputSink {
        fn drop(&mut self) { let _ = self.release_all(); unsafe { let _ = libc::ioctl(self.device.as_raw_fd(), UI_DEV_DESTROY); } }
    }

    unsafe fn ioctl(fd: i32, request: libc::c_ulong, argument: libc::c_ulong) -> io::Result<()> {
        if libc::ioctl(fd, request, argument) == -1 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
    fn as_bytes<T>(value: &T) -> &[u8] { unsafe { std::slice::from_raw_parts((value as *const T).cast(), size_of::<T>()) } }
    fn supported_keys() -> Vec<u16> {
        // Matches Sources/BridgeCore/KeyMapping.swift, avoiding uinput capabilities the bridge cannot emit.
        (1..=111).filter(|key| !matches!(key, 94 | 95 | 98 | 99 | 101 | 109)).collect()
    }
}

#[cfg(not(target_os = "linux"))]
mod uinput {
    use super::KeySink;
    use std::io;
    pub struct UinputSink;
    impl UinputSink { pub fn open() -> io::Result<Self> { Err(io::Error::new(io::ErrorKind::Unsupported, "uinput is available only on Linux")) } }
    impl KeySink for UinputSink {
        fn emit(&mut self, _: u16, _: i32) -> io::Result<()> { unreachable!() }
        fn release_all(&mut self) -> io::Result<()> { Ok(()) }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Pair => println!("{}", PairingSecret::generate().to_base64()),
        Command::Serve { bind, pairing_secret } => {
            let framer = SecureFramer::new(PairingSecret::parse(&pairing_secret)?);
            let listener = TcpListener::bind(&bind)?;
            let mut sink = uinput::UinputSink::open()?;
            eprintln!("Listening on {bind}; keyboard event contents are not logged.");
            for connection in listener.incoming() {
                let mut stream = connection?;
                let _ = handle_client(&mut stream, &framer, &mut sink);
                let _ = sink.release_all();
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() { Ok(()) => ExitCode::SUCCESS, Err(error) => { eprintln!("mac-deck-receiver: {error}"); ExitCode::FAILURE } }
}

#[cfg(test)]
mod receiver_tests {
    use super::*;
    #[derive(Default)] struct TestSink { held: BTreeSet<u16> }
    impl KeySink for TestSink {
        fn emit(&mut self, code: u16, value: i32) -> io::Result<()> { if value == 0 { self.held.remove(&code); } else { self.held.insert(code); }; Ok(()) }
        fn release_all(&mut self) -> io::Result<()> { self.held.clear(); Ok(()) }
    }
    #[test]
    fn release_all_clears_modifier_state() {
        let mut sink = TestSink::default();
        sink.emit(29, 1).unwrap(); sink.emit(42, 1).unwrap(); sink.release_all().unwrap();
        assert!(sink.held.is_empty());
    }
}
