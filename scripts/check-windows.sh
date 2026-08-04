#!/usr/bin/env bash
#
# Type-check the Windows target from Linux.
#
# ## Why this exists
#
# The desktop app ships on Windows, but `scripts/checks.sh` and the PR gate both
# run on Linux only, and everything Windows-specific sits behind `#[cfg(windows)]`
# — which the Linux compiler never parses past name resolution. The result is a
# gate that is green while the Windows build is broken, and the breakage surfaces
# on master or a tag, after merge. That is not hypothetical: the first native
# Windows commit reached this tree with six `cfg(windows)` compile errors that no
# local gate could see.
#
# `cargo check` needs no linker, so a full type-check of the Windows target is
# reachable from Linux — but it does need to *build the build scripts* of C
# dependencies for that target, and `aws-lc-sys` (via reqwest → rustls) is one.
# Hence mingw-w64 rather than plain rustup.
#
# ## Why the `gnu` target and not `msvc`
#
# `x86_64-pc-windows-msvc` is what ships (see `build.yml`), and it is the more
# faithful check. It is also not reachable here: its C dependencies want the MSVC
# toolchain, which does not cross-compile from Linux. `x86_64-pc-windows-gnu`
# shares the same `cfg(windows)` surface, the same `windows-sys` bindings and the
# same std API, so it catches the entire class of error this exists for. What it
# does NOT catch is MSVC-only linkage and ABI differences — so a green run here
# is evidence the Windows *source* is coherent, not that the shipped artifact
# links. CI on `windows-latest` remains the authority.
#
# ## Usage
#
#   scripts/check-windows.sh              # check demeteo-core
#   scripts/check-windows.sh --all        # also clippy, and the src-tauri crate
#
# Skips with exit 0 and an explanation when the toolchain is absent, so it is
# safe to call from a wrapper that runs on machines without mingw-w64.
#
# Setup on a machine that lacks it:
#   <your package manager> install mingw-w64-gcc
#   rustup target add x86_64-pc-windows-gnu
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

step() { printf '\n\033[1;34m==>\033[0m %s\n' "$1"; }
skip() { printf '\033[1;33m==>\033[0m %s\n' "$1"; exit 0; }

TARGET=x86_64-pc-windows-gnu

command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 \
  || skip "Windows check skipped: x86_64-w64-mingw32-gcc not found (install mingw-w64)."

rustup target list --installed | grep -qx "$TARGET" \
  || skip "Windows check skipped: rustup target $TARGET not installed."

# A separate target dir on purpose. Sharing one with the host build thrashes the
# cache — every alternation between host and Windows targets rebuilds the world —
# and holds cargo's lock, which blocks a concurrent `checks.sh`.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/src-tauri/target/win-check}"

export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc

step "Windows type-check — demeteo-core ($TARGET)"
cargo check -p demeteo-core --lib --all-targets --target "$TARGET"

if [[ "${1:-}" == "--all" ]]; then
  step "Windows clippy — demeteo-core ($TARGET)"
  cargo clippy -p demeteo-core --all-targets --target "$TARGET" -- -D warnings

  # Not in the default run: the Tauri crate drags in the webkit/GTK-adjacent
  # dependency graph, and a failure here is far more often a cross-build
  # environment gap than a real Windows source error.
  step "Windows type-check — src-tauri ($TARGET)"
  cargo check --manifest-path src-tauri/Cargo.toml --lib --target "$TARGET"
fi

printf '\n\033[1;32m✔ Windows target type-checks\033[0m\n'
