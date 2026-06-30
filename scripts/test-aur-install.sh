#!/usr/bin/env bash
#
# test-aur-install.sh — build the demeteo AUR package locally against an
# upstream tag and (optionally) install it with sudo pacman -U.
#
# Patches PKGBUILD + .SRCINFO in $AUR_REPO with the chosen tag + sha256,
# runs makepkg, offers to install, then restores the AUR working tree.
#
# Usage:
#   scripts/test-aur-install.sh                       interactive prompts
#   scripts/test-aur-install.sh stable                latest stable tag
#   scripts/test-aur-install.sh nightly               latest RC tag (local only)
#   scripts/test-aur-install.sh <tag>                 specific tag, e.g. v0.2.0 or 0.1.0-28
#
# Install (default: prompt after a successful build):
#   --install          install the .pkg.tar.* with sudo pacman -U
#   --no-install       skip install; print the install command and keep the package
#
# Env:
#   AUR_REPO      path to AUR clone (default: ~/aur/demeteo)
#   GITHUB_REPO   upstream slug (default: stevenyepes/demeteo)
#   KEEP_PKG      if set, do not delete the built .pkg.tar.* on exit
#
set -euo pipefail

MODE=""
INSTALL_CHOICE="prompt"

for arg in "$@"; do
  case "$arg" in
    --install)    INSTALL_CHOICE="yes" ;;
    --no-install) INSTALL_CHOICE="no" ;;
    -h|--help)
      sed -n '2,22p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    --*)
      printf '\033[1;31mX\033[0m unknown flag: %s\n' "$arg" >&2
      exit 64
      ;;
    *)
      if [[ -n "$MODE" ]]; then
        printf '\033[1;31mX\033[0m multiple modes given: %s and %s\n' "$MODE" "$arg" >&2
        exit 64
      fi
      MODE="$arg"
      ;;
  esac
done

AUR_REPO="${AUR_REPO:-$HOME/aur/demeteo}"
GITHUB_REPO="${GITHUB_REPO:-stevenyepes/demeteo}"

# Resolve the vendored PKGBUILD template relative to this script so the
# first-publish bootstrap works regardless of how the script is invoked.
SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
PKGBUILD_TEMPLATE="$SCRIPT_DIR/aur/demeteo/PKGBUILD"

log()  { printf '\033[1;34m>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mX\033[0m %s\n' "$*" >&2; exit 1; }
ok()   { printf '\033[1;32m+\033[0m %s\n' "$*"; }

for c in curl python3 makepkg sha256sum pacman sudo; do
  command -v "$c" >/dev/null || die "$c not found in PATH"
done

[[ -d "$AUR_REPO" ]] || mkdir -p "$AUR_REPO"
if [[ ! -f "$AUR_REPO/PKGBUILD" ]]; then
  [[ -f "$PKGBUILD_TEMPLATE" ]] || die "PKGBUILD missing in $AUR_REPO and no template at $PKGBUILD_TEMPLATE"
  warn "PKGBUILD missing at $AUR_REPO — bootstrapping from vendored template"
  cp "$PKGBUILD_TEMPLATE" "$AUR_REPO/PKGBUILD"
  if [[ ! -d "$AUR_REPO/.git" ]]; then
    (cd "$AUR_REPO" && \
      git init -q -b master && \
      git -c user.name=demeteo-bootstrap -c user.email=demeteo@localhost \
        add PKGBUILD && \
      git -c user.name=demeteo-bootstrap -c user.email=demeteo@localhost \
        commit -q -m "init: vendor PKGBUILD from scripts/aur/demeteo/PKGBUILD") || true
    warn "created empty git repo at $AUR_REPO (edit Maintainer/email before pushing to AUR)"
  fi
fi

log "pre-run cleanup of $AUR_REPO"
is_git_repo=0
if [[ -d "$AUR_REPO/.git" ]] && git -C "$AUR_REPO" rev-parse --git-dir >/dev/null 2>&1; then
  is_git_repo=1
