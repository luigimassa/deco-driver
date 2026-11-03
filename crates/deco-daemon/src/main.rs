mod actions;
mod dbus;
mod diag;
mod hid;
mod hotplug;
mod state;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use deco_config::DriverConfig;
use deco_input::InputDevices;
use deco_protocol::diagnose_event;
use deco_protocol::parse::{Caps, DeviceEvent, InterfaceRole};
use deco_protocol::{HOTPLUG_SETTLE, XP_PEN_VID};
use hidapi::HidApi;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;
use zbus::connection::Builder;

use crate::actions::ActionEngine;
use crate::dbus::{DriverService, DriverServiceSignals, PATH, SERVICE};
use crate::hid::{dump_reports, interface_role_at, list_deco02, open_and_init, read_loop};

struct HidReader {
    id: u64,
    path: String,
    role: InterfaceRole,
    stop: Arc<AtomicBool>,
}

fn digitizer_open(readers: &[HidReader]) -> bool {
    readers.iter().any(|r| r.role == InterfaceRole::Digitizer)
}
use crate::hotplug::PlugEvent;
use crate::state::{AppState, DeviceInfo};

fn main() {
    if std::env::args().any(|a| a == "--self-check" || a == "self-check") {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();
        std::process::exit(diag::run_cli());
    }
    if std::env::args().any(|a| a == "--validate" || a == "validate") {
        let secs = std::env::args()
            .find_map(|a| a.strip_prefix("--seconds=").map(|s| s.parse().ok()))
            .flatten()
            .unwrap_or(8);
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();
        match dump_reports(secs) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("validate: {e}");
                std::process::exit(2);
            }
        }
        return;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(e) = rt.block_on(run()) {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    diag::log_startup();

    let config = DriverConfig::load_or_default();
    let _ = config.save_default();
    let state = AppState::new(config);

    match InputDevices::open(
        deco_protocol::DEFAULT_MAX_X as i32,
        deco_protocol::DEFAULT_MAX_Y as i32,
        deco_protocol::DEFAULT_MAX_PRESSURE as i32,
    ) {
        Ok(devs) => {
            info!("uinput devices created");
            *state.input.lock().await = Some(devs);
        }
        Err(e) => {
            warn!(%e, "uinput unavailable — HID parse will still run, injection disabled");
        }
    }
    // hidraw is not exclusive: hide kernel "XP-PEN DECO 02 Stylus" so only uinput moves.
    let mut kernel_grab = deco_input::KernelGrab::default();
    kernel_grab.refresh(XP_PEN_VID, &deco_protocol::DECO02_PIDS);

    let service = DriverService {
        state: Arc::clone(&state),
    };
    let connection = Builder::session()?
        .name(SERVICE)?
        .serve_at(PATH, service)?
        .build()
        .await?;
    info!(service = SERVICE, "D-Bus name acquired");

    let (event_tx, event_rx) = mpsc::channel::<DeviceEvent>();
    let (plug_tx, plug_rx) = mpsc::channel::<PlugEvent>();
    let (exit_tx, exit_rx) = mpsc::channel::<u64>();
    hotplug::spawn(plug_tx.clone());

    // Initial scan after a short delay in case the tablet is already plugged.
    let tx0 = event_tx.clone();
    let plug0 = plug_tx.clone();
    tokio::task::spawn_blocking(move || {
        let Ok(api) = HidApi::new() else {
            return;
        };
        for (pid, path) in list_deco02(&api) {
            let _ = plug0.send(PlugEvent::Added { pid, path });
        }
        drop(tx0);
    });

    let mut engine = ActionEngine::default();
    let mut readers: Vec<HidReader> = Vec::new();
    let mut next_reader: u64 = 1;
    let mut last_pressure_emit = 0u32;
    let iface = connection
        .object_server()
        .interface::<_, DriverService>(PATH)
        .await?;

    let mut poll = tokio::time::interval(Duration::from_millis(20));
    let mut rescan = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = rescan.tick() => {
                if !digitizer_open(&readers) {
                    if let Ok(api) = HidApi::new() {
                        for (pid, path) in list_deco02(&api) {
                            let _ = plug_tx.send(PlugEvent::Added { pid, path });
                        }
                    }
                }
            }
            _ = poll.tick() => {}
        }
        while let Ok(id) = exit_rx.try_recv() {
            let role = readers.iter().find(|r| r.id == id).map(|r| r.role);
            readers.retain(|r| r.id != id);
            info!(id, ?role, "hid reader exited");
            if !digitizer_open(&readers) {
                *state.device.write().await = None;
                kernel_grab.clear();
            }
        }
        while let Ok(plug) = plug_rx.try_recv() {
            match plug {
                PlugEvent::Added { pid, path } => {
                    if path.is_empty() || readers.iter().any(|r| r.path == path) {
                        continue;
                    }
                    if interface_role_at(&path) == Some(InterfaceRole::Unused) {
                        debug!(%path, "skip mouse interface (no OUT endpoint)");
                        continue;
                    }
                    info!(pid = format_args!("{pid:#06x}"), %path, "device added");
                    if readers.is_empty() {
                        tokio::time::sleep(HOTPLUG_SETTLE).await;
                    }
                    let id = next_reader;
                    next_reader = next_reader.wrapping_add(1);
                    match attach(&state, &path, event_tx.clone(), exit_tx.clone(), id).await {
                        Ok(Some((info, stop, role))) => {
                            readers.push(HidReader {
                                id,
                                path: path.clone(),
                                role,
                                stop,
                            });
                            if role == InterfaceRole::Digitizer {
                                info!(
                                    pid = format_args!("{:#06x}", info.pid),
                                    name = %info.name,
                                    fw = %info.firmware,
                                    path = %path,
                                    interfaces = readers.len(),
                                    max_x = info.caps.max_x,
                                    max_y = info.caps.max_y,
                                    press = info.caps.max_pressure,
                                    "tablet attached"
                                );
                                debug_assert!(deco_protocol::is_deco02(info.pid));
                                debug_assert!(info.caps.max_x > 0 && info.caps.max_y > 0);
                                let _ = iface.device_added(info.pid, &info.name).await;
                                kernel_grab.refresh(XP_PEN_VID, &deco_protocol::DECO02_PIDS);
                            } else {
                                info!(%path, ?role, "command interface open");
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            error!(%e, pid = format_args!("{pid:#06x}"), %path, "failed to attach tablet")
                        }
                    }
                }
                PlugEvent::Removed { pid, path } => {
                    info!(pid = format_args!("{pid:#06x}"), %path, "device removed");
                    readers.retain(|r| {
                        if r.path == path || path.is_empty() {
                            r.stop.store(true, Ordering::Relaxed);
                            false
                        } else {
                            true
                        }
                    });
                    if !digitizer_open(&readers) {
                        *state.device.write().await = None;
                        kernel_grab.clear();
                        let _ = iface.device_removed(pid).await;
                    }
                }
            }
        }

        while let Ok(ev) = event_rx.try_recv() {
            if let DeviceEvent::Caps(caps) = ev {
                if let Some(dev) = state.device.write().await.as_mut() {
                    dev.caps = caps;
                }
                continue;
            }
            let cfg = state.config.read().await.clone();
            let caps = state
                .device
                .read()
                .await
                .as_ref()
                .map(|d| d.caps)
                .unwrap_or_default();
            for issue in diagnose_event(&ev, caps) {
                error!(%issue, "protocol assert");
            }
            if let Some(input) = state.input.lock().await.as_mut() {
                engine.handle(ev, &cfg, caps, &state.last_pressure, input);
            } else {
                warn!("dropping input event — uinput was not opened");
            }
        }

        let p = state.pressure();
        if p != last_pressure_emit {
            last_pressure_emit = p;
            let _ = iface.pressure_sample(p).await;
        }

        if !state.running.load(Ordering::Relaxed) {
            break;
        }
    }

    info!("daemon stopping");
    let _ = XP_PEN_VID;
    Ok(())
}

