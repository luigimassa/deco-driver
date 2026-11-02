use std::time::Duration;

use deco_protocol::commands::{init_commands, parse_indexed_caps, LINUX_RELEASE};
use deco_protocol::parse::{
    parse_digitizer_rdesc, parse_report, Caps, DeviceEvent, DECO02_HID_MAX_PRESSURE,
    DECO02_HID_MAX_X, DECO02_HID_MAX_Y,
};
use std::ffi::CString;

use deco_protocol::{
    device_name, diagnose_event, hex_dump, HID_POLL_TIMEOUT, HID_REPORT_LEN, XP_PEN_VID,
};
use hidapi::{HidApi, HidDevice};
use tracing::{debug, error, info, warn};

use crate::state::DeviceInfo;

pub fn list_deco02(api: &HidApi) -> Vec<(u16, String)> {
    let mut seen = std::collections::HashSet::new();
    let deco: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == XP_PEN_VID && deco_protocol::is_deco02(d.product_id()))
        .filter_map(|d| {
            let path = d.path().to_string_lossy().into_owned();
            seen.insert(path.clone())
                .then_some((d.product_id(), path))
        })
        .collect();
    let others: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == XP_PEN_VID && !deco_protocol::is_deco02(d.product_id()))
        .map(|d| format!("28bd:{:04x}", d.product_id()))
        .collect();
    if deco.is_empty() && !others.is_empty() {
        warn!(
            other = others.join(","),
            "XP-Pen USB present but PID is not Deco 02 (0803/0903)"
        );
    }
    deco
}

pub fn open_and_init(api: &HidApi, path: &str) -> Result<(HidDevice, DeviceInfo), String> {
    info!(%path, "opening hidraw");
    let cpath = CString::new(path).map_err(|e| e.to_string())?;
    let dev = api.open_path(&cpath).map_err(|e| {
        error!(%path, %e, "hid_open_path failed");
        format!("open {path}: {e}")
    })?;
    let pid = dev
        .get_device_info()
        .map(|i| i.product_id())
        .unwrap_or(0x0803);
    let mut firmware = String::new();
    if let Ok(Some(fw)) = dev.get_indexed_string(0x6E) {
        firmware = fw;
    }
    let mut caps = Caps {
        max_x: DECO02_HID_MAX_X,
        max_y: DECO02_HID_MAX_Y,
        max_pressure: DECO02_HID_MAX_PRESSURE,
        resolution: deco_protocol::DEFAULT_RESOLUTION_LPI,
    };
    if let Some(name) = path.rsplit('/').next() {
        let rdesc_path = format!("/sys/class/hidraw/{name}/device/report_descriptor");
        match std::fs::read(&rdesc_path) {
            Ok(bytes) => {
                if let Some(c) = parse_digitizer_rdesc(&bytes) {
                    info!(
                        max_x = c.max_x,
                        max_y = c.max_y,
                        press = c.max_pressure,
                        "caps from HID report descriptor"
                    );
                    caps = c;
                } else {
                    warn!(n = bytes.len(), "report descriptor has no digitizer axes");
                }
            }
            Err(e) => warn!(%e, %rdesc_path, "cannot read report descriptor"),
        }
    }
    match dev.get_indexed_string(100) {
        Ok(Some(s)) => {
            let mut bytes = Vec::new();
            for ch in s.encode_utf16() {
                bytes.extend_from_slice(&ch.to_le_bytes());
            }
            debug!(n = bytes.len(), hex = %hex_dump(&bytes), "indexed string 100");
            if let Some(c) = parse_indexed_caps(&bytes) {
                // Windows 24-bit space (often 0xCC64). Live reports are HID 0x07.
                info!(?c, "indexed string 100 caps ignored in favour of HID rdesc");
            } else {
                warn!(hex = %hex_dump(&bytes), "indexed string 100 did not parse as MaxX/Y/Press");
            }
        }
        Ok(None) => warn!("indexed string 100 empty"),
        Err(e) => warn!(%e, "hid_get_indexed_string(100) failed — using default caps"),
    }
    debug_assert!(caps.max_x > 0 && caps.max_y > 0);
    for cmd in init_commands() {
        debug_assert_eq!(cmd[0], 0x02);
        match dev.write(&cmd) {
            Ok(n) => debug!(n, hex = %hex_dump(&cmd), "init write"),
            Err(e) => error!(%e, hex = %hex_dump(&cmd), "init write failed"),
        }
    }
    // Give the device a moment to reply with a 0x06 'd' caps frame.
    let mut buf = [0u8; HID_REPORT_LEN];
    if let Ok(n) = dev.read_timeout(&mut buf, 150) {
        if n > 0 {
            let parsed = parse_report(&buf[..n]);
            for ev in parsed.events {
                if let DeviceEvent::Caps(c) = ev {
                    info!(?c, hex = %hex_dump(&buf[..n]), "ignoring vendor 0x06 'd' caps; using HID rdesc");
                    for issue in diagnose_event(&DeviceEvent::Caps(c), c) {
                        error!(%issue, "caps assert");
                    }
                }
            }
        }
    }
    info!(
        pid = format_args!("{pid:#06x}"),
        max_x = caps.max_x,
        max_y = caps.max_y,
        press = caps.max_pressure,
        firmware = %firmware,
        "tablet opened"
    );
    Ok((
        dev,
        DeviceInfo {
            pid,
            name: device_name(pid).to_string(),
            path: path.to_string(),
            firmware,
            caps,
        },
    ))
}

