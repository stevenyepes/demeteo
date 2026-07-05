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

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    # Native Linux dev — default build is already Linux x86_64 ELF.
    TARGET_FLAG=()
    OUT_DIR="target/release"
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
    OUT_DIR="target/x86_64-unknown-linux-musl/release"
    ;;
  *)
    echo "error: unsupported host $(uname -s)-$(uname -m) — install demeteo-runner manually." >&2
    exit 2
    ;;
esac

echo "==> Building demeteo-runner" >&2
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