async fn attach(
    state: &Arc<AppState>,
    path: &str,
    event_tx: mpsc::Sender<DeviceEvent>,
    exit_tx: mpsc::Sender<u64>,
    reader_id: u64,
) -> Result<Option<(DeviceInfo, Arc<AtomicBool>, InterfaceRole)>, String> {
    let path_owned = path.to_string();
    let (dev, info, role) = tokio::task::spawn_blocking(move || {
        let api = HidApi::new().map_err(|e| e.to_string())?;
        // Prefer the hidraw path; if empty, open first Deco 02.
        if path_owned.is_empty() {
            let list = list_deco02(&api);
            let (_, p) = list
                .first()
                .cloned()
                .ok_or_else(|| "no device".to_string())?;
            open_and_init(&api, &p)
        } else {
            open_and_init(&api, &path_owned)
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    if role == InterfaceRole::Unused {
        info!(path, "closed unused interface without writing");
        return Ok(None);
    }

    // Only the digitizer owns the published device. The command interface
    // has no axes; letting it overwrite caps hides the pen.
    if role == InterfaceRole::Digitizer {
        let caps: Caps = info.caps;
        *state.device.write().await = Some(info.clone());
        let mut slot = state.input.lock().await;
        let need_recreate = slot.as_ref().map_or(true, |input| {
            input.max_x != caps.max_x.max(1) as i32 || input.max_y != caps.max_y.max(1) as i32
        });
        if need_recreate {
            match InputDevices::open(
                caps.max_x.max(1) as i32,
                caps.max_y.max(1) as i32,
                caps.max_pressure.max(1) as i32,
            ) {
                Ok(devs) => {
                    info!(
                        max_x = caps.max_x,
                        max_y = caps.max_y,
                        "uinput recreated with HID caps"
                    );
                    *slot = Some(devs);
                }
                Err(e) => warn!(%e, "uinput recreate failed"),
            }
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let release = role == InterfaceRole::Command;
    std::thread::Builder::new()
        .name("deco-hid".into())
        .spawn(move || read_loop(dev, event_tx, stop2, release, reader_id, exit_tx))
        .map_err(|e| e.to_string())?;
    Ok(Some((info, stop, role)))
}
