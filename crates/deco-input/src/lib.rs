//! Three vendor-compatible uinput devices: XP-Pen Pen, Mouse, Eraser (spec §6).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::thread;
use std::time::Duration;

use thiserror::Error;

pub const PEN_NAME: &str = "XP-Pen Pen";
pub const MOUSE_NAME: &str = "XP-Pen Mouse";
pub const ERASER_NAME: &str = "XP-Pen Eraser";
pub const KEYBOARD_NAME: &str = "XP-Pen Keyboard";

pub const VENDOR: u16 = 0x28BD;
pub const PEN_PRODUCT: u16 = 2;
pub const MOUSE_PRODUCT: u16 = 0x22;
pub const KEYBOARD_PRODUCT: u16 = 0x23;
pub const VERSION: u16 = 1;
pub const BUS_USB: u16 = 3;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;

const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_WHEEL: u16 = 8;
const ABS_PRESSURE: u16 = 0x18;
const ABS_TILT_X: u16 = 0x1B;
const ABS_TILT_Y: u16 = 0x1C;

const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_WHEEL: u16 = 8;
const REL_WHEEL_HI_RES: u16 = 11;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
pub const BTN_TOOL_PEN: u16 = 0x140;
pub const BTN_TOOL_RUBBER: u16 = 0x141;
pub const BTN_TOOL_BRUSH: u16 = 0x142;
pub const BTN_STYLUS: u16 = 0x14A;
pub const BTN_STYLUS2: u16 = 0x14B;
pub const BTN_STYLUS3: u16 = 0x14C;

pub const KEY_ESC: u16 = 1;
pub const KEY_MINUS: u16 = 12;
pub const KEY_EQUAL: u16 = 13;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_Z: u16 = 44;
pub const KEY_X: u16 = 45;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_B: u16 = 48;
pub const KEY_S: u16 = 31;
pub const KEY_Y: u16 = 21;
pub const KEY_E: u16 = 18;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_SPACE: u16 = 57;
pub const KEY_LEFTBRACE: u16 = 26;
pub const KEY_RIGHTBRACE: u16 = 27;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_UP: u16 = 103;
pub const KEY_A: u16 = 30;
pub const KEY_TAB: u16 = 15;
pub const KEY_ENTER: u16 = 28;
pub const KEY_DELETE: u16 = 111;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_KPMINUS: u16 = 74;
pub const KEY_KPPLUS: u16 = 78;

const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405C_5503;
const UI_ABS_SETUP: libc::c_ulong = 0x401C_5504;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;
const UI_SET_ABSBIT: libc::c_ulong = 0x4004_5567;
const UI_SET_PROPBIT: libc::c_ulong = 0x4004_556E;
const INPUT_PROP_DIRECT: libc::c_int = 1;
const INPUT_PROP_POINTER: libc::c_int = 0;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("open {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("uinput ioctl failed: {0}")]
    Ioctl(io::Error),
    #[error("uinput write failed: {0}")]
    Write(#[from] io::Error),
}

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct InputAbsinfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    _pad: u16,
    absinfo: InputAbsinfo,
}

#[repr(C)]
struct InputEvent {
    sec: u64,
    usec: u64,
    type_: u16,
    code: u16,
    value: i32,
}