fi
if [[ "$is_git_repo" -eq 1 ]]; then
  if ! git -C "$AUR_REPO" diff --quiet -- PKGBUILD; then
    git -C "$AUR_REPO" checkout -- PKGBUILD
    log "  restored PKGBUILD from HEAD"
  fi
  if [[ -f "$AUR_REPO/.SRCINFO" ]]; then
    if git -C "$AUR_REPO" ls-files --error-unmatch .SRCINFO >/dev/null 2>&1; then
      : # tracked, leave it
    else
      rm -f "$AUR_REPO/.SRCINFO"
      log "  removed untracked .SRCINFO"
    fi
  fi
fi
shopt -s nullglob
find "$AUR_REPO" -maxdepth 1 -type f \( -name '*.tar.gz' -o -name '*.tar.zst' \) ! -name '*.pkg.tar.*' -print -delete 2>/dev/null | while read -r f; do
  log "  removed loose $f"
done
shopt -u nullglob
for d in src pkg src-tauri/target node_modules dist npm-cache cargo-home; do
  if [[ -d "$AUR_REPO/$d" ]]; then
    rm -rf "$AUR_REPO/$d"
    log "  removed $d/"
  fi
done

work=$(mktemp -d)

cleanup() {
  if [[ -f "$work/PKGBUILD.orig" ]]; then
    cp "$work/PKGBUILD.orig" "$AUR_REPO/PKGBUILD"
  fi
  if [[ -f "$work/SRCINFO.orig" ]]; then
    cp "$work/SRCINFO.orig" "$AUR_REPO/.SRCINFO"
  fi
  if [[ -z "${KEEP_PKG:-}" ]]; then
    rm -f "$AUR_REPO"/PKGBUILD.bak 2>/dev/null || true
  fi
  (cd "$AUR_REPO" \
    && shopt -s nullglob \
    && find . -maxdepth 1 -type f \( -name '*.tar.gz' -o -name '*.tar.zst' \) ! -name '*.pkg.tar.*' -delete \
    && rm -rf src pkg src-tauri/target node_modules dist npm-cache cargo-home \
    && shopt -u nullglob) 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

if [[ -z "$MODE" ]]; then
  printf "Test against which version?\n"
  printf "  1) latest stable\n"
  printf "  2) latest nightly (RC — local only, cannot push to AUR)\n"
  read -rp "Choice [1/2]: " choice
  case "$choice" in
    1|stable)     MODE="stable" ;;
    2|nightly|rc) MODE="nightly" ;;
    *)            die "invalid choice" ;;
  esac
fi

log "fetching release info from $GITHUB_REPO"
curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases" -o "$work/releases.json"

case "$MODE" in
  stable)
    TAG=$(python3 -c "
import json
data = json.load(open('$work/releases.json'))
stable = [r for r in data if not r.get('prerelease') and not r.get('draft')]
print(stable[0]['tag_name'] if stable else '')
")
    [[ -n "$TAG" ]] || die "no stable release found"
    log "mode=stable  tag=$TAG"
    ;;
  nightly)
    TAG=$(python3 -c "
import json
data = json.load(open('$work/releases.json'))
pre = [r for r in data if r.get('prerelease')]
print(pre[0]['tag_name'] if pre else '')
")
    [[ -n "$TAG" ]] || die "no nightly (RC) release found"
    log "mode=nightly tag=$TAG"
    warn "nightly builds cannot be published to AUR — this is a local install test"
    ;;
  *)
    TAG="$MODE"
    log "mode=specific tag=$TAG"
    ;;
esac

PKGVER="${TAG#v}"
TARBALL_URL="https://github.com/${GITHUB_REPO}/archive/refs/tags/${TAG}.tar.gz"

log "checking tarball: $TARBALL_URL"
curl -fsSI "$TARBALL_URL" >/dev/null || die "tarball not accessible (tag exists on GitHub?)"

log "downloading tarball to compute sha256"
curl -fsSL -o "$work/tarball.tar.gz" "$TARBALL_URL"
SHA256=$(sha256sum "$work/tarball.tar.gz" | awk '{print $1}')
log "sha256=$SHA256"

