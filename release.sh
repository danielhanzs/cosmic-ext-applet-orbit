#!/usr/bin/env bash
# Release Orbit: bump the version, refresh the Flatpak's crate list, test,
# commit, tag, push, and update the COSMIC Flatpak repository.
#
#   ./release.sh 0.2.1 "What changed, in one or two sentences."
#   ./release.sh --dry-run 0.2.1 "..."     # checks and shows, changes nothing
#
# Pushes with your own git/SSH setup. Needs: git, cargo, python3 (with venv),
# curl, and the cosmic-flatpak fork cloned next to this repository
# (../cosmic-flatpak), or COSMIC_FLATPAK=/path/to/it.
set -euo pipefail

ID=io.github.danielhanzs.CosmicOrbit
UPSTREAM=https://github.com/pop-os/cosmic-flatpak.git
GENERATOR=https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py

dry=false
if [ "${1:-}" = "--dry-run" ]; then
    dry=true
    shift
fi
version="${1:-}"
notes="${2:-}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || [ -z "$notes" ]; then
    echo "usage: $0 [--dry-run] X.Y.Z \"release notes\"" >&2
    exit 1
fi
tag="v$version"

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
run() {
    if $dry; then echo "   (dry run) $*"; else "$@"; fi
}

# rustup's cargo, when the shell's PATH doesn't have it.
export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null || { echo "No cargo: install Rust from https://rustup.rs" >&2; exit 1; }
pkg-config --exists xkbcommon wayland-client 2>/dev/null || {
    echo "Building needs: sudo apt install libxkbcommon-dev libwayland-dev pkg-config" >&2
    exit 1
}

repo="$(cd "$(dirname "$0")" && pwd)"
flatpak_repo="${COSMIC_FLATPAK:-$repo/../cosmic-flatpak}"
cd "$repo"

say "Checking"
[ -z "$(git status --porcelain)" ] || { echo "The working tree has changes: commit or stash them first." >&2; exit 1; }
[ "$(git branch --show-current)" = main ] || { echo "Not on main." >&2; exit 1; }
git fetch -q origin
# Ahead of origin is fine (the release pushes it); behind or apart is not.
git merge-base --is-ancestor origin/main HEAD || {
    echo "main is behind or apart from origin/main: pull first." >&2
    exit 1
}
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null || git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1; then
    echo "$tag exists already." >&2
    exit 1
fi
[ -d "$flatpak_repo/.git" ] || { echo "No cosmic-flatpak clone at $flatpak_repo (set COSMIC_FLATPAK)." >&2; exit 1; }
[ -z "$(git -C "$flatpak_repo" status --porcelain)" ] || { echo "$flatpak_repo has changes." >&2; exit 1; }
current="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
echo "   $current → $version"

say "Bumping the version"
if $dry; then
    echo "   (dry run) Cargo.toml, Cargo.lock, metainfo release $version, manifest tag $tag"
else
    sed -i "0,/^version = \".*\"/s//version = \"$version\"/" Cargo.toml
    cargo update -q --workspace --offline 2>/dev/null || cargo update -q --workspace
    VERSION="$version" TAG="$tag" NOTES="$notes" ID="$ID" python3 - <<'PY'
import datetime, html, json, os, re
version, tag, notes, app = os.environ["VERSION"], os.environ["TAG"], os.environ["NOTES"], os.environ["ID"]
path = f"data/{app}.metainfo.xml"
text = open(path).read()
if f'version="{version}"' not in text:
    today = datetime.date.today().isoformat()
    release = (
        f'    <release version="{version}" date="{today}">\n'
        f"      <description>\n        <p>{html.escape(notes)}</p>\n      </description>\n"
        f"    </release>\n"
    )
    text = text.replace("  <releases>\n", "  <releases>\n" + release, 1)
# Screenshots as they are at this release.
text = re.sub(r"(cosmic-orbit/)(?:main|v[0-9.]+)(/data/screenshots)", rf"\g<1>{tag}\2", text)
open(path, "w").write(text)
manifest = f"flatpak/{app}.json"
data = json.load(open(manifest))
source = data["modules"][0]["sources"][0]
source["tag"] = tag
source.pop("commit", None)
json.dump(data, open(manifest, "w"), indent=2)
open(manifest, "a").write("\n")
PY
fi

say "Refreshing flatpak/cargo-sources.json"
if $dry; then
    echo "   (dry run) download flatpak-cargo-generator.py into a temporary venv and run it"
else
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    curl -sfL "$GENERATOR" -o "$work/flatpak-cargo-generator.py"
    python3 -m venv "$work/venv"
    "$work/venv/bin/pip" -q install aiohttp tomlkit
    PATH="$work/venv/bin:$PATH" flatpak/generate-sources.sh "$work/flatpak-cargo-generator.py"
fi

say "Testing"
cargo test --release --quiet

say "Committing, tagging and pushing $tag"
run git add -A
run git commit -q -m "Release $version" -m "$notes"
run git tag -a "$tag" -m "Orbit $version"
run git push origin main "$tag"
commit="$(git rev-parse HEAD)"

say "Updating the COSMIC Flatpak repository"
cd "$flatpak_repo"
git remote get-url upstream >/dev/null 2>&1 || run git remote add upstream "$UPSTREAM"
$dry || git fetch -q upstream master
if git cat-file -e "upstream/master:app/$ID/$ID.json" 2>/dev/null; then
    # Already in: an update, on a branch of its own.
    branch="cosmic-orbit-$tag"
    message="$ID: update to $tag"
    run git checkout -q -B "$branch" upstream/master
else
    # Not in yet: the submission's branch, which updates its pull request.
    branch=add-cosmic-orbit
    message="Update $ID to $tag"
    if git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
        $dry || git fetch -q origin "$branch"
        run git checkout -q -B "$branch" "origin/$branch"
    else
        message="Add $ID applet"
        run git checkout -q -B "$branch" upstream/master
    fi
fi
if $dry; then
    echo "   (dry run) app/$ID/: manifest at $tag ($commit), cargo-sources.json; commit \"$message\"; push $branch"
else
    mkdir -p "app/$ID"
    COMMIT="$commit" SRC="$repo/flatpak/$ID.json" DST="app/$ID/$ID.json" python3 - <<'PY'
import json, os
data = json.load(open(os.environ["SRC"]))
data["modules"][0]["sources"][0]["commit"] = os.environ["COMMIT"]
json.dump(data, open(os.environ["DST"], "w"), indent=2)
open(os.environ["DST"], "a").write("\n")
PY
    cp "$repo/flatpak/cargo-sources.json" "app/$ID/cargo-sources.json"
    git add "app/$ID"
    git commit -q -m "$message"
    git push -q -u origin "$branch"
fi

owner="$(git remote get-url origin | sed -E 's#.*[:/]([^/]+)/[^/]+(\.git)?$#\1#')"
say "Done"
echo "   GitHub release:  https://github.com/danielhanzs/cosmic-orbit/releases/new?tag=$tag"
if [ "$branch" = add-cosmic-orbit ]; then
    echo "   Pull request:    the open one from $owner:$branch is updated (or open it:"
fi
echo "                    https://github.com/pop-os/cosmic-flatpak/compare/master...$owner:cosmic-flatpak:$branch?expand=1"