fn ioctl_int(fd: i32, req: libc::c_ulong, val: libc::c_int) -> Result<(), InputError> {
    let rc = unsafe { libc::ioctl(fd, req, val) };
    if rc < 0 {
        Err(InputError::Ioctl(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn ioctl_ptr<T>(fd: i32, req: libc::c_ulong, ptr: *const T) -> Result<(), InputError> {
    let rc = unsafe { libc::ioctl(fd, req, ptr) };
    if rc < 0 {
        Err(InputError::Ioctl(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn open_uinput() -> Result<File, InputError> {
    let candidates = ["/dev/uinput", "/dev/input/uinput"];
    let mut last = None;
    for path in candidates {
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(f) => return Ok(f),
            Err(e) => last = Some((path, e)),
        }
    }
    let (path, source) = last.unwrap();
    Err(InputError::Open {
        path: path.to_string(),
        source,
    })
}

fn name_bytes(name: &str) -> [u8; 80] {
    let mut buf = [0u8; 80];
    let raw = name.as_bytes();
    let n = raw.len().min(79);
    buf[..n].copy_from_slice(&raw[..n]);
    buf
}

fn setup_abs(fd: i32, code: u16, min: i32, max: i32, resolution: i32) -> Result<(), InputError> {
    ioctl_int(fd, UI_SET_ABSBIT, code as libc::c_int)?;
    let setup = UinputAbsSetup {
        code,
        _pad: 0,
        absinfo: InputAbsinfo {
            value: 0,
            minimum: min,
            maximum: max,
            fuzz: 0,
            flat: 0,
            resolution,
        },
    };
    ioctl_ptr(fd, UI_ABS_SETUP, &setup)
}

fn finish(fd: i32, name: &str, product: u16) -> Result<(), InputError> {
    let setup = UinputSetup {
        id: InputId {
            bustype: BUS_USB,
            vendor: VENDOR,
            product,
            version: VERSION,
        },
        name: name_bytes(name),
        ff_effects_max: 0,
    };
    ioctl_ptr(fd, UI_DEV_SETUP, &setup)?;
    ioctl_int(fd, UI_DEV_CREATE, 0)
}

fn emit(file: &mut File, type_: u16, code: u16, value: i32) -> io::Result<()> {
    let ev = InputEvent {
        sec: 0,
        usec: 0,
        type_,
        code,
        value,
    };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&ev as *const InputEvent).cast::<u8>(),
            std::mem::size_of::<InputEvent>(),
        )
    };
    file.write_all(bytes)
}

fn syn(file: &mut File) -> io::Result<()> {
    emit(file, EV_SYN, SYN_REPORT, 0)
}

pub struct AbsDevice {
    file: File,
}

impl AbsDevice {
    fn create_pen(max_x: i32, max_y: i32, max_pressure: i32) -> Result<Self, InputError> {
        let file = open_uinput()?;
        let fd = file.as_raw_fd();
        ioctl_int(fd, UI_SET_EVBIT, EV_SYN as libc::c_int)?;
        ioctl_int(fd, UI_SET_EVBIT, EV_KEY as libc::c_int)?;
        ioctl_int(fd, UI_SET_EVBIT, EV_ABS as libc::c_int)?;
        ioctl_int(fd, UI_SET_PROPBIT, INPUT_PROP_DIRECT)?;
        for key in [
            BTN_TOOL_PEN,
            BTN_TOOL_RUBBER,
            BTN_TOOL_BRUSH,
            BTN_STYLUS,
            BTN_STYLUS2,
            BTN_LEFT,
            BTN_RIGHT,
            BTN_MIDDLE,
            0xB1,
            0xB2,
        ] {
            ioctl_int(fd, UI_SET_KEYBIT, key as libc::c_int)?;
        }
        // libinput ignores tablets with ABS_X/Y resolution 0 ("missing tablet
        // capabilities: resolution"). 200 units/mm matches the vendor uinput spec
        // (5080 LPI).
        setup_abs(fd, ABS_X, 0, max_x, 200)?;
        setup_abs(fd, ABS_Y, 0, max_y, 200)?;
        setup_abs(fd, ABS_PRESSURE, 0, max_pressure, 0)?;
        setup_abs(fd, ABS_WHEEL, 0, 0x47, 0)?;
        setup_abs(fd, ABS_TILT_X, -63, 64, 0)?;
        setup_abs(fd, ABS_TILT_Y, -63, 64, 0)?;
        finish(fd, PEN_NAME, PEN_PRODUCT)?;
        Ok(Self { file })
    }

    fn create_eraser(max_x: i32, max_y: i32, max_pressure: i32) -> Result<Self, InputError> {
        let file = open_uinput()?;
        let fd = file.as_raw_fd();
        ioctl_int(fd, UI_SET_EVBIT, EV_SYN as libc::c_int)?;
        ioctl_int(fd, UI_SET_EVBIT, EV_KEY as libc::c_int)?;
        ioctl_int(fd, UI_SET_EVBIT, EV_ABS as libc::c_int)?;
        ioctl_int(fd, UI_SET_PROPBIT, INPUT_PROP_DIRECT)?;
        for key in [BTN_TOOL_RUBBER, BTN_STYLUS, BTN_STYLUS2] {
            ioctl_int(fd, UI_SET_KEYBIT, key as libc::c_int)?;
        }
        setup_abs(fd, ABS_X, 0, max_x, 200)?;
        setup_abs(fd, ABS_Y, 0, max_y, 200)?;
        setup_abs(fd, ABS_PRESSURE, 0, max_pressure, 0)?;
        setup_abs(fd, ABS_WHEEL, 0, 0x47, 0)?;
        setup_abs(fd, ABS_TILT_X, -63, 64, 0)?;
        setup_abs(fd, ABS_TILT_Y, -63, 64, 0)?;
        finish(fd, ERASER_NAME, PEN_PRODUCT)?;
        Ok(Self { file })
    }

    pub fn update(
        &mut self,
        x: i32,
        y: i32,
        pressure: i32,
        tilt_x: i32,
        tilt_y: i32,
        tool: u16,
        tool_on: bool,
        stylus: bool,
        stylus2: bool,
    ) -> io::Result<()> {
        emit(&mut self.file, EV_ABS, ABS_X, x)?;
        emit(&mut self.file, EV_ABS, ABS_Y, y)?;
        emit(&mut self.file, EV_ABS, ABS_PRESSURE, pressure)?;
        emit(&mut self.file, EV_ABS, ABS_TILT_X, tilt_x.clamp(-63, 64))?;
        emit(&mut self.file, EV_ABS, ABS_TILT_Y, tilt_y.clamp(-63, 64))?;
        emit(&mut self.file, EV_KEY, tool, i32::from(tool_on))?;
        emit(&mut self.file, EV_KEY, BTN_STYLUS, i32::from(stylus))?;
        emit(&mut self.file, EV_KEY, BTN_STYLUS2, i32::from(stylus2))?;
        syn(&mut self.file)
    }
}

impl Drop for AbsDevice {
    fn drop(&mut self) {
        let _ = ioctl_int(self.file.as_raw_fd(), UI_DEV_DESTROY, 0);
    }
}

pub struct RelDevice {
    file: File,
}

impl RelDevice {
    fn create_mouse() -> Result<Self, InputError> {
        let file = open_uinput()?;
        let fd = file.as_raw_fd();
        ioctl_int(fd, UI_SET_EVBIT, EV_SYN as libc::c_int)?;
        ioctl_int(fd, UI_SET_EVBIT, EV_KEY as libc::c_int)?;
        ioctl_int(fd, UI_SET_EVBIT, EV_REL as libc::c_int)?;
        ioctl_int(fd, UI_SET_PROPBIT, INPUT_PROP_POINTER)?;
        // Keyboard bits here make libinput treat the device as a keyboard and
        // drop REL_WHEEL. Express-key KEY_* go through KeyDevice.
        for key in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE] {
            ioctl_int(fd, UI_SET_KEYBIT, key as libc::c_int)?;
        }
        ioctl_int(fd, UI_SET_RELBIT, REL_X as libc::c_int)?;
        ioctl_int(fd, UI_SET_RELBIT, REL_Y as libc::c_int)?;
        ioctl_int(fd, UI_SET_RELBIT, REL_WHEEL as libc::c_int)?;
        ioctl_int(fd, UI_SET_RELBIT, REL_WHEEL_HI_RES as libc::c_int)?;
        finish(fd, MOUSE_NAME, MOUSE_PRODUCT)?;
        Ok(Self { file })
    }

    pub fn rel(&mut self, dx: i32, dy: i32) -> io::Result<()> {
        emit(&mut self.file, EV_REL, REL_X, dx)?;
        emit(&mut self.file, EV_REL, REL_Y, dy)?;
        syn(&mut self.file)
    }

    pub fn wheel(&mut self, delta: i32) -> io::Result<()> {
        emit(&mut self.file, EV_REL, REL_WHEEL, delta)?;
        emit(&mut self.file, EV_REL, REL_WHEEL_HI_RES, delta.saturating_mul(120))?;
        syn(&mut self.file)
    }

    /// Browser zoom: Ctrl + keypad +/- (layout-independent). Wheel-down is dropped
    /// by some GNOME/Chrome seats; keypad minus is the vendor shortcut (KEY 0x4E/0x4A).
    pub fn key(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        emit(&mut self.file, EV_KEY, code, i32::from(pressed))?;
        syn(&mut self.file)?;
        thread::sleep(Duration::from_micros(100));
        Ok(())
    }

    pub fn mouse_button(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        self.key(code, pressed)
    }
}

/// Keyboard-only uinput so GNOME/libinput actually delivers express-key shortcuts.
/// Injecting KEY_* on the Mouse device is ignored on Wayland.
pub struct KeyDevice {
    file: File,
}

impl KeyDevice {
    fn create() -> Result<Self, InputError> {
        let file = open_uinput()?;
        let fd = file.as_raw_fd();
        ioctl_int(fd, UI_SET_EVBIT, EV_SYN as libc::c_int)?;
        ioctl_int(fd, UI_SET_EVBIT, EV_KEY as libc::c_int)?;
        for code in 1..=0x100 {
            ioctl_int(fd, UI_SET_KEYBIT, code)?;
        }
        finish(fd, KEYBOARD_NAME, KEYBOARD_PRODUCT)?;
        Ok(Self { file })
    }

    pub fn key(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        emit(&mut self.file, EV_KEY, code, i32::from(pressed))?;
        syn(&mut self.file)?;
        thread::sleep(Duration::from_micros(100));
        Ok(())
    }

    pub fn tap(&mut self, code: u16) -> io::Result<()> {
        self.key(code, true)?;
        thread::sleep(Duration::from_millis(8));
        self.key(code, false)
    }

    pub fn combo(&mut self, codes: &[u16]) -> io::Result<()> {
        for c in codes {
            self.key(*c, true)?;
            thread::sleep(Duration::from_millis(8));
        }
        thread::sleep(Duration::from_millis(16));
        for c in codes.iter().rev() {
            self.key(*c, false)?;
            thread::sleep(Duration::from_millis(8));
        }
        Ok(())
    }

    pub fn zoom(&mut self, delta: i32) -> io::Result<()> {
        let code = if delta > 0 { KEY_KPPLUS } else { KEY_KPMINUS };
        self.key(KEY_LEFTCTRL, true)?;
        thread::sleep(Duration::from_millis(8));
        self.tap(code)?;
        self.key(KEY_LEFTCTRL, false)
    }
}

impl Drop for KeyDevice {
    fn drop(&mut self) {
        let _ = ioctl_int(self.file.as_raw_fd(), UI_DEV_DESTROY, 0);
    }
}

impl Drop for RelDevice {
    fn drop(&mut self) {
        let _ = ioctl_int(self.file.as_raw_fd(), UI_DEV_DESTROY, 0);
    }
}

pub struct InputDevices {
    pub pen: AbsDevice,
    pub mouse: RelDevice,
    pub eraser: AbsDevice,
    pub kbd: KeyDevice,
    pub max_x: i32,
    pub max_y: i32,
    pub max_pressure: i32,
    eraser_mode: bool,
}

impl InputDevices {
    pub fn open(max_x: i32, max_y: i32, max_pressure: i32) -> Result<Self, InputError> {
        Ok(Self {
            pen: AbsDevice::create_pen(max_x, max_y, max_pressure)?,
            mouse: RelDevice::create_mouse()?,
            eraser: AbsDevice::create_eraser(max_x, max_y, max_pressure)?,
            kbd: KeyDevice::create()?,
            max_x,
            max_y,
            max_pressure,
            eraser_mode: false,
        })
    }

    pub fn set_eraser_mode(&mut self, on: bool) {
        self.eraser_mode = on;
    }

    pub fn eraser_mode(&self) -> bool {
        self.eraser_mode
    }

    pub fn update_digit(
        &mut self,
        x: i32,
        y: i32,
        pressure: i32,
        tilt_x: i32,
        tilt_y: i32,
        in_range: bool,
        tip: bool,
        stylus: bool,
        stylus2: bool,
    ) -> io::Result<()> {
        let x = x.clamp(0, self.max_x);
        let y = y.clamp(0, self.max_y);
        let pressure = if tip {
            pressure.clamp(0, self.max_pressure)
        } else {
            0
        };
        if self.eraser_mode {
            let _ = self
                .pen
                .update(x, y, 0, tilt_x, tilt_y, BTN_TOOL_PEN, false, false, false);
            self.eraser.update(
                x,
                y,
                pressure,
                tilt_x,
                tilt_y,
                BTN_TOOL_RUBBER,
                in_range,
                stylus,
                stylus2,
            )
        } else {
            let _ = self.eraser.update(
                x,
                y,
                0,
                tilt_x,
                tilt_y,
                BTN_TOOL_RUBBER,
                false,
                false,
                false,
            );
            self.pen.update(
                x,
                y,
                pressure,
                tilt_x,
                tilt_y,
                BTN_TOOL_PEN,
                in_range,
                stylus,
                stylus2,
            )
        }
    }

    pub fn do_mouse_func(&mut self, id: u8, pressed: bool) -> io::Result<()> {
        match id {
            1 => self.mouse.mouse_button(BTN_LEFT, pressed),
            2 => self.mouse.mouse_button(BTN_RIGHT, pressed),
            3 => self.mouse.mouse_button(BTN_MIDDLE, pressed),
            4 if pressed => {
                self.mouse.mouse_button(BTN_LEFT, true)?;
                self.mouse.mouse_button(BTN_LEFT, false)?;
                thread::sleep(Duration::from_millis(20));
                self.mouse.mouse_button(BTN_LEFT, true)?;
                self.mouse.mouse_button(BTN_LEFT, false)
            }
            5 if pressed => self.mouse.wheel(1),
            6 if pressed => self.mouse.wheel(-1),
            7 => self.mouse.mouse_button(BTN_RIGHT, pressed),
            8 => self.mouse.mouse_button(BTN_MIDDLE, pressed),
            9 => self.mouse.mouse_button(BTN_MIDDLE, pressed),
            _ => Ok(()),
        }
    }

    #[allow(dead_code)]
    pub fn uinput_available() -> bool {
        Path::new("/dev/uinput").exists() || Path::new("/dev/input/uinput").exists()
    }
}

/// USB HID keyboard usage (page 0x07) → Linux evdev keycode.
pub fn hid_usage_evdev(usage: u8) -> Option<u16> {
    Some(match usage {
        0x04 => KEY_A,
        0x05 => KEY_B,
        0x06 => KEY_C,
        0x07 => 32,  // D
        0x08 => KEY_E,
        0x09 => 33,  // F
        0x0A => 34,  // G
        0x0B => 35,  // H
        0x0C => 23,  // I
        0x0D => 36,  // J
        0x0E => 37,  // K
        0x0F => 38,  // L
        0x10 => 50,  // M
        0x11 => 49,  // N
        0x12 => 24,  // O
        0x13 => 25,  // P
        0x14 => 16,  // Q
        0x15 => 19,  // R
        0x16 => KEY_S,
        0x17 => 20,  // T
        0x18 => 22,  // U
        0x19 => KEY_V,
        0x1A => 17,  // W
        0x1B => KEY_X,
        0x1C => KEY_Y,
        0x1D => KEY_Z,
        0x2A => KEY_DELETE, // backspace is 14; Delete is 0x4C. 0x2A=Backspace
        0x28 => KEY_ENTER,
        0x29 => KEY_ESC,
        0x2B => KEY_TAB,
        0x2C => KEY_SPACE,
        0x2D => KEY_MINUS,
        0x2E => KEY_EQUAL,
        0x2F => KEY_LEFTBRACE,
        0x30 => KEY_RIGHTBRACE,
        _ => return None,
    })
}

/// Linux evdev keycode from a simple name (`LeftCtrl`, `Z`, `minus`, …).
pub fn keycode(name: &str) -> Option<u16> {
    match name.to_ascii_lowercase().as_str() {
        "esc" | "escape" => Some(KEY_ESC),
        "ctrl" | "leftctrl" | "control" => Some(KEY_LEFTCTRL),
        "shift" | "leftshift" => Some(KEY_LEFTSHIFT),
        "alt" | "leftalt" => Some(KEY_LEFTALT),
        "super" | "meta" | "leftmeta" => Some(KEY_LEFTMETA),
        "space" => Some(KEY_SPACE),
        "tab" => Some(KEY_TAB),
        "enter" | "return" => Some(KEY_ENTER),
        "delete" | "del" => Some(KEY_DELETE),
        "z" => Some(KEY_Z),
        "x" => Some(KEY_X),
        "c" => Some(KEY_C),
        "v" => Some(KEY_V),
        "b" => Some(KEY_B),
        "e" => Some(KEY_E),
        "s" => Some(KEY_S),
        "y" => Some(KEY_Y),
        "a" => Some(KEY_A),
        "[" | "leftbrace" => Some(KEY_LEFTBRACE),
        "]" | "rightbrace" => Some(KEY_RIGHTBRACE),
        "-" | "minus" => Some(KEY_MINUS),
        "=" | "equal" | "plus" => Some(KEY_EQUAL),
        "pageup" => Some(KEY_PAGEUP),
        "pagedown" => Some(KEY_PAGEDOWN),
        other => other.parse().ok(),
    }
}

/// Keep kernel hid-generic event nodes from moving a second cursor.
/// hidraw is not exclusive — GNOME still sees "XP-PEN DECO 02 Stylus".
///
/// Do not drop this value while the tablet is plugged: releasing EVIOCGRAB
/// lets libinput take the stylus back (second cursor).
#[derive(Default)]
pub struct KernelGrab {
    held: Vec<(String, File)>,
}

impl KernelGrab {
    pub fn refresh(&mut self, vendor: u16, products: &[u16]) {
        let Ok(text) = std::fs::read_to_string("/proc/bus/input/devices") else {
            return;
        };
        const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
        let vid = format!("{vendor:04x}");
        for &product in products {
            let pid = format!("{product:04x}");
            for block in text.split("\n\n") {
                let want = block.contains(&format!("Vendor={vid}"))
                    && block.contains(&format!("Product={pid}"));
                if !want {
                    continue;
                }
                let Some(handlers) =
                    block.lines().find_map(|l| l.strip_prefix("H: Handlers="))
                else {
                    continue;
                };
                for tok in handlers.split_whitespace() {
                    let Some(n) = tok.strip_prefix("event") else { continue };
                    if !n.bytes().all(|b| b.is_ascii_digit()) {
                        continue;
                    }
                    let path = format!("/dev/input/event{n}");
                    if self.held.iter().any(|(p, _)| p == &path) {
                        continue;
                    }
                    let Ok(f) = OpenOptions::new().read(true).open(&path) else {
                        tracing::warn!(%path, "cannot open kernel event node");
                        continue;
                    };
                    let rc = unsafe { libc::ioctl(f.as_raw_fd(), EVIOCGRAB, 1) };
                    if rc == 0 {
                        tracing::info!(%path, "grabbed kernel input (hide from GNOME)");
                        self.held.push((path, f));
                    } else {
                        tracing::warn!(
                            %path,
                            err = %io::Error::last_os_error(),
                            "EVIOCGRAB failed"
                        );
                    }
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_names() {
        assert_eq!(keycode("LeftCtrl"), Some(KEY_LEFTCTRL));
        assert_eq!(keycode("Z"), Some(KEY_Z));
        assert_eq!(keycode("["), Some(KEY_LEFTBRACE));
        assert_eq!(keycode("nope"), None);
    }
}
