#!/usr/bin/env bash
# Build demeteo-runner for the Linux target that the remote-runner
# install expects (x86_64-unknown-linux-musl) and drop it in the dev
# cache that `crates/demeteo-core/src/infrastructure/runner/binary.rs`
# looks at first. On a native-Linux dev box we skip the cross target
# and use the default build (it's already ELF), so this script works
# the same on macOS-arm64, macOS-x86_64, and Linux-laptops.
#
# Usage: scripts/build-runner.sh
set -euo pipefail

ASSET_NAME="demeteo-runner-x86_64-unknown-linux-musl"
TMPDIR="${TMPDIR:-/tmp}"
CACHE_DIR="$TMPDIR/demeteo-runner-cache/dev"
CACHE_PATH="$CACHE_DIR/$ASSET_NAME"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Resolve cargo's real target directory rather than assuming `./target`:
# this repo pins `target-dir = "src-tauri/target"` in .cargo/config.toml
# (the same path CI reads the runner asset from), so a hardcoded `target/`
# would look in the wrong place and the binary check below would spuriously
# fail. Honour CARGO_TARGET_DIR, then ask cargo, then fall back to the pin.
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --no-deps --format-version 1 2>/dev/null \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')}"
TARGET_DIR="${TARGET_DIR:-$REPO_ROOT/src-tauri/target}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    # Native Linux dev — default build is already Linux x86_64 ELF.
    TARGET_FLAG=()
    OUT_DIR="$TARGET_DIR/release"
    ;;
  Linux-*)
    echo "error: this script only produces x86_64 Linux builds. On $(uname -m) Linux you'll need" >&2
    echo "       to install the cross target and adjust the asset name. Demeteo's remote" >&2
    echo "       runners all run on x86_64." >&2
    exit 2
    ;;
  Darwin-*)
    # Mac dev — needs the musl target so the result is a static Linux
    # ELF, not a Mach-O the remote can't execute.
    if ! rustup target list --installed | grep -q '^x86_64-unknown-linux-musl$'; then
      echo "==> Installing x86_64-unknown-linux-musl target (one-time)" >&2
      rustup target add x86_64-unknown-linux-musl
    fi
    TARGET_FLAG=(--target x86_64-unknown-linux-musl)
    OUT_DIR="$TARGET_DIR/x86_64-unknown-linux-musl/release"
    ;;
  *)
    echo "error: unsupported host $(uname -s)-$(uname -m) — install demeteo-runner manually." >&2
    exit 2
    ;;
esac

# Anchor the runner's self-reported version to the desktop app's, exactly
# as CI does. The laptop string-compares `demeteo-runner --version` against
# `app.package_info().version` (sourced from tauri.conf.json) to decide
# install-vs-upgrade, so a dev runner MUST report that same value — otherwise
# it always looks "stale" (the 0.1.0-vs-1.0.0 mismatch this fixes). We read
# tauri.conf.json (the app's authoritative version) rather than trusting the
# crate version so the dev runner tracks the app even mid-bump.
TAURI_CONF="$REPO_ROOT/src-tauri/tauri.conf.json"
DEMETEO_RUNNER_VERSION="$(grep -m1 '"version"' "$TAURI_CONF" | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
if [ -z "$DEMETEO_RUNNER_VERSION" ]; then
  echo "error: couldn't read version from $TAURI_CONF" >&2
  exit 1
fi
export DEMETEO_RUNNER_VERSION
echo "==> Building demeteo-runner (version $DEMETEO_RUNNER_VERSION, from tauri.conf.json)" >&2
cargo build --release -p demeteo-runner "${TARGET_FLAG[@]}"

BIN_SRC="$OUT_DIR/demeteo-runner"
if [ ! -f "$BIN_SRC" ]; then
  echo "error: build didn't produce $BIN_SRC" >&2
  exit 1
fi

mkdir -p "$CACHE_DIR"
cp "$BIN_SRC" "$CACHE_PATH"
chmod +x "$CACHE_PATH"

echo "==> demeteo-runner built and cached at $CACHE_PATH" >&2
echo "    Open Demeteo → Settings → Machines → click 'Enable remote runs' on any" >&2
echo "    machine. The app will pick this up automatically (no env var needed)." >&2