//! StatusNotifierItem tray (Ayatana / Ubuntu top-bar).

use std::sync::mpsc::Sender;

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use tracing::warn;

pub enum TrayCmd {
    Show,
    Quit,
}

struct DecoTray {
    tx: Sender<TrayCmd>,
}

impl ksni::Tray for DecoTray {
    fn id(&self) -> String {
        "it.bwlab.DecoUi".into()
    }

    fn title(&self) -> String {
        "Deco Driver".into()
    }

    fn icon_name(&self) -> String {
        "it.bwlab.DecoUi".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![ksni::Icon {
            width: 32,
            height: 32,
            data: pixmap_argb32(),
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Deco Driver".into(),
            description: "XP-Pen Deco 02".into(),
            icon_name: "it.bwlab.DecoUi".into(),
            icon_pixmap: self.icon_pixmap(),
        }
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayCmd::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Apri impostazioni".into(),
                icon_name: "document-properties".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCmd::Show);
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Esci".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn spawn(tx: Sender<TrayCmd>) {
    std::thread::Builder::new()
        .name("deco-tray".into())
        .spawn(move || match (DecoTray { tx }).spawn() {
            Ok(_handle) => std::thread::park(),
            Err(e) => warn!(%e, "tray icon unavailable (install gnome-shell-extension-appindicator)"),
        })
        .ok();
}

/// Fallback pixmap so the tray is never blank if the theme icon is missing.
fn pixmap_argb32() -> Vec<u8> {
    const W: i32 = 32;
    const H: i32 = 32;
    let mut out = vec![0u8; (W * H * 4) as usize];
    let put = |buf: &mut [u8], x: i32, y: i32, a: u8, r: u8, g: u8, b: u8| {
        if x < 0 || y < 0 || x >= W || y >= H {
            return;
        }
        let i = ((y * W + x) * 4) as usize;
        buf[i] = a;
        buf[i + 1] = r;
        buf[i + 2] = g;
        buf[i + 3] = b;
    };
    for y in 0..H {
        for x in 0..W {
            put(&mut out, x, y, 255, 28, 28, 30);
        }
    }
    for y in 6..26 {
        for x in 5..22 {
            let edge = y == 6 || y == 25 || x == 5 || x == 21;
            if edge {
                put(&mut out, x, y, 255, 110, 231, 183);
            } else if (7..20).contains(&y) && (7..19).contains(&x) {
                put(&mut out, x, y, 255, 17, 17, 17);
            } else {
                put(&mut out, x, y, 255, 44, 44, 46);
            }
        }
    }
    for t in 0..18 {
        let x = 14 + t / 2;
        let y = 8 + t;
        put(&mut out, x, y, 255, 167, 139, 250);
        put(&mut out, x + 1, y, 255, 196, 181, 253);
    }
    out
}
