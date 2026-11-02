//! Install / plug-in self-check. Prints OK/FAIL lines and returns them for D-Bus.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use deco_protocol::{device_name, is_deco02, XP_PEN_VID};
use hidapi::HidApi;
use serde::Serialize;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
        }
    }

    pub fn log(&self) {
        if self.ok {
            info!(check = self.name, detail = %self.detail, "OK");
        } else {
            error!(check = self.name, detail = %self.detail, "FAIL");
        }
    }
}

pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn failed(&self) -> usize {
        self.checks.iter().filter(|c| !c.ok).count()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for c in &self.checks {
            let tag = if c.ok { "OK  " } else { "FAIL" };
            out.push_str(&format!(
                "[{tag}] {name}: {detail}\n",
                name = c.name,
                detail = c.detail
            ));
        }
        out.push_str(&format!(
            "{} checks, {} failed\n",
            self.checks.len(),
            self.failed()
        ));
        out
    }
}

fn mode_of(path: &str) -> Option<u32> {
    fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

fn process_running(name: &str) -> bool {
    let Ok(out) = Command::new("ps").args(["-C", name, "-o", "pid="]).output() else {
        return false;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

fn udev_rule_installed() -> Option<String> {
    for p in [
        "/etc/udev/rules.d/99-deco-driver.rules",
        "/lib/udev/rules.d/99-deco-driver.rules",
        "/usr/lib/udev/rules.d/99-deco-driver.rules",
        "/lib/udev/rules.d/10-xp-pen.rules",
        "/etc/udev/rules.d/10-xp-pen.rules",
    ] {
        if Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

/// Probe the machine. Never panics: failures become FAIL rows.
pub fn collect() -> Report {
    let mut checks = Vec::new();

    let uinput_path = ["/dev/uinput", "/dev/input/uinput"]
        .into_iter()
        .find(|p| Path::new(p).exists());
    match uinput_path {
        Some(p) => {
            let mode = mode_of(p).unwrap_or(0);
            match fs::OpenOptions::new().read(true).write(true).open(p) {
                Ok(_) => checks.push(Check::ok("uinput", format!("{p} mode={mode:o} writable"))),
                Err(e) => checks.push(Check::fail(
                    "uinput",
                    format!("{p} mode={mode:o} not writable: {e} (install udev rule + re-login)"),
                )),
            }
        }
        None => checks.push(Check::fail(
            "uinput",
            "/dev/uinput missing (modprobe uinput)",
        )),
    }

    match udev_rule_installed() {
        Some(p) => checks.push(Check::ok("udev-rule", p)),
        None => checks.push(Check::fail(
            "udev-rule",
            "no 99-deco-driver.rules (or vendor 10-xp-pen.rules) — hidraw/uinput ACL may fail",
        )),
    }

    if process_running("PenTablet") || process_running("PenTablet.sh") {
        checks.push(Check::fail(
            "vendor-driver",
            "XP-Pen PenTablet is running — two uinput stacks will fight. Uninstall it.",
        ));
    } else {
        checks.push(Check::ok("vendor-driver", "PenTablet not running"));
    }

    match HidApi::new() {
        Ok(api) => {
            let xp: Vec<_> = api
                .device_list()
                .filter(|d| d.vendor_id() == XP_PEN_VID)
                .map(|d| {
                    (
                        d.product_id(),
                        d.path().to_string_lossy().into_owned(),
                        d.product_string().unwrap_or("").to_string(),
                    )
                })
                .collect();
            if xp.is_empty() {
                checks.push(Check::fail(
                    "usb",
                    "no USB VID 28bd — plug the Deco 02 (or check cable/port)",
                ));
            } else {
                let deco: Vec<_> = xp.iter().filter(|(pid, ..)| is_deco02(*pid)).collect();
                let listing = xp
                    .iter()
                    .map(|(pid, path, name)| format!("28bd:{pid:04x} {name} {path}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                if deco.is_empty() {
                    checks.push(Check::fail(
                        "usb",
                        format!("XP-Pen present but not Deco 02 (0803/0903): {listing}"),
                    ));
                } else {
                    checks.push(Check::ok(
                        "usb",
                        format!("{} ({})", device_name(deco[0].0), listing),
                    ));
                }
            }

            let hidraw: Vec<_> = glob_hidraw();
            if xp.is_empty() {
                checks.push(Check::ok(
                    "hidraw",
                    format!("{} nodes (no XP-Pen until plug)", hidraw.len()),
                ));
            } else if hidraw.is_empty() {
                checks.push(Check::fail(
                    "hidraw",
                    "XP-Pen USB seen but no /dev/hidraw* — kernel hid not bound",
                ));
            } else {
                let unreadable: Vec<_> = hidraw
                    .iter()
                    .filter(|p| fs::File::open(p).is_err())
                    .cloned()
                    .collect();
                if unreadable.len() == hidraw.len() {
                    checks.push(Check::fail(
                        "hidraw",
                        format!(
                            "{} — none readable (udev TAG+=uaccess, then replug)",
                            hidraw.join(", ")
                        ),
                    ));
                } else {
                    checks.push(Check::ok("hidraw", hidraw.join(", ")));
                }
            }
        }
        Err(e) => checks.push(Check::fail("hidapi", format!("HidApi::new failed: {e}"))),
    }

    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "?".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into());
    checks.push(Check::ok(
        "session",
        format!("type={session} desktop={desktop}"),
    ));
    if session.eq_ignore_ascii_case("tty") {
        warn!("not in a graphical session — uinput still works, UI will not");
    }

    let cfg = deco_config::DriverConfig::default_path();
    if cfg.exists() {
        checks.push(Check::ok("config", cfg.display().to_string()));
    } else {
        checks.push(Check::ok(
            "config",
            format!(
                "{} (not created yet, defaults will be written)",
                cfg.display()
            ),
        ));
    }

    Report { checks }
}

fn glob_hidraw() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/dev") {
        for e in rd.flatten() {
            let n = e.file_name();
            let s = n.to_string_lossy();
            if s.starts_with("hidraw") {
                v.push(format!("/dev/{s}"));
            }
        }
    }
    v.sort();
    v
}

pub fn run_cli() -> i32 {
    let report = collect();
    for c in &report.checks {
        c.log();
        let tag = if c.ok { "OK  " } else { "FAIL" };
        eprintln!("[{tag}] {}: {}", c.name, c.detail);
    }
    let n = report.failed();
    if n == 0 {
        eprintln!("self-check: all {} checks passed", report.checks.len());
        0
    } else {
        eprintln!("self-check: {n} failed — see FAIL lines above");
        2
    }
}

pub fn log_startup() {
    info!("self-check at daemon start");
    let report = collect();
    for c in &report.checks {
        c.log();
    }
    if report.failed() > 0 {
        error!(
            failed = report.failed(),
            "self-check found problems; daemon continues (tablet may still attach later)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_text_has_counts() {
        let r = Report {
            checks: vec![Check::ok("a", "yes"), Check::fail("b", "no")],
        };
        assert_eq!(r.failed(), 1);
        let t = r.to_text();
        assert!(t.contains("[FAIL] b"));
        assert!(t.contains("1 failed"));
    }
}
