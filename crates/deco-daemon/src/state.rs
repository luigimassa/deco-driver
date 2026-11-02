use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use deco_config::DriverConfig;
use deco_input::InputDevices;
use deco_protocol::parse::Caps;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub pid: u16,
    pub name: String,
    #[allow(dead_code)]
    pub path: String,
    pub firmware: String,
    pub caps: Caps,
}

pub struct AppState {
    pub config: RwLock<DriverConfig>,
    pub device: RwLock<Option<DeviceInfo>>,
    pub input: Mutex<Option<InputDevices>>,
    pub last_pressure: AtomicU32,
    pub running: AtomicBool,
}

impl AppState {
    pub fn new(config: DriverConfig) -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(config),
            device: RwLock::new(None),
            input: Mutex::new(None),
            last_pressure: AtomicU32::new(0),
            running: AtomicBool::new(true),
        })
    }

    pub fn pressure(&self) -> u32 {
        self.last_pressure.load(Ordering::Relaxed)
    }
}
