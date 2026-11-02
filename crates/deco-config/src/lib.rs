//! User configuration for Deco 02 (`~/.config/deco-driver/config.toml`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml parse: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("toml serialize: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Binding {
    Disabled,
    Key { keys: Vec<String> },
    Mouse { action: String },
    Scroll { direction: String },
    Builtin { id: u16 },
    EraserToggle,
}

impl Binding {
    pub fn key(keys: &[&str]) -> Self {
        Self::Key {
            keys: keys.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PressureCurve {
    pub p0: f64,
    pub p1x: f64,
    pub p1y: f64,
    pub p2: f64,
}

impl Default for PressureCurve {
    fn default() -> Self {
        Self {
            p0: 0.0,
            p1x: 0.5,
            p1y: 0.5,
            p2: 1.0,
        }
    }
}

impl PressureCurve {
    /// Map raw 0..1 pressure through a cubic Bezier (P0–P1–P1–P2 style vendor curve).
    pub fn apply(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let y0 = self.p0;
        let y1 = self.p1y;
        let y2 = self.p2;
        let x1 = self.p1x.clamp(0.01, 0.99);
        // Approximate: use t remapped so the control point x is honoured.
        let s = if (x1 - 0.5).abs() < 0.01 {
            t
        } else {
            // quadratic solve for parameter along (0, x1, 1)
            let a = 1.0 - 2.0 * x1;
            let b = 2.0 * x1;
            let c = -t;
            let disc = (b * b - 4.0 * a * c).max(0.0);
            if a.abs() < 1e-6 {
                (t / b).clamp(0.0, 1.0)
            } else {
                ((-b + disc.sqrt()) / (2.0 * a)).clamp(0.0, 1.0)
            }
        };
        let w = 1.0 - s;
        w * w * y0 + 2.0 * w * s * y1 + s * s * y2
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PenConfig {
    pub btn1: Binding,
    pub btn2: Binding,
    pub curve: PressureCurve,
    pub relative: bool,
    pub relative_speed: u8,
}

impl Default for PenConfig {
    fn default() -> Self {
        Self {
            btn1: Binding::Mouse {
                action: "right".into(),
            },
            btn2: Binding::EraserToggle,
            curve: PressureCurve::default(),
            relative: false,
            relative_speed: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeysConfig {
    pub k: [Binding; 6],
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            k: [
                Binding::key(&["LeftCtrl", "Z"]),
                Binding::key(&["LeftCtrl", "LeftShift", "Z"]),
                Binding::key(&["["]),
                Binding::key(&["]"]),
                Binding::key(&["LeftCtrl", "equal"]),
                Binding::key(&["LeftCtrl", "minus"]),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RingConfig {
    pub left: Binding,
    pub right: Binding,
    pub mode: u8,
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            left: Binding::Scroll {
                direction: "up".into(),
            },
            right: Binding::Scroll {
                direction: "down".into(),
            },
            mode: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MappingConfig {
    /// 0 / 90 / 180 / 270
    pub orientation: u16,
    pub tablet_x: f64,
    pub tablet_y: f64,
    pub tablet_w: f64,
    pub tablet_h: f64,
    /// 0 = schermi agganciati (whole desktop); 1-based index left-to-right.
    #[serde(default)]
    pub monitor: u32,
    #[serde(default)]
    pub keep_aspect: bool,
}

impl Default for MappingConfig {
    fn default() -> Self {
        Self {
            orientation: 0,
            tablet_x: 0.0,
            tablet_y: 0.0,
            tablet_w: 1.0,
            tablet_h: 1.0,
            monitor: 0,
            keep_aspect: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DriverConfig {
    pub pen: PenConfig,
    pub keys: KeysConfig,
    pub ring: RingConfig,
    pub mapping: MappingConfig,
}

impl DriverConfig {
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("deco-driver")
            .join("config.toml")
    }

    pub fn load_or_default() -> Self {
        match Self::load(&Self::default_path()) {
            Ok(c) => c,
            Err(_) => Self::default(),
        }
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn save_default(&self) -> Result<(), ConfigError> {
        self.save(&Self::default_path())
    }
}

/// GNOME `area` margins `[left, right, top, bottom]` as fractions of the tablet.
///
/// When `keep_aspect` is set, letterbox the tablet to the destination rectangle
/// (desktop bounding box or one monitor). GNOME keep-aspect stays off.
pub fn tablet_area_margins(
    mapping: &MappingConfig,
    dest_w: f64,
    dest_h: f64,
    keep_aspect: bool,
) -> [f64; 4] {
    let left = if mapping.tablet_x.abs() < 1e-6 {
        0.0
    } else {
        mapping.tablet_x.clamp(0.0, 1.0)
    };
    let top = if mapping.tablet_y.abs() < 1e-6 {
        0.0
    } else {
        mapping.tablet_y.clamp(0.0, 1.0)
    };
    let right = (1.0 - (mapping.tablet_x + mapping.tablet_w)).clamp(0.0, 1.0);
    let bottom = (1.0 - (mapping.tablet_y + mapping.tablet_h)).clamp(0.0, 1.0);
    if !keep_aspect || dest_w <= 0.0 || dest_h <= 0.0 {
        return [left, right, top, bottom];
    }
    let (tab_w, tab_h) = match mapping.orientation {
        90 | 270 => (
            deco_protocol::DECO02_HID_MAX_Y as f64,
            deco_protocol::DECO02_HID_MAX_X as f64,
        ),
        _ => (
            deco_protocol::DECO02_HID_MAX_X as f64,
            deco_protocol::DECO02_HID_MAX_Y as f64,
        ),
    };
    fit_area_to_dest(left, right, top, bottom, tab_w, tab_h, dest_w, dest_h)
}

fn fit_area_to_dest(
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    tab_w: f64,
    tab_h: f64,
    dest_w: f64,
    dest_h: f64,
) -> [f64; 4] {
    let rw = (1.0 - left - right).max(0.05);
    let rh = (1.0 - top - bottom).max(0.05);
    let src_aspect = (rw * tab_w) / (rh * tab_h);
    let dst_aspect = dest_w / dest_h;
    if (src_aspect - dst_aspect).abs() < 1e-6 {
        return [left, right, top, bottom];
    }
    if src_aspect > dst_aspect {
        let keep = rw * (dst_aspect / src_aspect);
        let extra = (rw - keep).max(0.0);
        [left + extra / 2.0, right + extra / 2.0, top, bottom]
    } else {
        let keep = rh * (src_aspect / dst_aspect);
        let extra = (rh - keep).max(0.0);
        [left, right, top + extra / 2.0, bottom + extra / 2.0]
    }
}

/// Map a tablet point through orientation + crop, then scale to uinput range.
pub fn map_point(
    x: u32,
    y: u32,
    src_max_x: u32,
    src_max_y: u32,
    dst_max_x: i32,
    dst_max_y: i32,
    mapping: &MappingConfig,
) -> (i32, i32) {
    let nx = if src_max_x == 0 {
        0.0
    } else {
        x as f64 / src_max_x as f64
    };
    let ny = if src_max_y == 0 {
        0.0
    } else {
        y as f64 / src_max_y as f64
    };
    let (mut nx, mut ny) = match mapping.orientation {
        90 => (ny, 1.0 - nx),
        180 => (1.0 - nx, 1.0 - ny),
        270 => (1.0 - ny, nx),
        _ => (nx, ny),
    };
    let w = mapping.tablet_w.max(0.05);
    let h = mapping.tablet_h.max(0.05);
    nx = ((nx - mapping.tablet_x) / w).clamp(0.0, 1.0);
    ny = ((ny - mapping.tablet_y) / h).clamp(0.0, 1.0);
    (
        (nx * dst_max_x as f64).round() as i32,
        (ny * dst_max_y as f64).round() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_curve_is_identity() {
        let c = PressureCurve::default();
        assert!((c.apply(0.0) - 0.0).abs() < 1e-6);
        assert!((c.apply(1.0) - 1.0).abs() < 1e-6);
        assert!((c.apply(0.5) - 0.5).abs() < 0.05);
    }

    #[test]
    fn map_identity() {
        let m = MappingConfig::default();
        let (x, y) = map_point(100, 50, 200, 100, 2000, 1000, &m);
        assert_eq!(x, 1000);
        assert_eq!(y, 500);
    }

    #[test]
    fn map_180() {
        let mut m = MappingConfig::default();
        m.orientation = 180;
        let (x, y) = map_point(0, 0, 100, 100, 100, 100, &m);
        assert_eq!(x, 100);
        assert_eq!(y, 100);
    }

    #[test]
    fn joined_area_uses_both_widths_and_tallest() {
        let mut m = MappingConfig::default();
        m.keep_aspect = true;
        let tab_w = deco_protocol::DECO02_HID_MAX_X as f64;
        let tab_h = deco_protocol::DECO02_HID_MAX_Y as f64;
        let dest_w = 1920.0 + 1920.0;
        let dest_h = 1200.0_f64.max(1080.0);
        let [left, right, top, bottom] = tablet_area_margins(&m, dest_w, dest_h, true);
        assert!(left.abs() < 1e-9 && right.abs() < 1e-9, "full tablet width");
        let used_h = 1.0 - top - bottom;
        let src_aspect = tab_w / (used_h * tab_h);
        let dst_aspect = dest_w / dest_h;
        assert!((src_aspect - dst_aspect).abs() < 1e-6);
        assert!((top - bottom).abs() < 1e-9, "letterbox is centered");
    }

    #[test]
    fn keep_aspect_1080p_letterboxes_16_10_tablet() {
        let mut m = MappingConfig::default();
        m.keep_aspect = true;
        let [left, right, top, bottom] = tablet_area_margins(&m, 1920.0, 1080.0, true);
        assert!(left.abs() < 1e-9 && right.abs() < 1e-9);
        assert!(top > 0.04 && top < 0.06, "expected ~5% unused top, got {top}");
        assert!((top - bottom).abs() < 1e-9);
        let tab_w = deco_protocol::DECO02_HID_MAX_X as f64;
        let tab_h = deco_protocol::DECO02_HID_MAX_Y as f64;
        let src_aspect = tab_w / ((1.0 - top - bottom) * tab_h);
        assert!((src_aspect - 1920.0 / 1080.0).abs() < 1e-6);
    }

    #[test]
    fn stretch_joined_uses_full_tablet() {
        let m = MappingConfig::default();
        let area = tablet_area_margins(&m, 3840.0, 1200.0, false);
        assert_eq!(area, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn roundtrip_toml() {
        let c = DriverConfig::default();
        let s = toml::to_string(&c).unwrap();
        let back: DriverConfig = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
