#!/usr/bin/env bash
#
# Provision the Wine prefix that `check-windows.sh --run` executes against.
#
# ## Why this exists
#
# `check-windows.sh` type-checks the Windows target and stops there, because
# `cargo check` needs no linker. Linking turns out to be reachable too: the
# `gnu` target links the whole `demeteo-core` test binary — `ssh2`, `keyring`,
# `reqwest` and all — and Wine then runs it. That upgrades the Linux-side gate
# from "the Windows source is coherent" to "the `cfg(windows)` bodies behave",
# for a loop measured in seconds rather than a CI round trip.
#
# What that costs is a prefix with the tools the suite shells out to. Git is
# the whole of it: ~70 of the ~114 failures against a bare prefix are nothing
# but a missing `git.exe`.
#
# ## Why PortableGit, extracted rather than installed
#
# The installer is Inno Setup and wants a working MSYS2 runtime to finish;
# the archive is the same tree without that dependency. The registry value the
# installer would have written is set here directly, so
# `shared/win/discovery.rs` resolves Git through its `HKLM\SOFTWARE\
# GitForWindows` arm — the arm a real install exercises — instead of falling
# through to the well-known-directory guess.
#
# ## Why PowerShell is deliberately absent
#
# Several tests use `pwsh` as the program they spawn. PowerShell 7 does install
# and start under Wine, and is then useless in the one way that matters: it
# prints nothing to stdout, nothing to stderr, and exits 0 for `pwsh -Command
# 'exit 42'`. A vehicle that reports success for everything asked of it turns
# its tests green while proving nothing — AGENTS.md §7's objection to a double
# that answers `Ok("")` to every call, arriving through the environment instead
# of through a stub. Leaving it out costs those tests an honest "program not
# found"; they are listed in `wine-known-failures.txt` and belong to CI.
#
# Usage:  scripts/setup-wine-prefix.sh [--force]
#
# The prefix is disposable: delete "$DEMETEO_WINEPREFIX" and run this again.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PREFIX="${DEMETEO_WINEPREFIX:-$HOME/.local/share/demeteo-wine}"
GIT_VERSION="${DEMETEO_WINE_GIT_VERSION:-2.55.0.windows.3}"
GIT_ARCHIVE_VERSION="${GIT_VERSION%.windows.*}.${GIT_VERSION##*.windows.}"

step() { printf '\n\033[1;34m==>\033[0m %s\n' "$1"; }
fail() { printf '\033[1;31m==>\033[0m %s\n' "$1" >&2; exit 1; }

command -v wine >/dev/null 2>&1 || fail "wine is not installed."
command -v 7z >/dev/null 2>&1 || fail "7z is not installed (p7zip)."
command -v curl >/dev/null 2>&1 || fail "curl is not installed."

if [[ "${1:-}" == "--force" ]]; then
  step "Removing $PREFIX"
  rm -rf "$PREFIX"
fi

export WINEPREFIX="$PREFIX"
# Wine offers to download Mono and Gecko on first boot. Nothing here is managed
# or HTML, and the dialogs block a non-interactive run.
export WINEDLLOVERRIDES="mscoree,mshtml="
export WINEDEBUG=-all

step "Booting prefix at $PREFIX"
mkdir -p "$PREFIX"
wineboot -u >/dev/null 2>&1

GIT_DIR="$PREFIX/drive_c/Program Files/Git"
if [[ -f "$GIT_DIR/cmd/git.exe" ]]; then
  step "Git for Windows already present"
else
  step "Fetching PortableGit $GIT_VERSION"
  archive="$(mktemp -t portablegit-XXXXXX.7z.exe)"
  trap 'rm -f "$archive"' EXIT
  curl -sSL --fail -o "$archive" \
    "https://github.com/git-for-windows/git/releases/download/v${GIT_VERSION}/PortableGit-${GIT_ARCHIVE_VERSION}-64-bit.7z.exe" \
    || fail "download failed — check the version in DEMETEO_WINE_GIT_VERSION."
  step "Extracting into the prefix"
  mkdir -p "$GIT_DIR"
  7z x -y -o"$GIT_DIR" "$archive" >/dev/null
fi

step "Registering Git with the prefix"
# `discovery.rs` reads this key first; a bare extraction leaves it unset and the
# resolver then depends on a fallback the real install never needs.
wine reg add 'HKLM\Software\GitForWindows' /v InstallPath /t REG_SZ \
  /d 'C:\Program Files\Git' /f >/dev/null 2>&1
# A Windows process gets its PATH from the registry. The Unix PATH does not
# reach it, so exporting one here would do nothing.
wine reg add 'HKCU\Environment' /v PATH /t REG_EXPAND_SZ \
  /d 'C:\Program Files\Git\cmd' /f >/dev/null 2>&1

step "Verifying"
version="$(wine cmd /c 'git --version' 2>/dev/null | tr -d '\r')"
[[ "$version" == git\ version\ * ]] || fail "git did not run inside the prefix."
printf '    %s\n' "$version"
bash_version="$(wine 'C:\Program Files\Git\bin\bash.exe' -c 'echo ${BASH_VERSION:-none}' 2>/dev/null | tr -d '\r')"
[[ -n "$bash_version" && "$bash_version" != "none" ]] || fail "Git Bash did not answer the version probe."
printf '    bash %s\n' "$bash_version"

printf '\n\033[1;32m✔ Prefix ready — run scripts/check-windows.sh --run\033[0m\n'
