#!/usr/bin/env bash
#
# update-pkgbuild.sh — refresh the vendored AUR PKGBUILD to a stable upstream
# release so publishing to AUR needs no hand-editing. Unlike
# scripts/test-aur-install.sh (which patches a throwaway AUR clone and reverts
# on exit), this writes scripts/aur/demeteo/PKGBUILD in place, persistently:
# it resolves the release tag, downloads the tag tarball to compute its
# sha256, rewrites pkgver + sha256sums, and regenerates .SRCINFO.
#
# The sha256 can only be computed once the tag tarball exists on GitHub, which
# is *after* promote.yml tags and build.yml publishes — that's why the release
# workflow deliberately does NOT touch this file and leaves it to this script.
#
# Usage:
#   scripts/aur/update-pkgbuild.sh                latest stable release
#   scripts/aur/update-pkgbuild.sh v1.2.3         a specific stable tag
#
# Nightly/RC tags are rejected on purpose: AUR pkgver forbids the hyphen in
# tags like 1.0.0-28 and nightlies aren't published to AUR. For a local
# nightly install test use: scripts/test-aur-install.sh nightly
#
# Env:
#   GITHUB_REPO   upstream slug (default: stevenyepes/demeteo)
#
set -euo pipefail

GITHUB_REPO="${GITHUB_REPO:-stevenyepes/demeteo}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PKG_DIR="$SCRIPT_DIR/demeteo"
PKGBUILD="$PKG_DIR/PKGBUILD"

log()  { printf '\033[1;34m>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mX\033[0m %s\n' "$*" >&2; exit 1; }
ok()   { printf '\033[1;32m+\033[0m %s\n' "$*"; }

for c in curl python3 sha256sum; do
  command -v "$c" >/dev/null || die "$c not found in PATH"
done
[[ -f "$PKGBUILD" ]] || die "PKGBUILD not found at $PKGBUILD"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

ARG="${1:-stable}"
if [[ "$ARG" == "stable" ]]; then
  log "resolving latest stable release from $GITHUB_REPO"
  curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases" -o "$work/releases.json"
  TAG=$(python3 -c "
import json
d = json.load(open('$work/releases.json'))
s = [r for r in d if not r.get('prerelease') and not r.get('draft')]
print(s[0]['tag_name'] if s else '')
")
  [[ -n "$TAG" ]] || die "no stable release found on $GITHUB_REPO"
else
  TAG="$ARG"
fi

PKGVER="${TAG#v}"
# makepkg forbids hyphen/colon/slash/whitespace in pkgver; a hyphen here means
# a nightly/RC tag, which we don't publish to AUR.
case "$PKGVER" in
  *[-:/\ ]*)
    die "tag '$TAG' isn't AUR-valid (contains -, :, / or space) — that's a nightly/RC.
       Nightlies aren't published to AUR. For a local install test run:
         scripts/test-aur-install.sh nightly" ;;
esac

TARBALL_URL="https://github.com/${GITHUB_REPO}/archive/refs/tags/${TAG}.tar.gz"
log "tag=$TAG  pkgver=$PKGVER"
log "verifying tarball is published: $TARBALL_URL"
curl -fsSI "$TARBALL_URL" >/dev/null \
  || die "tarball not accessible — does tag '$TAG' exist on GitHub, and is the release published?"

log "downloading tarball to compute sha256"
curl -fsSL -o "$work/src.tar.gz" "$TARBALL_URL"
SHA256=$(sha256sum "$work/src.tar.gz" | awk '{print $1}')
log "sha256=$SHA256"

CUR_PKGVER="$(sed -n 's/^pkgver=//p' "$PKGBUILD")"

# The PKGBUILD's `source=` builds its URL from ${pkgver}, so only pkgver and
# sha256sums need rewriting. Reset pkgrel to 1 whenever the version changes
# (pkgrel is the packaging revision *within* a version, per Arch convention).
sed -i \
  -e "s|^pkgver=.*|pkgver=${PKGVER}|" \
  -e "s|^sha256sums=.*|sha256sums=('${SHA256}')|" \
  "$PKGBUILD"
if [[ "$CUR_PKGVER" != "$PKGVER" ]]; then
  sed -i "s|^pkgrel=.*|pkgrel=1|" "$PKGBUILD"
  log "version changed ($CUR_PKGVER → $PKGVER) — reset pkgrel=1"
fi

if command -v makepkg >/dev/null; then
  log "regenerating .SRCINFO"
  (cd "$PKG_DIR" && makepkg --printsrcinfo > .SRCINFO)
  ok "wrote $PKG_DIR/.SRCINFO"
else
  warn "makepkg not found — skipped .SRCINFO regeneration; run this on an Arch box before pushing to AUR"
fi

ok "updated $PKGBUILD → pkgver=$PKGVER, sha256sums refreshed"
echo
echo "Review the diff, then publish to your AUR clone:"
echo "  AUR_REPO=\"\${AUR_REPO:-\$HOME/aur/demeteo}\""
echo "  cp \"$PKGBUILD\" \"$PKG_DIR/.SRCINFO\" \"\$AUR_REPO\"/"
echo "  (cd \"\$AUR_REPO\" && git commit -am 'upgpkg: demeteo ${PKGVER}-1' && git push)"
