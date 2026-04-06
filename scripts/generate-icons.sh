#!/bin/bash
# Generate PNG icons from the SVG source at standard sizes.
# Requires rsvg-convert (from librsvg2-bin) or inkscape.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SVG="$PROJECT_DIR/data/icons/hicolor/scalable/apps/com.thane.app.svg"
ICON_DIR="$PROJECT_DIR/data/icons/hicolor"

SIZES=(16 24 32 48 64 128 256 512)

if command -v rsvg-convert &>/dev/null; then
    CONVERTER="rsvg-convert"
elif command -v inkscape &>/dev/null; then
    CONVERTER="inkscape"
else
    echo "Error: rsvg-convert or inkscape required."
    echo "Install: sudo apt install librsvg2-bin  (or)  sudo apt install inkscape"
    exit 1
fi

for SIZE in "${SIZES[@]}"; do
    OUT_DIR="$ICON_DIR/${SIZE}x${SIZE}/apps"
    mkdir -p "$OUT_DIR"
    OUT_FILE="$OUT_DIR/com.thane.app.png"

    if [ "$CONVERTER" = "rsvg-convert" ]; then
        rsvg-convert -w "$SIZE" -h "$SIZE" "$SVG" -o "$OUT_FILE"
    else
        inkscape -w "$SIZE" -h "$SIZE" "$SVG" -o "$OUT_FILE"
    fi

    echo "Generated ${SIZE}x${SIZE} icon"
done

echo "Done. Icons written to $ICON_DIR/"
