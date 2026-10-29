// SPDX-License-Identifier: GPL-3.0-only

//! COSMIC's own switches: accessibility (magnifier, color inversion, screen
//! reader, high contrast), the touchpad, keeping awake. Adapted from
//! cosmic-ext-applet-quick-settings, which does these as COSMIC's applets do.

use cosmic::{
    cosmic_config::{Config, ConfigGet, ConfigSet},
    iced::{Subscription, stream},
};
use futures::SinkExt;
use tokio::sync::OnceCell;

pub use cosmic_settings_a11y_manager_subscription::{AccessibilityEvent, AccessibilityRequest, ColorFilter};
pub use cosmic_settings_accessibility_subscription::{Request as ReaderRequest, Response as ReaderResponse};

pub type A11ySender = cosmic::cctk::sctk::reexports::calloop::channel::Sender<AccessibilityRequest>;

#[derive(Clone, Debug)]
pub enum A11y {
    Ready(A11ySender),
    Event(AccessibilityEvent),
}

/// The magnifier and screen filters, through COSMIC's accessibility protocol.
pub fn a11y() -> Subscription<A11y> {
    Subscription::run(|| {
        stream::channel(8, async |mut out| {
            match cosmic_settings_a11y_manager_subscription::spawn_wayland_connection(1) {
                Ok((tx, mut rx)) => {
                    _ = out.send(A11y::Ready(tx)).await;
                    while let Some(event) = rx.recv().await {
                        if out.send(A11y::Event(event)).await.is_err() {
                            break;
                        }
                    }
                }
                Err(err) => tracing::warn!("no accessibility protocol: {err}"),
            }
            futures::future::pending().await
        })
    })
}

/// The screen reader (Orca), as COSMIC Settings switches it.
pub fn screen_reader() -> Subscription<ReaderResponse> {
    cosmic_settings_accessibility_subscription::subscription()
}

/// High contrast in both the dark and the light theme, as COSMIC's
/// accessibility applet switches it.
pub fn set_high_contrast(enabled: bool) {
    use cosmic::{
        cosmic_config::CosmicConfigEntry,
        cosmic_theme::{CosmicPalette, Theme, ThemeBuilder},
    };
    std::thread::spawn(move || {
        for dark in [true, false] {
            let result = (|| {
                let builder_config = if dark { ThemeBuilder::dark_config()? } else { ThemeBuilder::light_config()? };
                let mut builder = ThemeBuilder::get_entry(&builder_config).unwrap_or_else(|(_, b)| b);
                let palette = builder.palette.inner();
                builder.palette = match (dark, enabled) {
                    (true, true) => CosmicPalette::HighContrastDark(palette),
                    (true, false) => CosmicPalette::Dark(palette),
                    (false, true) => CosmicPalette::HighContrastLight(palette),
                    (false, false) => CosmicPalette::Light(palette),
                };
                builder.write_entry(&builder_config)?;
                let theme_config = if dark { Theme::dark_config()? } else { Theme::light_config()? };
                builder.build().write_entry(&theme_config)?;
                Ok::<(), cosmic::cosmic_config::Error>(())
            })();
            if let Err(err) = result {
                tracing::warn!("cannot set high contrast: {err:?}");
            }
        }
    });
}

// ---------------------------------------------------------------- touchpad

/// cosmic-comp's touchpad settings, as text: only `state` is changed, so the
/// rest (speed, scrolling…) stays as COSMIC Settings left it.
fn touchpad_files() -> (Option<std::path::PathBuf>, Vec<std::path::PathBuf>) {
    const KEY: &str = "com.system76.CosmicComp/v1/input_touchpad";
    let user = crate::host::config_dir().map(|dir| dir.join("cosmic").join(KEY));
    // A sandbox sees the host's /usr under /run/host.
    let data_dirs = if crate::host::in_flatpak() {
        "/run/host/usr/local/share:/run/host/usr/share".to_string()
    } else {
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
    };
    let system = data_dirs.split(':').map(|dir| std::path::Path::new(dir).join("cosmic").join(KEY)).collect();
    (user, system)
}

