#!/bin/sh
# Regenerate cargo-sources.json from Cargo.lock, for the offline build the
# COSMIC Flatpak repository runs. Needs flatpak-cargo-generator.py
# (https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo) and
# Python with aiohttp and tomlkit:
#   flatpak/generate-sources.sh path/to/flatpak-cargo-generator.py
set -e
cd "$(dirname "$0")/.."
python3 "$1" Cargo.lock -o flatpak/cargo-sources.json
# Cargo.toml patches cosmic-protocols through a second spelling of its URL,
# ending in "//" (as COSMIC's applets do), which the generator folds into the
# first: point that one at the vendored crates too.
python3 - <<'PY'
import json
path = "flatpak/cargo-sources.json"
sources = json.load(open(path))
extra = (
    '\n[source."https://github.com/pop-os/cosmic-protocols//"]\n'
    'git = "https://github.com/pop-os/cosmic-protocols//"\n'
    'replace-with = "vendored-sources"\n'
    'rev = "c0cff4d"\n'
)
for source in sources:
    if source.get("type") == "inline" and source.get("dest-filename") == "config":
        if extra not in source["contents"]:
            source["contents"] += extra
json.dump(sources, open(path, "w"), indent=4)
PY
