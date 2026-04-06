#!/usr/bin/env bash
#
# install.sh — Install thane from the extracted tarball.
#
# Usage:
#   tar xzf thane-linux-x86_64.tar.gz
#   cd thane-linux-x86_64
#   ./install.sh
#
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${BLUE}[thane]${NC} $*"; }
ok()    { echo -e "${GREEN}[thane]${NC} $*"; }
err()   { echo -e "${RED}[thane]${NC} $*" >&2; }

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Check runtime dependencies ───────────────────────────────────────────
info "Checking runtime dependencies..."
if command -v apt-get &>/dev/null; then
    MISSING=()
    for pkg in libgtk-4-1 libvte-2.91-gtk4-0 libwebkitgtk-6.0-4; do
        if ! dpkg -s "$pkg" &>/dev/null; then
            MISSING+=("$pkg")
        fi
    done
    if [ ${#MISSING[@]} -gt 0 ]; then
        info "Installing missing packages: ${MISSING[*]}"
        sudo apt-get update -qq
        sudo apt-get install -y -qq "${MISSING[@]}"
    fi
    ok "Runtime dependencies satisfied"
elif command -v dnf &>/dev/null; then
    sudo dnf install -y gtk4 vte291-gtk4 webkitgtk6.0 2>/dev/null || true
    ok "Runtime dependencies satisfied"
else
    info "Please ensure these runtime libraries are installed:"
    echo "  - GTK 4"
    echo "  - VTE (GTK 4 build)"
    echo "  - WebKitGTK 6.0"
fi

# ── Install binaries ─────────────────────────────────────────────────────
info "Installing binaries to /usr/local/bin..."
sudo install -m 755 "$SCRIPT_DIR/thane" /usr/local/bin/thane
sudo install -m 755 "$SCRIPT_DIR/thane-cli" /usr/local/bin/thane-cli
ok "Installed thane and thane-cli to /usr/local/bin"

# ── Install desktop entry ────────────────────────────────────────────────
DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$DESKTOP_DIR"
cp "$SCRIPT_DIR/com.thane.app.desktop" "$DESKTOP_DIR/"
ok "Desktop entry installed to $DESKTOP_DIR"

# ── Install icon ─────────────────────────────────────────────────────────
if [ -f "$SCRIPT_DIR/com.thane.app.svg" ]; then
    ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
    mkdir -p "$ICON_DIR"
    cp "$SCRIPT_DIR/com.thane.app.svg" "$ICON_DIR/"
    ok "Icon installed to $ICON_DIR"
fi

# ── Done ─────────────────────────────────────────────────────────────────
echo ""
ok "${BOLD}Installation complete!${NC}"
echo ""
echo "  Run:   thane"
echo "  Docs:  https://getthane.com/docs"
echo ""
