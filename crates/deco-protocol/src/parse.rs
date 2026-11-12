//! Device → host report parser (`CParseTabletData::ParseData`, spec §4).

use serde::{Deserialize, Serialize};

const FLAG_PEN: u16 = 1;
const FLAG_KEYBOARD: u16 = 2;
const FLAG_RING: u16 = 4;
const FLAG_PEN_BTN: u16 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caps {
    pub max_x: u32,
    pub max_y: u32,
    pub max_pressure: u32,
    pub resolution: u32,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            max_x: crate::DEFAULT_MAX_X,
            max_y: crate::DEFAULT_MAX_Y,
            max_pressure: crate::DEFAULT_MAX_PRESSURE,
            resolution: crate::DEFAULT_RESOLUTION_LPI,
        }
    }
}

/// Capability reply `buf[1]==0x06 && buf[2]==0x64 ('d')`.
pub fn parse_caps_report(buf: &[u8]) -> Option<Caps> {
    if buf.len() < 11 {
        return None;
    }
    if buf.get(1) != Some(&0x06) || buf.get(2) != Some(&0x64) {
        return None;
    }
    let max_x = u32::from(buf[3]) + u32::from(buf[4]) * 0x100 + u32::from(buf[5]) * 0x10000;
    let max_y = u32::from(buf[6]) + u32::from(buf[7]) * 0x100 + u32::from(buf[8]) * 0x10000;
    let max_pressure = u32::from(buf[9]) + u32::from(buf[10]) * 0x100;
    if max_x == 0 || max_y == 0 {
        return None;
    }
    Some(Caps {
        max_x,
        max_y,
        max_pressure: if max_pressure == 0 {
            crate::DEFAULT_MAX_PRESSURE
        } else {
            max_pressure
        },
        resolution: crate::DEFAULT_RESOLUTION_LPI,
    })
}

/// Deco 02 HID digitizer (report 0x07) logical max from the report descriptor.
/// Vendor string-100 / 0x06 'd' caps (0xCC64×0x73A0) do not match these axes.
pub const DECO02_HID_MAX_X: u32 = 0x5750;
pub const DECO02_HID_MAX_Y: u32 = 0x3692;
pub const DECO02_HID_MAX_PRESSURE: u32 = 0x1FFF;

/// Deco 02 exposes three HID interfaces. Only one of them accepts host commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceRole {
    /// Report 0x07 pen plus express keys (0x03 / 0x06). Interrupt IN only.
    Digitizer,
    /// Vendor page 0xFF0A, report 0x02. The only interface with an OUT endpoint.
    Command,
    /// Boot mouse. Host writes here stall the device (`POLLERR` on every interface).
    Unused,
}

/// Classify a report descriptor. Digitizer wins when both axes and an output report exist.
pub fn interface_role(rdesc: &[u8]) -> InterfaceRole {
    if parse_digitizer_rdesc(rdesc).is_some() {
        InterfaceRole::Digitizer
    } else if has_output_report(rdesc) {
        InterfaceRole::Command
    } else {
        InterfaceRole::Unused
    }
}

/// True when the descriptor contains a HID Output main item (interrupt OUT / SET_REPORT).
fn has_output_report(rdesc: &[u8]) -> bool {
    let mut i = 0;
    while i < rdesc.len() {
        let prefix = rdesc[i];
        i += 1;
        if prefix == 0xFE {
            if i + 1 >= rdesc.len() {
                break;
            }
            let size = rdesc[i] as usize;
            i += 2 + size;
            continue;
        }
        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if i + size > rdesc.len() {
            break;
        }
        i += size;
        let item_type = (prefix >> 2) & 0x03;
        let item_tag = prefix >> 4;
        // Main item, tag Output (0x91…).
        if item_type == 0 && item_tag == 9 {
            return true;
        }
    }
    false
}

