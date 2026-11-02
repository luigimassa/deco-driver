use std::sync::Arc;

use deco_config::{Binding, DriverConfig, MappingConfig, PressureCurve};
use tracing::{info, warn};
use zbus::fdo;
use zbus::interface;
use zbus::object_server::SignalEmitter;

use crate::state::AppState;

pub const SERVICE: &str = "it.bwlab.DecoDriver";
pub const PATH: &str = "/it/bwlab/DecoDriver";

pub struct DriverService {
    pub state: Arc<AppState>,
}

#[interface(name = "it.bwlab.DecoDriver1")]
impl DriverService {
    #[zbus(property)]
    async fn device_present(&self) -> bool {
        self.state.device.read().await.is_some()
    }

    #[zbus(property)]
    async fn pid(&self) -> u16 {
        self.state
            .device
            .read()
            .await
            .as_ref()
            .map(|d| d.pid)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn name(&self) -> String {
        self.state
            .device
            .read()
            .await
            .as_ref()
            .map(|d| d.name.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn max_pressure(&self) -> u32 {
        self.state
            .device
            .read()
            .await
            .as_ref()
            .map(|d| d.caps.max_pressure)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn firmware(&self) -> String {
        self.state
            .device
            .read()
            .await
            .as_ref()
            .map(|d| d.firmware.clone())
            .unwrap_or_default()
    }

    #[zbus(property(emits_changed_signal = "false"))]
    async fn last_pressure(&self) -> u32 {
        self.state.pressure()
    }

    async fn get_config(&self) -> fdo::Result<String> {
        let cfg = self.state.config.read().await;
        serde_json::to_string(&*cfg).map_err(|e| fdo::Error::Failed(e.to_string()))
    }

    async fn set_config(&self, json: String) -> fdo::Result<()> {
        let parsed: DriverConfig =
            serde_json::from_str(&json).map_err(|e| fdo::Error::InvalidArgs(e.to_string()))?;
        {
            let mut cfg = self.state.config.write().await;
            *cfg = parsed;
            if let Err(e) = cfg.save_default() {
                warn!(%e, "failed to persist config");
            }
        }
        info!("config replaced via D-Bus");
        Ok(())
    }

    async fn set_key_binding(&self, index: u8, kind: String, payload: String) -> fdo::Result<()> {
        if index as usize >= 6 {
            return Err(fdo::Error::InvalidArgs("key index 0..5".into()));
        }
        let binding = parse_binding(&kind, &payload)?;
        let mut cfg = self.state.config.write().await;
        cfg.keys.k[index as usize] = binding;
        cfg.save_default()
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn set_ring_binding(
        &self,
        side: String,
        kind: String,
        payload: String,
    ) -> fdo::Result<()> {
        let binding = parse_binding(&kind, &payload)?;
        let mut cfg = self.state.config.write().await;
        match side.as_str() {
            "left" => cfg.ring.left = binding,
            "right" => cfg.ring.right = binding,
            _ => return Err(fdo::Error::InvalidArgs("side must be left|right".into())),
        }
        cfg.save_default()
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn set_pressure_curve(&self, p0: f64, p1x: f64, p1y: f64, p2: f64) -> fdo::Result<()> {
        let mut cfg = self.state.config.write().await;
        cfg.pen.curve = PressureCurve { p0, p1x, p1y, p2 };
        cfg.save_default()
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn set_mapping(
        &self,
        orientation: u16,
        tablet_x: f64,
        tablet_y: f64,
        tablet_w: f64,
        tablet_h: f64,
        monitor: u32,
    ) -> fdo::Result<()> {
        let mut cfg = self.state.config.write().await;
        cfg.mapping = MappingConfig {
            orientation,
            tablet_x,
            tablet_y,
            tablet_w,
            tablet_h,
            monitor,
            keep_aspect: cfg.mapping.keep_aspect,
        };
        cfg.save_default()
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn reset_defaults(&self) -> fdo::Result<()> {
        let mut cfg = self.state.config.write().await;
        *cfg = DriverConfig::default();
        cfg.save_default()
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn list_monitors(&self) -> Vec<String> {
        mutter_monitors()
            .await
            .unwrap_or_else(|| vec!["Primary".into()])
    }

    async fn self_check(&self) -> String {
        crate::diag::collect().to_text()
    }

    #[zbus(signal)]
    async fn device_added(emitter: &SignalEmitter<'_>, pid: u16, name: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn device_removed(emitter: &SignalEmitter<'_>, pid: u16) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn pressure_sample(emitter: &SignalEmitter<'_>, value: u32) -> zbus::Result<()>;
}

fn parse_binding(kind: &str, payload: &str) -> fdo::Result<Binding> {
    match kind {
        "disabled" => Ok(Binding::Disabled),
        "eraser-toggle" => Ok(Binding::EraserToggle),
        "key" => Ok(Binding::Key {
            keys: payload
                .split('+')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }),
        "mouse" => Ok(Binding::Mouse {
            action: payload.to_string(),
        }),
        "scroll" => Ok(Binding::Scroll {
            direction: payload.to_string(),
        }),
        "builtin" => {
            let id = payload
                .parse()
                .map_err(|_| fdo::Error::InvalidArgs("builtin id".into()))?;
            Ok(Binding::Builtin { id })
        }
        other => Err(fdo::Error::InvalidArgs(format!("unknown kind {other}"))),
    }
}

async fn mutter_monitors() -> Option<Vec<String>> {
    let conn = zbus::Connection::session().await.ok()?;
    let reply = conn
        .call_method(
            Some("org.gnome.Mutter.DisplayConfig"),
            "/org/gnome/Mutter/DisplayConfig",
            Some("org.gnome.Mutter.DisplayConfig"),
            "GetCurrentState",
            &(),
        )
        .await
        .ok()?;
    // (uint serial, a(cxa{sv}) monitors, …) — pull connector names if possible.
    let body = reply.body();
    let parsed: Result<(u32, Vec<(String, String, String, String)>), _> = body.deserialize();
    if let Ok((_serial, monitors)) = parsed {
        let names = monitors
            .into_iter()
            .map(|m| {
                if m.1.is_empty() {
                    m.0
                } else {
                    format!("{} ({})", m.1, m.0)
                }
            })
            .collect();
        return Some(names);
    }
    None
}
