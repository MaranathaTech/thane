#!/bin/bash
# Build an AppImage for thane.
# Requires: linuxdeploy, cargo, and system libraries (GTK4, VTE, WebKitGTK).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
BUILD_DIR="$PROJECT_DIR/target/appimage"
APPDIR="$BUILD_DIR/AppDir"

echo "==> Building release binary..."
cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "==> Preparing AppDir..."
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/metainfo"
mkdir -p "$APPDIR/usr/share/glib-2.0/schemas"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"

# Binaries
cp "$PROJECT_DIR/target/release/thane" "$APPDIR/usr/bin/"
cp "$PROJECT_DIR/target/release/thane-cli" "$APPDIR/usr/bin/"

# Desktop file
cp "$PROJECT_DIR/data/com.thane.app.desktop" "$APPDIR/usr/share/applications/"
# AppImage requires desktop file at root too
cp "$PROJECT_DIR/data/com.thane.app.desktop" "$APPDIR/"

# Metadata
cp "$PROJECT_DIR/data/com.thane.app.metainfo.xml" "$APPDIR/usr/share/metainfo/"

# GSettings schema
cp "$PROJECT_DIR/data/com.thane.app.gschema.xml" "$APPDIR/usr/share/glib-2.0/schemas/"
glib-compile-schemas "$APPDIR/usr/share/glib-2.0/schemas/"

# Icon
cp "$PROJECT_DIR/data/icons/hicolor/scalable/apps/com.thane.app.svg" \
   "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
# AppImage requires icon at root
cp "$PROJECT_DIR/data/icons/hicolor/scalable/apps/com.thane.app.svg" \
   "$APPDIR/com.thane.app.svg"

# AppRun entry point
cat > "$APPDIR/AppRun" << 'APPRUN'
#!/bin/bash
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
export GSETTINGS_SCHEMA_DIR="$HERE/usr/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$HERE/usr/share:${XDG_DATA_DIRS:-/usr/share}"
exec "$HERE/usr/bin/thane" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

echo "==> Building AppImage..."
# Download linuxdeploy if not available
LINUXDEPLOY="$BUILD_DIR/linuxdeploy-x86_64.AppImage"
if [ ! -f "$LINUXDEPLOY" ]; then
    echo "    Downloading linuxdeploy..."
    curl -fsSL -o "$LINUXDEPLOY" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
    chmod +x "$LINUXDEPLOY"
fi

export OUTPUT="$BUILD_DIR/thane-x86_64.AppImage"
"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --desktop-file "$APPDIR/usr/share/applications/com.thane.app.desktop" \
    --icon-file "$APPDIR/usr/share/icons/hicolor/scalable/apps/com.thane.app.svg" \
    --output appimage

echo "==> AppImage created: $OUTPUT"
echo "    Size: $(du -h "$OUTPUT" | cut -f1)"

# Upload to Cloudflare R2
echo "==> Uploading to Cloudflare R2..."
R2_ENV="$SCRIPT_DIR/../r2.env"
if [ ! -f "$R2_ENV" ]; then
    echo "    Skipping R2 upload: dist/r2.env not found"
else
    eval "$(ansible-vault view "$R2_ENV" --vault-password-file "${VAULT_PASSWORD_FILE:--}")"
    R2_ENDPOINT="https://643e3fafa040d350bb335f2d61023844.r2.cloudflarestorage.com"
    AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" \
    AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY" \
    aws --endpoint-url "$R2_ENDPOINT" s3 cp "$OUTPUT" s3://thane/releases/thane-x86_64.AppImage
    echo "==> Uploaded to R2: releases/thane-x86_64.AppImage"
fi