fn touchpad_text() -> Option<(String, std::path::PathBuf)> {
    let (user, system) = touchpad_files();
    let user = user?;
    let text = std::fs::read_to_string(&user)
        .ok()
        .or_else(|| system.iter().find_map(|p| std::fs::read_to_string(p).ok()))?;
    Some((text, user))
}

/// Where `state:`'s value is in the text.
fn state_value(text: &str) -> Option<std::ops::Range<usize>> {
    let key = text.find("state")?;
    let colon = key + text[key..].find(':')?;
    let rest = &text[colon + 1..];
    let start = colon + 1 + rest.len() - rest.trim_start().len();
    let end = start + text[start..].find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(text.len() - start);
    Some(start..end)
}

/// None: no touchpad settings at all.
pub fn touchpad() -> Option<bool> {
    let (text, _) = touchpad_text()?;
    Some(&text[state_value(&text)?] != "Disabled")
}

pub fn set_touchpad(enabled: bool) {
    let result = (|| {
        let (mut text, path) = touchpad_text().ok_or("no touchpad settings")?;
        let range = state_value(&text).ok_or("no state in the touchpad settings")?;
        text.replace_range(range, if enabled { "Enabled" } else { "Disabled" });
        std::fs::create_dir_all(path.parent().ok_or("no directory")?).map_err(|e| e.to_string())?;
        // Written whole, for cosmic-comp, which watches the file.
        let new = path.with_extension("new");
        std::fs::write(&new, text).map_err(|e| e.to_string())?;
        std::fs::rename(&new, &path).map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })();
    if let Err(err) = result {
        tracing::warn!("cannot set the touchpad: {err}");
    }
}

/// Fires when cosmic-comp's settings change (touchpad).
pub fn comp_changes() -> Subscription<()> {
    Subscription::run(|| {
        stream::channel(4, async |out| {
            let out = std::sync::Mutex::new(out);
            let watcher = Config::new("com.system76.CosmicComp", 1).ok().and_then(|c| {
                c.watch(move |_, _| {
                    if let Ok(mut out) = out.lock() {
                        _ = out.try_send(());
                    }
                })
                .ok()
            });
            futures::future::pending::<()>().await;
            drop(watcher);
        })
    })
}

// ---------------------------------------------------------------- keep awake

/// Inhibitions end with the bus connection that made them: keep one.
async fn session() -> zbus::Result<zbus::Connection> {
    static BUS: OnceCell<zbus::Connection> = OnceCell::const_new();
    BUS.get_or_try_init(zbus::Connection::session).await.cloned()
}

const SCREENSAVER: &str = "org.freedesktop.ScreenSaver";

/// Keeps the screen on and the computer awake; the cookie ends it.
pub async fn inhibit_idle(reason: &str) -> Option<u32> {
    let conn = session().await.ok()?;
    let reply = conn
        .call_method(Some(SCREENSAVER), "/org/freedesktop/ScreenSaver", Some(SCREENSAVER), "Inhibit", &(crate::APP_ID, reason))
        .await
        .inspect_err(|e| tracing::warn!("cannot keep awake: {e}"))
        .ok()?;
    reply.body().deserialize().ok()
}

pub async fn allow_idle(cookie: u32) {
    if let Ok(conn) = session().await {
        _ = conn
            .call_method(Some(SCREENSAVER), "/org/freedesktop/ScreenSaver", Some(SCREENSAVER), "UnInhibit", &(cookie,))
            .await;
    }
}

// ---------------------------------------------------------------- tiling

/// Whether new workspaces tile; the active one is switched through the
/// workspace protocol (`workspaces.rs`).
pub fn autotile() -> bool {
    Config::new("com.system76.CosmicComp", 1).and_then(|c| c.get("autotile")).unwrap_or(false)
}

pub fn set_autotile(on: bool) {
    if let Err(err) = Config::new("com.system76.CosmicComp", 1).and_then(|c| c.set("autotile", on)) {
        tracing::warn!("cannot set tiling: {err}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn touchpad_state() {
        let text = "(\n    state: DisabledOnExternalMouse,\n    acceleration: None,\n)";
        assert_eq!(&text[super::state_value(text).unwrap()], "DisabledOnExternalMouse");
    }
}
