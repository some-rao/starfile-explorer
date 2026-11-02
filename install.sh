#!/bin/sh
# Installs StarFile Explorer for the current user, no root needed:
#   curl -fsSL https://raw.githubusercontent.com/some-rao/starfile-explorer/main/install.sh | sh
# Remove it again with:
#   curl -fsSL https://raw.githubusercontent.com/some-rao/starfile-explorer/main/install.sh | sh -s -- --uninstall
set -eu

REPO="some-rao/starfile-explorer"
APP_ID="io.github.some_rao.StarFile"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$BIN_DIR/starfile"
DESKTOP="$DATA_DIR/applications/$APP_ID.desktop"
ICON="$DATA_DIR/icons/hicolor/256x256/apps/$APP_ID.png"
METAINFO="$DATA_DIR/metainfo/$APP_ID.metainfo.xml"

refresh_caches() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$DATA_DIR/applications" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$DATA_DIR/icons/hicolor" || true
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON" "$METAINFO"
    refresh_caches
    echo "StarFile Explorer removed. Settings are kept in ~/.config/StarFile."
    exit 0
fi

if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
    echo "Only x86_64 Linux builds are published. Build from source for other systems:" >&2
    echo "  https://github.com/$REPO#building" >&2
    exit 1
fi

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        echo "curl or wget is needed." >&2
        exit 1
    fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading the latest release..."
fetch "https://github.com/$REPO/releases/latest/download/starfile-linux-x86_64.tar.gz" "$TMP/starfile.tar.gz"
tar -xzf "$TMP/starfile.tar.gz" -C "$TMP"
SRC="$TMP/starfile"

# Older releases shipped only the binary; take the desktop files from the repo then
RAW="https://raw.githubusercontent.com/$REPO/main"
[ -f "$SRC/$APP_ID.desktop" ] || fetch "$RAW/packaging/linux/$APP_ID.desktop" "$SRC/$APP_ID.desktop"
[ -f "$SRC/$APP_ID.metainfo.xml" ] || fetch "$RAW/packaging/linux/$APP_ID.metainfo.xml" "$SRC/$APP_ID.metainfo.xml"
[ -f "$SRC/$APP_ID.png" ] || fetch "$RAW/assets/icon.png" "$SRC/$APP_ID.png"

mkdir -p "$BIN_DIR" "$(dirname "$DESKTOP")" "$(dirname "$ICON")" "$(dirname "$METAINFO")"
install -m 755 "$SRC/starfile" "$BIN"
install -m 644 "$SRC/$APP_ID.png" "$ICON"
install -m 644 "$SRC/$APP_ID.metainfo.xml" "$METAINFO"
# Desktop sessions don't always have ~/.local/bin in PATH, so point at the binary directly
sed "s|^Exec=starfile|Exec=$BIN|" "$SRC/$APP_ID.desktop" > "$DESKTOP"
chmod 644 "$DESKTOP"
refresh_caches

echo "Installed to $BIN"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Add $BIN_DIR to your PATH to start it from a terminal." ;;
esac
