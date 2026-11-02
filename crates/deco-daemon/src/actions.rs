use std::collections::HashMap;

use deco_config::{Binding, DriverConfig};
use deco_input::{
    hid_usage_evdev, keycode, InputDevices, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, KEY_DOWN,
    KEY_LEFTALT, KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_PAGEDOWN, KEY_PAGEUP, KEY_UP,
};
use deco_protocol::parse::{Caps, DeviceEvent, KeyState, PenState, RingDirection};
use tracing::{debug, info, warn};

pub struct ActionEngine {
    last_keys: [bool; 12],
    last_pen_btns: u8,
    last_x: i32,
    last_y: i32,
    passthrough_held: Vec<u16>,
    /// Firmware HID combo (mods, usage) → UI slot 0..5. Live K5/K6 are claimed
    /// so unknown keys fill Tasto 1–4 in first-seen order.
    fingerprints: HashMap<(u8, u8), usize>,
}

impl Default for ActionEngine {
    fn default() -> Self {
        let mut fingerprints = HashMap::new();
        fingerprints.insert((0x05, 0x1D), 4); // Ctrl+Alt+Z → K5
        fingerprints.insert((0x01, 0x16), 5); // Ctrl+S → K6
        Self {
            last_keys: [false; 12],
            last_pen_btns: 0,
            last_x: 0,
            last_y: 0,
            passthrough_held: Vec::new(),
            fingerprints,
        }
    }
}

impl ActionEngine {
    pub fn handle(
        &mut self,
        event: DeviceEvent,
        cfg: &DriverConfig,
        caps: Caps,
        last_pressure: &std::sync::atomic::AtomicU32,
        input: &mut InputDevices,
    ) {
        match event {
            DeviceEvent::Pen(pen) => self.on_pen(pen, cfg, caps, last_pressure, input),
            DeviceEvent::Keys(keys) => {
                self.release_passthrough(input);
                self.on_keys(keys, cfg, input);
            }
            DeviceEvent::HidPassthrough { mods, usages } => {
                self.on_passthrough(mods, &usages, cfg, input);
            }
            DeviceEvent::Ring(dir) => self.on_ring(dir, cfg, input),
            DeviceEvent::Caps(caps) => {
                debug!(?caps, "caps update");
            }
            DeviceEvent::Handshake => {
                debug!("F8 handshake (ignored on Deco 02)");
            }
            DeviceEvent::Mode(m) => debug!(m, "device mode"),
            DeviceEvent::Ack | DeviceEvent::Unknown => {}
        }
    }

    fn on_pen(
        &mut self,
        pen: PenState,
        cfg: &DriverConfig,
        caps: Caps,
        last_pressure: &std::sync::atomic::AtomicU32,
        input: &mut InputDevices,
    ) {
        let (x, y) = deco_config::map_point(
            pen.x,
            pen.y,
            caps.max_x.max(1),
            caps.max_y.max(1),
            input.max_x,
            input.max_y,
            &cfg.mapping,
        );

        let t = if caps.max_pressure == 0 {
            0.0
        } else {
            pen.pressure as f64 / caps.max_pressure as f64
        };
        let curved = cfg.pen.curve.apply(t);
        let pressure = (curved * input.max_pressure as f64).round() as i32;
        let stored = pressure.max(0) as u32;
        last_pressure.store(stored, std::sync::atomic::Ordering::Relaxed);
        if stored > 0 && pen.tip {
            tracing::debug!(raw = pen.pressure, curved = stored, "pen pressure");
        }

        if cfg.pen.relative && pen.in_range {
            let speed = (cfg.pen.relative_speed.max(1) as i32).min(20);
            let dx = (x - self.last_x) * speed / 5;
            let dy = (y - self.last_y) * speed / 5;
            if dx != 0 || dy != 0 {
                let _ = input.mouse.rel(dx, dy);
            }
        } else {
            let _ = input.update_digit(
                x,
                y,
                pressure,
                i32::from(pen.tilt_x),
                i32::from(pen.tilt_y),
                pen.in_range,
                pen.tip,
                pen.btn_pressed & 1 != 0,
                pen.btn_pressed & 2 != 0,
            );
        }
        self.last_x = x;
        self.last_y = y;

        let changed = pen.btn_pressed ^ self.last_pen_btns;
        if changed & 1 != 0 {
            fire(&cfg.pen.btn1, pen.btn_pressed & 1 != 0, input);
        }
        if changed & 2 != 0 {
            fire(&cfg.pen.btn2, pen.btn_pressed & 2 != 0, input);
        }
        self.last_pen_btns = pen.btn_pressed;
    }