/// Read Generic Desktop X/Y and Digitizer Tip Pressure logical max for report ID 7.
pub fn parse_digitizer_rdesc(rdesc: &[u8]) -> Option<Caps> {
    let mut i = 0;
    let mut usage_page = 0u32;
    let mut report_id = 0u8;
    let mut usage = 0u32;
    let mut logical_max = 0u32;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut max_pressure = 0u32;
    while i < rdesc.len() {
        let prefix = rdesc[i];
        i += 1;
        if prefix == 0xFE {
            if i + 1 >= rdesc.len() {
                break;
            }
            let size = rdesc[i] as usize;
            i += 2 + size;
            continue;
        }
        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if i + size > rdesc.len() {
            break;
        }
        let mut data = 0u32;
        for (b, v) in rdesc[i..i + size].iter().enumerate() {
            data |= u32::from(*v) << (8 * b);
        }
        i += size;
        let item_type = (prefix >> 2) & 0x03;
        let item_tag = prefix >> 4;
        match (item_type, item_tag) {
            (1, 0) => usage_page = data,
            (1, 2) => logical_max = data,
            (1, 8) => report_id = data as u8,
            (2, 0) => usage = data,
            (0, 8) if report_id == 7 => {
                // Input item: bind current usage to this axis.
                if usage_page == 0x01 && usage == 0x30 && logical_max > 0 {
                    max_x = logical_max;
                } else if usage_page == 0x01 && usage == 0x31 && logical_max > 0 {
                    max_y = logical_max;
                } else if usage_page == 0x0D && usage == 0x30 && logical_max > 0 {
                    max_pressure = logical_max;
                }
            }
            _ => {}
        }
    }
    if max_x == 0 || max_y == 0 {
        return None;
    }
    Some(Caps {
        max_x,
        max_y,
        max_pressure: if max_pressure == 0 {
            DECO02_HID_MAX_PRESSURE
        } else {
            max_pressure
        },
        resolution: crate::DEFAULT_RESOLUTION_LPI,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PenState {
    pub x: u32,
    pub y: u32,
    pub pressure: u32,
    pub tilt_x: i8,
    pub tilt_y: i8,
    /// Raw `buf[1]` (tip / in-range / buttons nibble).
    pub raw: u8,
    pub tip: bool,
    pub in_range: bool,
    pub eraser: bool,
    pub btn_changed: u8,
    pub btn_pressed: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyState {
    /// Up to 12 express-key slots; Deco 02 uses 6.
    pub pressed: [bool; 12],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    Pen(PenState),
    Keys(KeyState),
    /// Unmapped firmware keyboard combo (report 0x03) — inject as-is.
    HidPassthrough { mods: u8, usages: Vec<u8> },
    Ring(RingDirection),
    Caps(Caps),
    Mode(u8),
    Ack,
    Handshake,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReport {
    pub flags: u16,
    pub events: Vec<DeviceEvent>,
}

fn is_pen_frame(buf: &[u8]) -> bool {
    if buf.len() < 2 {
        return false;
    }
    // Vendor: buf[1] == 0xC0 or (buf[1] & 0x30) == 0x20.
    // Live packets use 0xC0..0xCF for hover/tip/buttons, so also match the 0xC0 nibble.
    (buf[1] & 0xF0) == 0xC0 || (buf[1] & 0x30) == 0x20
}

fn parse_pen(buf: &[u8]) -> PenState {
    let long = buf.len() >= 0x0C;
    let mut x = u32::from(buf[2]) + u32::from(buf[3]) * 0x100;
    let mut y = u32::from(buf[4]) + u32::from(buf[5]) * 0x100;
    if long && buf.len() > 11 {
        x += u32::from(buf[10]) * 0x10000;
        y += u32::from(buf[11]) * 0x10000;
    }
    let mut pressure = u32::from(buf[6]) + u32::from(buf[7] & 0x1F) * 0x100;
    if buf.len() >= 0x0E {
        pressure += u32::from(buf[0x0D] & 7) * 0x2000;
    }
    let tilt_x = if buf.len() > 8 { buf[8] as i8 } else { 0 };
    let tilt_y = if buf.len() > 9 { buf[9] as i8 } else { 0 };
    let nibble = buf[1] & 0x0F;
    let tip = nibble & 0x01 != 0;
    let btn1 = nibble & 0x02 != 0;
    let btn2 = nibble & 0x04 != 0;
    let eraser = nibble & 0x08 != 0;
    let in_range = buf[1] == 0xC0 || (buf[1] & 0x20) != 0 || tip || pressure > 0;
    let mut btn_pressed = 0u8;
    if btn1 {
        btn_pressed |= 1;
    }
    if btn2 {
        btn_pressed |= 2;
    }
    PenState {
        x,
        y,
        pressure,
        tilt_x,
        tilt_y,
        raw: buf[1],
        tip,
        in_range,
        eraser,
        btn_changed: nibble,
        btn_pressed,
    }
}

/// `ChangeDataToKeyState`: buf[0]==6 → 32 bits from [1..4]; else 40 bits from [2..6].
fn parse_keys(buf: &[u8]) -> KeyState {
    let mut pressed = [false; 12];
    let bits = if buf[0] == 0x06 {
        let mut v = 0u64;
        for (i, b) in buf.get(1..5).unwrap_or(&[]).iter().enumerate() {
            v |= u64::from(*b) << (8 * i);
        }
        v
    } else {
        let mut v = 0u64;
        for (i, b) in buf.get(2..7).unwrap_or(&[]).iter().enumerate() {
            v |= u64::from(*b) << (8 * i);
        }
        v
    };
    for (i, slot) in pressed.iter_mut().enumerate() {
        *slot = bits & (1 << i) != 0;
    }
    KeyState { pressed }
}

fn ring_from_subtype(subtype: u8) -> Option<RingDirection> {
    match subtype {
        1 | 4 | 0x10 => Some(RingDirection::Right),
        2 | 8 | 0x20 | 0xFF => Some(RingDirection::Left),
        _ => None,
    }
}

/// Compact hex dump of a HID report (first 16 bytes).
pub fn hex_dump(buf: &[u8]) -> String {
    buf.iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runtime checks on a parsed event vs device caps. Empty = looks sane.
pub fn diagnose_event(ev: &DeviceEvent, caps: Caps) -> Vec<String> {
    match ev {
        DeviceEvent::Pen(p) => {
            let mut issues = Vec::new();
            debug_assert!(
                p.pressure <= caps.max_pressure.saturating_mul(4) || caps.max_pressure == 0,
                "pressure {} vs cap {}",
                p.pressure,
                caps.max_pressure
            );
            if caps.max_x > 0 && p.x > caps.max_x.saturating_mul(2) {
                issues.push(format!(
                    "pen X {} is > 2× MaxX {} — parser/caps mismatch",
                    p.x, caps.max_x
                ));
            }
            if caps.max_y > 0 && p.y > caps.max_y.saturating_mul(2) {
                issues.push(format!(
                    "pen Y {} is > 2× MaxY {} — parser/caps mismatch",
                    p.y, caps.max_y
                ));
            }
            if p.pressure > caps.max_pressure.saturating_mul(2) && caps.max_pressure > 0 {
                issues.push(format!(
                    "pressure {} is > 2× MaxPress {} — 19-bit extend?",
                    p.pressure, caps.max_pressure
                ));
            }
            issues
        }
        DeviceEvent::Caps(c) => {
            let mut issues = Vec::new();
            debug_assert!(c.max_x > 0 && c.max_y > 0, "caps axes must be non-zero");
            if c.max_x == 0 || c.max_y == 0 {
                issues.push("caps MaxX/MaxY are 0 — indexed string 100 / 0x06 'd' failed".into());
            }
            if c.max_pressure == 0 {
                issues.push("caps MaxPress is 0".into());
            }
            issues
        }
        _ => Vec::new(),
    }
}

fn empty_keys() -> ParsedReport {
    ParsedReport {
        flags: FLAG_KEYBOARD,
        events: vec![DeviceEvent::Keys(KeyState::default())],
    }
}

/// HID report 0x03 key slots: Deco 02 omits the boot reserved byte
/// (`03 mods key…`). Classic boot keyboard is `03 mods 00 key…`.
fn hid_keycodes(buf: &[u8]) -> Vec<u8> {
    let start = if buf.len() > 3 && buf[2] == 0 && buf[3] != 0 {
        3
    } else {
        2
    };
    buf.get(start..)
        .unwrap_or(&[])
        .iter()
        .copied()
        .filter(|c| *c != 0)
        .take(6)
        .collect()
}

fn slot_from_hid(mods: u8, key: u8) -> Option<usize> {
    let ctrl = mods & 0x01 != 0;
    let shift = mods & 0x02 != 0;
    let alt = mods & 0x04 != 0;
    match (ctrl, shift, alt, key) {
        (true, true, false, 0x1D) => Some(1),  // Ctrl+Shift+Z → K2
        (true, false, true, 0x1D) => Some(4),  // Ctrl+Alt+Z → K5 (live Deco 02)
        (true, false, false, 0x1D) => Some(0), // Ctrl+Z → K1
        (false, false, false, 0x2F) => Some(2), // [
        (false, false, false, 0x30) => Some(3), // ]
        (true, _, false, 0x2E) => Some(4),     // Ctrl+=
        (true, _, _, 0x16) => Some(5),         // Ctrl+S → K6 (even with Alt held)
        (true, _, false, 0x2D) => Some(5),     // Ctrl+-
        _ => None,
    }
}

/// Deco 02 report ID 0x03: boot keyboard (modifiers + 6 HID usages).
/// Firmware encodes express keys as whatever shortcuts were last programmed.
fn parse_hid_keyboard(buf: &[u8]) -> ParsedReport {
    if buf.len() < 3 {
        return empty_keys();
    }
    let mods = buf[1];
    let codes = hid_keycodes(buf);
    let ctrl = mods & 0x01 != 0;
    let shift = mods & 0x02 != 0;

    // Modifier-only frames are key-up, not a leftover Ctrl passthrough.
    if codes.is_empty() {
        return empty_keys();
    }

    // Roller detents: Ctrl+= / Ctrl+- with no other key.
    if ctrl && !shift && codes == [0x2E] {
        return ParsedReport {
            flags: FLAG_RING,
            events: vec![DeviceEvent::Ring(RingDirection::Right)],
        };
    }
    if ctrl && !shift && codes == [0x2D] {
        return ParsedReport {
            flags: FLAG_RING,
            events: vec![DeviceEvent::Ring(RingDirection::Left)],
        };
    }

    let mut pressed = [false; 12];
    for &key in &codes {
        if let Some(i) = slot_from_hid(mods, key) {
            pressed[i] = true;
        }
    }
    if pressed.iter().any(|p| *p) {
        return ParsedReport {
            flags: FLAG_KEYBOARD,
            events: vec![DeviceEvent::Keys(KeyState { pressed })],
        };
    }
    ParsedReport {
        flags: FLAG_KEYBOARD,
        events: vec![DeviceEvent::HidPassthrough {
            mods,
            usages: codes,
        }],
    }
}

/// Report ID 0x07 digitizer (Tip, Barrel, Eraser, Invert, InRange, X16, Y16, Pressure16).
fn parse_hid_digitizer(buf: &[u8]) -> ParsedReport {
    if buf.len() < 8 {
        return ParsedReport {
            flags: 0,
            events: vec![DeviceEvent::Unknown],
        };
    }
    let b = buf[1];
    // Bit 2 = Eraser (eraser touching), bit 3 = Invert (eraser side in range).
    // Neither is a button: the tablet says "erasing" for as long as they are set.
    let erasing = b & 0x04 != 0;
    let eraser = erasing || b & 0x08 != 0;
    let tip = b & 0x01 != 0 || erasing;
    let btn1 = b & 0x02 != 0;
    let in_range = b & 0x10 != 0 || tip || eraser;
    let x = u32::from(u16::from_le_bytes([buf[2], buf[3]]));
    let y = u32::from(u16::from_le_bytes([buf[4], buf[5]]));
    let pressure = u32::from(u16::from_le_bytes([buf[6], buf[7]]));
    let mut btn_pressed = 0u8;
    if btn1 {
        btn_pressed |= 1;
    }
    ParsedReport {
        flags: FLAG_PEN | FLAG_PEN_BTN,
        events: vec![DeviceEvent::Pen(PenState {
            x,
            y,
            pressure,
            tilt_x: 0,
            tilt_y: 0,
            raw: b,
            tip,
            in_range,
            eraser,
            btn_changed: b & 0x0F,
            btn_pressed,
        })],
    }
}

fn parse_vendor_frame(buf: &[u8]) -> ParsedReport {
    if buf.len() > 2 {
        match buf[1] {
            0x06 if buf.get(2) == Some(&0x64) => {
                if let Some(caps) = parse_caps_report(buf) {
                    return ParsedReport {
                        flags: 0,
                        events: vec![DeviceEvent::Caps(caps)],
                    };
                }
            }
            0xB4 | 0xB0 | 0xB6 => {
                return ParsedReport {
                    flags: 0,
                    events: vec![DeviceEvent::Ack],
                };
            }
            0xF6 => {
                let probe = ((u16::from(buf.get(2).copied().unwrap_or(0) & 0xF0)
                    + u16::from(buf.get(3).copied().unwrap_or(0)) * 0x100)
                    >> 4) as u16;
                let mode = match probe {
                    0x301 => 1,
                    0x501 => 2,
                    0x801 => 3,
                    _ => 0,
                };
                return ParsedReport {
                    flags: 0,
                    events: vec![DeviceEvent::Mode(mode)],
                };
            }
            0xF8 => {
                return ParsedReport {
                    flags: 0,
                    events: vec![DeviceEvent::Handshake],
                };
            }
            0xF2 => {
                return ParsedReport {
                    flags: 0,
                    events: vec![DeviceEvent::Ack],
                };
            }
            _ => {}
        }
    }

    let mut flags = 0u16;
    let mut events = Vec::new();

    if buf[0] == 0x06 {
        flags |= FLAG_KEYBOARD;
        events.push(DeviceEvent::Keys(parse_keys(buf)));
        return ParsedReport { flags, events };
    }

    if is_pen_frame(buf) && buf.len() >= 8 {
        flags |= FLAG_PEN;
        flags |= FLAG_PEN_BTN;
        events.push(DeviceEvent::Pen(parse_pen(buf)));
        return ParsedReport { flags, events };
    }

    if buf.len() > 1 {
        if let Some(dir) = ring_from_subtype(buf[1]) {
            flags |= FLAG_RING;
            events.push(DeviceEvent::Ring(dir));
            return ParsedReport { flags, events };
        }
        if buf[0] != 0x02 || buf[1] != 0x00 {
            flags |= FLAG_KEYBOARD;
            events.push(DeviceEvent::Keys(parse_keys(buf)));
            return ParsedReport { flags, events };
        }
    }

    ParsedReport {
        flags,
        events: vec![DeviceEvent::Unknown],
    }
}

/// Parse one HID interrupt report. Empty / timeout frames yield no events.
pub fn parse_report(buf: &[u8]) -> ParsedReport {
    debug_assert!(
        buf.len() <= crate::HID_REPORT_LEN || buf.is_empty(),
        "HID report longer than vendor 64-byte buffer: {}",
        buf.len()
    );
    if buf.is_empty() {
        return ParsedReport {
            flags: 0,
            events: vec![],
        };
    }

    // Deco 02 exposes standard HID collections alongside vendor report 0x02.
    // Live roller: 03 01 2e = LeftCtrl + KeyboardEqual — must not use vendor ring (buf[1]==1).
    match buf[0] {
        0x03 => parse_hid_keyboard(buf),
        0x06 => ParsedReport {
            flags: FLAG_KEYBOARD,
            events: vec![DeviceEvent::Keys(parse_keys(buf))],
        },
        0x07 => parse_hid_digitizer(buf),
        // Mouse iface 0: button bits 1/2/4/8 must not be parsed as vendor ring.
        0x01 | 0x08 | 0x09 => ParsedReport {
            flags: 0,
            events: vec![],
        },
        _ => parse_vendor_frame(buf),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pen_short_c0_tip() {
        // x=0x1234 y=0x0567 pressure=0x0102 tip bit
        let buf = [0x02, 0xC1, 0x34, 0x12, 0x67, 0x05, 0x02, 0x01, 0x00, 0x00];
        let parsed = parse_report(&buf);
        match &parsed.events[0] {
            DeviceEvent::Pen(p) => {
                assert_eq!(p.x, 0x1234);
                assert_eq!(p.y, 0x0567);
                assert_eq!(p.pressure, 0x0102);
                assert!(p.tip);
                assert!(p.in_range);
            }
            other => panic!("expected pen, got {other:?}"),
        }
        assert_eq!(parsed.flags & FLAG_PEN, FLAG_PEN);
    }

    #[test]
    fn pen_family_0x20_and_long_pressure() {
        let mut buf = [0u8; 16];
        buf[1] = 0x21; // (buf[1] & 0x30) == 0x20, tip
        buf[2] = 0x01;
        buf[3] = 0x00;
        buf[4] = 0x02;
        buf[5] = 0x00;
        buf[6] = 0x00;
        buf[7] = 0x10; // pressure low + high nibble
        buf[10] = 0x01; // X hi
        buf[11] = 0x02; // Y hi
        buf[0x0D] = 0x01; // +0x2000
        let parsed = parse_report(&buf);
        match &parsed.events[0] {
            DeviceEvent::Pen(p) => {
                assert_eq!(p.x, 0x010001);
                assert_eq!(p.y, 0x020002);
                assert_eq!(p.pressure, 0x1000 + 0x2000);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn keyboard_buf0_06_bits() {
        let mut buf = [0u8; 10];
        buf[0] = 0x06;
        buf[1] = 0b0010_0101; // keys 0, 2, 5
        let parsed = parse_report(&buf);
        match &parsed.events[0] {
            DeviceEvent::Keys(k) => {
                assert!(k.pressed[0]);
                assert!(!k.pressed[1]);
                assert!(k.pressed[2]);
                assert!(k.pressed[5]);
                assert!(!k.pressed[3]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ring_right_and_left() {
        assert!(matches!(
            parse_report(&[0x02, 0x01, 0, 0]).events[0],
            DeviceEvent::Ring(RingDirection::Right)
        ));
        assert!(matches!(
            parse_report(&[0x02, 0x08, 0, 0]).events[0],
            DeviceEvent::Ring(RingDirection::Left)
        ));
    }

    #[test]
    fn hid_keyboard_roller_is_not_a_key() {
        // Live Deco 02: report ID 3, LeftCtrl + KeyboardEqual
        let parsed = parse_report(&[0x03, 0x01, 0x2E, 0, 0, 0, 0, 0]);
        assert!(
            matches!(parsed.events[0], DeviceEvent::Ring(RingDirection::Right)),
            "got {:?}",
            parsed.events
        );
        let parsed = parse_report(&[0x03, 0x01, 0x2D, 0, 0, 0, 0, 0]);
        assert!(matches!(
            parsed.events[0],
            DeviceEvent::Ring(RingDirection::Left)
        ));
    }

    #[test]
    fn hid_keyboard_express_keys() {
        match &parse_report(&[0x03, 0x01, 0x1D, 0, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => assert!(k.pressed[0] && !k.pressed[1]),
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x03, 0x1D, 0, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => assert!(k.pressed[1] && !k.pressed[0]),
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x00, 0x2F, 0, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => assert!(k.pressed[2]),
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x00, 0, 0, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => assert!(k.pressed.iter().all(|p| !p)),
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x01, 0x00, 0x1D, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => assert!(k.pressed[0]),
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x01, 0x16, 0, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => assert!(k.pressed[5], "Ctrl+S is K6, got {k:?}"),
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x05, 0x1D, 0, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => {
                assert!(k.pressed[4] && !k.pressed[0], "Ctrl+Alt+Z is K5, got {k:?}")
            }
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x05, 0x1D, 0x16, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => {
                assert!(k.pressed[4] && k.pressed[5], "K5+K6 chord, got {k:?}")
            }
            other => panic!("{other:?}"),
        }
        match &parse_report(&[0x03, 0x01, 0, 0, 0, 0, 0, 0]).events[0] {
            DeviceEvent::Keys(k) => assert!(k.pressed.iter().all(|p| !p)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mouse_report_is_not_ring() {
        let parsed = parse_report(&[0x08, 0x01, 0, 0, 0, 0, 0, 0]);
        assert!(
            parsed.events.is_empty(),
            "mouse iface must not be vendor ring, got {:?}",
            parsed.events
        );
    }

    #[test]
    fn hid_digitizer_eraser_bits_are_not_a_button() {
        // Report descriptor: bit2 = Eraser (touching), bit3 = Invert (eraser side in range).
        let frame = |status: u8| {
            let buf = [0x07, status, 0, 0, 0, 0, 0x10, 0x00];
            match parse_report(&buf).events.remove(0) {
                DeviceEvent::Pen(p) => p,
                other => panic!("{other:?}"),
            }
        };
        let hover = frame(0x98);
        assert!(hover.eraser && hover.in_range && !hover.tip);
        assert_eq!(hover.btn_pressed, 0);
        let touch = frame(0x94);
        assert!(touch.eraser && touch.tip);
        assert_eq!(touch.btn_pressed, 0);
        let pen = frame(0x91);
        assert!(!pen.eraser && pen.tip);
    }

    #[test]
    fn hid_digitizer_inrange_is_not_ring() {
        // bit4 = InRange. Vendor ring treated 0x10 as Right — must not.
        let mut buf = [0u8; 8];
        buf[0] = 0x07;
        buf[1] = 0x10;
        buf[2] = 0x10;
        buf[3] = 0x00;
        buf[4] = 0x20;
        buf[5] = 0x00;
        match &parse_report(&buf).events[0] {
            DeviceEvent::Pen(p) => {
                assert!(p.in_range);
                assert!(!p.tip);
                assert_eq!(p.x, 0x10);
                assert_eq!(p.y, 0x20);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn caps_reply_d() {
        let buf = [
            0x02, 0x06, b'd', 0x64, 0xCC, 0x00, 0xA0, 0x73, 0x00, 0xFF, 0x1F,
        ];
        match parse_report(&buf).events[0] {
            DeviceEvent::Caps(c) => {
                assert_eq!(c.max_x, 0xCC64);
                assert_eq!(c.max_y, 0x73A0);
                assert_eq!(c.max_pressure, 0x1FFF);
            }
            ref other => panic!("{other:?}"),
        }
    }

    #[test]
    fn diagnose_flags_out_of_range_xy() {
        let caps = Caps {
            max_x: 100,
            max_y: 100,
            max_pressure: 8191,
            resolution: 5080,
        };
        let pen = DeviceEvent::Pen(PenState {
            x: 500,
            y: 10,
            pressure: 1,
            tilt_x: 0,
            tilt_y: 0,
            raw: 0xC1,
            tip: true,
            in_range: true,
            eraser: false,
            btn_changed: 1,
            btn_pressed: 0,
        });
        let issues = diagnose_event(&pen, caps);
        assert!(issues.iter().any(|s| s.contains("pen X")));
        assert!(diagnose_event(&DeviceEvent::Ack, caps).is_empty());
    }

    #[test]
    fn hex_dump_truncates() {
        let s = hex_dump(&[0x02, 0xC1, 0x00]);
        assert_eq!(s, "02 c1 00");
    }

    #[test]
    fn deco02_rdesc_digitizer_axes() {
        let rdesc = [
            0x05, 0x0d, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x07, 0x09, 0x20, 0xa1, 0x00, 0x09, 0x42,
            0x09, 0x44, 0x09, 0x45, 0x09, 0x3c, 0x09, 0x32, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01,
            0x95, 0x05, 0x81, 0x02, 0x95, 0x03, 0x81, 0x03, 0x05, 0x01, 0x09, 0x30, 0x15, 0x00,
            0x26, 0x50, 0x57, 0x55, 0x0d, 0x65, 0x13, 0x35, 0x00, 0x46, 0x50, 0x57, 0x75, 0x10,
            0x95, 0x01, 0x81, 0x02, 0x09, 0x31, 0x15, 0x00, 0x26, 0x92, 0x36, 0x55, 0x0d, 0x65,
            0x13, 0x35, 0x00, 0x46, 0x92, 0x36, 0x75, 0x10, 0x95, 0x01, 0x81, 0x02, 0x05, 0x0d,
            0x09, 0x30, 0x15, 0x00, 0x26, 0xff, 0x1f, 0x75, 0x10, 0x95, 0x01, 0x81, 0x02,
        ];
        let caps = parse_digitizer_rdesc(&rdesc).expect("digitizer axes");
        assert_eq!(caps.max_x, DECO02_HID_MAX_X);
        assert_eq!(caps.max_y, DECO02_HID_MAX_Y);
        assert_eq!(caps.max_pressure, DECO02_HID_MAX_PRESSURE);
        assert_eq!(interface_role(&rdesc), InterfaceRole::Digitizer);
    }

    #[test]
    fn deco02_vendor_iface_is_command_mouse_is_unused() {
        // Interface 2 from a live Deco 02: usage page 0xFF0A, report 0x02, output 7 bytes.
        let vendor = [
            0x06, 0x0a, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x02, 0x09, 0x02, 0x75, 0x08, 0x95,
            0x09, 0x15, 0x00, 0x26, 0xff, 0x00, 0x81, 0x02, 0x09, 0x03, 0x75, 0x08, 0x95, 0x07,
            0x15, 0x00, 0x26, 0xff, 0x00, 0x91, 0x02, 0xc0,
        ];
        assert_eq!(interface_role(&vendor), InterfaceRole::Command);
        // Interface 0 is a boot mouse: input and feature items, no output.
        let mouse = [
            0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x08, 0x09, 0x01, 0xa1, 0x00, 0x81, 0x02,
            0xc0, 0xc0, 0x05, 0x01, 0x09, 0x00, 0xa1, 0x01, 0x85, 0x01, 0xb1, 0x02, 0xc0,
        ];
        assert_eq!(interface_role(&mouse), InterfaceRole::Unused);
    }
}
