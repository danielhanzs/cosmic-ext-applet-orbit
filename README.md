# Orbit

**A control panel applet for the COSMIC desktop** — quick toggles, sliders, a
calendar with your synced events, and a "dynamic island" for what's playing,
filming or recording, all in one popup on your panel. Think the Windows 11
quick settings or a phone's control center, built natively for COSMIC with
libcosmic, following your system theme.

<p align="center">
  <img src="data/icons/scalable/apps/io.github.danielhanzs.CosmicOrbit.svg" width="128" alt="Orbit icon">
</p>

## Features

### Quick tiles
Wi-Fi · Bluetooth · VPN · Power mode · Do not disturb · Dark style ·
Airplane mode · Microphone · Keep awake · Tiling · Magnifier · Invert colors ·
Screen reader · High contrast · Touchpad, plus shortcuts to Screenshot and
Workspaces.

- **Click** a tile to switch it; **the arrow** opens its details: Wi-Fi
  networks (join, with a password prompt), Bluetooth devices (connect, battery
  level), VPN connections, power profiles.
- **Edit tiles**: drag to reorder, click to switch between wide (name and
  state) and small (icon only), remove with −, add with +, or reset.
- On a bright accent color (lime, yellow…) text turns black so it stays
  readable; on dark accents it stays white.

### Looks like COSMIC
Orbit follows your **Appearance** settings: accent color, dark/light,
roundness (round, slightly round, square), **frosted glass** for applet
popups. The space between the panel icons is yours to set. Show
the popup as one surface or as **separate islands** — status and actions,
controls, calendar, and the dynamic island each on its own.
- State is **live in both directions**: Orbit listens to NetworkManager,
  BlueZ, UPower, power-profiles-daemon, PipeWire, the compositor and COSMIC's
  own settings, so changes made anywhere else show up immediately.

### Panel
Status icons (network, VPN, Bluetooth, volume, battery, camera and
microphone in use), the clock, and the tiles' own icons: none, only the
active ones, or all of them with the inactive ones faint, with adjustable
spacing. The battery icon
shows its charge and a clear bolt while plugged in.

### Sliders
Volume (with output device picker and mute) and screen brightness.

### Calendar
- Today's date and your next event at a glance; click to open the month
  (remembered).
- Days with events are marked with a dot; pick a day to see its events.
- Events come from the calendars synced to the system — Evolution Data
  Server, filled by GNOME Online Accounts (Google, Microsoft 365, Nextcloud,
  CalDAV…). Repeating events are handled.

### Dynamic island
At the bottom of the popup, what's running that you should know about:
- the media playing (any MPRIS player: Spotify, browsers, …) with cover art
  and controls,
- apps using the **camera** (green) or the **microphone** (orange) — also
  shown as icons on the panel.

### Layouts
Open Orbit as a **dropdown** under the panel, a **side panel** sliding in from
the edge, or **fullscreen** with a two-column layout.

### Localization
English and Hungarian. The interface follows your language
(`LC_MESSAGES`), while dates, month names, the first day of the week and the
12/24-hour clock follow your **region format** (`LC_TIME`) — so an English
system with Hungarian formats shows "október 29., csütörtök".

### VPN and Wi-Fi passwords that just work
Connections whose password is stored "for this user only" (agent-owned)
can only connect while a NetworkManager *secret agent* answers for them.
COSMIC Settings is one — but only while it's open, which is why VPNs often
fail to connect from other applets. **Orbit is a secret agent itself**:

1. it looks the password up in your keyring (GNOME Keyring / Secret
   Service), unlocking it with the keyring's own dialog if it's locked;
2. if it isn't there, it asks in the popup, and can remember it in the
   keyring for next time.

Only NetworkManager itself may ask Orbit for secrets; other callers are
refused.

## Requirements

