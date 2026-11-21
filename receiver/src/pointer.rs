//! Optional uinput mouse. Device creation is attempted only after authentication.
use std::io;

#[cfg(target_os = "linux")]
mod platform {
    use super::io;
    use std::{
        collections::BTreeSet,
        fs::{File, OpenOptions},
        io::Write,
        mem::size_of,
        os::fd::AsRawFd,
    };

    const EV_SYN: u16 = 0;
    const EV_KEY: u16 = 1;
    const EV_REL: u16 = 2;
    const REL_X: u16 = 0;
    const REL_Y: u16 = 1;
    const REL_HWHEEL: u16 = 6;
    const REL_WHEEL: u16 = 8;
    const BTN_LEFT: u16 = 0x110;
    const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
    const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
    const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;
    const UI_DEV_CREATE: libc::c_ulong = 0x5501;
    const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

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

    pub struct PointerDevice {
        device: File,
        held: BTreeSet<u8>,
    }
    impl PointerDevice {
        pub fn open() -> io::Result<Self> {
            let mut device = OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/uinput")?;
            unsafe {
                ioctl(&device, UI_SET_EVBIT, EV_KEY)?;
                ioctl(&device, UI_SET_EVBIT, EV_REL)?;
                ioctl(&device, UI_SET_EVBIT, EV_SYN)?;
                for code in [BTN_LEFT, BTN_LEFT + 1] {
                    ioctl(&device, UI_SET_KEYBIT, code)?;
                }
                for code in [REL_X, REL_Y, REL_HWHEEL, REL_WHEEL] {
                    ioctl(&device, UI_SET_RELBIT, code)?;
                }
            }
            let mut setup: UinputUserDev = unsafe { std::mem::zeroed() };
            let name = b"MacDeck virtual mouse";
            setup.name[..name.len()].copy_from_slice(name);
            setup.id = InputId {
                bustype: 3,
                vendor: 0x1d6b,
                product: 0x0105,
                version: 1,
            };
            device.write_all(as_bytes(&setup))?;
            unsafe {
                ioctl(&device, UI_DEV_CREATE, 0)?;
            }
            Ok(Self {
                device,
                held: BTreeSet::new(),
            })
        }
        fn event(&mut self, type_: u16, code: u16, value: i32) -> io::Result<()> {
            let event = InputEvent {
                time: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_,
                code,
                value,
            };
            self.device.write_all(as_bytes(&event))
        }
        fn sync(&mut self) -> io::Result<()> {
            self.event(EV_SYN, 0, 0)
        }
        pub fn motion(&mut self, x: i16, y: i16) -> io::Result<()> {
            if x != 0 {
                self.event(EV_REL, REL_X, x.into())?;
            }
            if y != 0 {
                self.event(EV_REL, REL_Y, y.into())?;
            }
            self.sync()
        }
        pub fn scroll(&mut self, horizontal: i16, vertical: i16) -> io::Result<()> {
            if horizontal != 0 {
                self.event(EV_REL, REL_HWHEEL, horizontal.into())?;
            }
            if vertical != 0 {
                self.event(EV_REL, REL_WHEEL, vertical.into())?;
            }
            self.sync()
        }
        pub fn button(&mut self, button: u8, pressed: bool) -> io::Result<()> {
            if button > 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported mouse button",
                ));
            }
            if pressed {
                self.held.insert(button);
            } else {
                self.held.remove(&button);
            }
            self.event(EV_KEY, BTN_LEFT + u16::from(button), i32::from(pressed))?;
            self.sync()
        }
        pub fn release_all(&mut self) -> io::Result<()> {
            for button in std::mem::take(&mut self.held) {
                self.event(EV_KEY, BTN_LEFT + u16::from(button), 0)?;
            }
            self.sync()
        }
    }
    impl Drop for PointerDevice {
        fn drop(&mut self) {
            let _ = self.release_all();
            unsafe {
                let _ = libc::ioctl(self.device.as_raw_fd(), UI_DEV_DESTROY);
            }
        }
    }
    unsafe fn ioctl(file: &File, request: libc::c_ulong, argument: u16) -> io::Result<()> {
        if libc::ioctl(file.as_raw_fd(), request, argument as libc::c_ulong) == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    fn as_bytes<T>(value: &T) -> &[u8] {
        unsafe { std::slice::from_raw_parts((value as *const T).cast(), size_of::<T>()) }
    }
}
#[cfg(not(target_os = "linux"))]
mod platform {
    use super::io;
    pub struct PointerDevice;
    impl PointerDevice {
        pub fn open() -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "uinput is available only on Linux",
            ))
        }
        pub fn motion(&mut self, _: i16, _: i16) -> io::Result<()> {
            unreachable!()
        }
        pub fn scroll(&mut self, _: i16, _: i16) -> io::Result<()> {
            unreachable!()
        }
        pub fn button(&mut self, _: u8, _: bool) -> io::Result<()> {
            unreachable!()
        }
        pub fn release_all(&mut self) -> io::Result<()> {
            unreachable!()
        }
    }
}
pub use platform::PointerDevice;
