#!/bin/sh
# Manual installer for the Demeteo headless runner (M2.1).
#
# Not wired into the app yet — that's M7.1's "one-click enable remote
# runs" from MachinesView, which pushes the binary over SFTP and drives
# this same install/enable/linger sequence remotely. Until then, run this
# by hand on the target Linux machine after building
# `cargo build --release -p demeteo-runner` (or placing a prebuilt
# static binary at the path below).
#
# Usage: packaging/install.sh [path-to-demeteo-runner-binary]

set -e

BIN_SRC="${1:-target/release/demeteo-runner}"
BIN_DST="$HOME/.local/bin/demeteo-runner"
UNIT_SRC="$(dirname "$0")/systemd/demeteo-runner.service"
UNIT_DST="$HOME/.config/systemd/user/demeteo-runner.service"

if [ ! -f "$BIN_SRC" ]; then
    echo "error: binary not found at $BIN_SRC (build with: cargo build --release -p demeteo-runner)" >&2
    exit 1
fi

mkdir -p "$(dirname "$BIN_DST")" "$(dirname "$UNIT_DST")" "$HOME/.local/share/demeteo-runner"
cp "$BIN_SRC" "$BIN_DST"
chmod +x "$BIN_DST"
cp "$UNIT_SRC" "$UNIT_DST"

systemctl --user daemon-reload
systemctl --user enable --now demeteo-runner

echo "demeteo-runner installed and started as a systemd --user service."
echo "Check status with: systemctl --user status demeteo-runner"
echo "Tail logs with:    journalctl --user -u demeteo-runner -f"

# Lingering (R2) is what lets the unit survive SSH logout and start at
# boot with no interactive login — required for "close the laptop, the
# run keeps going." Needs admin/polkit on some distros; if it fails here,
# the unit is still installed and running for the current session, but
# won't survive logout/reboot until an admin runs this for you (§10.8).
if loginctl enable-linger "$(whoami)" 2>/dev/null; then
    echo "Lingering enabled — this service will survive logout and reboot."
else
    echo "warning: could not enable lingering (loginctl enable-linger $(whoami))." >&2
    echo "         The service is running now but will NOT survive SSH logout" >&2
    echo "         or a reboot until an administrator runs that command." >&2
fi