- COSMIC desktop 1.11 or newer (built against the same libcosmic as
  COSMIC 1.11's own applets; tested on Pop!_OS 24.04)
- NetworkManager, BlueZ, UPower, power-profiles-daemon
- PipeWire with WirePlumber (`wpctl`) and `pactl`
- `curl` (cover art), `cosmic-settings`, `cosmic-osd`
- Optional, for calendar events: `evolution-data-server` and
  `gnome-online-accounts-gtk`

## Installation

### 1. Build dependencies

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev \
    pulseaudio-utils curl git
```

Rust (1.93 or newer, as libcosmic asks) from [rustup.rs](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Build and install

```sh
git clone https://github.com/danielhanzs/cosmic-orbit.git
cd cosmic-orbit
./install.sh            # installs into ~/.local
# or system wide:
# sudo ./install.sh /usr
```

### 3. Add it to the panel

**Settings → Desktop → Panel → Applets → Add applet → Orbit.**

If it doesn't show up in the list, log out and back in once, so the panel
picks up the new desktop entry.

### 4. Calendar (optional)

```sh
sudo apt install evolution-data-server gnome-online-accounts-gtk
gnome-online-accounts-gtk     # add your Google / Microsoft / Nextcloud account
```

Orbit shows a button to open the accounts dialog while no calendar is set up.

### Or as a Flatpak

```sh
flatpak install --user flathub org.flatpak.Builder org.freedesktop.Sdk//25.08 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08 com.system76.Cosmic.BaseApp//stable
flatpak run org.flatpak.Builder --user --install --force-clean \
    flatpak/build flatpak/local.json
```

`flatpak/local.json` builds the working tree, fetching crates while it
builds. `flatpak/io.github.danielhanzs.CosmicOrbit.json` is the manifest for
the COSMIC Flatpak repository: it builds the `v0.1.0` tag offline, from
`flatpak/cargo-sources.json`, which `flatpak/generate-sources.sh` makes from
`Cargo.lock`.

In the sandbox Orbit runs `wpctl`, `pactl`, `curl`, `cosmic-settings`… on
the host through `flatpak-spawn --host`, as COSMIC applets in Flatpak do.

## Updating

```sh
cd cosmic-orbit
git pull
./install.sh
```

The panel keeps running the old binary until the applet restarts: log out
and in, or remove Orbit from the panel and add it again.

## Uninstalling

Remove Orbit from the panel first, then:

```sh
rm ~/.local/bin/cosmic-orbit \
   ~/.local/share/applications/io.github.danielhanzs.CosmicOrbit.desktop \
   ~/.local/share/icons/hicolor/scalable/apps/io.github.danielhanzs.CosmicOrbit*.svg
rm -r ~/.config/cosmic/io.github.danielhanzs.CosmicOrbit   # settings
rm -r ~/.cache/io.github.danielhanzs.CosmicOrbit           # cover art
```

## Troubleshooting

The panel collects the applet's log:

```sh
journalctl --user -f | grep CosmicOrbit
```

More detail: set `RUST_LOG=cosmic_orbit=debug` in the environment the panel
starts with.

- **No calendar events** — check that `evolution-data-server` is installed
  and an account with a calendar is added in GNOME Online Accounts.
- **VPN asks for a password every time** — turn on *Remember* in the prompt.
- **Brightness slider missing** — the screen exposes no backlight to COSMIC
  (common on desktop monitors).

## Development

```sh
cargo build --release
cargo test
```

| File | What it does |
|------|--------------|
| `src/app.rs` | the applet: panel button, popup layouts, tiles, pages |
| `src/sys.rs` | NetworkManager, BlueZ, UPower, power profiles, brightness, change signals |
| `src/agent.rs` | NetworkManager secret agent and keyring access |
| `src/audio.rs` | volume, outputs, microphone and camera use |
| `src/media.rs` | MPRIS player and cover art |
| `src/calendar.rs` | Evolution Data Server events |
| `src/config.rs` | saved preferences and COSMIC settings |
| `src/i18n.rs` | translations and date formats |

Translations are plain structs in `src/i18n.rs`: copy `EN`, translate, and
pick it by locale.

## Credits

Inspired by DrakRosmann's
[cosmic-ext-applet-quick-settings](https://github.com/DrakRosmann/cosmic-ext-applet-quick-settings),
whose approach to COSMIC's settings and protocols informed this rewrite.

## License

[GPL-3.0-only](LICENSE)
