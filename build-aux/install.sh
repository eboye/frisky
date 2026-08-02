#!/usr/bin/env bash
# Installs an already-built release binary and its data files into a prefix.
#
#   ./build-aux/install.sh              -> ~/.local
#   ./build-aux/install.sh /usr/local   -> system-wide (needs root)
#
# The Flatpak manifest does its own installing; this is for distro-style
# installs, the AppImage staging tree, and the binary tarball.

set -euo pipefail

APP_ID="io.github.eboye.Frisky"
PREFIX="${1:-$HOME/.local}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="$repo_root/target/release/frisky-gtk"

if [[ ! -x "$binary" ]]; then
    echo "error: $binary not found — run 'cargo build --release' first" >&2
    exit 1
fi

install -Dm755 "$binary" "$PREFIX/bin/frisky-gtk"

install -Dm644 "$repo_root/data/$APP_ID.desktop" \
    "$PREFIX/share/applications/$APP_ID.desktop"
install -Dm644 "$repo_root/data/$APP_ID.metainfo.xml" \
    "$PREFIX/share/metainfo/$APP_ID.metainfo.xml"
install -Dm644 "$repo_root/data/icons/$APP_ID.svg" \
    "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"

# D-Bus activation needs the binary's absolute path.
mkdir -p "$PREFIX/share/dbus-1/services"
sed "s|@BINDIR@|$PREFIX/bin|" "$repo_root/data/$APP_ID.service.in" \
    > "$PREFIX/share/dbus-1/services/$APP_ID.service"

# Without a compiled schema, GSettings aborts at startup.
install -Dm644 "$repo_root/data/$APP_ID.gschema.xml" \
    "$PREFIX/share/glib-2.0/schemas/$APP_ID.gschema.xml"
glib-compile-schemas "$PREFIX/share/glib-2.0/schemas"

# Refresh the desktop and icon caches where the tools exist. Missing them only
# delays the app appearing in the launcher, so never fail the install over it.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$PREFIX/share/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true
fi

echo "Installed to $PREFIX"
if [[ ":$PATH:" != *":$PREFIX/bin:"* ]]; then
    echo "note: $PREFIX/bin is not on your PATH"
fi
