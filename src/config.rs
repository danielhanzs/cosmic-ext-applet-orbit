// SPDX-License-Identifier: GPL-3.0-only

//! What the user chose, kept with cosmic-config like COSMIC's own settings,
//! and the COSMIC settings the tiles flip.

use cosmic::cosmic_config::{Config, ConfigGet, ConfigSet};

use crate::app::Tile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Layout {
    Dropdown,
    Side,
    Fullscreen,
}

/// The gap between the icons on the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Spacing {
    Small,
    Medium,
    Large,
}

impl Spacing {
    pub fn gap(self) -> u16 {
        match self {
            Spacing::Small => 2,
            Spacing::Medium => 6,
            Spacing::Large => 12,
        }
    }
}

/// Which tiles show their icon on the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PanelIcons {
    None,
    Active,
    All,
}

#[derive(Debug, Clone)]
pub struct Prefs {
    /// Shown tiles, in order; the rest are hidden.
    pub tiles: Vec<Tile>,
    pub layout: Layout,
    pub calendar: bool,
    pub clock: bool,
    /// Tiles shown as only their icon, a quarter wide.
    pub small: Vec<Tile>,
    pub panel_icons: PanelIcons,
    /// The calendar's month open, or only today's line.
    pub calendar_open: bool,
    /// Each part of the popup on its own, or all on one surface.
    pub islands: bool,
    pub spacing: Spacing,
    /// The sound page's per-app volumes open.
    pub mixer_open: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            tiles: Tile::DEFAULT.to_vec(),
            layout: Layout::Dropdown,
            calendar: true,
            clock: true,
            small: Vec::new(),
            panel_icons: PanelIcons::Active,
            calendar_open: false,
            islands: false,
            spacing: Spacing::Medium,
            mixer_open: false,
        }
    }
}

fn config() -> Option<Config> {
    Config::new(crate::APP_ID, 1).ok()
}

impl Prefs {
    pub fn load() -> Self {
        let mut prefs = Self::default();
        if let Some(c) = config() {
            if let Ok(v) = c.get("tiles") {
                prefs.tiles = v;
            }
            if let Ok(v) = c.get("layout") {
                prefs.layout = v;
            }
            if let Ok(v) = c.get("calendar") {
                prefs.calendar = v;
            }
            if let Ok(v) = c.get("clock") {
                prefs.clock = v;
            }
            if let Ok(v) = c.get("small") {
                prefs.small = v;
            }
            if let Ok(v) = c.get("panel_icons") {
                prefs.panel_icons = v;
            }
            if let Ok(v) = c.get("calendar_open") {
                prefs.calendar_open = v;
            }
            if let Ok(v) = c.get("islands") {
                prefs.islands = v;
            }
            if let Ok(v) = c.get("spacing") {
                prefs.spacing = v;
            }
            if let Ok(v) = c.get("mixer_open") {
                prefs.mixer_open = v;
            }
        }
        prefs
    }

    pub fn save(&self) {
        let Some(c) = config() else { return };
        let result = c
            .set("tiles", &self.tiles)
            .and(c.set("layout", self.layout))

            .and(c.set("calendar", self.calendar))
            .and(c.set("clock", self.clock))
            .and(c.set("small", &self.small))
            .and(c.set("panel_icons", self.panel_icons))
            .and(c.set("calendar_open", self.calendar_open))
            .and(c.set("islands", self.islands))
            .and(c.set("spacing", self.spacing))
            .and(c.set("mixer_open", self.mixer_open));
        if let Err(err) = result {
            tracing::warn!("cannot save settings: {err}");
        }
    }
}

/// A COSMIC on/off setting, read and written as COSMIC Settings does.
pub struct Flag(pub &'static str, pub &'static str);

pub const DND: Flag = Flag("com.system76.CosmicNotifications", "do_not_disturb");
pub const DARK: Flag = Flag("com.system76.CosmicTheme.Mode", "is_dark");

impl Flag {
    pub fn get(&self) -> bool {
        Config::new(self.0, 1).and_then(|c| c.get(self.1)).unwrap_or(false)
    }

    pub fn set(&self, on: bool) {
        if let Err(err) = Config::new(self.0, 1).and_then(|c| c.set(self.1, on)) {
            tracing::warn!("cannot set {}: {err}", self.1);
        }
    }
}

/// Fires when the COSMIC settings the tiles show change elsewhere.
pub fn changes() -> cosmic::iced::Subscription<()> {
    cosmic::iced::Subscription::run(|| {
        cosmic::iced::stream::channel(4, async |out| {
            let watchers: Vec<_> = [DND.0, DARK.0]
                .into_iter()
                .filter_map(|name| {
                    let out = std::sync::Mutex::new(out.clone());
                    Config::new(name, 1)
                        .ok()?
                        .watch(move |_, _| {
                            if let Ok(mut out) = out.lock() {
                                _ = out.try_send(());
                            }
                        })
                        .ok()
                })
                .collect();
            futures::future::pending::<()>().await;
            drop(watchers);
        })
    })
}