    fn on_keys(&mut self, keys: KeyState, cfg: &DriverConfig, input: &mut InputDevices) {
        for i in 0..6 {
            if keys.pressed[i] != self.last_keys[i] {
                info!(slot = i + 1, pressed = keys.pressed[i], "express key");
                fire(&cfg.keys.k[i], keys.pressed[i], input);
            }
        }
        self.last_keys = keys.pressed;
    }

    fn on_passthrough(
        &mut self,
        mods: u8,
        usages: &[u8],
        cfg: &DriverConfig,
        input: &mut InputDevices,
    ) {
        self.release_passthrough(input);
        if let Some(slot) = self.slot_for_fingerprint(mods, usages) {
            let mut pressed = [false; 12];
            pressed[slot] = true;
            self.on_keys(KeyState { pressed }, cfg, input);
            return;
        }
        let codes = hid_combo_codes(mods, usages);
        if codes.is_empty() {
            return;
        }
        info!(?codes, "hid passthrough press");
        for c in &codes {
            let _ = input.kbd.key(*c, true);
        }
        self.passthrough_held = codes;
    }

    fn slot_for_fingerprint(&mut self, mods: u8, usages: &[u8]) -> Option<usize> {
        let key = *usages.first()?;
        if let Some(&slot) = self.fingerprints.get(&(mods, key)) {
            return Some(slot);
        }
        let used: Vec<usize> = self.fingerprints.values().copied().collect();
        let slot = (0..6).find(|i| !used.contains(i))?;
        info!(slot = slot + 1, mods, key, "learned express key fingerprint");
        self.fingerprints.insert((mods, key), slot);
        Some(slot)
    }

    fn release_passthrough(&mut self, input: &mut InputDevices) {
        if self.passthrough_held.is_empty() {
            return;
        }
        for c in self.passthrough_held.iter().rev() {
            let _ = input.kbd.key(*c, false);
        }
        self.passthrough_held.clear();
    }

    fn on_ring(&mut self, dir: RingDirection, cfg: &DriverConfig, input: &mut InputDevices) {
        let binding = match dir {
            RingDirection::Left => &cfg.ring.left,
            RingDirection::Right => &cfg.ring.right,
        };
        tracing::info!(?dir, "roller");
        fire(binding, true, input);
        fire(binding, false, input);
    }
}

fn hid_combo_codes(mods: u8, usages: &[u8]) -> Vec<u16> {
    let mut codes = Vec::new();
    if mods & 0x01 != 0 {
        codes.push(KEY_LEFTCTRL);
    }
    if mods & 0x02 != 0 {
        codes.push(KEY_LEFTSHIFT);
    }
    if mods & 0x04 != 0 {
        codes.push(KEY_LEFTALT);
    }
    if mods & 0x08 != 0 {
        codes.push(KEY_LEFTMETA);
    }
    for u in usages {
        if let Some(k) = hid_usage_evdev(*u) {
            codes.push(k);
        }
    }
    codes
}

fn ctrl_zoom_delta(keys: &[String]) -> Option<i32> {
    if keys.len() != 2 {
        return None;
    }
    let lower: Vec<String> = keys.iter().map(|k| k.to_ascii_lowercase()).collect();
    let ctrl = lower
        .iter()
        .any(|k| k == "leftctrl" || k == "ctrl" || k == "control");
    if !ctrl {
        return None;
    }
    if lower.iter().any(|k| k == "equal" || k == "plus" || k == "kpplus") {
        Some(1)
    } else if lower.iter().any(|k| k == "minus" || k == "kpminus") {
        Some(-1)
    } else {
        None
    }
}

