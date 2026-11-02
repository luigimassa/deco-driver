//! Hardware profile for Deco 02 (template taken from vendor `<Deco03>`).

use crate::XP_PEN_VID;

pub const VID: u16 = XP_PEN_VID;

/// USB PIDs listed as "Deco 02" in `name_config.ini`.
pub const DECO02_PIDS: [u16; 2] = [0x0803, 0x0903];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceProfile {
    pub name: &'static str,
    pub pids: &'static [u16],
    pub key_num: u8,
    pub ring_num: u8,
    pub pen_buttons: u8,
    pub trackpad: bool,
    pub default_max_x: u32,
    pub default_max_y: u32,
    pub default_max_pressure: u32,
}

/// Deco 02: 6 express keys + roller, no trackpad. Same DeviceInfo as vendor Deco03.
pub const DECO02: DeviceProfile = DeviceProfile {
    name: "Deco 02",
    pids: &DECO02_PIDS,
    key_num: 6,
    ring_num: 1,
    pen_buttons: 2,
    trackpad: false,
    default_max_x: crate::DEFAULT_MAX_X,
    default_max_y: crate::DEFAULT_MAX_Y,
    default_max_pressure: crate::DEFAULT_MAX_PRESSURE,
};

impl DeviceProfile {
    pub fn for_pid(pid: u16) -> Option<&'static DeviceProfile> {
        if DECO02.pids.contains(&pid) {
            Some(&DECO02)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deco02_matches_vendor_deco03_caps() {
        assert_eq!(DECO02.key_num, 6);
        assert_eq!(DECO02.ring_num, 1);
        assert!(!DECO02.trackpad);
    }
}
