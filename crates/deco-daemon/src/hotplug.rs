use std::sync::mpsc::Sender;

use deco_protocol::{is_deco02, XP_PEN_VID};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub enum PlugEvent {
    Added { pid: u16, path: String },
    Removed { pid: u16, path: String },
}

pub fn spawn(tx: Sender<PlugEvent>) {
    std::thread::Builder::new()
        .name("deco-udev".into())
        .spawn(move || {
            if let Err(e) = run(tx) {
                warn!(%e, "udev monitor exited");
            }
        })
        .expect("spawn udev thread");
}

fn run(tx: Sender<PlugEvent>) -> Result<(), String> {
    let socket = udev::MonitorBuilder::new()
        .map_err(|e| e.to_string())?
        .match_subsystem("hidraw")
        .map_err(|e| e.to_string())?
        .listen()
        .map_err(|e| e.to_string())?;

    info!("udev hidraw monitor started");
    loop {
        let mut saw = false;
        for event in socket.iter() {
            saw = true;
            let action = event.action().map(|s| s.to_string_lossy().into_owned());
            let Some(dev) = event.device().parent() else {
                continue;
            };
            let vid = attr_hex(&dev, "idVendor").or_else(|| walk_vid_pid(&event.device(), true));
            let pid = attr_hex(&dev, "idProduct").or_else(|| walk_vid_pid(&event.device(), false));
            let (Some(vid), Some(pid)) = (vid, pid) else {
                continue;
            };
            if vid != XP_PEN_VID || !is_deco02(pid) {
                continue;
            }
            let path = event
                .device()
                .devnode()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            match action.as_deref() {
                Some("add") | Some("bind") => {
                    debug!(pid = format_args!("{pid:#06x}"), %path, "udev add");
                    let _ = tx.send(PlugEvent::Added { pid, path });
                }
                Some("remove") | Some("unbind") => {
                    debug!(pid = format_args!("{pid:#06x}"), %path, "udev remove");
                    let _ = tx.send(PlugEvent::Removed { pid, path });
                }
                _ => {}
            }
        }
        if !saw {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
}

fn attr_hex(dev: &udev::Device, key: &str) -> Option<u16> {
    let raw = dev.attribute_value(key)?;
    u16::from_str_radix(raw.to_string_lossy().trim(), 16).ok()
}

fn walk_vid_pid(dev: &udev::Device, vendor: bool) -> Option<u16> {
    let mut cur = Some(dev.clone());
    while let Some(d) = cur {
        let key = if vendor { "idVendor" } else { "idProduct" };
        if let Some(v) = attr_hex(&d, key) {
            return Some(v);
        }
        cur = d.parent();
    }
    None
}