fn fire(binding: &Binding, pressed: bool, input: &mut InputDevices) {
    match binding {
        Binding::Disabled => {}
        Binding::Key { keys } => {
            if let Some(delta) = ctrl_zoom_delta(keys) {
                if pressed {
                    info!(delta, "zoom");
                    if let Err(e) = input.kbd.zoom(delta) {
                        warn!(%e, "zoom failed");
                    }
                }
                return;
            }
            let codes: Vec<u16> = keys.iter().filter_map(|n| keycode(n)).collect();
            if codes.is_empty() {
                warn!(?keys, "unknown key names");
                return;
            }
            if pressed {
                for c in &codes {
                    if let Err(e) = input.kbd.key(*c, true) {
                        warn!(%e, "key down failed");
                    }
                }
            } else {
                for c in codes.iter().rev() {
                    if let Err(e) = input.kbd.key(*c, false) {
                        warn!(%e, "key up failed");
                    }
                }
            }
        }
        Binding::Mouse { action } => {
            if matches!(action.as_str(), "zoom-in" | "zoom") {
                if pressed {
                    info!(delta = 1, "zoom");
                    let _ = input.kbd.zoom(1);
                }
                return;
            }
            if action == "zoom-out" {
                if pressed {
                    info!(delta = -1, "zoom");
                    let _ = input.kbd.zoom(-1);
                }
                return;
            }
            let id = match action.as_str() {
                "left" => 1,
                "right" => 2,
                "middle" => 3,
                "double" => 4,
                "wheel-up" => 5,
                "wheel-down" => 6,
                _ => {
                    warn!(action, "unknown mouse action");
                    return;
                }
            };
            let _ = input.do_mouse_func(id, pressed);
        }
        Binding::Scroll { direction } => {
            if !pressed {
                return;
            }
            let down = direction != "up";
            let delta = if down { -1 } else { 1 };
            let page = if down { KEY_PAGEDOWN } else { KEY_PAGEUP };
            let arrow = if down { KEY_DOWN } else { KEY_UP };
            info!(down, "scroll binding");
            for _ in 0..5 {
                if let Err(e) = input.mouse.wheel(delta) {
                    warn!(%e, "wheel failed");
                }
            }
            // Wayland: wheel on a second pointer often misses the page under the pen.
            let _ = input.kbd.tap(page);
            for _ in 0..3 {
                let _ = input.kbd.tap(arrow);
            }
        }
        Binding::Builtin { id } => {
            if !pressed {
                return;
            }
            dispatch_builtin(*id, input);
        }
        Binding::EraserToggle => {
            if pressed {
                let next = !input.eraser_mode();
                input.set_eraser_mode(next);
            }
        }
    }
}

fn dispatch_builtin(id: u16, input: &mut InputDevices) {
    match id {
        1 => {
            let _ = input
                .kbd
                .combo(&[deco_input::KEY_LEFTCTRL, deco_input::KEY_Z]);
        }
        2 => {
            let _ = input.kbd.combo(&[
                deco_input::KEY_LEFTCTRL,
                deco_input::KEY_LEFTSHIFT,
                deco_input::KEY_Z,
            ]);
        }
        3 => {
            let _ = input.kbd.tap(deco_input::KEY_LEFTBRACE);
        }
        4 => {
            let _ = input.kbd.tap(deco_input::KEY_RIGHTBRACE);
        }
        5 => {
            let _ = input
                .kbd
                .combo(&[deco_input::KEY_LEFTCTRL, deco_input::KEY_EQUAL]);
        }
        6 => {
            let _ = input
                .kbd
                .combo(&[deco_input::KEY_LEFTCTRL, deco_input::KEY_MINUS]);
        }
        21 => {
            let _ = input.kbd.tap(deco_input::KEY_B);
        }
        22 => {
            let _ = input.kbd.tap(deco_input::KEY_E);
        }
        103 => {
            input.set_eraser_mode(!input.eraser_mode());
        }
        201..=211 => {
            let _ = input.do_mouse_func((id - 200) as u8, true);
            let _ = input.do_mouse_func((id - 200) as u8, false);
        }
        other => debug!(other, "unhandled builtin actid"),
    }
}

#[allow(dead_code)]
pub fn _keep_btns() {
    let _ = (BTN_LEFT, BTN_RIGHT, BTN_MIDDLE);
}
