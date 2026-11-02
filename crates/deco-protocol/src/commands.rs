//! Host → device reports (report ID 0x02). See XPPenDriverSpec.md §3.1.

/// Interrupt/feature report ID used for every `hid_write` in the vendor driver.
pub const REPORT_ID: u8 = 0x02;

/// `DeviceThread::LinuxRelease` payload (`hid_write(h, {02 00 2B 00…}, 10)`).
pub const LINUX_RELEASE: [u8; 10] = [0x02, 0x00, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

/// Command types accepted by `CUSBTablet::SentCmd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// type 0: `02 B4 01 v1 …` (len 12). Init uses (v1=1) then (v1=2).
    Light { v1: u8, v2: u8 },
    /// type 1: `02 B4 01 01 v1 v2`
    LightExt { v1: u8, v2: u8 },
    /// type 2: `02 B4 02 01 v1 …`
    LightMode { v1: u8, v2: u8 },
    /// type 3: `02 B0 v1 …` (len 10)
    B0 { v1: u8 },
    /// type 0x6E..0x77 gated by firmware > 0x959 and PenType == 2.
    B5 { subtype: u8, v1: u8, v2: u8 },
}

pub struct ReportId;

impl Command {
    pub fn encode(self) -> Vec<u8> {
        match self {
            Command::Light { v1, v2 } => {
                let mut b = vec![0x02, 0xB4, 0x01, v1, v2, 0, 0, 0, 0, 0, 0, 0];
                if v2 == 0 {
                    b[4] = 0;
                }
                b.truncate(12);
                b
            }
            Command::LightExt { v1, v2 } => {
                vec![0x02, 0xB4, 0x01, 0x01, v1, v2, 0, 0, 0, 0, 0, 0]
            }
            Command::LightMode { v1, v2 } => {
                let mut b = vec![0x02, 0xB4, 0x02, 0x01, v1, 0, 0, 0, 0, 0, 0, 0];
                if v1 == 1 {
                    b[5] = v2;
                }
                b
            }
            Command::B0 { v1 } => vec![0x02, 0xB0, v1, 0, 0, 0, 0, 0, 0, 0],
            Command::B5 { subtype, v1, v2 } => match subtype {
                0x00 => vec![0x02, 0xB5, 0x00, 0xF0, 0, 0, 0, 0, 0, 0],
                0x01 => vec![0x02, 0xB5, 0x01, 0x01, 0x00, v1, v2, 0, 0, 0],
                0x02 => vec![0x02, 0xB5, 0x01, 0x02, 0x00, v1, v2, 0, 0, 0],
                _ => vec![0x02, 0xB5, subtype, 0, 0, v1, v2, 0, 0, 0],
            },
        }
    }
}

/// Init sequence after `hid_open_path`: `SentCmd(0, 1, 0)` then `SentCmd(0, 2, 0)`.
pub fn init_commands() -> [Vec<u8>; 2] {
    [
        Command::Light { v1: 1, v2: 0 }.encode(),
        Command::Light { v1: 2, v2: 0 }.encode(),
    ]
}

/// Firmware numeric gate for B5 commands (`toInt() > 0x959`).
pub fn firmware_allows_b5(fw: &str) -> bool {
    let digits: String = fw.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().map(|n| n > 0x959).unwrap_or(false)
}

/// Parse USB indexed string 100 as the vendor does (byte-wise 24-bit axes).
pub fn parse_indexed_caps(bytes: &[u8]) -> Option<super::parse::Caps> {
    if bytes.len() < 8 {
        return None;
    }
    let max_x = u32::from(bytes[0])
        + u32::from(bytes[1]) * 0x100
        + u32::from(bytes.get(2).copied().unwrap_or(0)) * 0x10000;
    let max_y = u32::from(bytes[3])
        + u32::from(bytes[4]) * 0x100
        + u32::from(bytes.get(5).copied().unwrap_or(0)) * 0x10000;
    let max_pressure = u32::from(bytes[6]) + u32::from(bytes[7]) * 0x100;
    if max_x == 0 || max_y == 0 {
        return None;
    }
    Some(super::parse::Caps {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_reports_match_vendor() {
        let [a, b] = init_commands();
        assert_eq!(&a[..4], &[0x02, 0xB4, 0x01, 0x01]);
        assert_eq!(a.len(), 12);
        assert_eq!(&b[..4], &[0x02, 0xB4, 0x01, 0x02]);
        assert_eq!(LINUX_RELEASE[0], 0x02);
        assert_eq!(LINUX_RELEASE[2], 0x2B);
    }

    #[test]
    fn firmware_gate() {
        assert!(!firmware_allows_b5("2000"));
        assert!(firmware_allows_b5("2400"));
        assert!(firmware_allows_b5("V3.4.9 2500"));
    }

    #[test]
    fn indexed_caps_24bit() {
        let caps = parse_indexed_caps(&[0x64, 0xCC, 0x00, 0xA0, 0x73, 0x00, 0xFF, 0x1F]).unwrap();
        assert_eq!(caps.max_x, 0xCC64);
        assert_eq!(caps.max_y, 0x73A0);
        assert_eq!(caps.max_pressure, 0x1FFF);
    }
}
