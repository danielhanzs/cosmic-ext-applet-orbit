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
    /// As COSMIC's interface density sets it.
    Native,
}

impl Spacing {
    /// `native`: the theme's, from the interface density.
    pub fn gap(self, native: u16) -> u16 {
        match self {
            Spacing::Native => native,
            Spacing::Small => 2,
            Spacing::Medium => 6,
            Spacing::Large => 12,
        }
    }
}

/// A tile of the user's own: a command to run, and optionally one whose
/// success means "on". Both run through `sh -c`, on the host.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Custom {
    pub id: u32,
    pub name: String,
    pub command: String,
    /// An icon name; empty: a generic one.
    pub icon: String,
    /// Exit 0: the tile is lit. Empty: a plain launcher.
    pub status: String,
    /// Ask before running (log out, power off…).
    pub confirm: bool,
}

/// How strongly a tile shows that it's on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TileStyle {
    /// The whole tile in the accent.
    High,
    /// Only its icon, on a square of the accent.
    Medium,
    /// Never lit: its text tells.
    Low,
}

/// How long a dropped Wi-Fi is marked on the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WifiBadge {
    Off,
    HalfMinute,
    Minute,
    FiveMinutes,
    /// Until it's back.
    Back,
}

impl WifiBadge {
    pub fn lasts(self) -> Option<std::time::Duration> {
        let secs = match self {
            WifiBadge::Off => 0,
            WifiBadge::HalfMinute => 30,
            WifiBadge::Minute => 60,
            WifiBadge::FiveMinutes => 300,
            WifiBadge::Back => return None,
        };
        Some(std::time::Duration::from_secs(secs))
    }
}

/// Which tiles show their icon on the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PanelIcons {
    None,
    Active,
    All,
}

/// A part of the popup, shown or not, in an order of the user's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Section {
    Status,
    Controls,
    Calendar,
    Weather,
    Notifications,
    Island,
    /// Sound on a card of its own: levels, devices, apps.
    Audio,
    /// Processor, memory and network use.
    System,
    /// The open to-dos.
    Tasks,
    /// Other places' time, and timers.
    Clocks,
}

impl Section {
    pub const ALL: [Section; 10] = [
        Section::Status,
        Section::Controls,
        Section::Calendar,
        Section::Weather,
        Section::Notifications,
        Section::Island,
        Section::Audio,
        Section::System,
        Section::Tasks,
        Section::Clocks,
    ];
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Shown tiles, in order; the rest are hidden.
    pub tiles: Vec<Tile>,
    pub layout: Layout,
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
    /// The pomodoro's time left on the panel while it runs.
    pub pomodoro_panel: bool,
    /// The battery on the panel red when low and not charging.
    pub low_battery_badge: bool,
    /// How long the panel shows that Wi-Fi dropped.
    pub wifi_badge: WifiBadge,
    /// Tiles of the user's own, each running a command. Not with the rest:
    /// COSMIC's config folder is shared with other Flatpaks that could put
    /// commands here for Orbit to run on the host. See `custom_path`.
    #[serde(skip)]
    pub custom: Vec<Custom>,
    pub tile_style: TileStyle,
    /// Shown parts of the popup, in order; the rest are hidden.
    pub sections: Vec<Section>,
    /// The weather's place; none: the time zone's city.
    pub weather_place: Option<crate::weather::Place>,
    /// The weather now on the panel.
    pub weather_panel: bool,
    /// The forecast open, or only today's line.
    pub weather_open: bool,
    /// The forecast's hours shown, and how many (up to 24).
    pub weather_hourly: bool,
    pub weather_hours: u8,
    /// Days ahead in the forecast (up to 7).
    pub weather_days: u8,
    /// The notifications' list open, or only their count.
    pub notifications_open: bool,
    /// Other places' clocks, by time zone ("America/New_York").
    pub clocks: Vec<String>,
    /// Columns the popup's parts sit in, 1 to 3; none: two in fullscreen,
    /// one otherwise.
    pub columns: Option<u8>,
    /// Parts put in another column than their own.
    pub placement: Vec<(Section, u8)>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            tiles: Tile::DEFAULT.to_vec(),
            layout: Layout::Dropdown,
            clock: true,
            small: Vec::new(),
            panel_icons: PanelIcons::Active,
            calendar_open: false,
            islands: false,
            spacing: Spacing::Medium,
            mixer_open: false,
            pomodoro_panel: true,
            low_battery_badge: true,
            wifi_badge: WifiBadge::Off,
            custom: Vec::new(),
            tile_style: TileStyle::High,
            // The cards added later start off, there to turn on.
            sections: Section::ALL
                .into_iter()
                .filter(|s| !matches!(s, Section::Audio | Section::System | Section::Tasks | Section::Clocks))
                .collect(),
            weather_place: None,
            weather_panel: false,
            weather_open: false,
            weather_hourly: true,
            weather_hours: 24,
            weather_days: 5,
            notifications_open: true,
            columns: None,
            clocks: Vec::new(),
            placement: Vec::new(),
        }
    }
}

