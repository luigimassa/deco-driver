# Deco Driver

Userspace driver for the **XP-Pen Deco 02** (USB `28bd:0803` / `28bd:0903`) on GNOME / Ubuntu.

No kernel module. The daemon reads HID from `hidraw`, injects a tablet through `uinput`, and a GTK4 / libadwaita app maps the pen onto your monitors.

## Ubuntu package

On Ubuntu 25.10 / 26.04 (amd64):

```bash
sudo apt install ./deco-driver_0.1.0_amd64.deb
```

Then log out and back in (udev + user systemd). The tray icon appears in the top bar; click it (or open **Deco Driver** from the app grid) for settings.

Uninstall:

```bash
sudo apt remove deco-driver
```

If the official XP-Pen `PenTablet` app is installed, remove it first so the two stacks do not fight over the stylus.

## From source

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libudev-dev
./packaging/install.sh          # ~/.local + systemd --user
sudo install -m644 packaging/udev/99-deco-driver.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Build a `.deb`:

```bash
./packaging/deb/build-deb.sh    # writes dist/deco-driver_*_amd64.deb
```

## What you get

| Piece | Role |
|---|---|
| `deco-daemon` | HID parse, express keys, roller, uinput, D-Bus `it.bwlab.DecoDriver` |
| `deco-ui` | Settings + StatusNotifier tray (`deco-ui --background`) |
| Mapping | One monitor, or both joined; optional keep-aspect letterbox |

Closing the window hides it in the tray. **Esci** in the tray menu quits.

## License

MIT. Protocol notes in `XPPenDriverSpec.md`.
