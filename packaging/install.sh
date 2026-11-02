#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"

echo "Building deco-daemon and deco-ui…"
echo "Need: pkg-config libgtk-4-dev libadwaita-1-dev libudev-dev (sudo apt install …)"
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/usr/lib/x86_64-linux-gnu/pkgconfig}"
cargo build --release --workspace --manifest-path "$ROOT/Cargo.toml"

install -Dm755 "$ROOT/target/release/deco-daemon" "$PREFIX/bin/deco-daemon"
install -Dm755 "$ROOT/target/release/deco-ui" "$PREFIX/bin/deco-ui"
install -Dm644 "$ROOT/packaging/systemd/deco-daemon.service" \
    "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/deco-daemon.service"
install -Dm644 "$ROOT/packaging/applications/it.bwlab.DecoUi.desktop" \
    "$PREFIX/share/applications/it.bwlab.DecoUi.desktop"
install -Dm644 "$ROOT/packaging/applications/it.bwlab.DecoUi-autostart.desktop" \
    "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/it.bwlab.DecoUi.desktop"
install -Dm644 "$ROOT/packaging/dbus/it.bwlab.DecoDriver.service" \
    "$PREFIX/share/dbus-1/services/it.bwlab.DecoDriver.service"
install -Dm644 "$ROOT/packaging/icons/hicolor/scalable/apps/it.bwlab.DecoUi.svg" \
    "$PREFIX/share/icons/hicolor/scalable/apps/it.bwlab.DecoUi.svg"

systemctl --user daemon-reload
systemctl --user enable --now deco-daemon.service

echo
echo "== post-install self-check =="
"$PREFIX/bin/deco-daemon" --self-check || true

echo
echo "User bits installed. Udev rule still needs root:"
echo "  sudo install -m644 $ROOT/packaging/udev/99-deco-driver.rules /etc/udev/rules.d/"
echo "  sudo udevadm control --reload-rules && sudo udevadm trigger"
echo "If the vendor XP-Pen driver is installed, uninstall it first to avoid two uinput stacks."
echo
echo "After plugging the tablet:"
echo "  journalctl --user -u deco-daemon.service -f"
echo "  deco-daemon --validate --seconds=8"
echo "  $ROOT/packaging/validate.sh"