fn config() -> Option<Config> {
    Config::new(crate::APP_ID, 1).ok()
}

impl Prefs {
    pub fn load() -> Self {
        let mut prefs = Self::load_shared();
        prefs.custom = load_custom();
        prefs
    }

    fn load_shared() -> Self {
        let Some(c) = config() else { return Self::default() };
        if let Ok(prefs) = c.get::<Prefs>("prefs") {
            return prefs;
        }
        // Kept a key each before 0.2.
        let mut prefs = Self::default();
        {
            if let Ok(v) = c.get("tiles") {
                prefs.tiles = v;
            }
            if let Ok(v) = c.get("layout") {
                prefs.layout = v;
            }
            if let Ok(false) = c.get::<bool>("calendar") {
                prefs.sections.retain(|s| *s != Section::Calendar);
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
            if let Ok(v) = c.get("pomodoro_panel") {
                prefs.pomodoro_panel = v;
            }
        }
        prefs
    }

    pub fn columns(&self) -> usize {
        match self.columns {
            Some(n) => n.clamp(1, 3) as usize,
            None if self.layout == Layout::Fullscreen => 2,
            None => 1,
        }
    }

    /// The column a part sits in: the one it was put in, or its own.
    pub fn column(&self, section: Section) -> usize {
        let own = match section {
            Section::Status | Section::Controls => 0,
            Section::Calendar | Section::Weather | Section::Tasks | Section::Clocks => 1,
            Section::Notifications | Section::Island | Section::Audio | Section::System => 2,
        };
        let put = self.placement.iter().find(|(s, _)| *s == section).map(|(_, c)| *c as usize);
        put.unwrap_or(own).min(self.columns() - 1)
    }

    pub fn save(&self) {
        // A preview's settings are for that run only.
        if std::env::var_os("ORBIT_PREVIEW").is_some() {
            return;
        }
        let Some(c) = config() else { return };
        if let Err(err) = c.set("prefs", self) {
            tracing::warn!("cannot save settings: {err}");
        }
    }
}

/// Where the user's own tiles are kept: Orbit's own config folder, which in
/// the Flatpak is private to it (~/.var/app/<id>/config), unlike COSMIC's.
fn custom_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config")))?;
    Some(base.join(crate::APP_ID).join("custom-tiles.json"))
}

/// The user's tiles. A file that doesn't read (a hand edit gone wrong) is
/// moved aside, not later overwritten with nothing.
fn load_custom() -> Vec<Custom> {
    let Some(path) = custom_path() else { return Vec::new() };
    let Ok(bytes) = std::fs::read(&path) else { return Vec::new() };
    serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        let aside = path.with_extension("json.bad");
        tracing::warn!("your tiles in {} don't read ({err}); kept as {}", path.display(), aside.display());
        _ = std::fs::rename(&path, aside);
        Vec::new()
    })
}

impl Prefs {
    /// The user's tiles, saved on their own: only when they change.
    pub fn save_custom(&self) {
        if std::env::var_os("ORBIT_PREVIEW").is_none() {
            save_custom(&self.custom);
        }
    }
}

/// Written whole and then moved into place, readable by the user alone.
fn save_custom(custom: &[Custom]) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let Some(path) = custom_path() else { return };
    let Some(dir) = path.parent() else { return };
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let part = path.with_extension("json.part");
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&part)?;
        file.write_all(&serde_json::to_vec_pretty(custom).unwrap_or_default())?;
        file.sync_all()?;
        std::fs::rename(part, &path)
    };
    if let Err(err) = write() {
        tracing::warn!("cannot save your tiles: {err}");
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
            // cosmic-comp's: the keyboard layouts.
            let watchers: Vec<_> = [DND.0, DARK.0, "com.system76.CosmicComp"]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_find_their_column() {
        let mut prefs = Prefs::default();
        assert_eq!(prefs.columns(), 1);
        assert_eq!(prefs.column(Section::Island), 0);
        prefs.layout = Layout::Fullscreen;
        assert_eq!((prefs.column(Section::Controls), prefs.column(Section::Island)), (0, 1));
        prefs.columns = Some(3);
        assert_eq!(prefs.column(Section::Notifications), 2);
        prefs.placement.push((Section::Notifications, 0));
        assert_eq!(prefs.column(Section::Notifications), 0);
        prefs.placement = vec![(Section::Status, 2)];
        prefs.columns = Some(2);
        assert_eq!(prefs.column(Section::Status), 1);
    }
}
