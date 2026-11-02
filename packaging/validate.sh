#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DAEMON="${DAEMON:-}"
if [[ -z "$DAEMON" ]]; then
  if command -v deco-daemon >/dev/null; then
    DAEMON=deco-daemon
  elif [[ -x "$ROOT/target/debug/deco-daemon" ]]; then
    DAEMON="$ROOT/target/debug/deco-daemon"
  elif [[ -x "$ROOT/target/release/deco-daemon" ]]; then
    DAEMON="$ROOT/target/release/deco-daemon"
  else
    echo "FAIL: deco-daemon binary not found (build or install first)"
    exit 2
  fi
fi

echo "== deco-daemon --self-check =="
"$DAEMON" --self-check || true
echo
echo "== lsusb XP-Pen (28bd) =="
lsusb -d 28bd: || echo "no XP-Pen USB device present"
echo
echo "== hidraw =="
ls -l /dev/hidraw* 2>/dev/null || echo "no hidraw"
echo
echo "== uinput =="
ls -l /dev/uinput /dev/input/uinput 2>/dev/null || echo "no uinput node"
echo
echo "== vendor driver conflict =="
pgrep -a PenTablet || echo "PenTablet not running (good)"
echo
echo "== D-Bus =="
if busctl --user status it.bwlab.DecoDriver >/dev/null 2>&1; then
  busctl --user call it.bwlab.DecoDriver /it/bwlab/DecoDriver it.bwlab.DecoDriver1 SelfCheck
else
  echo "deco-daemon not on session bus"
fi
echo
echo "== journal (last 40 deco-daemon lines) =="
journalctl --user -u deco-daemon.service -n 40 --no-pager 2>/dev/null || true
echo
echo "== HID dump (8s) if tablet present =="
if lsusb -d 28bd: >/dev/null 2>&1; then
  "$DAEMON" --validate --seconds=8
else
  echo "skip live dump — connect the Deco 02 and re-run"
fi
