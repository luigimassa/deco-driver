use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod tray;

use adw::prelude::*;
use deco_config::{Binding, DriverConfig};
use gtk::gdk;
use gtk::gio;
use gtk::gio::prelude::SettingsExt;
use gtk::glib;
use gtk::glib::prelude::ToVariant;
use gtk::prelude::Cast;
use tracing::{info, warn};
use zbus::blocking::Connection;

const TABLET_GSETTINGS: &str = "org.gnome.desktop.peripherals.tablet";
const TABLET_PATHS: &[&str] = &[
    "/org/gnome/desktop/peripherals/tablets/28bd:0002/",
    "/org/gnome/desktop/peripherals/tablets/28bd:0803/",
    "/org/gnome/desktop/peripherals/tablets/28bd:0903/",
];

#[derive(Clone)]
struct Screen {
    label: String,
    manufacturer: String,
    model: String,
    serial: String,
    connector: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn xml_tag(block: &str, name: &str) -> String {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    block
        .split(&open)
        .nth(1)
        .and_then(|s| s.split(&close).next())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Serial from ~/.config/monitors.xml only when vendor+product match the live GDK monitor.
fn monitor_serial(connector: &str, manufacturer: &str, model: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{home}/.config/monitors.xml");
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let mut fallback = String::new();
    for block in text.split("<monitorspec>").skip(1) {
        let Some(end) = block.find("</monitorspec>") else {
            continue;
        };
        let b = &block[..end];
        if xml_tag(b, "connector") != connector {
            continue;
        }
        let v = xml_tag(b, "vendor");
        let p = xml_tag(b, "product");
        let s = xml_tag(b, "serial");
        if v == manufacturer && p == model {
            return s;
        }
        if !s.is_empty() && (v == manufacturer || p == model) {
            fallback = s;
        }
    }
    fallback
}

fn variant_i32(v: &glib::Variant) -> i32 {
    v.get::<i32>().unwrap_or(0)
}

fn variant_str(v: &glib::Variant) -> String {
    v.str().unwrap_or("").to_string()
}

fn dict_flag(dict: &glib::Variant, key: &str) -> bool {
    for i in 0..dict.n_children() {
        let entry = dict.child_value(i);
        if entry.n_children() < 2 {
            continue;
        }
        if variant_str(&entry.child_value(0)) != key {
            continue;
        }
        let val = entry.child_value(1);
        let inner = val.as_variant().unwrap_or(val);
        return inner.get::<bool>().unwrap_or(false);
    }
    false
}

/// Live layout + EDID from Mutter (vendor/product/serial GNOME actually matches).
fn mutter_screens() -> Option<Vec<(i32, i32, i32, i32, String, String, String, String)>> {
    let conn = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok()?;
    let reply = conn
        .call_sync(
            Some("org.gnome.Mutter.DisplayConfig"),
            "/org/gnome/Mutter/DisplayConfig",
            "org.gnome.Mutter.DisplayConfig",
            "GetCurrentState",
            None,
            None,
            gio::DBusCallFlags::NONE,
            4000,
            None::<&gio::Cancellable>,
        )
        .ok()?;
    if reply.n_children() < 3 {
        return None;
    }
    let monitors = reply.child_value(1);
    let logical = reply.child_value(2);
    let mut sizes: std::collections::HashMap<String, (i32, i32)> = std::collections::HashMap::new();
    for i in 0..monitors.n_children() {
        let mon = monitors.child_value(i);
        if mon.n_children() < 2 {
            continue;
        }
        let spec = mon.child_value(0);
        if spec.n_children() < 1 {
            continue;
        }
        let connector = variant_str(&spec.child_value(0));
        let modes = mon.child_value(1);
        let mut chosen: Option<(i32, i32)> = None;
        let mut preferred: Option<(i32, i32)> = None;
        for m in 0..modes.n_children() {
            let mode = modes.child_value(m);
            if mode.n_children() < 3 {
                continue;
            }
            let w = variant_i32(&mode.child_value(1));
            let h = variant_i32(&mode.child_value(2));
            if w <= 0 || h <= 0 {
                continue;
            }
            let props = if mode.n_children() > 6 {
                mode.child_value(6)
            } else {
                continue;
            };
            if dict_flag(&props, "is-current") {
                chosen = Some((w, h));
                break;
            }
            if preferred.is_none() && dict_flag(&props, "is-preferred") {
                preferred = Some((w, h));
            }
        }
        if let Some(wh) = chosen.or(preferred) {
            sizes.insert(connector, wh);
        }
    }
    let mut raw = Vec::new();
    for i in 0..logical.n_children() {
        let lm = logical.child_value(i);
        if lm.n_children() < 6 {
            continue;
        }
        let x = variant_i32(&lm.child_value(0));
        let y = variant_i32(&lm.child_value(1));
        let specs = lm.child_value(5);
        for s in 0..specs.n_children() {
            let spec = specs.child_value(s);
            if spec.n_children() < 4 {
                continue;
            }
            let connector = variant_str(&spec.child_value(0));
            let vendor = variant_str(&spec.child_value(1));
            let product = variant_str(&spec.child_value(2));
            let serial = variant_str(&spec.child_value(3));
            let (w, h) = sizes.get(&connector).copied().unwrap_or((0, 0));
            if w == 0 || h == 0 {
                continue;
            }
            raw.push((x, y, w, h, connector, vendor, product, serial));
        }
    }
    if raw.is_empty() {
        None
    } else {
        Some(raw)
    }
}

fn gdk_screens() -> Vec<(i32, i32, i32, i32, String, String, String, String)> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let model = display.monitors();
    let n = model.n_items();
    let mut raw = Vec::with_capacity(n as usize);
    for i in 0..n {
        let Some(obj) = model.item(i) else { continue };
        let Ok(mon) = obj.downcast::<gdk::Monitor>() else {
            continue;
        };
        let geo = mon.geometry();
        let conn = mon.connector().unwrap_or_default().to_string();
        let manufacturer = mon.manufacturer().unwrap_or_default().to_string();
        let model_name = mon.model().unwrap_or_default().to_string();
        let serial = monitor_serial(&conn, &manufacturer, &model_name);
        raw.push((
            geo.x(),
            geo.y(),
            geo.width(),
            geo.height(),
            conn,
            manufacturer,
            model_name,
            serial,
        ));
    }
    raw
}

fn screens_bbox(screens: &[Screen]) -> (f64, f64) {
    let min_x = screens.iter().map(|s| s.x).min().unwrap_or(0);
    let min_y = screens.iter().map(|s| s.y).min().unwrap_or(0);
    let max_x = screens.iter().map(|s| s.x + s.width).max().unwrap_or(0);
    let max_y = screens.iter().map(|s| s.y + s.height).max().unwrap_or(0);
    (
        (max_x - min_x).max(1) as f64,
        (max_y - min_y).max(1) as f64,
    )
}

fn list_screens() -> Vec<Screen> {
    let mut raw = mutter_screens().unwrap_or_else(gdk_screens);
    raw.sort_by_key(|t| (t.0, t.1));
    let nscr = raw.len();
    let horizontal = nscr == 2 && (raw[0].0 - raw[1].0).abs() >= (raw[0].1 - raw[1].1).abs();
    raw.into_iter()
        .enumerate()
        .map(|(i, (x, y, w, h, conn, manufacturer, model_name, serial))| {
            let name = match (manufacturer.as_str(), model_name.as_str()) {
                ("", "") => conn.clone(),
                ("", m) => m.to_string(),
                (man, "") => format!("{man} ({conn})"),
                (man, m) => format!("{man} {m}"),
            };
            let side = match (nscr, horizontal, i) {
                (2, true, 0) => "sinistra",
                (2, true, _) => "destra",
                (2, false, 0) => "sopra",
                (2, false, _) => "sotto",
                _ => "",
            };
            let label = if side.is_empty() {
                format!("Schermo {} · {name} · {w}×{h}", i + 1)
            } else {
                format!("{side} · {name} · {w}×{h}")
            };
            Screen {
                label,
                manufacturer,
                model: model_name,
                serial,
                connector: conn,
                x,
                y,
                width: w,
                height: h,
            }
        })
        .collect()
}

fn apply_gnome_screen(_screens: &[Screen], cfg: &deco_config::MappingConfig) {
    let screens = list_screens();
    // GNOME output is exactly [vendor, product, serial]. Empty = whole desktop.
    let output: Vec<String> = if cfg.monitor == 0 || screens.is_empty() {
        vec![String::new(), String::new(), String::new()]
    } else {
        let idx = (cfg.monitor as usize).saturating_sub(1);
        match screens.get(idx) {
            Some(s) => vec![s.manufacturer.clone(), s.model.clone(), s.serial.clone()],
            None => vec![String::new(), String::new(), String::new()],
        }
    };
    let (dest_w, dest_h) = if screens.is_empty() {
        (0.0, 0.0)
    } else if cfg.monitor == 0 {
        screens_bbox(&screens)
    } else {
        let idx = (cfg.monitor as usize).saturating_sub(1);
        screens
            .get(idx)
            .map(|s| (s.width as f64, s.height as f64))
            .unwrap_or((0.0, 0.0))
    };
    // Letterbox on the tablet via `area`. GNOME keep-aspect maps a landscape
    // tablet onto one monitor of an extended desktop (Samsung-only).
    let margins = deco_config::tablet_area_margins(cfg, dest_w, dest_h, cfg.keep_aspect);
    let area = margins.to_variant();
    for path in TABLET_PATHS {
        let settings = gio::Settings::with_path(TABLET_GSETTINGS, path);
        if let Err(e) = settings.set_strv("output", output.as_slice()) {
            warn!(%e, %path, "gsettings output");
        } else {
            info!(
                ?output,
                %path,
                dest_w,
                dest_h,
                keep_aspect = cfg.keep_aspect,
                ?margins,
                connector = screens.get(cfg.monitor.saturating_sub(1) as usize).map(|s| s.connector.as_str()).unwrap_or(""),
                "mapped tablet to screen via GNOME gsettings"
            );
        }
        let _ = settings.set_boolean("keep-aspect", false);
        let _ = settings.set_boolean("left-handed", cfg.orientation == 180);
        if let Err(e) = settings.set_value("area", &area) {
            warn!(%e, %path, "gsettings area");
        }
    }
}

const APP_ID: &str = "it.bwlab.DecoUi";

#[zbus::proxy(
    interface = "it.bwlab.DecoDriver1",
    default_service = "it.bwlab.DecoDriver",
    default_path = "/it/bwlab/DecoDriver",
    gen_blocking = true,
    gen_async = false
)]
trait DecoDriver {
    #[zbus(property)]
    fn device_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn pid(&self) -> zbus::Result<u16>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn max_pressure(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn firmware(&self) -> zbus::Result<String>;
    #[zbus(property(emits_changed_signal = "false"))]
    fn last_pressure(&self) -> zbus::Result<u32>;

    fn get_config(&self) -> zbus::Result<String>;
    fn set_config(&self, json: &str) -> zbus::Result<()>;
    fn set_key_binding(&self, index: u8, kind: &str, payload: &str) -> zbus::Result<()>;
    fn set_ring_binding(&self, side: &str, kind: &str, payload: &str) -> zbus::Result<()>;
    fn set_pressure_curve(&self, p0: f64, p1x: f64, p1y: f64, p2: f64) -> zbus::Result<()>;
    fn set_mapping(
        &self,
        orientation: u16,
        tablet_x: f64,
        tablet_y: f64,
        tablet_w: f64,
        tablet_h: f64,
        monitor: u32,
    ) -> zbus::Result<()>;
    fn reset_defaults(&self) -> zbus::Result<()>;
    fn list_monitors(&self) -> zbus::Result<Vec<String>>;
    fn self_check(&self) -> zbus::Result<String>;
}

fn presets() -> Vec<(&'static str, Binding)> {
    vec![
        ("Nessuna azione", Binding::Disabled),
        (
            "Click sinistro",
            Binding::Mouse {
                action: "left".into(),
            },
        ),
        (
            "Click destro",
            Binding::Mouse {
                action: "right".into(),
            },
        ),
        (
            "Click centrale",
            Binding::Mouse {
                action: "middle".into(),
            },
        ),
        (
            "Doppio click",
            Binding::Mouse {
                action: "double".into(),
            },
        ),
        (
            "Scorri in su",
            Binding::Scroll {
                direction: "up".into(),
            },
        ),
        (
            "Scorri in giù",
            Binding::Scroll {
                direction: "down".into(),
            },
        ),
        ("Gomma digitale", Binding::EraserToggle),
        ("Annulla", Binding::key(&["LeftCtrl", "Z"])),
        ("Ripeti", Binding::key(&["LeftCtrl", "LeftShift", "Z"])),
        ("Pennello più piccolo", Binding::key(&["["])),
        ("Pennello più grande", Binding::key(&["]"])),
        (
            "Zoom avanti",
            Binding::Mouse {
                action: "zoom-in".into(),
            },
        ),
        (
            "Zoom indietro",
            Binding::Mouse {
                action: "zoom-out".into(),
            },
        ),
        ("Strumento pennello", Binding::key(&["B"])),
        ("Strumento gomma", Binding::key(&["E"])),
        ("Salva", Binding::key(&["LeftCtrl", "S"])),
        ("Copia", Binding::key(&["LeftCtrl", "C"])),
        ("Incolla", Binding::key(&["LeftCtrl", "V"])),
        ("Taglia", Binding::key(&["LeftCtrl", "X"])),
        ("Spazio (pan nella tela)", Binding::key(&["Space"])),
        ("Maiusc", Binding::key(&["LeftShift"])),
        ("Ctrl", Binding::key(&["LeftCtrl"])),
        ("Alt", Binding::key(&["LeftAlt"])),
        ("Tab", Binding::key(&["Tab"])),
        ("Cancella", Binding::key(&["Delete"])),
        ("Invio", Binding::key(&["Return"])),
        ("Esc", Binding::key(&["Escape"])),
    ]
}

fn preset_labels() -> Vec<&'static str> {
    presets().into_iter().map(|(l, _)| l).collect()
}