# makepkg rejects hyphens, colons, slashes, whitespace in pkgver. Hyphenated
# nightly tags like 0.1.0-28 are rewritten with dots so the local build can
# proceed; the resulting binary reports its version from tauri.conf.json, so
# this only affects package metadata.
MAK_PKGVER="${PKGVER//-/.}"
if [[ "$PKGVER" != "$MAK_PKGVER" ]]; then
  warn "pkgver '$PKGVER' has a hyphen — using '$MAK_PKGVER' for local build (AUR pushes must use AUR-valid versions)"
fi

cp "$AUR_REPO/PKGBUILD" "$work/PKGBUILD.orig"
cp "$AUR_REPO/.SRCINFO" "$work/SRCINFO.orig" 2>/dev/null || : # SRCINFO may be missing on first run
# pkgver is rewritten to the makepkg-valid form (hyphens → dots) so the
# build can proceed; the tarball's source directory and Tauri's output
# artifact still use the original tag-with-hyphen, so we swap ${pkgver}
# back to ${PKGVER} in the cd/find sites — every path that points at the
# extracted upstream tree or the produced .deb.
sed -i \
  -e "s|^pkgver=.*|pkgver=${MAK_PKGVER}|" \
  -e "s|^source=(.*)|source=(\"${TARBALL_URL}\")|" \
  -e "s|^sha256sums=.*|sha256sums=('${SHA256}')|" \
  -e "s|\"\\\${srcdir}/\\\${pkgname}-\\\${pkgver}\"|\"\\\${srcdir}/\\\${pkgname}-${PKGVER}\"|g" \
  -e "s|\"\\\${pkgname}-\\\${pkgver}\"|\"\\\${pkgname}-${PKGVER}\"|g" \
  -e "s|\\\${pkgname}_\\\${pkgver}_|\\\${pkgname}_${PKGVER}_|g" \
  "$AUR_REPO/PKGBUILD"

if [[ "$MAK_PKGVER" =~ [-:/[[:space:]] ]]; then
  warn "pkgver '${MAK_PKGVER}' still contains characters makepkg rejects; SRCINFO left as-is"
else
  log "regenerating .SRCINFO"
  (cd "$AUR_REPO" && makepkg --printsrcinfo > .SRCINFO)
fi

log "running makepkg (no auto-install of system deps; ~10 min on first build)"
build_log="$work/build.log"
build_ok=0
set +e
(cd "$AUR_REPO" && makepkg -f --nocheck) 2>&1 | tee "$build_log"
build_ok=${PIPESTATUS[0]}
set -e

echo
if [[ $build_ok -ne 0 ]]; then
  die "build failed — last 60 lines of $build_log:"
  tail -n 60 "$build_log" >&2
fi
ok "build succeeded for $TAG (makepkg pkgver=$MAK_PKGVER, original=$PKGVER)"

shopt -s nullglob
pkgs=( "$AUR_REPO"/*.pkg.tar.* )
shopt -u nullglob
if [[ ${#pkgs[@]} -eq 0 ]]; then
  die "build reported success but no .pkg.tar.* produced"
fi
echo
echo "Built package(s):"
ls -1 "${pkgs[@]}" 2>/dev/null

should_install() {
  case "$INSTALL_CHOICE" in
    yes) return 0 ;;
    no)  return 1 ;;
    prompt)
      read -rp "Install now with sudo pacman -U? [y/N]: " ans
      [[ "${ans,,}" == "y" || "${ans,,}" == "yes" ]]
      ;;
  esac
}

echo
if should_install; then
  log "installing with sudo pacman -U --needed"
  sudo pacman -U --needed "${pkgs[@]}"
  ok "install complete"
else
  echo "To install manually:"
  echo "  sudo pacman -U ${pkgs[*]}"
fi

# Keep the .pkg.tar.* on disk either way — re-install on other machines or
# share without rebuilding. KEEP_PKG already implies this; the explicit
# statement here documents the policy even when KEEP_PKG is unset.
echo
ok "package kept at:"
for p in "${pkgs[@]}"; do echo "  $p"; done
