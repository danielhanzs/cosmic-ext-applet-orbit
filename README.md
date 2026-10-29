# Orbit — control panel applet for COSMIC

A Windows/phone-style control panel for the COSMIC panel: quick tiles,
volume and brightness, a calendar with your synced events, and a "dynamic
island" for what's playing, filming or recording — in one popup.

- **Tiles**: Wi-Fi, Bluetooth, VPN, power mode, do not disturb, dark style,
  airplane mode, microphone. Click toggles; the arrow opens details (networks,
  devices, VPNs, profiles). Drag to reorder, hide, or show only the active ones.
- **Opens as** dropdown, side panel (slides in), or fullscreen.
- **Calendar**: events from Evolution Data Server, i.e. the accounts added in
  GNOME Online Accounts (Google, Microsoft, Nextcloud, CalDAV…); days with
  events get a dot. Dates follow your region format (LC_TIME), even when the
  language is another.
- **Privacy**: which apps use the camera or the microphone, on the panel and
  in the island.
- **Clock** on the panel (can be turned off if you keep COSMIC's own).
- **VPN / Wi-Fi passwords**: the applet is a NetworkManager secret agent, so
  connections with "store for this user only" passwords connect from the
  panel without COSMIC Settings open. Passwords come from the keyring, or are
  asked in the popup and optionally saved there; a locked keyring is unlocked
  with its own dialog.
- Follows the COSMIC theme (accent, dark/light, corner radius).

## Install

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev pulseaudio-utils
# Rust from https://rustup.rs
./install.sh
```

Then add **Orbit** in Settings → Desktop → Panel → Applets.

For calendar events:

```sh
sudo apt install evolution-data-server gnome-online-accounts-gtk
gnome-online-accounts-gtk   # add your accounts
```

## Runtime tools

`wpctl` (WirePlumber), `pactl`, `curl` (album art), `cosmic-settings`,
`cosmic-osd`, `loginctl`.

## License

GPL-3.0-only