fn normalize_binding(b: &Binding) -> Binding {
    match b {
        Binding::Key { keys } if keys.len() == 2 => {
            let lower: Vec<String> = keys.iter().map(|k| k.to_ascii_lowercase()).collect();
            let ctrl = lower
                .iter()
                .any(|k| k == "leftctrl" || k == "ctrl" || k == "control");
            if ctrl && lower.iter().any(|k| k == "equal" || k == "plus") {
                return Binding::Mouse {
                    action: "zoom-in".into(),
                };
            }
            if ctrl && lower.iter().any(|k| k == "minus") {
                return Binding::Mouse {
                    action: "zoom-out".into(),
                };
            }
            b.clone()
        }
        other => other.clone(),
    }
}

fn preset_index(b: &Binding) -> u32 {
    let b = normalize_binding(b);
    presets()
        .iter()
        .position(|(_, p)| p == &b)
        .unwrap_or(0) as u32
}

fn preset_binding(i: u32) -> Binding {
    presets()
        .into_iter()
        .nth(i as usize)
        .map(|(_, b)| b)
        .unwrap_or(Binding::Disabled)
}

fn binding_combo(title: &str, subtitle: &str) -> adw::ComboRow {
    let labels = preset_labels();
    let row = adw::ComboRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.set_model(Some(&gtk::StringList::new(&labels)));
    row
}