pub fn read_loop(
    dev: HidDevice,
    tx: std::sync::mpsc::Sender<DeviceEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut buf = [0u8; HID_REPORT_LEN];
    let timeout = HID_POLL_TIMEOUT.as_millis() as i32;
    let mut seen = 0u32;
    let mut errors = 0u32;
    let mut last_kind = String::new();
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        match dev.read_timeout(&mut buf, timeout) {
            Ok(0) => continue,
            Ok(n) => {
                errors = 0;
                let parsed = parse_report(&buf[..n]);
                seen += 1;
                if seen <= 24 || buf[0] != 0x07 {
                    info!(
                        n,
                        hex = %hex_dump(&buf[..n]),
                        events = ?parsed.events,
                        "hid frame"
                    );
                }
                for ev in parsed.events {
                    if matches!(ev, DeviceEvent::Unknown) {
                        warn!(n, hex = %hex_dump(&buf[..n]), "unparsed HID report");
                        continue;
                    }
                    if matches!(ev, DeviceEvent::Ack) {
                        debug!(hex = %hex_dump(&buf[..n]), "ack");
                        continue;
                    }
                    let kind = format!("{ev:?}");
                    if kind != last_kind && seen <= 64 {
                        info!(event = %kind, "event kind");
                        last_kind = kind;
                    }
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                errors += 1;
                if errors == 1 || errors % 25 == 0 {
                    error!(%e, errors, "hid read error (unplug? permissions?)");
                }
                std::thread::sleep(Duration::from_millis(100));
                if errors > 50 {
                    error!("too many hid errors — leaving reader");
                    break;
                }
            }
        }
    }
    info!(seen, "hid reader stopping, sending LinuxRelease");
    if let Err(e) = dev.write(&LINUX_RELEASE) {
        warn!(%e, "LinuxRelease write failed");
    }
}

pub fn dump_reports(seconds: u64) -> Result<(), String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let devices = list_deco02(&api);
    if devices.is_empty() {
        return Err("no Deco 02 (28bd:0803/0903) found — plug the tablet and retry".into());
    }
    let (pid, path) = &devices[0];
    println!("opening {path} pid={pid:#06x}");
    let (dev, info) = open_and_init(&api, path)?;
    println!(
        "caps max_x={} max_y={} press={} fw={}",
        info.caps.max_x, info.caps.max_y, info.caps.max_pressure, info.firmware
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    let mut buf = [0u8; HID_REPORT_LEN];
    while std::time::Instant::now() < deadline {
        if let Ok(n) = dev.read_timeout(&mut buf, 100) {
            if n == 0 {
                continue;
            }
            print!("raw");
            for b in &buf[..n.min(16)] {
                print!(" {b:02x}");
            }
            println!();
            let parsed = parse_report(&buf[..n]);
            println!("  -> {:?}", parsed.events);
            for ev in &parsed.events {
                for issue in diagnose_event(ev, info.caps) {
                    println!("  ASSERT {issue}");
                }
            }
        }
    }
    let _ = dev.write(&LINUX_RELEASE);
    Ok(())
}
