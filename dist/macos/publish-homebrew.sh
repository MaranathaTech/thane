#!/usr/bin/env bash
#
# publish-homebrew.sh — Update and push the Homebrew cask formula for thane.
#
# Called by dist/macos/package.sh after DMG upload.
#
# Prerequisites:
#   - gh CLI authenticated with push access to MaranathaTech/homebrew-tap
#   - shasum installed (ships with macOS)
#
# Usage:
#   ./dist/macos/publish-homebrew.sh <version> <dmg-path>
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
    err "Usage: ./dist/macos/publish-homebrew.sh <version> <dmg-path>"
    exit 1
fi

VERSION="$1"
DMG_PATH="$2"

if [ ! -f "$DMG_PATH" ]; then
    err "DMG not found: $DMG_PATH"
    exit 1
fi

TAP_REPO="MaranathaTech/homebrew-tap"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

info "Publishing thane $VERSION to Homebrew tap..."

# ── Calculate SHA256 ──────────────────────────────────────────────────────
info "Calculating SHA256 of DMG..."
SHA256=$(shasum -a 256 "$DMG_PATH" | awk '{print $1}')
ok "SHA256: $SHA256"

# ── Update local formula ─────────────────────────────────────────────────
info "Updating local formula..."
FORMULA="$SCRIPT_DIR/thane.rb"
sed -i.bak "s/^  version .*/  version \"${VERSION}\"/" "$FORMULA"
sed -i.bak "s/^  sha256 .*/  sha256 \"${SHA256}\"/" "$FORMULA"
rm -f "${FORMULA}.bak"

# ── Clone tap repo and update ────────────────────────────────────────────
info "Cloning homebrew tap..."
if ! gh repo clone "$TAP_REPO" "$WORK_DIR/tap" 2>/dev/null; then
    err "Failed to clone $TAP_REPO — ensure gh is authenticated"
    exit 1
fi

mkdir -p "$WORK_DIR/tap/Casks"
cp "$FORMULA" "$WORK_DIR/tap/Casks/thane.rb"

cd "$WORK_DIR/tap"

if git diff --quiet 2>/dev/null; then
    ok "Formula already up to date"
    exit 0
fi

git add Casks/thane.rb
git commit -m "Update thane to ${VERSION}"
git push origin main

ok "${BOLD}Homebrew tap updated to ${VERSION}${NC}"