fn string_combo(title: &str, subtitle: &str, labels: &[&str]) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.set_model(Some(&gtk::StringList::new(labels)));
    row
}

struct Ui {
    proxy: Option<DecoDriverProxy<'static>>,
    status: gtk::Label,
    pid: gtk::Label,
    fw: gtk::Label,
    press_max: gtk::Label,
    pressure: gtk::LevelBar,
    pressure_label: gtk::Label,
    p1x: gtk::Scale,
    p1y: gtk::Scale,
    keys: [adw::ComboRow; 6],
    pen_btn1: adw::ComboRow,
    ring_left: adw::ComboRow,
    ring_right: adw::ComboRow,
    orientation: adw::ComboRow,
    tx: gtk::SpinButton,
    ty: gtk::SpinButton,
    tw: gtk::SpinButton,
    th: gtk::SpinButton,
    monitor: adw::ComboRow,
    keep_aspect: gtk::Switch,
    screens: Vec<Screen>,
    diag: gtk::Label,
    diag_enabled: gtk::Switch,
}

fn connect() -> Option<DecoDriverProxy<'static>> {
    let conn = Box::leak(Box::new(Connection::session().ok()?));
    DecoDriverProxy::new(conn).ok()
}

fn apply_config(ui: &Ui, cfg: &DriverConfig) {
    ui.p1x.set_value(cfg.pen.curve.p1x);
    ui.p1y.set_value(cfg.pen.curve.p1y);
    ui.pen_btn1.set_selected(preset_index(&cfg.pen.btn1));
    for i in 0..6 {
        ui.keys[i].set_selected(preset_index(&cfg.keys.k[i]));
    }
    ui.ring_left.set_selected(preset_index(&cfg.ring.left));
    ui.ring_right.set_selected(preset_index(&cfg.ring.right));
    let ori = match cfg.mapping.orientation {
        90 => 1u32,
        180 => 2,
        270 => 3,
        _ => 0,
    };
    ui.orientation.set_selected(ori);
    ui.tx.set_value(cfg.mapping.tablet_x);
    ui.ty.set_value(cfg.mapping.tablet_y);
    ui.tw.set_value(cfg.mapping.tablet_w);
    ui.th.set_value(cfg.mapping.tablet_h);
    let n = ui.monitor.model().map(|m| m.n_items()).unwrap_or(1);
    ui.monitor
        .set_selected(cfg.mapping.monitor.min(n.saturating_sub(1)));
    ui.keep_aspect.set_active(cfg.mapping.keep_aspect);
}

