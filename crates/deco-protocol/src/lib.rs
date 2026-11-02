//! HID protocol for XP-Pen tablets, focused on Deco 02 (VID 0x28BD, PID 0x0803/0x0903).
//!
//! Layouts come from static reverse engineering of PenTablet 3.4.9 (`XPPenDriverSpec.md`).
//! Live report offsets should be confirmed with `--validate` on real hardware.

pub mod commands;
pub mod parse;
pub mod profile;

pub use commands::{Command, ReportId, LINUX_RELEASE, REPORT_ID};
pub use parse::{
    diagnose_event, hex_dump, parse_digitizer_rdesc, parse_report, Caps, DeviceEvent, KeyState,
    ParsedReport, PenState, RingDirection, DECO02_HID_MAX_PRESSURE, DECO02_HID_MAX_X,
    DECO02_HID_MAX_Y,
};
pub use profile::{DeviceProfile, DECO02, DECO02_PIDS, VID};

/// USB vendor ID shared by all XP-Pen / Ugee tablets in this driver family.
pub const XP_PEN_VID: u16 = 0x28BD;

/// Default ABS ranges used by the vendor uinput devices when caps are unknown.
pub const DEFAULT_MAX_X: u32 = 0xCC64;
pub const DEFAULT_MAX_Y: u32 = 0x73A0;
pub const DEFAULT_MAX_PRESSURE: u32 = 0x1FFF;
pub const DEFAULT_RESOLUTION_LPI: u32 = 0x13D8;
pub const DEFAULT_TILT_MIN: i32 = -63;
pub const DEFAULT_TILT_MAX: i32 = 64;

/// Hotplug settle delay used by the vendor (`usleep(600000)` after attach).
pub const HOTPLUG_SETTLE: std::time::Duration = std::time::Duration::from_millis(600);

/// HID interrupt poll timeout used by `DeviceThread::run`.
pub const HID_POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// Read buffer size used by the vendor (`hid_read_timeout(..., 0x40, 100)`).
pub const HID_REPORT_LEN: usize = 0x40;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deco02_pids_are_known() {
        assert!(is_deco02(0x0803));
        assert!(is_deco02(0x0903));
        assert!(!is_deco02(0x0902));
    }
}

/// Returns true if `pid` is a Deco 02 USB product ID.
pub fn is_deco02(pid: u16) -> bool {
    DECO02_PIDS.contains(&pid)
}

/// Marketing name for a known Deco 02 PID.
pub fn device_name(pid: u16) -> &'static str {
    match pid {
        0x0803 | 0x0903 => "Deco 02",
        _ => "XP-Pen tablet",
    }
}
