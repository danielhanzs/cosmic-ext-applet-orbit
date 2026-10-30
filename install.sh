#!/bin/sh
# Build and install for the current user (default) or under another prefix:
#   ./install.sh            -> ~/.local
#   sudo ./install.sh /usr  -> system wide
set -e
prefix="${1:-$HOME/.local}"
id=io.github.danielhanzs.CosmicExtAppletOrbit
bin=cosmic-ext-applet-orbit
cd "$(dirname "$0")"
cargo build --release
install -Dm0755 "target/release/$bin" "$prefix/bin/$bin"
mkdir -p "$prefix/share/applications"
sed "s|@bindir@|$prefix/bin|" "data/$id.desktop.in" > "$prefix/share/applications/$id.desktop"
for icon in data/icons/scalable/apps/*.svg; do install -Dm0644 "$icon" "$prefix/share/icons/hicolor/scalable/apps/$(basename "$icon")"; done
install -Dm0644 "data/$id.metainfo.xml" "$prefix/share/metainfo/$id.metainfo.xml"
echo "Installed. Add \"Orbit\" in Settings > Desktop > Panel > Applets."