fn collect_config(ui: &Ui) -> DriverConfig {
    let mut cfg = DriverConfig::default();
    cfg.pen.curve.p1x = ui.p1x.value();
    cfg.pen.curve.p1y = ui.p1y.value();
    cfg.pen.btn1 = preset_binding(ui.pen_btn1.selected());
    for i in 0..6 {
        cfg.keys.k[i] = preset_binding(ui.keys[i].selected());
    }
    cfg.ring.left = preset_binding(ui.ring_left.selected());
    cfg.ring.right = preset_binding(ui.ring_right.selected());
    cfg.mapping.orientation = match ui.orientation.selected() {
        1 => 90,
        2 => 180,
        3 => 270,
        _ => 0,
    };
    cfg.mapping.tablet_x = ui.tx.value();
    cfg.mapping.tablet_y = ui.ty.value();
    cfg.mapping.tablet_w = ui.tw.value();
    cfg.mapping.tablet_h = ui.th.value();
    cfg.mapping.monitor = ui.monitor.selected();
    cfg.mapping.keep_aspect = ui.keep_aspect.is_active();
    cfg
}

fn refresh(ui: &Ui) {
    let Some(proxy) = ui.proxy.as_ref() else {
        ui.status
            .set_text("Demone non in esecuzione (it.bwlab.DecoDriver)");
        if ui.diag_enabled.is_active() {
            ui.diag.set_text(
                "FAIL: deco-daemon non è sul bus di sessione.\nAvvio: systemctl --user start deco-daemon",
            );
        }
        return;
    };
    match proxy.device_present() {
        Ok(true) => {
            let name = proxy.name().unwrap_or_default();
            let pid = proxy.pid().unwrap_or(0);
            ui.status
                .set_text(&format!("Collegata · {name}  (28bd:{pid:04x})"));
            ui.pid.set_text(&format!("0x{pid:04X}"));
            ui.fw.set_text(&proxy.firmware().unwrap_or_default());
            ui.press_max
                .set_text(&proxy.max_pressure().unwrap_or(0).to_string());
        }
        Ok(false) => {
            ui.status.set_text("In attesa della Deco 02…");
            ui.pid.set_text("—");
            ui.fw.set_text("—");
        }
        Err(e) => {
            ui.status.set_text(&format!("Errore D-Bus: {e}"));
        }
    }
    if let Ok(p) = proxy.last_pressure() {
        let max = proxy.max_pressure().unwrap_or(8191).max(1);
        ui.pressure.set_max_value(max as f64);
        ui.pressure.set_value(p as f64);
        ui.pressure_label.set_text(&format!("{p}"));
    }
    if !ui.diag_enabled.is_active() {
        return;
    }
    use std::sync::atomic::{AtomicU32, Ordering};
    static DIAG_TICK: AtomicU32 = AtomicU32::new(0);
    if DIAG_TICK.fetch_add(1, Ordering::Relaxed) % 8 == 0 {
        match proxy.self_check() {
            Ok(text) => ui.diag.set_text(&text),
            Err(e) => ui.diag.set_text(&format!("SelfCheck D-Bus error: {e}")),
        }
    }
}

