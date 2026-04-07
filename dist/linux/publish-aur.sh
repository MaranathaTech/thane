#!/usr/bin/env bash
#
# publish-aur.sh — Update and push the AUR PKGBUILD for thane.
#
# Called by dist/linux/package.sh after tarball upload.
#
# Prerequisites:
#   - SSH key configured for aur.archlinux.org
#   - git installed
#
# Usage:
#   ./dist/linux/publish-aur.sh <version> <tarball-sha256>
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Colors ─────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${BLUE}==>${NC} $*"; }
ok()    { echo -e "${GREEN}==>${NC} $*"; }
err()   { echo -e "${RED}==>${NC} $*" >&2; }

# ── Validate args ─────────────────────────────────────────────────────────
if [ -z "${1:-}" ] || [ -z "${2:-}" ]; then
    err "Usage: ./dist/linux/publish-aur.sh <version> <tarball-sha256>"
    exit 1
fi

VERSION="$1"
SHA256="$2"
AUR_REMOTE="ssh://aur@aur.archlinux.org/thane.git"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

info "Publishing thane $VERSION to AUR..."

# ── Clone AUR repo ────────────────────────────────────────────────────────
info "Cloning AUR repo..."
if ! git clone "$AUR_REMOTE" "$WORK_DIR/aur" 2>/dev/null; then
    # First time — init the repo
    mkdir -p "$WORK_DIR/aur"
    cd "$WORK_DIR/aur"
    git init
    git remote add origin "$AUR_REMOTE"
fi

cd "$WORK_DIR/aur"

# ── Update PKGBUILD ──────────────────────────────────────────────────────
info "Updating PKGBUILD..."
cp "$SCRIPT_DIR/aur/PKGBUILD" ./PKGBUILD

# Replace version and checksum
sed -i "s/^pkgver=.*/pkgver=${VERSION}/" PKGBUILD
sed -i "s/^sha256sums=.*/sha256sums=('${SHA256}')/" PKGBUILD

# ── Generate .SRCINFO ────────────────────────────────────────────────────
if command -v makepkg &>/dev/null; then
    info "Generating .SRCINFO..."
    makepkg --printsrcinfo > .SRCINFO
else
    info "makepkg not found — generating minimal .SRCINFO..."
    cat > .SRCINFO <<EOF
pkgbase = thane
	pkgdesc = AI-native terminal workspace manager for Linux
	pkgver = ${VERSION}
	pkgrel = 1
	url = https://github.com/MaranathaTech/thane
	arch = x86_64
	license = AGPL-3.0-or-later
	makedepends = rust
	makedepends = cargo
	makedepends = meson
	makedepends = glib2
	depends = gtk4
	depends = vte4
	depends = webkit2gtk-5.0
	depends = dbus
	optdepends = ghostty: for Ghostty-compatible terminal configuration
	source = thane-${VERSION}.tar.gz::https://github.com/MaranathaTech/thane/archive/v${VERSION}.tar.gz
	sha256sums = ${SHA256}

pkgname = thane
EOF
fi

# ── Commit and push ──────────────────────────────────────────────────────
info "Committing and pushing..."
git add PKGBUILD .SRCINFO
git commit -m "Update to ${VERSION}"
git push origin master

ok "${BOLD}AUR package updated to ${VERSION}${NC}"
