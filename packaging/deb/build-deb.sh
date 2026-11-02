#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ARCH="$(dpkg --print-architecture)"
NAME="deco-driver"
PKG="${NAME}_${VERSION}_${ARCH}"
STAGE="$ROOT/dist/$PKG"
DEB="$ROOT/dist/${PKG}.deb"

echo "Building $NAME $VERSION ($ARCH)"
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/usr/lib/x86_64-linux-gnu/pkgconfig}"
cargo build --release --workspace --manifest-path "$ROOT/Cargo.toml"

rm -rf "$STAGE"
install -Dm755 "$ROOT/target/release/deco-daemon" "$STAGE/usr/bin/deco-daemon"
install -Dm755 "$ROOT/target/release/deco-ui" "$STAGE/usr/bin/deco-ui"
install -Dm644 "$ROOT/packaging/systemd/deco-daemon.system.service" \
    "$STAGE/usr/lib/systemd/user/deco-daemon.service"
install -Dm644 "$ROOT/packaging/dbus/it.bwlab.DecoDriver.system.service" \
    "$STAGE/usr/share/dbus-1/services/it.bwlab.DecoDriver.service"
install -Dm644 "$ROOT/packaging/applications/it.bwlab.DecoUi.desktop" \
    "$STAGE/usr/share/applications/it.bwlab.DecoUi.desktop"
install -Dm644 "$ROOT/packaging/applications/it.bwlab.DecoUi-autostart.desktop" \
    "$STAGE/etc/xdg/autostart/it.bwlab.DecoUi.desktop"
install -Dm644 "$ROOT/packaging/udev/99-deco-driver.rules" \
    "$STAGE/usr/lib/udev/rules.d/99-deco-driver.rules"
install -Dm644 "$ROOT/packaging/icons/hicolor/scalable/apps/it.bwlab.DecoUi.svg" \
    "$STAGE/usr/share/icons/hicolor/scalable/apps/it.bwlab.DecoUi.svg"

if command -v rsvg-convert >/dev/null 2>&1; then
  for sz in 48 64 128; do
    mkdir -p "$STAGE/usr/share/icons/hicolor/${sz}x${sz}/apps"
    rsvg-convert -w "$sz" -h "$sz" \
      "$ROOT/packaging/icons/hicolor/scalable/apps/it.bwlab.DecoUi.svg" \
      -o "$STAGE/usr/share/icons/hicolor/${sz}x${sz}/apps/it.bwlab.DecoUi.png"
  done
fi

mkdir -p "$STAGE/DEBIAN" "$STAGE/debian"
install -m755 "$ROOT/packaging/deb/postinst" "$STAGE/DEBIAN/postinst"
install -m755 "$ROOT/packaging/deb/prerm" "$STAGE/DEBIAN/prerm"

# dpkg-shlibdeps wants a debian/control skeleton next to the staged tree.
cat > "$STAGE/debian/control" <<EOF
Source: $NAME
Package: $NAME
Architecture: $ARCH
Depends: \${shlibs:Depends}
EOF
SHLIBS="$(
  cd "$STAGE"
  dpkg-shlibdeps -O usr/bin/deco-ui usr/bin/deco-daemon 2>/dev/null | sed 's/^shlibs:Depends=//'
)"
rm -rf "$STAGE/debian"
SIZE_KB="$(du -sk "$STAGE" | awk '{print $1}')"

cat > "$STAGE/DEBIAN/control" <<EOF
Package: $NAME
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Maintainer: Luigi Massa <lmassa@bwlab.it>
Installed-Size: $SIZE_KB
Depends: ${SHLIBS:-libgtk-4-1, libadwaita-1-0, libglib2.0-0t64, libudev1}
Homepage: https://github.com/luigimassa/deco-driver
Description: Userspace driver for XP-Pen Deco 02 on GNOME/Ubuntu
 GTK4 settings app and systemd user daemon that talk to the Deco 02 over
 hidraw and inject pen/keys through uinput. Maps the tablet onto GNOME
 monitors without the vendor kernel module.
EOF

chmod 755 "$STAGE/usr/bin/deco-daemon" "$STAGE/usr/bin/deco-ui"
find "$STAGE" -type d -exec chmod 755 {} +
mkdir -p "$ROOT/dist"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB"
echo "Built $DEB"
dpkg-deb -I "$DEB"