fn save(ui: &Ui) {
    let Some(proxy) = ui.proxy.as_ref() else {
        return;
    };
    let cfg = collect_config(ui);
    match serde_json::to_string(&cfg) {
        Ok(json) => {
            if let Err(e) = proxy.set_config(&json) {
                warn!(%e, "set_config failed");
            } else {
                apply_gnome_screen(&ui.screens, &cfg.mapping);
            }
        }
        Err(e) => warn!(%e, "serialize config"),
    }
}

fn pref_row(title: &str, widget: &impl IsA<gtk::Widget>) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    widget.set_valign(gtk::Align::Center);
    row.add_suffix(widget);
    // Scale/Label as activatable_widget recurse in measure → stack overflow.
    let w = widget.as_ref();
    if !w.is::<gtk::Scale>() && !w.is::<gtk::Label>() {
        row.set_activatable_widget(Some(widget));
    }
    row
}

fn spin(min: f64, max: f64, step: f64, value: f64) -> gtk::SpinButton {
    gtk::SpinButton::with_range(min, max, step).tap(|s| {
        s.set_value(value);
        s.set_digits(2);
        s.set_valign(gtk::Align::Center);
    })
}

trait Tap: Sized {
    fn tap(self, f: impl FnOnce(&Self)) -> Self;
}
impl<T> Tap for T {
    fn tap(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }
}

fn build_window(app: &adw::Application, background: bool) {
    let proxy = connect();
    if proxy.is_none() {
        warn!("could not connect to deco-daemon");
    }

    let header = adw::HeaderBar::new();
    let switcher = adw::ViewSwitcher::new();
    header.set_title_widget(Some(&switcher));

    let stack = adw::ViewStack::new();
    switcher.set_stack(Some(&stack));

    let status = gtk::Label::builder()
        .label("Connessione…")
        .wrap(true)
        .xalign(0.0)
        .build();
    status.add_css_class("title-3");
    let pid = gtk::Label::new(Some("—"));
    let fw = gtk::Label::new(Some("—"));
    let press_max = gtk::Label::new(Some("—"));
    let pressure = gtk::LevelBar::builder()
        .min_value(0.0)
        .max_value(8191.0)
        .hexpand(true)
        .width_request(180)
        .height_request(8)
        .build();
    let pressure_label = gtk::Label::new(Some("0"));

    let device_page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::builder()
        .title("Tavoletta")
        .description("XP-Pen Deco 02")
        .build();
    g.add(&pref_row("Stato", &status));
    g.add(&pref_row("Product ID", &pid));
    g.add(&pref_row("Firmware", &fw));
    g.add(&pref_row("Pressione massima", &press_max));
    let diag_enabled = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(false)
        .build();
    let diag_row = pref_row("Diagnostica", &diag_enabled);
    diag_row.set_subtitle("Mostra i controlli USB / hidraw / demone");
    g.add(&diag_row);
    device_page.add(&g);
    let diag = gtk::Label::builder()
        .label("self-check in attesa…")
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["monospace"])
        .build();
    let dg = adw::PreferencesGroup::builder()
        .title("Esito diagnostica")
        .description("OK/FAIL dal demone — collega la tavoletta e osserva qui")
        .build();
    dg.add(&diag);
    dg.set_visible(false);
    device_page.add(&dg);

    diag_enabled.connect_active_notify({
        let dg = dg.clone();
        move |sw| dg.set_visible(sw.is_active())
    });

    let p1x = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.05, 0.95, 0.01);
    p1x.set_value(0.5);
    p1x.set_hexpand(true);
    p1x.set_draw_value(true);
    let p1y = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    p1y.set_value(0.5);
    p1y.set_hexpand(true);
    p1y.set_draw_value(true);
    let pen_btn1 = binding_combo(
        "Tasto superiore",
        "Sul fusto, il più vicino alla punta",
    );
    // The tablet reports this button as its own eraser (HID Invert/Eraser),
    // not as a remappable button: show it, don't offer a choice.
    let pen_btn2 = adw::ActionRow::builder()
        .title("Tasto inferiore")
        .subtitle("Gomma: tienilo premuto mentre tocchi per cancellare")
        .build();
    let pen_page = adw::PreferencesPage::new();
    let pg = adw::PreferencesGroup::builder()
        .title("Penna")
        .description("Scegli cosa fa ciascun tasto della stilo")
        .build();
    pg.add(&pen_btn1);
    pg.add(&pen_btn2);
    pg.add(&pref_row("Curva X", &p1x));
    pg.add(&pref_row("Curva Y", &p1y));
    let test = adw::ActionRow::builder()
        .title("Pressione in tempo reale")
        .subtitle("Premi la punta sulla tavoletta: il numero e la barra salgono (0–8191)")
        .activatable(false)
        .build();
    test.add_suffix(&pressure);
    test.add_suffix(&pressure_label);
    pg.add(&test);
    pen_page.add(&pg);

    let keys = [
        binding_combo("Tasto 1", "Il più in alto, a sinistra della tavoletta"),
        binding_combo("Tasto 2", "Secondo dall’alto"),
        binding_combo("Tasto 3", "Terzo dall’alto"),
        binding_combo("Tasto 4", "Terzo dal basso"),
        binding_combo("Tasto 5", "Secondo dal basso"),
        binding_combo("Tasto 6", "Il più in basso"),
    ];
    let keys_page = adw::PreferencesPage::new();
    let kg = adw::PreferencesGroup::builder()
        .title("Tasti rapidi")
        .description("I sei tasti a sinistra, dall’alto in basso")
        .build();
    for k in &keys {
        kg.add(k);
    }
    keys_page.add(&kg);

    let ring_left = binding_combo("Gira a sinistra", "Rotella al centro, senso antiorario");
    let ring_right = binding_combo("Gira a destra", "Rotella al centro, senso orario");
    let ring_page = adw::PreferencesPage::new();
    let rg = adw::PreferencesGroup::builder()
        .title("Rotella")
        .description("La ghiera silenziosa al centro della tavoletta")
        .build();
    rg.add(&ring_left);
    rg.add(&ring_right);
    ring_page.add(&rg);

    let orientation = string_combo(
        "Orientamento",
        "Come tieni la tavoletta. Mancino = 180°",
        &[
            "Normale",
            "Ruotata 90°",
            "Mancino (180°)",
            "Ruotata 270°",
        ],
    );
    let tx = spin(0.0, 1.0, 0.05, 0.0);
    let ty = spin(0.0, 1.0, 0.05, 0.0);
    let tw = spin(0.05, 1.0, 0.05, 1.0);
    let th = spin(0.05, 1.0, 0.05, 1.0);
    let screens = list_screens();
    let (bw, bh) = screens_bbox(&screens);
    let mut monitor_labels: Vec<String> =
        vec![format!("Schermi agganciati · {bw:.0}×{bh:.0}")];
    monitor_labels.extend(screens.iter().map(|s| s.label.clone()));
    let monitor_refs: Vec<&str> = monitor_labels.iter().map(String::as_str).collect();
    let monitor = string_combo(
        "Schermo",
        "Come il software XP-Pen: un monitor, oppure i due schermi agganciati",
        &monitor_refs,
    );
    let keep_aspect = gtk::Switch::builder().valign(gtk::Align::Center).build();
    let map_page = adw::PreferencesPage::new();
    let mg = adw::PreferencesGroup::builder()
        .title("Schermo")
        .description("Agganciati: la penna copre l’intero desktop esteso (entrambi gli schermi). Solo schermo: tutta l’area di quel monitor.")
        .build();
    mg.add(&monitor);
    mg.add(&pref_row("Mantieni le proporzioni", &keep_aspect));
    mg.add(&orientation);
    let tg = adw::PreferencesGroup::builder()
        .title("Area della tavoletta")
        .description("Ritaglio attivo (0–1). Tutta l’area è 0, 0, 1, 1.")
        .build();
    tg.add(&pref_row("X", &tx));
    tg.add(&pref_row("Y", &ty));
    tg.add(&pref_row("Larghezza", &tw));
    tg.add(&pref_row("Altezza", &th));
    map_page.add(&mg);
    map_page.add(&tg);

    stack.add_titled_with_icon(
        &device_page,
        Some("device"),
        "Device",
        "input-tablet-symbolic",
    );
    stack.add_titled_with_icon(&pen_page, Some("pen"), "Pen", "document-edit-symbolic");
    stack.add_titled_with_icon(&keys_page, Some("keys"), "Keys", "input-keyboard-symbolic");
    stack.add_titled_with_icon(
        &ring_page,
        Some("roller"),
        "Roller",
        "view-refresh-symbolic",
    );
    stack.add_titled_with_icon(&map_page, Some("map"), "Mapping", "video-display-symbolic");

    let save_btn = gtk::Button::builder()
        .label("Salva")
        .css_classes(["suggested-action"])
        .build();
    let reset_btn = gtk::Button::with_label("Ripristina predefiniti");
    header.pack_start(&reset_btn);
    header.pack_end(&save_btn);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Deco Driver")
        .default_width(820)
        .default_height(640)
        .content(&toolbar)
        .build();
    window.set_hide_on_close(true);
    std::mem::forget(app.hold());

    let (tray_tx, tray_rx) = std::sync::mpsc::channel::<tray::TrayCmd>();
    tray::spawn(tray_tx);
    glib::timeout_add_local(std::time::Duration::from_millis(100), {
        let window = window.clone();
        let app = app.clone();
        move || {
            while let Ok(cmd) = tray_rx.try_recv() {
                match cmd {
                    tray::TrayCmd::Show => {
                        window.set_visible(true);
                        window.present();
                    }
                    tray::TrayCmd::Quit => app.quit(),
                }
            }
            glib::ControlFlow::Continue
        }
    });

    let ui = Rc::new(RefCell::new(Ui {
        proxy,
        status,
        pid,
        fw,
        press_max,
        pressure,
        pressure_label,
        p1x,
        p1y,
        keys,
        pen_btn1,
        ring_left,
        ring_right,
        orientation,
        tx,
        ty,
        tw,
        th,
        monitor,
        keep_aspect,
        screens,
        diag,
        diag_enabled,
    }));

    {
        let ui_ref = ui.borrow();
        if let Some(proxy) = ui_ref.proxy.as_ref() {
            if let Ok(json) = proxy.get_config() {
                if let Ok(cfg) = serde_json::from_str::<DriverConfig>(&json) {
                    apply_config(&ui_ref, &cfg);
                    apply_gnome_screen(&ui_ref.screens, &cfg.mapping);
                }
            }
        }
        refresh(&ui_ref);
    }

    let mapping_ready = Rc::new(Cell::new(false));
    ui.borrow().monitor.connect_selected_notify({
        let ui = Rc::clone(&ui);
        let mapping_ready = Rc::clone(&mapping_ready);
        move |_| {
            if !mapping_ready.get() {
                return;
            }
            let Ok(ui) = ui.try_borrow() else {
                return;
            };
            let cfg = collect_config(&ui);
            apply_gnome_screen(&ui.screens, &cfg.mapping);
            if let Some(proxy) = ui.proxy.as_ref() {
                if let Ok(json) = serde_json::to_string(&cfg) {
                    let _ = proxy.set_config(&json);
                }
            }
        }
    });
    mapping_ready.set(true);

    save_btn.connect_clicked({
        let ui = Rc::clone(&ui);
        move |_| save(&ui.borrow())
    });
    reset_btn.connect_clicked({
        let ui = Rc::clone(&ui);
        move |_| {
            if let Some(proxy) = ui.borrow().proxy.as_ref() {
                let _ = proxy.reset_defaults();
                if let Ok(json) = proxy.get_config() {
                    if let Ok(cfg) = serde_json::from_str::<DriverConfig>(&json) {
                        apply_config(&ui.borrow(), &cfg);
                        apply_gnome_screen(&ui.borrow().screens, &cfg.mapping);
                    }
                }
            }
        }
    });

    glib::timeout_add_local(std::time::Duration::from_millis(50), {
        let ui = Rc::clone(&ui);
        move || {
            refresh(&ui.borrow());
            glib::ControlFlow::Continue
        }
    });

    if background {
        window.set_visible(false);
    } else {
        window.present();
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.add_main_option(
        "background",
        b'b'.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Avvia nascosto, solo icona nella tray",
        None,
    );
    let background = Rc::new(Cell::new(false));
    app.connect_handle_local_options({
        let background = Rc::clone(&background);
        move |_, dict| {
            background.set(dict.lookup_value("background", None).is_some());
            std::ops::ControlFlow::Continue(())
        }
    });
    app.connect_activate({
        let background = Rc::clone(&background);
        move |app| {
            if let Some(win) = app.windows().into_iter().next() {
                win.present();
                return;
            }
            build_window(app, background.get());
        }
    });
    app.run();
}
