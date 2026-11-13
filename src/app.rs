// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use cosmic::{
    Application, Element, Task, app,
    cosmic_theme::palette::WithAlpha,
    iced::{self, Alignment, Color, Length, Limits, Subscription, window},
    surface,
    theme,
    widget::{self, button, container, icon, text},
};
use jiff::{Zoned, civil};
use zbus::zvariant::OwnedObjectPath;

use crate::{
    agent,
    audio::{self, Audio},
    calendar::{self, Event},
    config::{DARK, DND, Layout, PanelIcons, Prefs, Section, Spacing},
    icons,
    desktop,
    style,
    keyboard, swipe::swipe, system, tasks, workspaces,
    i18n::{self, T},
    media::{self, Media},
    notifications::{self, Notification},
    pomodoro,
    updates,
    weather::{self, Weather},
    sys::{self, AccessPoint, BatteryState, Bluetooth, Network, Profiles, Source},
};

const WIDTH: f32 = 380.0;
/// Between the popup's columns.
const GAP: f32 = 24.0;
const SLIDE: Duration = Duration::from_millis(220);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tile {
    Wifi,
    Bluetooth,
    Vpn,
    Power,
    Dnd,
    Dark,
    Airplane,
    Mic,
    KeepAwake,
    Tiling,
    Magnifier,
    InvertColors,
    ScreenReader,
    HighContrast,
    Screenshot,
    Workspaces,
    Touchpad,
    Hotspot,
    Pomodoro,
    Keyboard,
    Dns,
    GameMode,
    /// The user's own, by its id.
    Custom(u32),
}

impl Tile {
    pub const ALL: [Tile; 22] = [
        Tile::Wifi,
        Tile::Bluetooth,
        Tile::Vpn,
        Tile::Power,
        Tile::Dnd,
        Tile::Dark,
        Tile::Airplane,
        Tile::Mic,
        Tile::KeepAwake,
        Tile::Tiling,
        Tile::Magnifier,
        Tile::InvertColors,
        Tile::ScreenReader,
        Tile::HighContrast,
        Tile::Screenshot,
        Tile::Workspaces,
        Tile::Touchpad,
        Tile::Hotspot,
        Tile::Pomodoro,
        Tile::Keyboard,
        Tile::Dns,
        Tile::GameMode,
    ];

    /// Shown at first, and after a reset.
    pub const DEFAULT: [Tile; 8] = [
        Tile::Wifi,
        Tile::Bluetooth,
        Tile::Vpn,
        Tile::Power,
        Tile::Dnd,
        Tile::Dark,
        Tile::Airplane,
        Tile::Mic,
    ];

    fn name(self) -> &'static str {
        match self {
            Tile::Wifi => T.wifi,
            Tile::Bluetooth => T.bluetooth,
            Tile::Vpn => T.vpn,
            Tile::Power => T.power,
            Tile::Dnd => T.dnd,
            Tile::Dark => T.dark,
            Tile::Airplane => T.airplane,
            Tile::Mic => T.mic,
            Tile::KeepAwake => T.keep_awake,
            Tile::Tiling => T.tiling,
            Tile::Magnifier => T.magnifier,
            Tile::InvertColors => T.invert_colors,
            Tile::ScreenReader => T.screen_reader,
            Tile::HighContrast => T.high_contrast,
            Tile::Screenshot => T.screenshot,
            Tile::Workspaces => T.workspaces,
            Tile::Touchpad => T.touchpad,
            Tile::Hotspot => T.hotspot,
            Tile::Pomodoro => T.pomodoro,
            Tile::Keyboard => T.keyboard,
            Tile::Dns => "DNS",
            Tile::GameMode => T.game_mode,
            Tile::Custom(_) => "",
        }
    }

    /// Already on the panel as status icons, so not again as tiles.
    fn has_status_icon(self) -> bool {
        matches!(self, Tile::Wifi | Tile::Bluetooth | Tile::Vpn | Tile::Mic)
    }
}

/// Lays tiles out in rows of four columns: wide tiles take two, small ones
/// one. Each tile keeps its index in `tiles`, for dragging.
fn rows(tiles: &[Tile], small: &[Tile]) -> Vec<Vec<(usize, Tile)>> {
    let mut rows: Vec<Vec<(usize, Tile)>> = vec![Vec::new()];
    let mut used = 0;
    for (i, tile) in tiles.iter().enumerate() {
        let width = if small.contains(tile) { 1 } else { 2 };
        if used + width > 4 {
            rows.push(Vec::new());
            used = 0;
        }
        rows.last_mut().expect("a row").push((i, *tile));
        used += width;
    }
    rows.retain(|r| !r.is_empty());
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slider {
    Volume,
    Brightness,
    Mic,
    KeyboardLight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Main,
    Wifi,
    Bluetooth,
    Vpn,
    Power,
    Sound,
    Hotspot,
    Pomodoro,
    Keyboard,
    Dns,
    /// Asking before running one of the user's tiles.
    Confirm(u32),
    Edit,
    Password,
}

pub struct App {
    core: app::Core,
    prefs: Prefs,
    popup: Option<window::Id>,
    opened: Instant,
    page: Page,
    net: Network,
    bt: Bluetooth,
    battery: Option<BatteryState>,
    peripherals: Vec<sys::Peripheral>,
    profiles: Profiles,
    brightness: Option<f32>,
    audio: Audio,
    media: Option<Media>,
    dnd: bool,
    dark: bool,
    now: Zoned,
    /// The day picked in the calendar, and the month shown.
    selected: civil::Date,
    visible: civil::Date,
    /// The shown month's events by day; None: no calendar service.
    events: Option<HashMap<civil::Date, Vec<Event>>>,
    /// Today's events, for the calendar's closed line.
    today: Option<Vec<Event>>,
    /// Apps using a camera.
    camera: Vec<String>,
    camera_checking: bool,
    a11y: Option<desktop::A11ySender>,
    magnifier: Option<bool>,
    /// Inverted colors, and the color filter kept as it is.
    screen_filter: Option<(bool, Option<desktop::ColorFilter>)>,
    screen_reader: Option<bool>,
    reader: Option<tokio::sync::mpsc::UnboundedSender<desktop::ReaderRequest>>,
    /// Keeping awake: the inhibition's cookie.
    inhibit: Option<u32>,
    inhibiting: bool,
    workspaces: Option<cosmic::cctk::sctk::reexports::calloop::channel::Sender<workspaces::Command>>,
    /// The keyboard layouts set, and which is in use.
    layouts: Vec<keyboard::Layout>,
    layout: usize,
    /// Whether the active workspace tiles.
    tiling: Option<bool>,
    touchpad: Option<bool>,
    /// A tile being dragged in the editor, and where it would go.
    drag: Option<usize>,
    drag_over: Option<usize>,
    /// Reads on their way, and those with news since they began.
    pending: HashSet<Source>,
    dirty: HashSet<Source>,
    /// Per slider: the value waiting to be set, and whether a set runs.
    waiting: [Option<f32>; 4],
    writing: [bool; 4],
    audio_pending: bool,
    audio_dirty: bool,
    /// The month the events were read for, and when.
    events_read: Option<(civil::Date, Instant)>,
    /// A slider being dragged: refreshes don't move it meanwhile.
    adjusting: Option<Instant>,
    password: Option<(agent::Request, String)>,
    remember: bool,
    screen: (u32, u32),
    /// The hotspot's password, read when its page opens.
    hotspot_password: Option<String>,
    /// Updates waiting.
    updates: usize,
    /// An app's volume slider being dragged: its index and level.
    stream_drag: Option<(u32, f32)>,
    /// The sound card's device list open: inputs (true) or outputs.
    picking: Option<bool>,
    edit_tab: EditTab,
    /// The last read of the machine's load, what it showed, and the
    /// processor's recent past (0..1, oldest first).
    sample: Option<system::Sample>,
    load: system::Load,
    cpu_history: std::collections::VecDeque<f32>,
    sampling: bool,
    /// The open to-dos; None: no task service.
    tasks: Option<Vec<tasks::Task>>,
    /// The UIDs of to-dos being ticked off.
    completing: Vec<String>,
    /// Running timers: when each ends, and how long it was.
    timers: Vec<(Instant, Duration)>,
    /// The editor's clock search, and the time zones it matches.
    clock_input: String,
    clock_matches: Vec<String>,
    /// A timer's own length being typed, in minutes.
    timer_input: String,
    /// The connection's name servers: ours, or (None) its own or unknown.
    dns: Option<sys::Dns>,
    /// The battery's charge limit, where it has one: its path, and on.
    charge_limit: Option<(zbus::zvariant::OwnedObjectPath, bool)>,
    /// The keyboard's backlight: name, level, most.
    keyboard_light: Option<(String, u32, u32)>,
    /// Game Mode: None, not installed; and whether we hold it on.
    gamemode: Option<bool>,
    gamemode_held: bool,
    /// When Wi-Fi last dropped, while that's shown; and whether it was up.
    wifi_lost: Option<Instant>,
    wifi_was_up: bool,
    /// The user's tiles whose status says on; when last asked.
    custom_on: Vec<u32>,
    custom_asked: Option<Instant>,
    custom_polling: bool,
    /// The user's tile being made or changed in the editor.
    custom_form: Option<crate::config::Custom>,
    /// Launchers installed here, offered as tiles.
    suggestions: Vec<Suggestion>,
    pomodoro: pomodoro::Pomodoro,
    weather: Option<Weather>,
    /// When the weather was last asked for, and which ask is the latest.
    weather_read: Option<Instant>,
    weather_asked: u64,
    /// The city being typed in the editor, the places matching it, and
    /// which search is the latest.
    city_input: String,
    places: Vec<weather::Place>,
    city_search: u64,
    /// Newest first.
    notifications: Vec<Notification>,
    /// The last notification key given.
    keys: u64,
}

#[derive(Debug, Clone)]
pub enum Message {
    Toggle,
    Close,
    Closed(window::Id),
    Page(Page),
    Screen(String, Option<(u32, u32)>),
    Tick,
    Frame,
    Changed(Source),
    AudioChanged,
    FlagsChanged,
    Network(Option<Network>),
    Bluetooth(Option<Bluetooth>),
    Battery(Option<BatteryState>, Vec<sys::Peripheral>),
    Profiles(Option<Profiles>),
    Brightness(Option<f32>),
    Audio(Audio),
    Media(Option<Media>),
    Events(civil::Date, Option<HashMap<civil::Date, Vec<Event>>>),
    CheckCamera,
    A11y(desktop::A11y),
    Reader(desktop::ReaderResponse),
    Workspaces(workspaces::Event),
    /// Switch to the keyboard layout at this place in the list.
    KeyboardLayout(usize),
    Inhibited(Option<u32>),
    ResetTiles,
    PanelIcons(PanelIcons),
    SetIslands(bool),
    SetSpacing(Spacing),
    ToggleCalendar,
    Camera(Vec<String>),
    TileClick(Tile),
    Volume(f32),
    MicVolume(f32),
    /// The sound card's outputs (false) or inputs list, opened or closed.
    Picking(bool),
    SetBrightness(f32),
    Written(Slider),
    Released,
    Mute,
    Output(String),
    Join(AccessPoint),
    Connect(OwnedObjectPath),
    Disconnect(OwnedObjectPath),
    SetWifi(bool),
    SetBluetooth(bool),
    SetHotspot(bool),
    HotspotPassword(Option<String>),
    CheckUpdates,
    Input(String),
    OpenUrl(String),
    PomodoroToggle,
    PomodoroSkip,
    PomodoroReset,
    PomodoroPanel(bool),
    StreamVolume(u32, f32),
    StreamRelease,
    StreamMute(u32),
    ToggleMixer,
    Updates(usize),
    BtConnect(OwnedObjectPath, bool),
    Profile(String),
    MediaControl(&'static str),
    Day(jiff::civil::Date),
    PrevMonth,
    NextMonth,
    Secret(agent::Event),
    PasswordInput(String),
    Remember(bool),
    PasswordSubmit,
    PasswordCancel,
    DragStart(usize),
    DragOver(usize),
    DragEnd,
    ShowTile(Tile, bool),
    SetLayout(Layout),
    CheckWeather,
    Weather(u64, Option<Weather>),
    City(String),
    SearchCity(u64),
    Places(u64, Vec<weather::Place>),
    PickPlace(weather::Place),
    WeatherPanel(bool),
    CitySubmit,
    ToggleWeather,
    Notification(notifications::Event),
    NotificationOpen(u64),
    NotificationDismiss(u64),
    NotificationsClear,
    NotificationsLoaded(Vec<Notification>),
    ToggleNotifications,
    MoveSection(usize, bool),
    ShowSection(Section, bool),
    SetColumns(u8),
    EditTab(EditTab),
    System(Option<system::Sample>),
    Tasks(Option<Vec<tasks::Task>>),
    /// By its UID: the list may have been read again meanwhile.
    CompleteTask(String),
    TaskDone(String, Option<Vec<tasks::Task>>),
    /// A timer of this many minutes.
    StartTimer(u64),
    /// By when it ends: others may have ended meanwhile.
    StopTimer(Instant),
    TimerInput(String),
    /// The extras read when the popup opens.
    Extras(Option<sys::Dns>, Option<(zbus::zvariant::OwnedObjectPath, bool)>, Option<(String, u32, u32)>, Option<bool>),
    SetDns(sys::Dns),
    SetChargeLimit(bool),
    KeyboardLight(f32),
    GameModeHeld(bool),
    LowBatteryBadge(bool),
    RunCustom(u32),
    CustomOn(Vec<u32>),
    /// The editor's form: open on a tile (or a new one), edited, saved, gone.
    EditCustom(Option<u32>),
    CustomForm(CustomField, String),
    CustomConfirm(bool),
    SaveCustom,
    DeleteCustom,
    CancelCustom,
    Suggestions(Vec<Suggestion>),
    AddSuggestion(Suggestion),
    WifiBadge(crate::config::WifiBadge),
    TileStyle(crate::config::TileStyle),
    ClockInput(String),
    AddClock(String),
    RemoveClock(usize),
    WeatherHourly(bool),
    WeatherHours(u8),
    WeatherDays(u8),
    /// To the next column, or (false) the one before.
    SectionColumn(Section, bool),
    ShowClock(bool),
    Spawn(&'static str, &'static [&'static str]),
    None,
}

fn run(future: impl Future<Output = Message> + Send + 'static) -> app::Task<Message> {
    Task::future(async move { cosmic::action::app(future.await) })
}

fn fire(future: impl Future<Output = ()> + Send + 'static) -> app::Task<Message> {
    run(async move {
        future.await;
        Message::None
    })
}

fn log<T>(what: &str, result: zbus::Result<T>) {
    if let Err(err) = result {
        tracing::warn!("{what}: {err}");
    }
}

fn profile_name(profile: &str) -> &'static str {
    match profile {
        "performance" => T.performance,
        "power-saver" => T.power_saver,
        _ => T.balanced,
    }
}

fn wifi_icon(strength: u8) -> &'static str {
    match strength {
        0..=24 => "network-wireless-signal-weak-symbolic",
        25..=49 => "network-wireless-signal-ok-symbolic",
        50..=74 => "network-wireless-signal-good-symbolic",
        _ => "network-wireless-signal-excellent-symbolic",
    }
}

/// A tile as shown: icon, subtitle, lit, and the page with more of it.
struct TileView {
    icon: icon::Handle,
    title: String,
    subtitle: String,
    on: bool,
    page: Option<Page>,
}

impl App {
    fn refresh(&mut self, source: Source) -> app::Task<Message> {
        if !self.pending.insert(source) {
            // News during a read: read again once this one is in.
            self.dirty.insert(source);
            return Task::none();
        }
        // Signals come in bursts: wait for the burst to pass.
        let wait = || tokio::time::sleep(Duration::from_millis(250));
        match source {
            Source::Network => run(async move {
                wait().await;
                Message::Network(sys::network().await.inspect_err(|e| tracing::warn!("network: {e}")).ok())
            }),
            Source::Bluetooth => run(async move {
                wait().await;
                Message::Bluetooth(sys::bluetooth().await.ok())
            }),
            Source::Battery => run(async move {
                wait().await;
                let (battery, peripherals) = tokio::join!(sys::battery(), sys::peripherals());
                Message::Battery(battery.ok().flatten(), peripherals.unwrap_or_default())
            }),
            Source::Profiles => run(async move {
                wait().await;
                Message::Profiles(sys::profiles().await.ok())
            }),
            Source::Brightness => run(async move {
                wait().await;
                Message::Brightness(sys::brightness().await.ok().flatten())
            }),
            Source::Media => run(async move {
                wait().await;
                Message::Media(media::read().await)
            }),
        }
    }

    /// Sets a slider's value: one write at a time each, always the latest
    /// value, so a drag's steps can't land out of order. `None`: a write is
    /// done, start the next if one waits.
    fn write(&mut self, slider: Slider, level: Option<f32>) -> app::Task<Message> {
        let index = slider as usize;
        match level {
            Some(level) => self.waiting[index] = Some(level),
            None => self.writing[index] = false,
        }
        if self.writing[index] {
            return Task::none();
        }
        let Some(level) = self.waiting[index].take() else { return Task::none() };
        self.writing[index] = true;
        let light = self.keyboard_light.clone();
        run(async move {
            match slider {
                Slider::Volume => audio::set_volume(level).await,
                Slider::Brightness => log("brightness", sys::set_brightness(level).await),
                Slider::Mic => audio::set_mic_volume(level).await,
                Slider::KeyboardLight => {
                    if let Some((name, _, max)) = light {
                        log("keyboard light", sys::set_keyboard_light(name, (level * max as f32).round() as u32).await);
                    }
                }
            }
            Message::Written(slider)
        })
    }

    /// A read is in: another, if news came meanwhile.
    fn settled(&mut self, source: Source) -> app::Task<Message> {
        self.pending.remove(&source);
        if self.dirty.remove(&source) { self.refresh(source) } else { Task::none() }
    }

    /// The shown month's events, unless read in the last few minutes.
    fn load_events(&mut self) -> app::Task<Message> {
        let first = self.visible;
        if self.events_read.is_some_and(|(month, at)| month == first && at.elapsed() < Duration::from_secs(300)) {
            return Task::none();
        }
        self.events_read = Some((first, Instant::now()));
        run(async move { Message::Events(first, calendar::month(first).await) })
    }

    fn adjusting(&self) -> bool {
        self.adjusting.is_some_and(|at| at.elapsed() < Duration::from_secs(1))
    }

    fn vpn_on(&self) -> bool {
        self.net.vpns.iter().any(|v| v.active.is_some())
    }

    fn airplane(&self) -> bool {
        !self.net.wifi_enabled && !self.bt.powered
    }

    fn tile(&self, tile: Tile) -> TileView {
        let on_off = |on: bool| if on { T.on } else { T.off }.to_string();
        let (icon, on, subtitle, page) = match tile {
            Tile::Wifi if self.net.wifi_device.is_none() => {
                let wired = self.net.primary.as_ref().filter(|p| p.wired);
                return TileView {
                    icon: named("network-wired-symbolic"),
                    title: T.wired.into(),
                    subtitle: wired.map_or(T.not_connected.into(), |p| p.name.clone()),
                    on: wired.is_some(),
                    page: None,
                };
            }
            Tile::Wifi => {
                let ap = self.net.access_points.iter().find(|ap| ap.active);
                let subtitle = match ap {
                    Some(ap) => ap.ssid.clone(),
                    None if self.net.wifi_enabled => T.not_connected.into(),
                    None => T.off.into(),
                };
                let icon = match ap {
                    Some(ap) => wifi_icon(ap.strength),
                    None if self.net.wifi_enabled => "network-wireless-offline-symbolic",
                    None => "network-wireless-disabled-symbolic",
                };
                (named(icon), self.net.wifi_enabled, subtitle, Some(Page::Wifi))
            }
            Tile::Bluetooth => {
                let names: Vec<_> =
                    self.bt.devices.iter().filter(|d| d.connected).map(|d| d.name.as_str()).collect();
                let subtitle = if self.bt.powered && !names.is_empty() {
                    names.join(", ")
                } else {
                    on_off(self.bt.powered)
                };
                let icon = if self.bt.powered { "bluetooth-active-symbolic" } else { "bluetooth-disabled-symbolic" };
                (named(icon), self.bt.powered, subtitle, Some(Page::Bluetooth))
            }
            Tile::Vpn => {
                let active = self.net.vpns.iter().find(|v| v.active.is_some());
                let subtitle = match active {
                    Some(v) if v.active.as_ref().is_some_and(|a| a.1) => v.name.clone(),
                    Some(_) => T.connecting.into(),
                    None => T.off.into(),
                };
                (named("network-vpn-symbolic"), active.is_some(), subtitle, Some(Page::Vpn))
            }
            Tile::Power => {
                let active = &self.profiles.active;
                (
                    icons::power_profile(active),
                    !active.is_empty() && active != "balanced",
                    profile_name(active).into(),
                    Some(Page::Power),
                )
            }
            Tile::Dnd => (named("notification-disabled-symbolic"), self.dnd, on_off(self.dnd), None),
            Tile::Dark => (icons::dark_style(), self.dark, on_off(self.dark), None),
            Tile::Airplane => (named("airplane-mode-symbolic"), self.airplane(), on_off(self.airplane()), None),
            Tile::Mic => {
                let muted = self.audio.mic_muted;
                let icon = if muted { "microphone-disabled-symbolic" } else { "audio-input-microphone-symbolic" };
                let subtitle = if muted { T.muted } else { T.on };
                (named(icon), muted, subtitle.into(), None)
            }
            Tile::KeepAwake => (icons::keep_awake(), self.inhibit.is_some(), on_off(self.inhibit.is_some()), None),
            Tile::Tiling => {
                let on = self.tiling.unwrap_or_else(desktop::autotile);
                (named("view-grid-symbolic"), on, on_off(on), None)
            }
            Tile::Magnifier => {
                let on = self.magnifier.unwrap_or(false);
                (named("zoom-in-symbolic"), on, on_off(on), None)
            }
            Tile::InvertColors => {
                let on = self.screen_filter.is_some_and(|f| f.0);
                (named("preferences-color-symbolic"), on, on_off(on), None)
            }
            Tile::ScreenReader => {
                let on = self.screen_reader.unwrap_or(false);
                (named("preferences-desktop-accessibility-symbolic"), on, on_off(on), None)
            }
            Tile::HighContrast => {
                let on = self.core.system_theme().cosmic().is_high_contrast;
                (named("color-select-symbolic"), on, on_off(on), None)
            }
            // Shortcuts: never lit.
            Tile::Screenshot => (named("accessories-screenshot-symbolic"), false, String::new(), None),
            Tile::Workspaces => (named("view-app-grid-symbolic"), false, String::new(), None),
            Tile::Pomodoro => {
                let timer = &self.pomodoro;
                let subtitle = if timer.started() {
                    format!("{} · {}", self.phase_name(), pomodoro::clock(timer.remaining(Instant::now())))
                } else {
                    on_off(false)
                };
                (named("alarm-symbolic"), timer.running(), subtitle, Some(Page::Pomodoro))
            }
            Tile::Hotspot => {
                let hotspot = self.net.hotspot.as_ref();
                let on = hotspot.is_some_and(|h| h.active.is_some());
                let subtitle = match hotspot {
                    Some(h) if on => h.ssid.clone(),
                    _ => on_off(false),
                };
                (named("network-wireless-hotspot-symbolic"), on, subtitle, Some(Page::Hotspot))
            }
            Tile::Dns => {
                let name = match self.dns {
                    Some(choice) => choice.name().unwrap_or(T.automatic).to_string(),
                    None => String::new(),
                };
                let on = self.dns.is_some_and(|d| d != sys::Dns::Automatic);
                (named("network-server-symbolic"), on, name, Some(Page::Dns))
            }
            Tile::GameMode => {
                let on = self.gamemode == Some(true);
                (named("input-gaming-symbolic"), on, on_off(on), None)
            }
            Tile::Custom(id) => {
                let custom = self.prefs.custom.iter().find(|c| c.id == id);
                let icon = custom.map_or("", |c| c.icon.as_str());
                let on = self.custom_on.contains(&id);
                let stateful = custom.is_some_and(|c| !c.status.trim().is_empty());
                let icon = named(if icon.trim().is_empty() { "application-x-executable-symbolic" } else { icon.trim() });
                (icon, on, if stateful { on_off(on) } else { String::new() }, None)
            }
            Tile::Keyboard => {
                let name = self.layouts.get(self.layout).map_or(String::new(), |l| l.name.clone());
                (named("input-keyboard-symbolic"), false, name, Some(Page::Keyboard))
            }
            Tile::Touchpad => {
                let on = self.touchpad.unwrap_or(false);
                let icon = if on { "input-touchpad-symbolic" } else { "touchpad-disabled-symbolic" };
                (named(icon), on, on_off(on), None)
            }
        };
        TileView { icon, title: self.tile_name(tile), subtitle, on, page }
    }

    fn click(&mut self, tile: Tile) -> app::Task<Message> {
        match tile {
            Tile::Wifi if self.net.wifi_device.is_none() => Task::none(),
            Tile::Wifi => {
                let on = !self.net.wifi_enabled;
                fire(async move { log("wifi", sys::set_wifi(on).await) })
            }
            Tile::Bluetooth => self.update(Message::SetBluetooth(!self.bt.powered)),
            Tile::Vpn => {
                let active: Vec<_> = self.net.vpns.iter().filter_map(|v| v.active.clone()).collect();
                if !active.is_empty() {
                    return Task::batch(active.into_iter().map(|(path, _)| self.update(Message::Disconnect(path))));
                }
                match self.net.vpns.first() {
                    // The one used last.
                    Some(vpn) => self.update(Message::Connect(vpn.path.clone())),
                    None => self.update(Message::Spawn("cosmic-settings", &["vpn"])),
                }
            }
            Tile::Power => {
                let list = &self.profiles.available;
                let at = list.iter().position(|p| *p == self.profiles.active).unwrap_or(0);
                match list.get((at + 1) % list.len().max(1)) {
                    Some(next) => self.update(Message::Profile(next.clone())),
                    None => Task::none(),
                }
            }
            Tile::Dnd => {
                self.dnd = !self.dnd;
                DND.set(self.dnd);
                Task::none()
            }
            Tile::Dark => {
                self.dark = !self.dark;
                DARK.set(self.dark);
                Task::none()
            }
            Tile::Airplane => {
                let on = self.airplane();
                let adapter = self.bt.adapter.clone();
                fire(async move {
                    log("wifi", sys::set_wifi(on).await);
                    if let Some(adapter) = adapter {
                        log("bluetooth", sys::set_bluetooth(adapter, on).await);
                    }
                })
            }
            Tile::Mic => fire(audio::toggle_mute(true)),
            // One request at a time: a double click would leave an
            // inhibition no one releases.
            Tile::KeepAwake if self.inhibiting => Task::none(),
            Tile::KeepAwake => match self.inhibit.take() {
                Some(cookie) => fire(desktop::allow_idle(cookie)),
                None => {
                    self.inhibiting = true;
                    run(async { Message::Inhibited(desktop::inhibit_idle(T.keep_awake).await) })
                }
            },
            Tile::Tiling => {
                let on = !self.tiling.unwrap_or_else(desktop::autotile);
                match &self.workspaces {
                    Some(tx) => _ = tx.send(workspaces::Command::Tiling(on)),
                    None => desktop::set_autotile(on),
                }
                self.tiling = Some(on);
                Task::none()
            }
            Tile::Magnifier => {
                if let (Some(tx), Some(on)) = (&self.a11y, self.magnifier) {
                    _ = tx.send(desktop::AccessibilityRequest::Magnifier(!on));
                }
                Task::none()
            }
            Tile::InvertColors => {
                if let (Some(tx), Some((inverted, filter))) = (&self.a11y, self.screen_filter) {
                    _ = tx.send(desktop::AccessibilityRequest::ScreenFilter { inverted: !inverted, filter });
                }
                Task::none()
            }
            Tile::ScreenReader => {
                if let (Some(tx), Some(on)) = (&self.reader, self.screen_reader) {
                    _ = tx.send(desktop::ReaderRequest::ScreenReader(!on));
                    self.screen_reader = Some(!on);
                }
                Task::none()
            }
            Tile::HighContrast => {
                desktop::set_high_contrast(!self.core.system_theme().cosmic().is_high_contrast);
                Task::none()
            }
            Tile::Screenshot => self.update(Message::Spawn("cosmic-screenshot", &[])),
            Tile::Workspaces => self.update(Message::Spawn("cosmic-workspaces", &[])),
            Tile::Pomodoro => self.update(Message::PomodoroToggle),
            Tile::Dns => self.update(Message::Page(Page::Dns)),
            Tile::Custom(id) => match self.prefs.custom.iter().find(|c| c.id == id) {
                Some(c) if c.confirm => self.update(Message::Page(Page::Confirm(id))),
                Some(_) => self.update(Message::RunCustom(id)),
                None => Task::none(),
            },
            Tile::GameMode => {
                let hold = !self.gamemode_held;
                self.gamemode = Some(hold);
                run(async move {
                    let done = desktop::hold_gamemode(hold).await;
                    Message::GameModeHeld(if done { hold } else { !hold })
                })
            }
            // On to the next layout; with one, its page.
            Tile::Keyboard if self.layouts.len() > 1 => {
                self.update(Message::KeyboardLayout((self.layout + 1) % self.layouts.len()))
            }
            Tile::Keyboard => self.update(Message::Page(Page::Keyboard)),
            Tile::Hotspot => self.update(Message::SetHotspot(!self.net.hotspot.as_ref().is_some_and(|h| h.active.is_some()))),
            Tile::Touchpad => {
                let on = !self.touchpad.unwrap_or(false);
                desktop::set_touchpad(on);
                self.touchpad = Some(on);
                Task::none()
            }
        }
    }

    fn open(&mut self) -> app::Task<Message> {
        let Some(parent) = self.core.main_window_id() else {
            return Task::none();
        };
        let id = window::Id::unique();
        self.popup = Some(id);
        self.opened = Instant::now();
        if self.page != Page::Password {
            self.page = Page::Main;
        }
        // The calendar opens on today, whatever month was left shown.
        let today = self.now.date();
        if self.visible != today.first_of_month() {
            self.selected = today;
            self.visible = today.first_of_month();
        }
        let mut tasks = vec![self.load_events(), self.refresh(Source::Media)];
        tasks.push(run(extras()));
        if self.prefs.sections.contains(&Section::Tasks) {
            tasks.push(run(async { Message::Tasks(tasks::open().await) }));
        }
        if self.weather_read.is_none_or(|at| at.elapsed() > Duration::from_secs(30 * 60)) {
            tasks.push(self.update(Message::CheckWeather));
        }
        if let Some(device) = self.net.wifi_device.clone() {
            tasks.push(fire(async move { _ = sys::scan_wifi(device).await }));
        }
        tasks.push(cosmic::surface::surface_task(surface::action::app_popup(
            |_: &Self| Default::default(),
            move |app: &mut Self| {
                let applet = &app.core.applet;
                let mut settings = applet.get_popup_settings(parent, id, None, None, None);
                let (w, h) = app.screen;
                // What the panel takes, and a little air.
                let panel = app.panel_room() as u32;
                let p = &mut settings.positioner;
                let width = app.main_width();
                match app.prefs.layout {
                    Layout::Dropdown => {
                        p.size_limits = Limits::NONE
                            .min_width(WIDTH)
                            .max_width(width)
                            .min_height(1.0)
                            .max_height(h.saturating_sub(panel) as f32);
                    }
                    Layout::Side => {
                        let size = (width as u32 + 40, h.saturating_sub(panel));
                        // Far to the right: the compositor slides it back to the edge.
                        p.offset.0 = w as i32;
                        p.size = Some(size);
                        p.size_limits = Limits::NONE.width(size.0 as f32).height(size.1 as f32);
                    }
                    Layout::Fullscreen => {
                        let size = (w, h.saturating_sub(panel));
                        p.size = Some(size);
                        p.size_limits = Limits::NONE.width(size.0 as f32).height(size.1 as f32);
                    }
                }
                // Slide and resize to stay on screen.
                p.constraint_adjustment = 1 | 2 | 16 | 32;
                settings
            },
            None,
        )));
        Task::batch(tasks)
    }

    fn close(&mut self) -> app::Task<Message> {
        if self.page != Page::Password {
            self.page = Page::Main;
        }
        // Next time, the load from then: not averaged over the time closed.
        self.sample = None;
        self.cpu_history.clear();
        match self.popup.take() {
            Some(id) => cosmic::surface::surface_task(surface::action::destroy_popup(id)),
            None => Task::none(),
        }
    }

    // ------------------------------------------------------------ views

    fn header(&self) -> Element<'_, Message> {
        let mut row = widget::Row::new().spacing(8).align_y(Alignment::Center);
        if let Some(b) = &self.battery {
            row = row
                .push(icon::icon(icons::battery(b.percent, b.charging)).size(16))
                .push(text::body(format!("{:.0}%", b.percent)));
        }
        // Mice, headsets, phones…: their charge, named on hover.
        for p in self.peripherals.iter().take(3) {
            let chip = widget::Row::new()
                .spacing(2)
                .align_y(Alignment::Center)
                .push(icon::from_name(p.icon).size(14))
                .push(text::caption(format!("{:.0}%", p.percent)));
            row = row.push(widget::tooltip(chip, text::caption(p.name.clone()), widget::tooltip::Position::Bottom));
        }
        let action = |name: &'static str, message: Message| {
            button::icon(icon::from_name(name)).padding(8).on_press(message)
        };
        let mut row = row.push(widget::space().width(Length::Fill));
        if self.updates > 0 {
            let badge = widget::Row::new()
                .spacing(4)
                .align_y(Alignment::Center)
                .push(icon::from_name("software-update-available-symbolic").size(16))
                .push(text::caption(self.updates.to_string()));
            row = row.push(widget::tooltip(
                button::custom(badge).class(theme::Button::Icon).padding(8).on_press(Message::Spawn("cosmic-store", &[])),
                text::caption(T.updates),
                widget::tooltip::Position::Bottom,
            ));
        }
        row
            .push(action("edit-symbolic", Message::Page(Page::Edit)))
            .push(action("emblem-system-symbolic", Message::Spawn("cosmic-settings", &[])))
            .push(action("system-lock-screen-symbolic", Message::Spawn("loginctl", &["lock-session"])))
            .push(action("system-shutdown-symbolic", Message::Spawn("cosmic-osd", &["shutdown"])))
            .into()
    }

    fn sliders(&self) -> Element<'_, Message> {
        let line = |name: &'static str, mute: Option<Message>, slider: Element<'static, Message>, level: f32, page: Option<Page>| {
            let mut row = widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(button::icon(icon::from_name(name)).padding(6).on_press_maybe(mute))
                .push(slider)
                .push(percent(level));
            if let Some(page) = page {
                row = row.push(button::icon(icon::from_name("go-next-symbolic")).padding(6).on_press(Message::Page(page)));
            }
            row
        };
        let volume_icon = volume_icon(self.audio.muted, self.audio.volume);
        let volume = widget::slider(0.0..=1.0, self.audio.volume, Message::Volume)
            .step(0.01_f32)
            .on_release(Message::Released)
            .into();
        let mut column = widget::Column::new().spacing(4);
        // The sound card has its own.
        if !self.prefs.sections.contains(&Section::Audio) {
            column = column.push(line(volume_icon, Some(Message::Mute), volume, self.audio.volume, Some(Page::Sound)));
        }
        if let Some(level) = self.brightness {
            let slider = widget::slider(0.05..=1.0, level, Message::SetBrightness)
                .step(0.01_f32)
                .on_release(Message::Released)
                .into();
            column = column.push(line("display-brightness-symbolic", None, slider, level, None));
        }
        if let Some((_, level, max)) = &self.keyboard_light {
            let slider = widget::slider(0.0..=1.0, *level as f32 / *max as f32, Message::KeyboardLight)
                .step(1.0 / *max as f32)
                .on_release(Message::Released)
                .into();
            column = column.push(line("keyboard-brightness-symbolic", None, slider, *level as f32 / *max as f32, None));
        }
        column.into()
    }

    fn tile_view(&self, tile: Tile) -> Element<'_, Message> {
        let view = self.tile(tile);
        let small = self.prefs.small.contains(&tile);
        let style = self.prefs.tile_style;
        let lit = view.on && style == crate::config::TileStyle::High;
        // Medium and low: the icon on a square of its own, lit at medium.
        let glyph = |handle: icon::Handle| -> Element<'_, Message> {
            if style == crate::config::TileStyle::High {
                icon::icon(handle).size(20).into()
            } else {
                container(icon::icon(handle).size(18))
                    .padding(7)
                    .class(style::icon_box(view.on && style == crate::config::TileStyle::Medium))
                    .into()
            }
        };
        let content: Element<'_, Message> = if small {
            widget::container(glyph(view.icon.clone())).center(Length::Fill).into()
        } else {
            let mut label = widget::Column::new().push(text::body(view.title.clone()).font(cosmic::font::bold()));
            if !view.subtitle.is_empty() {
                label = label.push(text::caption(view.subtitle).wrapping(iced::widget::text::Wrapping::None));
            }
            let mut row = widget::Row::new()
                .spacing(10)
                .align_y(Alignment::Center)
                .push(glyph(view.icon.clone()))
                .push(widget::container(label).width(Length::Fill).clip(true));
            if let Some(page) = view.page {
                row = row.push(
                    button::icon(icon::from_name("go-next-symbolic"))
                        .class(style::on_tile(lit))
                        .padding(4)
                        .on_press(Message::Page(page)),
                );
            }
            row.into()
        };
        let button = button::custom(content)
            .class(style::tile(lit))
            .padding(if small { [10, 0] } else { [10, 12] })
            .width(Length::Fill)
            .height(Length::Fixed(60.0))
            .on_press(Message::TileClick(tile));
        let tip: Element<'_, Message> = button.into();
        if small {
            widget::tooltip(tip, text::caption(view.title), widget::tooltip::Position::Top).into()
        } else {
            tip
        }
    }

    fn tiles(&self) -> Element<'_, Message> {
        let shown: Vec<Tile> = self.prefs.tiles.iter().copied().filter(|t| self.available(*t)).collect();
        let mut column = widget::Column::new().spacing(8);
        for row in rows(&shown, &self.prefs.small) {
            let mut line = widget::Row::new().spacing(8);
            let mut used = 0;
            for (_, tile) in row {
                let width = if self.prefs.small.contains(&tile) { 1 } else { 2 };
                used += width;
                line = line.push(widget::container(self.tile_view(tile)).width(Length::FillPortion(width)));
            }
            if used < 4 {
                line = line.push(widget::space().width(Length::FillPortion(4 - used)));
            }
            column = column.push(line);
        }
        column.into()
    }

    fn month_grid(&self) -> Element<'_, Message> {
        let first = self.visible;
        let today = self.now.date();
        let lead = i18n::week_offset(first);
        let start = first.saturating_sub(jiff::Span::new().days(lead));
        let title = i18n::month_title(first);
        let mut grid = widget::Column::new().spacing(2).push(
            widget::Row::new()
                .align_y(Alignment::Center)
                .push(button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::PrevMonth))
                .push(widget::container(text::heading(title)).center_x(Length::Fill))
                .push(button::icon(icon::from_name("go-next-symbolic")).on_press(Message::NextMonth)),
        );
        let mut names = widget::Row::new();
        for head in i18n::weekday_heads() {
            names = names.push(widget::container(text::caption(head)).center_x(Length::Fill));
        }
        grid = grid.push(names);
        for week in 0..6 {
            let monday = start.saturating_add(jiff::Span::new().days(week * 7));
            if week > 0 && monday.month() != first.month() {
                break;
            }
            let mut row = widget::Row::new().spacing(2);
            for i in 0..7 {
                let day = monday.saturating_add(jiff::Span::new().days(i));
                if day.month() != first.month() {
                    row = row.push(widget::space().width(Length::Fill).height(Length::Fixed(38.0)));
                    continue;
                }
                let selected = day == self.selected;
                let busy = self.events.as_ref().is_some_and(|e| e.contains_key(&day));
                let mut number = text::body(day.day().to_string());
                if day == today && !selected {
                    number = number.class(theme::Text::Accent).font(cosmic::font::bold());
                }
                let dot: Element<'_, Message> = if busy { dot(selected) } else { widget::space().height(5).into() };
                let cell = widget::Column::new()
                    .spacing(2)
                    .align_x(Alignment::Center)
                    .push(number)
                    .push(dot);
                row = row.push(
                    button::custom(widget::container(cell).center_x(Length::Fill))
                        .class(if selected { style::picked() } else { style::flat() })
                        .padding([4, 0])
                        .width(Length::Fill)
                        .height(Length::Fixed(38.0))
                        .on_press(Message::Day(day)),
                );
            }
            grid = grid.push(row);
        }
        grid.into()
    }

    fn calendar_view(&self) -> Element<'_, Message> {
        // Closed: today, and what's next today.
        let today = self.now.date();
        let next = self.today.as_ref().and_then(|events| {
            let now = self.now.time();
            events.iter().find(|e| e.end.or(e.start).is_none_or(|t| t >= now))
        });
        let summary = match (next, &self.events) {
            (Some(e), _) => match e.start {
                Some(start) => format!("{} {}", i18n::clock(start), e.summary),
                None => e.summary.clone(),
            },
            // Today had events, all over by now.
            (None, Some(_)) if self.today.as_ref().is_some_and(|e| !e.is_empty()) => T.no_more_events.to_string(),
            (None, Some(_)) => T.no_events.to_string(),
            (None, None) => T.no_calendar.to_string(),
        };
        let chevron = if self.prefs.calendar_open { "go-up-symbolic" } else { "go-down-symbolic" };
        let head = button::custom(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(
                    widget::Column::new()
                        .push(text::title4(i18n::long_date(today)))
                        .push(text::caption(summary).wrapping(iced::widget::text::Wrapping::None))
                        .width(Length::Fill),
                )
                .push_maybe(self.meeting().and_then(|m| self.join_button(m)))
                .push(icon::from_name(chevron).size(16)),
        )
        .class(style::flat())
        .padding(12)
        .width(Length::Fill)
        .on_press(Message::ToggleCalendar);
        if !self.prefs.calendar_open {
            return head.into();
        }
        let mut column = widget::Column::new().spacing(8).push(self.month_grid());
        match &self.events {
            None => {
                column = column.push(
                    widget::Row::new()
                        .align_y(Alignment::Center)
                        .push(widget::space().width(Length::Fill))
                        .push(button::text(T.add_accounts).on_press(Message::Spawn("gnome-online-accounts-gtk", &[]))),
                );
            }
            Some(days) => {
                column = column.push(text::heading(i18n::long_date(self.selected)));
                let events = days.get(&self.selected).map(Vec::as_slice).unwrap_or_default();
                if events.is_empty() {
                    column = column.push(text::caption(T.no_events));
                }
                for event in events {
                    let when = match (event.start, event.end) {
                        (Some(s), Some(e)) => format!("{} – {}", i18n::clock(s), i18n::clock(e)),
                        (Some(s), None) => i18n::clock(s),
                        _ => T.all_day.to_string(),
                    };
                    let mut info = widget::Column::new().push(text::body(event.summary.as_str()));
                    if !event.location.is_empty() {
                        info = info.push(text::caption(event.location.as_str()));
                    }
                    column = column.push(
                        widget::Row::new()
                            .spacing(12)
                            .align_y(Alignment::Center)
                            .push(text::caption(when).width(Length::Fixed(90.0)))
                            .push(container(info).width(Length::Fill))
                            .push_maybe(self.join_button(event)),
                    );
                }
            }
        }
        widget::Column::new()
            .push(head)
            .push(container(column).padding([0, 12, 12, 12]))
            .into()
    }

    /// What's running that speaks up: the music, who records.
    fn island(&self) -> Option<Element<'_, Message>> {
        let mut column = widget::Column::new().spacing(8);
        let mut any = false;
        if let Some(media) = &self.media {
            any = true;
            let art: Element<'_, Message> = match &media.art {
                Some(handle) => widget::image(handle.clone()).width(44).height(44).into(),
                None => icon::from_name("audio-x-generic-symbolic").size(44).into(),
            };
            let play = if media.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" };
            let control = |name: &'static str, method: &'static str| {
                button::icon(icon::from_name(name)).padding(6).on_press(Message::MediaControl(method))
            };
            column = column.push(
                widget::Row::new()
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(art)
                    .push(
                        widget::container(
                            widget::Column::new()
                                .push(text::body(media.title.as_str()).wrapping(iced::widget::text::Wrapping::None))
                                .push(text::caption(media.artist.as_str()).wrapping(iced::widget::text::Wrapping::None)),
                        )
                        .width(Length::Fill)
                        .clip(true),
                    )
                    .push(control("media-skip-backward-symbolic", "Previous"))
                    .push(control(play, "PlayPause"))
                    .push(control("media-skip-forward-symbolic", "Next")),
            );
        }
        if let Some(meeting) = self.meeting() {
            any = true;
            let when = meeting.start.map(i18n::clock).unwrap_or_default();
            column = column.push(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("x-office-calendar-symbolic").size(16))
                    .push(
                        container(text::body(format!("{when} {}", meeting.summary)).wrapping(iced::widget::text::Wrapping::None))
                            .width(Length::Fill)
                            .clip(true),
                    )
                    .push_maybe(self.join_button(meeting)),
            );
        }
        if self.pomodoro.started() {
            any = true;
            column = column.push(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("alarm-symbolic").size(16))
                    .push(
                        text::body(format!(
                            "{} · {}",
                            self.phase_name(),
                            pomodoro::clock(self.pomodoro.remaining(Instant::now()))
                        ))
                        .width(Length::Fill),
                    )
                    .push(self.pomodoro_controls()),
            );
        }
        // In the colors phones use: green camera, orange microphone.
        let privacy = [
            (&self.camera, "camera-web-symbolic", "filming", Color::from_rgb(0.3, 0.85, 0.4)),
            (&self.audio.recording, "audio-input-microphone-symbolic", "recording", Color::from_rgb(1.0, 0.65, 0.2)),
        ];
        for (apps, icon_name, what, color) in privacy {
            if apps.is_empty() {
                continue;
            }
            any = true;
            column = column.push(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name(icon_name).size(16))
                    .push(text::caption(i18n::using(what, apps)).class(theme::Text::Color(color))),
            );
        }
        any.then(|| {
            container(column)
                .padding([10, 14])
                .width(Length::Fill)
                .class(theme::Container::custom(|theme| {
                    let cosmic = theme.cosmic();
                    container::Style {
                        background: Some(Color::from(cosmic.palette.neutral_1.with_alpha(0.9)).into()),
                        text_color: Some(cosmic.palette.neutral_9.into()),
                        icon_color: Some(cosmic.palette.neutral_9.into()),
                        border: iced::Border { radius: cosmic.corner_radii.radius_m.into(), ..Default::default() },
                        ..Default::default()
                    }
                }))
                .into()
        })
    }

    /// A section as shown: padded, flush to its edges (its head lights
    /// edge to edge on hover), or with a surface of its own (the island).
    fn section(&self, section: Section) -> Option<(Element<'_, Message>, Look)> {
        Some(match section {
            Section::Status => (self.header(), Look::Padded),
            Section::Controls => (
                widget::Column::new().spacing(14).push(self.sliders()).push(self.tiles()).into(),
                Look::Padded,
            ),
            Section::Calendar => (self.calendar_view(), Look::Flush),
            Section::Weather => (self.weather_view(), Look::Flush),
            Section::Notifications => (self.notifications_view(), Look::Flush),
            Section::Island => (self.island()?, Look::Own),
            Section::Audio => (self.sound_card(), Look::Padded),
            Section::System => (self.system_view(), Look::Padded),
            Section::Tasks => (self.tasks_view(), Look::Padded),
            Section::Clocks => (self.clocks_view(), Look::Padded),
        })
    }

    /// The popup's parts, in the user's order and columns: on one surface,
    /// or each an island of its own.
    fn main_page(&self) -> Element<'_, Message> {
        let islands = self.prefs.islands;
        let fit = self.columns();
        let mut columns: Vec<Vec<Element<'_, Message>>> = (0..fit).map(|_| Vec::new()).collect();
        for &s in &self.prefs.sections {
            if let Some(part) = self.section(s) {
                // A column past the screen's edge: into the last that fits.
                columns[self.prefs.column(s).min(fit - 1)].push(shape(part, islands));
            }
        }
        columns
            .into_iter()
            .fold(widget::Row::new().spacing(GAP), |row, parts| {
                row.push(container(stack(parts, islands)).width(Length::Fixed(WIDTH)))
            })
            .into()
    }

    /// What the panel takes of the screen's height, and a little air.
    fn panel_room(&self) -> f32 {
        let applet = &self.core.applet;
        (applet.suggested_size(true).1 + 2 * applet.suggested_padding(true).0) as f32 + 16.0
    }

    fn tile_name(&self, tile: Tile) -> String {
        match tile {
            Tile::Custom(id) => self.prefs.custom.iter().find(|c| c.id == id).map_or(String::new(), |c| c.name.clone()),
            _ => tile.name().to_string(),
        }
    }

    /// Changes a setting and keeps it.
    fn pref(&mut self, change: impl FnOnce(&mut Prefs)) {
        change(&mut self.prefs);
        self.prefs.save();
    }

    /// Whether a tile has something to work with here.
    fn available(&self, tile: Tile) -> bool {
        match tile {
            Tile::GameMode => self.gamemode.is_some(),
            Tile::Custom(id) => self.prefs.custom.iter().any(|c| c.id == id),
            _ => true,
        }
    }

    /// Each second: the pomodoro's phase, and a new day.
    fn tick(&mut self) -> app::Task<Message> {
        if let Some(phase) = self.pomodoro.tick(Instant::now()) {
            let (summary, body) = match phase {
                pomodoro::Phase::Focus => (T.pomodoro_focus_again, T.focus),
                _ => (T.pomodoro_break_time, self.phase_name()),
            };
            return fire(desktop::notify(summary.to_string(), body.to_string()));
        }
        let now = Zoned::now();
        if now.date() != self.now.date() {
            self.selected = now.date();
            self.visible = now.date().first_of_month();
            self.now = now;
            return self.load_events();
        }
        self.now = now;
        Task::none()
    }

    /// The shown part above (or below) the `i`th in its column.
    fn neighbor(&self, i: usize, up: bool) -> Option<usize> {
        let sections = &self.prefs.sections;
        let column = self.prefs.column(*sections.get(i)?);
        let same = |&j: &usize| self.prefs.column(sections[j]) == column;
        if up { (0..i).rev().find(same) } else { (i + 1..sections.len()).find(same) }
    }

    /// The columns set, as many as the screen has room for (and a side
    /// panel's margin).
    fn columns(&self) -> usize {
        let room = ((self.screen.0 as f32 - 40.0 + GAP) / (WIDTH + GAP)) as usize;
        self.prefs.columns().min(room.max(1))
    }

    /// The main page's width: its columns side by side.
    fn main_width(&self) -> f32 {
        let n = self.columns() as f32;
        WIDTH * n + GAP * (n - 1.0)
    }

    fn weather_view(&self) -> Element<'_, Message> {
        let chevron = if self.prefs.weather_open { "go-up-symbolic" } else { "go-down-symbolic" };
        let degrees = |t: f64| format!("{:.0}°", t);
        let Some(w) = &self.weather else {
            // Not yet read, offline, or no place found.
            return container(
                widget::Row::new()
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("weather-overcast-symbolic").size(24))
                    .push(text::caption(T.weather_unavailable).width(Length::Fill))
                    .push(button::icon(icon::from_name("view-refresh-symbolic")).padding(4).on_press(Message::CheckWeather)),
            )
            .padding(12)
            .into();
        };
        let wind = if w.fahrenheit { "mph" } else { "km/h" };
        let head = button::custom(
            widget::Row::new()
                .spacing(12)
                .align_y(Alignment::Center)
                .push(icon::from_name(w.condition.icon(w.day)).size(32))
                .push(
                    widget::Column::new()
                        .push(
                            widget::Row::new()
                                .spacing(8)
                                .align_y(Alignment::Center)
                                .push(text::title4(degrees(w.temperature)))
                                .push(text::body(condition_name(w.condition))),
                        )
                        .push(text::caption(format!(
                            "{} · {} {} · {} {:.0} {wind}",
                            w.place,
                            T.feels_like,
                            degrees(w.feels_like),
                            T.wind,
                            w.wind
                        )).wrapping(iced::widget::text::Wrapping::None))
                        .width(Length::Fill),
                )
                .push(icon::from_name(chevron).size(16)),
        )
        .class(style::flat())
        .padding(12)
        .width(Length::Fill)
        .on_press(Message::ToggleWeather);
        if !self.prefs.weather_open {
            return head.into();
        }
        // The card's inside: the popup's width, less the one surface's padding and its own.
        let inside = WIDTH - if self.prefs.islands { 0.0 } else { 28.0 } - 24.0;
        let cell = (inside - 4.0 * 4.0) / 5.0;
        // Today is in the head: the days after it; more than five scroll sideways.
        let shown = self.prefs.weather_days.clamp(1, 7) as usize;
        let mut days = widget::Row::new().spacing(4);
        for day in w.days.iter().skip(1).take(shown) {
            days = days.push(
                widget::Column::new()
                    .spacing(4)
                    .align_x(Alignment::Center)
                    .width(if shown > 5 { Length::Fixed(cell) } else { Length::Fill })
                    .push(text::caption(i18n::weekday_short(day.date)))
                    .push(icon::from_name(day.condition.icon(true)).size(24))
                    .push(text::caption(format!("{} / {}", degrees(day.high), degrees(day.low)))),
            );
        }
        // The next hours, five at a time, scrolled sideways.
        let (rain, wind) = if w.fahrenheit { ("in", "mph") } else { ("mm", "km/h") };
        let hours = w.hours.iter().take(self.prefs.weather_hours as usize).fold(widget::Row::new().spacing(4).padding([0, 0, 10, 0]), |row, h| {
            row.push(
                widget::Column::new()
                    .spacing(2)
                    .align_x(Alignment::Center)
                    .width(Length::Fixed(cell))
                    .push(text::caption(i18n::clock(h.time)))
                    .push(icon::from_name(h.condition.icon(h.day)).size(20))
                    .push(text::body(degrees(h.temperature)))
                    .push(text::caption(format!("{}% {:.1}{rain}", h.chance, h.precipitation)).wrapping(iced::widget::text::Wrapping::None))
                    .push(text::caption(format!("{:.0} {wind}", h.wind))),
            )
        });
        let days: Element<'_, Message> =
            if shown > 5 { widget::scrollable::horizontal(days.padding([0, 0, 10, 0])).into() } else { days.into() };
        let mut column = widget::Column::new().push(head);
        if self.prefs.weather_hourly {
            column = column
                .push(container(widget::scrollable::horizontal(hours)).padding([0, 12, 8, 12]))
                .push(widget::divider::horizontal::light());
        }
        column.push(container(days).padding([8, 12, 12, 12])).into()
    }

    fn notifications_view(&self) -> Element<'_, Message> {
        let chevron = if self.prefs.notifications_open { "go-up-symbolic" } else { "go-down-symbolic" };
        let count = self.notifications.len();
        let mut head_row = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(icon::from_name(if count > 0 { "notification-new-symbolic" } else { "notification-symbolic" }).size(18))
            .push(text::heading(T.notifications))
            .push(text::caption(if count > 0 { count.to_string() } else { String::new() }).width(Length::Fill));
        if count > 0 && self.prefs.notifications_open {
            head_row = head_row.push(button::text(T.clear_all).on_press(Message::NotificationsClear));
        }
        let head = button::custom(head_row.push(icon::from_name(chevron).size(16)))
            .class(style::flat())
            .padding(12)
            .width(Length::Fill)
            .on_press(Message::ToggleNotifications);
        if !self.prefs.notifications_open {
            return head.into();
        }
        let mut list = widget::Column::new().spacing(4).padding([0, 8, 8, 8]);
        if count == 0 {
            list = list.push(container(text::caption(T.no_notifications)).padding([0, 4, 4, 4]));
        }
        for n in self.notifications.iter().take(20) {
            let image: Element<'_, Message> = if n.icon.starts_with('/') {
                widget::image(n.icon.as_str()).width(24).height(24).into()
            } else if n.icon.is_empty() {
                icon::from_name("dialog-information-symbolic").size(24).into()
            } else {
                icon::from_name(n.icon.as_str()).size(24).into()
            };
            let when = jiff::Timestamp::try_from(n.time)
                .ok()
                .map(|t| t.to_zoned(jiff::tz::TimeZone::system()))
                .map(|z| if z.date() == self.now.date() { i18n::clock(z.time()) } else { i18n::short_date(z.date()) })
                .unwrap_or_default();
            let mut info = widget::Column::new()
                .push(
                    widget::Row::new()
                        .spacing(6)
                        .push(text::caption(n.app.as_str()).width(Length::Fill))
                        .push(text::caption(when)),
                )
                .push(text::body(n.summary.as_str()).font(cosmic::font::bold()));
            if !n.body.is_empty() {
                info = info.push(text::caption(n.body.as_str()));
            }
            let card = button::custom(
                widget::Row::new()
                    .spacing(10)
                    .push(image)
                    .push(container(info).width(Length::Fill))
                    .push(button::icon(icon::from_name("window-close-symbolic")).padding(2).on_press(Message::NotificationDismiss(n.key))),
            )
            .class(style::flat())
            .padding(8)
            .width(Length::Fill)
            .on_press(Message::NotificationOpen(n.key));
            list = list.push(card);
        }
        widget::Column::new().push(head).push(list).into()
    }

    fn page_header(&self, title: &str, toggle: Option<(bool, fn(bool) -> Message)>) -> Element<'_, Message> {
        let mut row = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(button::icon(icon::from_name("go-previous-symbolic")).padding(8).on_press(Message::Page(Page::Main)))
            .push(text::title4(title.to_string()).width(Length::Fill));
        if let Some((on, message)) = toggle {
            row = row.push(widget::toggler(on).on_toggle(message));
        }
        row.into()
    }

    fn item(
        &self,
        icon_handle: icon::Handle,
        title: impl Into<String>,
        subtitle: Option<String>,
        checked: bool,
        message: Option<Message>,
    ) -> Element<'static, Message> {
        let mut label = widget::Column::new().push(text::body(title.into()));
        if let Some(subtitle) = subtitle {
            label = label.push(text::caption(subtitle));
        }
        let mut row = widget::Row::new()
            .spacing(12)
            .align_y(Alignment::Center)
            .push(icon::icon(icon_handle).size(20))
            .push(widget::container(label).width(Length::Fill));
        if checked {
            row = row.push(icon::from_name("object-select-symbolic").size(16));
        }
        button::custom(row)
            .class(theme::Button::MenuItem)
            .padding([8, 10])
            .width(Length::Fill)
            .on_press_maybe(message)
            .into()
    }

    /// COSMIC Settings, on its page for this.
    fn more(&self, page: &'static [&'static str]) -> Element<'_, Message> {
        button::text(T.more_settings).on_press(Message::Spawn("cosmic-settings", page)).into()
    }

    fn sub_page(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(4);
        match self.page {
            Page::Wifi => {
                column = column.push(self.page_header(T.wifi, Some((self.net.wifi_enabled, Message::SetWifi))));
                if self.net.wifi_enabled {
                    for ap in &self.net.access_points {
                        let message = match (ap.active, &self.net.wifi_device) {
                            (true, _) | (_, None) => None,
                            (false, Some(_)) => Some(Message::Join(ap.clone())),
                        };
                        let subtitle = ap.active.then(|| T.connected.to_string());
                        let name = if ap.secure { format!("{} 🔒", ap.ssid) } else { ap.ssid.clone() };
                        column = column.push(self.item(named(wifi_icon(ap.strength)), name, subtitle, ap.active, message));
                    }
                }
                column = column.push(self.more(&["wireless"]));
            }
            Page::Bluetooth => {
                column = column.push(self.page_header(T.bluetooth, Some((self.bt.powered, Message::SetBluetooth))));
                if self.bt.powered {
                    for device in &self.bt.devices {
                        let subtitle = match (device.connected, device.battery) {
                            (true, Some(b)) => Some(format!("{} · {b}%", T.connected)),
                            (true, None) => Some(T.connected.to_string()),
                            _ => None,
                        };
                        let icon_name = format!("{}-symbolic", device.icon);
                        column = column.push(self.item(
                            named(&icon_name),
                            device.name.clone(),
                            subtitle,
                            device.connected,
                            Some(Message::BtConnect(device.path.clone(), !device.connected)),
                        ));
                    }
                }
                column = column.push(self.more(&["bluetooth"]));
            }
            Page::Vpn => {
                column = column.push(self.page_header(T.vpn, None));
                for vpn in &self.net.vpns {
                    let (subtitle, message) = match &vpn.active {
                        Some((path, up)) => (
                            Some(if *up { T.connected } else { T.connecting }.to_string()),
                            Message::Disconnect(path.clone()),
                        ),
                        None => (None, Message::Connect(vpn.path.clone())),
                    };
                    column = column.push(self.item(
                        named("network-vpn-symbolic"),
                        vpn.name.clone(),
                        subtitle,
                        vpn.active.is_some(),
                        Some(message),
                    ));
                }
                column = column.push(self.more(&["vpn"]));
            }
            Page::Power => {
                column = column.push(self.page_header(T.power, None));
                for profile in &self.profiles.available {
                    column = column.push(self.item(
                        icons::power_profile(profile),
                        profile_name(profile),
                        None,
                        *profile == self.profiles.active,
                        Some(Message::Profile(profile.clone())),
                    ));
                }
                if let Some((_, on)) = &self.charge_limit {
                    column = column.push(
                        widget::Column::new()
                            .padding([8, 10])
                            .spacing(2)
                            .push(switch(T.charge_limit, *on, Message::SetChargeLimit))
                            .push(text::caption(T.charge_limit_hint)),
                    );
                }
                column = column.push(self.more(&["power"]));
            }
            Page::Confirm(id) => {
                let name = self.tile_name(Tile::Custom(id));
                column = column
                    .push(self.page_header(&name, None))
                    .push(container(text::body(T.confirm_run)).padding([8, 10]))
                    .push(
                        widget::Row::new()
                            .spacing(8)
                            .padding([8, 10])
                            .push(widget::space().width(Length::Fill))
                            .push(button::standard(T.cancel).on_press(Message::Page(Page::Main)))
                            .push(button::destructive(T.run).on_press(Message::RunCustom(id))),
                    );
            }
            Page::Dns => {
                column = column.push(self.page_header("DNS", None));
                for choice in sys::Dns::ALL {
                    let servers = choice.servers().iter().map(|s| std::net::Ipv4Addr::from(*s).to_string()).collect::<Vec<_>>().join(", ");
                    column = column.push(self.item(
                        named("network-server-symbolic"),
                        choice.name().unwrap_or(T.automatic),
                        Some(if servers.is_empty() { T.dns_automatic_hint.to_string() } else { servers }),
                        self.dns == Some(choice),
                        Some(Message::SetDns(choice)),
                    ));
                }
                column = column.push(container(text::caption(T.dns_hint)).padding([8, 10]));
                column = column.push(self.more(&["wired"]));
            }
            Page::Sound => {
                column = column.push(self.page_header(T.sound_output, None));
                for output in &self.audio.outputs {
                    column = column.push(self.item(
                        named("audio-speakers-symbolic"),
                        output.description.clone(),
                        None,
                        output.default,
                        Some(Message::Output(output.name.clone())),
                    ));
                }
                if !self.audio.inputs.is_empty() {
                    column = column.push(container(text::heading(T.mic)).padding([12, 10, 4, 10]));
                    for input in &self.audio.inputs {
                        column = column.push(self.item(
                            named("audio-input-microphone-symbolic"),
                            input.description.clone(),
                            None,
                            input.default,
                            Some(Message::Input(input.name.clone())),
                        ));
                    }
                }
                column = column.push(self.mixer());
                column = column.push(self.more(&["sound"]));
            }
            Page::Pomodoro => {
                column = column.push(self.page_header(T.pomodoro, None));
                let left = pomodoro::clock(self.pomodoro.remaining(Instant::now()));
                column = column.push(
                    widget::Column::new()
                        .spacing(8)
                        .padding([8, 10])
                        .align_x(Alignment::Center)
                        .width(Length::Fill)
                        .push(text::title1(left))
                        .push(text::body(self.phase_name()))
                        .push(self.pomodoro_controls()),
                );
                column = column.push(
                    widget::Row::new()
                        .padding([8, 10])
                        .align_y(Alignment::Center)
                        .push(text::body(T.show_on_panel).width(Length::Fill))
                        .push(widget::toggler(self.prefs.pomodoro_panel).on_toggle(Message::PomodoroPanel)),
                );
            }
            Page::Hotspot => {
                let hotspot = self.net.hotspot.as_ref();
                let on = hotspot.is_some_and(|h| h.active.is_some());
                column = column.push(self.page_header(T.hotspot, Some((on, Message::SetHotspot))));
                if let Some(h) = hotspot {
                    let line = |label: &'static str, value: String| {
                        widget::Row::new()
                            .spacing(12)
                            .padding([8, 10])
                            .push(text::body(label).width(Length::Fill))
                            .push(text::body(value).font(cosmic::font::bold()))
                    };
                    column = column.push(line(T.network_name, h.ssid.clone()));
                    if let Some(password) = &self.hotspot_password {
                        column = column.push(line(T.password, password.clone()));
                    }
                } else {
                    column = column.push(container(text::caption(T.hotspot_hint)).padding([8, 10]));
                }
                column = column.push(self.more(&["wireless"]));
            }
            Page::Keyboard => {
                column = column.push(self.page_header(T.keyboard, None));
                for (i, layout) in self.layouts.iter().enumerate() {
                    column = column.push(self.item(
                        named("input-keyboard-symbolic"),
                        layout.name.clone(),
                        Some(if layout.variant.is_empty() { layout.code.clone() } else { format!("{} · {}", layout.code, layout.variant) }),
                        i == self.layout,
                        Some(Message::KeyboardLayout(i)),
                    ));
                }
                column = column.push(self.more(&["keyboard"]));
            }
            Page::Edit => column = column.push(self.edit_page()),
            Page::Password => column = column.push(self.password_page()),
            Page::Main => {}
        }
        column.into()
    }

    fn next_key(&mut self) -> u64 {
        self.keys += 1;
        self.keys
    }

    /// Today's next video meeting, from ten minutes before it starts until
    /// it ends.
    fn meeting(&self) -> Option<&Event> {
        let now = self.now.time();
        let soon = now.saturating_add(jiff::Span::new().minutes(10));
        self.today.as_ref()?.iter().find(|e| {
            e.link.is_some()
                && e.start.is_some_and(|start| start <= soon)
                && e.end.or(e.start).is_some_and(|end| end >= now)
        })
    }

    fn join_button(&self, event: &Event) -> Option<Element<'_, Message>> {
        let link = event.link.clone()?;
        Some(
            button::custom(
                widget::Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("camera-video-symbolic").size(14))
                    .push(text::caption(T.join)),
            )
            .class(style::picked())
            .padding([4, 10])
            .on_press(Message::OpenUrl(link))
            .into(),
        )
    }

    fn phase_name(&self) -> &'static str {
        match self.pomodoro.phase {
            pomodoro::Phase::Focus => T.focus,
            pomodoro::Phase::Break => T.short_break,
            pomodoro::Phase::LongBreak => T.long_break,
        }
    }

    /// The pomodoro's controls: start or pause, skip, reset.
    fn pomodoro_controls(&self) -> Element<'_, Message> {
        let play = if self.pomodoro.running() { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" };
        widget::Row::new()
            .spacing(4)
            .push(button::icon(icon::from_name(play)).padding(6).on_press(Message::PomodoroToggle))
            .push(button::icon(icon::from_name("media-skip-forward-symbolic")).padding(6).on_press(Message::PomodoroSkip))
            .push(button::icon(icon::from_name("view-refresh-symbolic")).padding(6).on_press(Message::PomodoroReset))
            .into()
    }

    /// Sound on a card: output and microphone levels, each with its device
    /// (a click lists the others), and the apps playing, folded away.
    fn sound_card(&self) -> Element<'_, Message> {
        let device = |list: &[audio::Output], inputs: bool| {
            let current = list.iter().find(|d| d.default).map_or("", |d| d.description.as_str());
            let open = self.picking == Some(inputs);
            let chevron = if open { "go-up-symbolic" } else { "go-down-symbolic" };
            let mut column = widget::Column::new().push(
                button::custom(
                    widget::Row::new()
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .push(text::caption(current.to_string()).width(Length::Fill).wrapping(iced::widget::text::Wrapping::None))
                        .push(icon::from_name(chevron).size(16)),
                )
                .class(style::flat())
                .padding([4, 10])
                .width(Length::Fill)
                .on_press(Message::Picking(inputs)),
            );
            if open {
                let icon = if inputs { "audio-input-microphone-symbolic" } else { "audio-speakers-symbolic" };
                for d in list {
                    let pick = if inputs { Message::Input(d.name.clone()) } else { Message::Output(d.name.clone()) };
                    column = column.push(self.item(named(icon), d.description.clone(), None, d.default, Some(pick)));
                }
            }
            column
        };
        let level = |icon: &'static str, mute: Message, value: f32, set: fn(f32) -> Message| {
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(button::icon(icon::from_name(icon)).padding(6).on_press(mute))
                .push(widget::slider(0.0..=1.0, value, set).step(0.01_f32).on_release(Message::Released))
                .push(percent(value))
        };
        let mut column = widget::Column::new()
            .spacing(4)
            .push(level(volume_icon(self.audio.muted, self.audio.volume), Message::Mute, self.audio.volume, Message::Volume))
            .push(device(&self.audio.outputs, false));
        if !self.audio.inputs.is_empty() {
            let mic = if self.audio.mic_muted { "microphone-disabled-symbolic" } else { "audio-input-microphone-symbolic" };
            column = column
                .push(level(mic, Message::TileClick(Tile::Mic), self.audio.mic_volume, Message::MicVolume))
                .push(device(&self.audio.inputs, true));
        }
        column.push(self.mixer()).into()
    }

    /// The processor (with its last minute or so), the memory, the network.
    fn system_view(&self) -> Element<'_, Message> {
        let line = |label: &'static str, value: String| {
            widget::Row::new()
                .align_y(Alignment::Center)
                .push(text::heading(label).width(Length::Fill))
                .push(text::caption(value))
        };
        // The newest on the right, each second or two a bar.
        let history = self.cpu_history.iter().fold(
            widget::Row::new().spacing(2).height(Length::Fixed(28.0)).align_y(Alignment::End),
            |row, level| {
                row.push(
                    container(widget::space())
                        .width(Length::Fixed(5.0))
                        .height(Length::Fixed(2.0 + 26.0 * level.clamp(0.0, 1.0)))
                        .class(theme::Container::custom(|theme| container::Style {
                            background: Some(Color::from(theme.cosmic().accent_color()).into()),
                            ..Default::default()
                        })),
                )
            },
        );
        let (used, total) = self.sample.map_or((0, 1), |s| s.memory);
        widget::Column::new()
            .spacing(6)
            .push(line("CPU", format!("{:.0}%", self.load.cpu * 100.0)))
            .push(container(history).align_right(Length::Fill).clip(true))
            .push(line("RAM", system::gigabytes((used, total))))
            .push(widget::determinate_linear(used as f32 / total.max(1) as f32).width(Length::Fill))
            .push(
                widget::Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("go-down-symbolic").size(16))
                    .push(text::caption(system::rate(self.load.down)).width(Length::Fill))
                    .push(icon::from_name("go-up-symbolic").size(16))
                    .push(text::caption(system::rate(self.load.up))),
            )
            .into()
    }

    /// The open to-dos, soonest first: ticked off with a click.
    fn tasks_view(&self) -> Element<'_, Message> {
        let tasks = self.tasks.as_deref().unwrap_or_default();
        let mut column = widget::Column::new().spacing(8).push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(icon::from_name("checkbox-checked-symbolic").size(18))
                .push(text::heading(T.section_tasks))
                .push(text::caption(if tasks.is_empty() { String::new() } else { tasks.len().to_string() })),
        );
        if tasks.is_empty() {
            column = column.push(text::caption(T.no_tasks));
        }
        let today = self.now.date();
        for task in tasks.iter().take(8) {
            let mut label = widget::Column::new().push(text::body(task.summary.as_str()));
            if let Some(due) = task.due {
                let due_text = text::caption(i18n::short_date(due));
                label = label.push(if due < today { due_text.class(theme::Text::Accent) } else { due_text });
            }
            column = column.push(
                widget::Row::new()
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(widget::checkbox(false).on_toggle({
                        let uid = task.uid.clone();
                        move |_| Message::CompleteTask(uid.clone())
                    }))
                    .push(label.width(Length::Fill)),
            );
        }
        column.into()
    }

    /// Other places' time, then timers: started with a click, counted down.
    fn clocks_view(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(8);
        let here = self.now.offset().seconds();
        for zone in &self.prefs.clocks {
            let Ok(tz) = jiff::tz::TimeZone::get(zone) else { continue };
            let there = self.now.with_time_zone(tz);
            let city = zone.rsplit('/').next().unwrap_or(zone).replace('_', " ");
            let apart = there.offset().seconds() - here;
            let mut note = match (apart / 3600, (apart % 3600).abs() / 60) {
                (h, 0) => format!("{h:+} h"),
                (h, m) => format!("{}{}:{m:02}", if apart < 0 { "-" } else { "+" }, h.abs()),
            };
            if there.date() != self.now.date() {
                note = format!("{} · {note}", i18n::weekday_short(there.date()));
            }
            column = column.push(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(widget::Column::new().push(text::body(city)).push(text::caption(note)).width(Length::Fill))
                    .push(text::title4(i18n::clock(there.time()))),
            );
        }
        if !self.prefs.clocks.is_empty() {
            column = column.push(widget::divider::horizontal::light());
        }
        let presets = [5u64, 10, 15, 30].into_iter().fold(
            widget::Row::new().spacing(4).align_y(Alignment::Center).push(icon::from_name("alarm-symbolic").size(18)),
            |row, minutes| row.push(button::text(format!("{minutes} min")).on_press(Message::StartTimer(minutes))),
        );
        // Any other length, in minutes.
        let minutes = self.timer_input.trim().parse::<u64>().ok().filter(|m| (1..=24 * 60).contains(m));
        let custom = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                widget::text_input("", self.timer_input.as_str())
                    .on_input(Message::TimerInput)
                    .on_submit_maybe(minutes.map(|m| move |_| Message::StartTimer(m)))
                    .width(Length::Fixed(90.0)),
            )
            .push(text::caption("min").width(Length::Fill))
            .push(button::icon(icon::from_name("media-playback-start-symbolic")).padding(6).on_press_maybe(minutes.map(Message::StartTimer)));
        column = column.push(presets).push(custom);
        let now = Instant::now();
        for (ends, length) in &self.timers {
            let left = ends.saturating_duration_since(now);
            column = column.push(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(text::body(pomodoro::clock(left)))
                    .push(widget::determinate_linear(1.0 - left.as_secs_f32() / length.as_secs_f32().max(1.0)).width(Length::Fill))
                    .push(button::icon(icon::from_name("window-close-symbolic")).padding(4).on_press(Message::StopTimer(*ends))),
            );
        }
        column.into()
    }

    /// Each app's volume, folded away unless opened (remembered).
    fn mixer(&self) -> Element<'_, Message> {
        let chevron = if self.prefs.mixer_open { "go-up-symbolic" } else { "go-down-symbolic" };
        let head = button::custom(
            widget::Row::new()
                .align_y(Alignment::Center)
                .push(text::heading(T.applications).width(Length::Fill))
                .push(icon::from_name(chevron).size(16)),
        )
        .class(style::flat())
        .padding([10, 10])
        .width(Length::Fill)
        .on_press(Message::ToggleMixer);
        let column = widget::Column::new().spacing(6).push(head);
        if !self.prefs.mixer_open {
            return column.into();
        }
        column.push(self.streams()).into()
    }

    /// Each playing app with its volume.
    fn streams(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(6);
        if self.audio.streams.is_empty() {
            column = column.push(container(text::caption(T.no_apps_playing)).padding([0, 10]));
        }
        for stream in &self.audio.streams {
            let level = match self.stream_drag {
                Some((index, level)) if index == stream.index => level,
                _ => stream.volume,
            };
            let mute = if stream.muted { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" };
            let index = stream.index;
            column = column.push(
                widget::Column::new()
                    .padding([0, 10])
                    .push(
                        widget::Row::new()
                            .spacing(8)
                            .align_y(Alignment::Center)
                            .push(icon::from_name(stream.icon.as_str()).size(18))
                            .push(text::caption(stream.app.as_str())),
                    )
                    .push(
                        widget::Row::new()
                            .spacing(8)
                            .align_y(Alignment::Center)
                            .push(button::icon(icon::from_name(mute)).padding(4).on_press(Message::StreamMute(index)))
                            .push(
                                widget::slider(0.0..=1.0, level, move |v| Message::StreamVolume(index, v))
                                    .step(0.01_f32)
                                    .on_release(Message::StreamRelease),
                            ),
                    ),
            );
        }
        column.into()
    }

    fn edit_page(&self) -> Element<'_, Message> {
        let mut header = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(button::icon(icon::from_name("go-previous-symbolic")).padding(8).on_press(Message::Page(Page::Main)))
            .push(text::title4(T.edit).width(Length::Fill));
        if self.edit_tab == EditTab::Tiles {
            header = header.push(widget::tooltip(
                button::icon(icon::from_name("edit-undo-symbolic")).padding(8).on_press(Message::ResetTiles),
                text::caption(T.reset),
                widget::tooltip::Position::Bottom,
            ));
        }
        // Tabs: the picked one in the accent, underlined.
        let mut tabs = widget::Row::new();
        for (tab, name) in
            [(EditTab::General, T.general), (EditTab::Tiles, T.tiles), (EditTab::Cards, T.cards), (EditTab::About, T.about)]
        {
            let picked = tab == self.edit_tab;
            let label = button::custom(
                container(text::body(name).wrapping(iced::widget::text::Wrapping::None)).center_x(Length::Fill).clip(true),
            )
                .class(style::flat())
                .padding([8, 4])
                .width(Length::Fill)
                .on_press(Message::EditTab(tab));
            let line = container(widget::space())
                .width(Length::Fill)
                .height(Length::Fixed(2.0))
                .class(theme::Container::custom(move |theme| container::Style {
                    background: picked.then(|| Color::from(theme.cosmic().accent_color()).into()),
                    ..Default::default()
                }));
            let label: Element<'_, Message> = if picked {
                container(label).class(theme::Container::custom(|theme| container::Style {
                    text_color: Some(theme.cosmic().accent_text_color().into()),
                    ..Default::default()
                }))
                .into()
            } else {
                label.into()
            };
            tabs = tabs.push(widget::Column::new().width(Length::Fill).push(label).push(line));
        }
        widget::Column::new()
            .spacing(12)
            .push(header)
            .push(tabs)
            .push(match self.edit_tab {
                EditTab::General => self.edit_general(),
                EditTab::Tiles => self.edit_tiles(),
                EditTab::Cards => self.edit_cards(),
                EditTab::About => self.edit_about(),
            })
            .into()
    }

    /// How the popup opens and sits, the cards in it, the tiles' look, and
    /// what the panel shows.
    fn edit_general(&self) -> Element<'_, Message> {
        use crate::config::{TileStyle, WifiBadge};
        let opens = widget::Column::new()
            .spacing(10)
            .push(option(T.dropdown, T.dropdown_hint, Layout::Dropdown, self.prefs.layout, Message::SetLayout))
            .push(option(T.side, T.side_hint, Layout::Side, self.prefs.layout, Message::SetLayout))
            .push(option(T.fullscreen, T.fullscreen_hint, Layout::Fullscreen, self.prefs.layout, Message::SetLayout));
        let mut column_count = widget::Row::new().spacing(4);
        for n in 1..=3u8 {
            column_count = column_count.push(choice(n.to_string(), n as usize == self.prefs.columns(), Message::SetColumns(n)));
        }
        let looks = widget::Column::new()
            .spacing(10)
            .push(option(T.style_high, T.style_high_hint, TileStyle::High, self.prefs.tile_style, Message::TileStyle))
            .push(option(T.style_medium, T.style_medium_hint, TileStyle::Medium, self.prefs.tile_style, Message::TileStyle))
            .push(option(T.style_low, T.style_low_hint, TileStyle::Low, self.prefs.tile_style, Message::TileStyle));
        let mut icons = widget::Row::new().spacing(4);
        for (mode, name) in [(PanelIcons::None, T.icons_none), (PanelIcons::Active, T.icons_active), (PanelIcons::All, T.icons_all)] {
            icons = icons.push(choice(name, mode == self.prefs.panel_icons, Message::PanelIcons(mode)));
        }
        let mut spacing = widget::Row::new().spacing(4);
        for (size, name) in
            [(Spacing::Small, T.small), (Spacing::Medium, T.medium), (Spacing::Large, T.large), (Spacing::Native, T.native)]
        {
            spacing = spacing.push(choice(name, size == self.prefs.spacing, Message::SetSpacing(size)));
        }
        let mut wifi = widget::Row::new().spacing(4);
        for (badge, name) in [
            (WifiBadge::Off, T.off),
            (WifiBadge::HalfMinute, "30 s"),
            (WifiBadge::Minute, "1 min"),
            (WifiBadge::FiveMinutes, "5 min"),
            (WifiBadge::Back, T.until_back),
        ] {
            wifi = wifi.push(choice(name, badge == self.prefs.wifi_badge, Message::WifiBadge(badge)));
        }
        widget::Column::new()
            .spacing(16)
            .push(group(T.layout, T.layout_hint, opens.into()))
            .push(group(
                T.columns,
                T.columns_hint,
                widget::Column::new()
                    .spacing(10)
                    .push(column_count)
                    .push(described(switch(T.islands, self.prefs.islands, Message::SetIslands), T.islands_hint))
                    .into(),
            ))
            .push(group(T.cards, T.cards_hint, self.edit_sections()))
            .push(group(T.tile_style, T.tile_style_hint, looks.into()))
            .push(group(
                T.panel,
                T.panel_hint,
                widget::Column::new()
                    .spacing(10)
                    .push(switch(T.show_clock, self.prefs.clock, Message::ShowClock))
                    .push(text::body(T.panel_icons))
                    .push(icons)
                    .push(text::body(T.panel_spacing))
                    .push(spacing)
                    .into(),
            ))
            .push(group(
                T.badges,
                T.badges_hint,
                widget::Column::new()
                    .spacing(10)
                    .push(described(switch(T.low_battery, self.prefs.low_battery_badge, Message::LowBatteryBadge), T.low_battery_hint))
                    .push(described(text::body(T.wifi_dropped).into(), T.wifi_dropped_hint))
                    .push(wifi)
                    .into(),
            ))
            .into()
    }

    /// Who made Orbit, and where to find more.
    fn edit_about(&self) -> Element<'_, Message> {
        const HOME: &str = "https://github.com/danielhanzs/cosmic-ext-applet-orbit";
        let link = |label: &'static str, url: String| {
            button::custom(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(text::body(label).width(Length::Fill))
                    .push(icon::from_name("go-next-symbolic").size(16)),
            )
            .class(style::flat())
            .padding([8, 10])
            .width(Length::Fill)
            .on_press(Message::OpenUrl(url))
        };
        widget::Column::new()
            .spacing(12)
            .push(
                widget::Column::new()
                    .spacing(4)
                    .push(text::title3("Orbit"))
                    .push(text::caption(format!("{} {}", T.version, env!("CARGO_PKG_VERSION"))))
                    .push(text::body(T.about_text)),
            )
            .push(
                widget::Row::new()
                    .push(text::body(T.developer).width(Length::Fill))
                    .push(text::body("Dániel Hánzs")),
            )
            .push(widget::Row::new().push(text::body(T.license).width(Length::Fill)).push(text::body("GPL-3.0")))
            .push(widget::divider::horizontal::light())
            .push(link(T.website, HOME.to_string()))
            .push(link(T.report_problem, format!("{HOME}/issues")))
            .push(link(T.help_translate, format!("{HOME}/tree/main/i18n")))
            .push(text::caption(T.trademark))
            .into()
    }

    /// The editor's tiles: the shown ones as they lay out (drag to move,
    /// click to resize, − to hide), and the others to add.
    fn edit_tiles(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(10);

        // The shown tiles, as they lay out: drag to move, click to resize.
        let mut grid = widget::Column::new().spacing(8);
        for row in rows(&self.prefs.tiles, &self.prefs.small) {
            let mut line = widget::Row::new().spacing(8);
            let mut used = 0;
            for (i, tile) in row {
                let small = self.prefs.small.contains(&tile);
                let width = if small { 1 } else { 2 };
                used += width;
                let mut inner = widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::icon(self.tile(tile).icon).size(18));
                if !small {
                    inner = inner.push(
                        widget::container(text::body(self.tile_name(tile)).wrapping(iced::widget::text::Wrapping::None))
                            .width(Length::Fill)
                            .clip(true),
                    );
                } else {
                    inner = inner.push(widget::space().width(Length::Fill));
                }
                inner = inner.push(
                    button::icon(icon::from_name("list-remove-symbolic"))
                        .class(style::picked())
                        .padding(2)
                        .on_press(Message::ShowTile(tile, false)),
                );
                let dragged = self.drag == Some(i);
                let target = self.drag.is_some() && self.drag_over == Some(i) && !dragged;
                let cell = container(inner)
                    .padding([10, 10])
                    .height(Length::Fixed(52.0))
                    .width(Length::Fill)
                    .class(theme::Container::custom(move |theme| {
                        let cosmic = theme.cosmic();
                        container::Style {
                            background: Some(Color::from(cosmic.button.base).scale_alpha(if dragged { 0.5 } else { 1.0 }).into()),
                            text_color: Some(cosmic.button.on.into()),
                            icon_color: Some(cosmic.button.on.into()),
                            border: iced::Border {
                                color: if target { cosmic.accent_color().into() } else { Color::TRANSPARENT },
                                width: 2.0,
                                radius: cosmic.corner_radii.radius_m.into(),
                            },
                            ..Default::default()
                        }
                    }));
                line = line.push(
                    container(
                        widget::mouse_area(cell)
                            .interaction(iced::mouse::Interaction::Grab)
                            .on_press(Message::DragStart(i))
                            .on_enter(Message::DragOver(i))
                            .on_release(Message::DragEnd),
                    )
                    .width(Length::FillPortion(width)),
                );
            }
            if used < 4 {
                line = line.push(widget::space().width(Length::FillPortion(4 - used)));
            }
            grid = grid.push(line);
        }
        column = column.push(
            container(grid).padding(8).width(Length::Fill).class(theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
                container::Style {
                    border: iced::Border {
                        color: cosmic.accent_color().into(),
                        width: 1.0,
                        radius: cosmic.corner_radii.radius_m.into(),
                    },
                    ..Default::default()
                }
            })),
        );

        // The others, to add.
        let hidden: Vec<Tile> = Tile::ALL
            .into_iter()
            .chain(self.prefs.custom.iter().map(|c| Tile::Custom(c.id)))
            .filter(|t| !self.prefs.tiles.contains(t) && self.available(*t))
            .collect();
        if !hidden.is_empty() {
            column = column.push(text::heading(T.add));
            for four in hidden.chunks(4) {
                let mut line = widget::Row::new().spacing(8);
                for tile in four {
                    let cell = widget::Column::new()
                        .spacing(4)
                        .align_x(Alignment::Center)
                        .push(
                            button::custom(
                                widget::Row::new()
                                    .spacing(4)
                                    .align_y(Alignment::Center)
                                    .push(icon::icon(self.tile(*tile).icon).size(18))
                                    .push(icon::from_name("list-add-symbolic").size(12)),
                            )
                            .class(style::tile(false))
                            .padding([12, 14])
                            .on_press(Message::ShowTile(*tile, true)),
                        )
                        .push(text::caption(self.tile_name(*tile)).align_x(iced::alignment::Horizontal::Center));
                    line = line.push(widget::container(cell).center_x(Length::FillPortion(1)));
                }
                for _ in four.len()..4 {
                    line = line.push(widget::space().width(Length::FillPortion(1)));
                }
                column = column.push(line);
            }
        }

        widget::Column::new()
            .spacing(16)
            .push(group(T.tiles, T.edit_hint, column.into()))
            .push(group(T.your_tiles, T.your_tiles_hint, self.edit_custom()))
            .into()
    }

    /// The user's own tiles: each to change, a new one, launchers to add.
    fn edit_custom(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(8);
        if let Some(form) = &self.custom_form {
            let field = |label: &'static str, hint: &'static str, value: &str, which: CustomField| {
                widget::Column::new()
                    .spacing(4)
                    .push(text::body(label))
                    .push(widget::text_input(hint, value.to_string()).on_input(move |v| Message::CustomForm(which, v)))
            };
            let ready = !form.name.trim().is_empty() && !form.command.trim().is_empty();
            let mut buttons = widget::Row::new().spacing(8).push(widget::space().width(Length::Fill));
            if form.id != 0 {
                buttons = buttons.push(button::destructive(T.delete).on_press(Message::DeleteCustom));
            }
            buttons = buttons
                .push(button::standard(T.cancel).on_press(Message::CancelCustom))
                .push(button::suggested(T.save).on_press_maybe(ready.then_some(Message::SaveCustom)));
            return column
                .push(field(T.name, "", &form.name, |c| &mut c.name))
                .push(field(T.command, "cosmic-term", &form.command, |c| &mut c.command))
                .push(field(T.icon_name, "application-x-executable-symbolic", &form.icon, |c| &mut c.icon))
                .push(described(
                    field(T.status_command, "systemctl --user is-active -q syncthing", &form.status, |c| &mut c.status).into(),
                    T.status_command_hint,
                ))
                .push(switch(T.ask_first, form.confirm, Message::CustomConfirm))
                .push(buttons)
                .into();
        }
        for c in &self.prefs.custom {
            let icon = if c.icon.trim().is_empty() { "application-x-executable-symbolic" } else { c.icon.trim() };
            column = column.push(
                button::custom(
                    widget::Row::new()
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .push(icon::from_name(icon).size(18))
                        .push(
                            widget::Column::new()
                                .push(text::body(c.name.as_str()))
                                .push(text::caption(c.command.as_str()).wrapping(iced::widget::text::Wrapping::None))
                                .width(Length::Fill),
                        )
                        .push(icon::from_name("edit-symbolic").size(16)),
                )
                .class(style::flat())
                .padding([6, 8])
                .width(Length::Fill)
                .on_press(Message::EditCustom(Some(c.id))),
            );
        }
        column = column.push(button::standard(T.new_tile).leading_icon(icon::from_name("list-add-symbolic")).on_press(Message::EditCustom(None)));
        // Installed launchers not yet a tile.
        let offered: Vec<Suggestion> = self
            .suggestions
            .iter()
            .copied()
            .filter(|s| !self.prefs.custom.iter().any(|c| c.command == s.command))
            .collect();
        if !offered.is_empty() {
            column = column.push(text::body(T.suggestions));
            let row = offered.into_iter().fold(widget::Row::new().spacing(6), |row, s| {
                row.push(
                    button::custom(
                        widget::Row::new()
                            .spacing(6)
                            .align_y(Alignment::Center)
                            .push(icon::from_name(s.icon).size(16))
                            .push(text::caption(s.name)),
                    )
                    .class(style::tile(false))
                    .padding([6, 10])
                    .on_press(Message::AddSuggestion(s)),
                )
            });
            column = column.push(widget::scrollable::horizontal(row.padding([0, 0, 8, 0])));
        }
        column.into()
    }

    /// The popup's cards: shown or not, in which column and order.
    fn edit_sections(&self) -> Element<'_, Message> {
        // The popup's parts: shown or not, moved up and down.
        let mut sections = widget::Column::new().spacing(2);
        let columns = self.prefs.columns();
        let mut order: Vec<usize> = (0..self.prefs.sections.len()).collect();
        order.sort_by_key(|&i| self.prefs.column(self.prefs.sections[i]));
        for i in order {
            let section = self.prefs.sections[i];
            let column = self.prefs.column(section);
            let mut row = widget::Row::new()
                .spacing(4)
                .align_y(Alignment::Center)
                .push(text::body(section_name(section)).width(Length::Fill));
            if columns > 1 {
                row = row
                    .push(button::icon(icon::from_name("go-previous-symbolic")).on_press_maybe((column > 0).then_some(Message::SectionColumn(section, false))))
                    .push(text::caption((column + 1).to_string()))
                    .push(button::icon(icon::from_name("go-next-symbolic")).on_press_maybe((column + 1 < columns).then_some(Message::SectionColumn(section, true))));
            }
            sections = sections.push(
                row.push(button::icon(icon::from_name("go-up-symbolic")).on_press_maybe(self.neighbor(i, true).map(|_| Message::MoveSection(i, true))))
                    .push(button::icon(icon::from_name("go-down-symbolic")).on_press_maybe(self.neighbor(i, false).map(|_| Message::MoveSection(i, false))))
                    .push(widget::toggler(true).on_toggle_maybe((section != Section::Status).then_some(move |on| Message::ShowSection(section, on)))),
            );
        }
        for section in Section::ALL.iter().filter(|s| !self.prefs.sections.contains(s)) {
            sections = sections.push(
                widget::Row::new()
                    .align_y(Alignment::Center)
                    .push(text::body(section_name(*section)).width(Length::Fill))
                    .push(widget::toggler(false).on_toggle(|on| Message::ShowSection(*section, on))),
            );
        }
        sections.into()
    }

    /// Each card's own settings.
    fn edit_cards(&self) -> Element<'_, Message> {
        let city = widget::text_input(T.city_auto, self.city_input.as_str())
            .on_input(Message::City)
            .on_submit(|_| Message::CitySubmit);
        let count = |options: &[u8], now: u8, message: fn(u8) -> Message| {
            options.iter().fold(widget::Row::new().spacing(4), |row, &n| {
                row.push(choice(n.to_string(), n == now, message(n)))
            })
        };
        let mut weather = widget::Column::new()
            .spacing(10)
            .push(widget::Row::new().spacing(8).align_y(Alignment::Center).push(text::body(T.city)).push(city))
            .push(self.places.iter().fold(widget::Column::new(), |column, place| {
                column.push(
                    button::custom(text::body(place.label()))
                        .class(style::flat())
                        .padding([6, 10])
                        .width(Length::Fill)
                        .on_press(Message::PickPlace(place.clone())),
                )
            }))
            .push(switch(T.show_on_panel, self.prefs.weather_panel, Message::WeatherPanel))
            .push(switch(T.hourly, self.prefs.weather_hourly, Message::WeatherHourly));
        if self.prefs.weather_hourly {
            weather = weather.push(text::body(T.hours)).push(count(&[6, 12, 24], self.prefs.weather_hours, Message::WeatherHours));
        }
        weather = weather.push(text::body(T.days)).push(count(&[3, 5, 7], self.prefs.weather_days, Message::WeatherDays));
        let mut clocks = widget::Column::new().spacing(10);
        for (i, zone) in self.prefs.clocks.iter().enumerate() {
            clocks = clocks.push(
                widget::Row::new()
                    .align_y(Alignment::Center)
                    .push(text::body(zone.replace('_', " ")).width(Length::Fill))
                    .push(button::icon(icon::from_name("list-remove-symbolic")).on_press(Message::RemoveClock(i))),
            );
        }
        clocks = clocks
            .push(widget::text_input(T.add_clock, self.clock_input.as_str()).on_input(Message::ClockInput))
            .push(self.clock_matches.iter().fold(widget::Column::new(), |column, zone| {
                column.push(
                    button::custom(text::body(zone.replace('_', " ")))
                        .class(style::flat())
                        .padding([6, 10])
                        .width(Length::Fill)
                        .on_press(Message::AddClock(zone.clone())),
                )
            }));
        widget::Column::new()
            .spacing(16)
            .push(group(T.weather, T.weather_hint, weather.into()))
            .push(group(T.section_clocks, T.clocks_hint, clocks.into()))
            .into()
    }

    fn password_page(&self) -> Element<'_, Message> {
        let Some((request, value)) = &self.password else {
            return text::body("").into();
        };
        widget::Column::new()
            .spacing(12)
            .push(text::title4(format!("{} {}", T.password_for, request.connection)))
            .push(
                widget::secure_input(T.password, value.as_str(), None, true)
                    .on_input(Message::PasswordInput)
                    .on_submit(|_| Message::PasswordSubmit),
            )
            .push(
                widget::Row::new()
                    .align_y(Alignment::Center)
                    .push(text::body(T.remember).width(Length::Fill))
                    .push(widget::toggler(self.remember).on_toggle(Message::Remember)),
            )
            .push(
                widget::Row::new()
                    .spacing(8)
                    .push(widget::space().width(Length::Fill))
                    .push(button::standard(T.cancel).on_press(Message::PasswordCancel))
                    .push(button::suggested(T.connect).on_press(Message::PasswordSubmit)),
            )
            .into()
    }
}

/// The popup's own surface, for the layouts that draw it themselves.
fn panel<'a>(body: Element<'a, Message>, width: f32) -> widget::Container<'a, Message, cosmic::Theme> {
    container(body)
        .width(Length::Fixed(width))
        .class(theme::Container::custom(|theme| {
            let cosmic = theme.cosmic();
            let bg = cosmic.background(theme.transparent);
            container::Style {
                background: Some(Color::from(bg.base).into()),
                text_color: Some(bg.on.into()),
                icon_color: Some(bg.on.into()),
                border: iced::Border {
                    radius: cosmic.corner_radii.radius_m.into(),
                    width: 1.0,
                    color: bg.divider.into(),
                },
                ..Default::default()
            }
        }))
}

/// The mark of a day with events: the accent, or what reads on it.
fn dot<'a>(on_accent: bool) -> Element<'a, Message> {
    container(widget::space())
        .width(5)
        .height(5)
        .class(theme::Container::custom(move |theme| {
            let cosmic = theme.cosmic();
            let color = if on_accent { cosmic.accent_button.on } else { cosmic.accent_color() };
            container::Style {
                background: Some(Color::from(color).into()),
                border: iced::Border { radius: 3.0.into(), ..Default::default() },
                ..Default::default()
            }
        }))
        .into()
}

/// A preview's popup on a solid background, in a magenta frame that marks
/// it out in a screenshot of the whole screen.
fn preview_frame(body: Element<'_, Message>) -> Element<'_, Message> {
    let solid = container(body).class(theme::Container::custom(|theme| container::Style {
        background: Some(Color::from(theme.cosmic().background(false).base).into()),
        ..Default::default()
    }));
    container(solid)
        .padding(4)
        .class(theme::Container::custom(|_| container::Style {
            background: Some(Color::from_rgb(1.0, 0.0, 1.0).into()),
            ..Default::default()
        }))
        .into()
}

/// ORBIT_PREVIEW's options, when set.
fn preview() -> Option<String> {
    std::env::var("ORBIT_PREVIEW").ok().filter(|v| !v.is_empty())
}

/// A section on its own surface (islands), or as part of the one.
fn shape<'a>((element, look): (Element<'a, Message>, Look), islands: bool) -> Element<'a, Message> {
    match (look, islands) {
        (Look::Own, _) => element,
        (Look::Padded, true) => panel(container(element).padding(14).into(), WIDTH).into(),
        (Look::Padded, false) => element,
        (Look::Flush, true) => panel(element, WIDTH).padding(0).into(),
        (Look::Flush, false) => container(element).width(Length::Fill).class(theme::Container::Card).into(),
    }
}

/// Sections one under the other: apart as islands, or padded on the one surface.
fn stack<'a>(parts: Vec<Element<'a, Message>>, islands: bool) -> Element<'a, Message> {
    let column = parts.into_iter().fold(widget::Column::new().spacing(if islands { 8 } else { 14 }), |c, p| c.push(p));
    if islands { column.into() } else { container(column).padding(14).into() }
}

fn volume_icon(muted: bool, level: f32) -> &'static str {
    match (muted, level) {
        (true, _) => "audio-volume-muted-symbolic",
        (_, v) if v < 0.34 => "audio-volume-low-symbolic",
        (_, v) if v < 0.67 => "audio-volume-medium-symbolic",
        _ => "audio-volume-high-symbolic",
    }
}

/// What's read when the popup opens, all at once.
async fn extras() -> Message {
    let (dns, charge, light, gamemode) = tokio::join!(sys::dns(), sys::charge_limit(), sys::keyboard_light(), desktop::gamemode());
    Message::Extras(dns, charge, light, gamemode)
}

/// One of the editor's fields of a tile of the user's.
type CustomField = fn(&mut crate::config::Custom) -> &mut String;

/// A launcher offered as a tile, when it's installed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Suggestion {
    name: &'static str,
    command: &'static str,
    icon: &'static str,
}

impl Suggestion {
    fn all() -> [Suggestion; 5] {
        let s = |name, command, icon| Suggestion { name, command, icon };
        [
            s(T.terminal, "cosmic-term", "utilities-terminal-symbolic"),
            s(T.files, "cosmic-files", "system-file-manager-symbolic"),
            s(T.text_editor, "cosmic-edit", "accessories-text-editor-symbolic"),
            s(T.system_monitor, "observatory", "utilities-system-monitor-symbolic"),
            s(T.app_store, "cosmic-store", "system-software-install-symbolic"),
        ]
    }

    /// The ones installed on the host.
    async fn installed() -> Vec<Suggestion> {
        let mut found = Vec::new();
        for s in Self::all() {
            let script = format!("command -v {}", s.command);
            if crate::host::command("sh").args(["-c", &script]).output().await.is_ok_and(|o| o.status.success()) {
                found.push(s);
            }
        }
        found
    }
}

/// A slider's level beside it: "45%", in a fixed width so sliders line up.
fn percent(level: f32) -> Element<'static, Message> {
    text::caption(format!("{:.0}%", level * 100.0))
        .width(Length::Fixed(34.0))
        .align_x(iced::alignment::Horizontal::Right)
        .into()
}

/// A choice among a few: the picked one lit.
fn choice(label: impl Into<std::borrow::Cow<'static, str>>, picked: bool, message: Message) -> button::TextButton<'static, Message> {
    let b = button::text(label).on_press(message);
    if picked { b.class(style::picked()) } else { b }
}

/// A setting that's on or off, its name before it.
fn switch(label: &'static str, on: bool, message: fn(bool) -> Message) -> Element<'static, Message> {
    widget::Row::new()
        .align_y(Alignment::Center)
        .push(text::body(label).width(Length::Fill))
        .push(widget::toggler(on).on_toggle(message))
        .into()
}

/// The editor's pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTab {
    General,
    Tiles,
    Cards,
    About,
}

/// A group of settings: its title, a line saying what it's for, then it.
fn group<'a>(title: &'static str, description: &'static str, content: Element<'a, Message>) -> Element<'a, Message> {
    widget::Column::new()
        .spacing(10)
        .push(widget::divider::horizontal::light())
        .push(widget::Column::new().spacing(2).push(text::title4(title)).push(text::caption(description)))
        .push(content)
        .into()
}

/// A setting with a line under it saying what it does.
fn described<'a>(setting: Element<'a, Message>, description: &'static str) -> Element<'a, Message> {
    widget::Column::new().spacing(2).push(setting).push(text::caption(description)).into()
}

/// One of a few, by a radio button, with what it means under it.
fn option<V: Copy + Eq + 'static>(
    label: &'static str,
    description: &'static str,
    value: V,
    picked: V,
    message: fn(V) -> Message,
) -> Element<'static, Message> {
    widget::Column::new()
        .spacing(2)
        .push(widget::radio(label, value, Some(picked), message))
        .push(container(text::caption(description)).padding([0, 0, 0, 28]))
        .into()
}

/// How a section sits in the popup.
#[derive(Clone, Copy)]
enum Look {
    Padded,
    Flush,
    Own,
}

fn section_name(section: Section) -> &'static str {
    match section {
        Section::Status => T.section_status,
        Section::Controls => T.section_controls,
        Section::Calendar => T.calendar,
        Section::Weather => T.weather,
        Section::Notifications => T.notifications,
        Section::Island => T.section_island,
        Section::Audio => T.section_audio,
        Section::System => T.section_system,
        Section::Tasks => T.section_tasks,
        Section::Clocks => T.section_clocks,
    }
}

fn condition_name(condition: weather::Condition) -> &'static str {
    use weather::Condition::*;
    match condition {
        Clear => T.weather_clear,
        PartlyCloudy => T.weather_partly_cloudy,
        Cloudy => T.weather_cloudy,
        Fog => T.weather_fog,
        Drizzle => T.weather_drizzle,
        Rain => T.weather_rain,
        Snow => T.weather_snow,
        Storm => T.weather_storm,
    }
}

/// An icon from the icon theme.
fn named(name: &str) -> icon::Handle {
    icon::from_name(name.to_string()).symbolic(true).handle()
}

/// How a panel icon is drawn.
#[derive(Clone, Copy, PartialEq)]
enum Shade {
    /// The panel's own color.
    Plain,
    /// Faint: a function that's off.
    Faint,
    /// The theme's warning red: a low battery, a dropped Wi-Fi.
    Alert,
}

fn status_icon<'a>(handle: icon::Handle, size: u16, shade: Shade) -> Element<'a, Message> {
    icon::icon(handle)
        .size(size)
        .class(theme::Svg::Custom(std::rc::Rc::new(move |theme| iced::widget::svg::Style {
            color: Some(match shade {
                Shade::Alert => theme.cosmic().destructive_color().into(),
                _ => Color::from(theme.cosmic().background(theme.transparent).on).scale_alpha(if shade == Shade::Faint { 0.4 } else { 1.0 }),
            }),
        })))
        .into()
}

impl cosmic::Application for App {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = crate::APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let mut app = Self {
            core,
            prefs: Prefs::load(),
            popup: None,
            opened: Instant::now(),
            page: Page::Main,
            net: Network::default(),
            bt: Bluetooth::default(),
            battery: None,
            peripherals: Vec::new(),
            profiles: Profiles::default(),
            brightness: None,
            audio: Audio::default(),
            media: None,
            dnd: DND.get(),
            dark: DARK.get(),
            now: Zoned::now(),
            selected: Zoned::now().date(),
            visible: Zoned::now().date().first_of_month(),
            events: None,
            today: None,
            camera: Vec::new(),
            camera_checking: false,
            a11y: None,
            magnifier: None,
            screen_filter: None,
            screen_reader: None,
            reader: None,
            inhibit: None,
            inhibiting: false,
            workspaces: None,
            layouts: keyboard::layouts(),
            layout: 0,
            tiling: None,
            touchpad: desktop::touchpad(),
            drag: None,
            drag_over: None,
            pending: HashSet::new(),
            dirty: HashSet::new(),
            adjusting: None,
            waiting: [None; 4],
            writing: [false; 4],
            audio_pending: false,
            audio_dirty: false,
            events_read: None,
            password: None,
            remember: true,
            screen: (1920, 1080),
            hotspot_password: None,
            updates: 0,
            stream_drag: None,
            picking: None,
            edit_tab: EditTab::General,
            sample: None,
            load: system::Load::default(),
            cpu_history: std::collections::VecDeque::new(),
            sampling: false,
            tasks: None,
            completing: Vec::new(),
            timers: Vec::new(),
            clock_input: String::new(),
            clock_matches: Vec::new(),
            timer_input: String::new(),
            dns: None,
            charge_limit: None,
            keyboard_light: None,
            gamemode: None,
            gamemode_held: false,
            wifi_lost: None,
            wifi_was_up: false,
            custom_on: Vec::new(),
            custom_asked: None,
            custom_polling: false,
            custom_form: None,
            suggestions: Vec::new(),
            pomodoro: pomodoro::Pomodoro::default(),
            weather: None,
            weather_read: None,
            weather_asked: 0,
            city_input: String::new(),
            places: Vec::new(),
            city_search: 0,
            notifications: Vec::new(),
            keys: 0,
        };
        let mut tasks: Vec<_> = [
            Source::Network,
            Source::Bluetooth,
            Source::Battery,
            Source::Profiles,
            Source::Brightness,
            Source::Media,
        ]
        .into_iter()
        .map(|s| app.refresh(s))
        .collect();
        tasks.push(run(async { Message::Audio(audio::read().await) }));
        // Kept notifications: their icons are looked up in the apps' entries, off this thread.
        tasks.push(run(async {
            Message::NotificationsLoaded(tokio::task::spawn_blocking(notifications::load).await.unwrap_or_default())
        }));
        app.city_input = app.prefs.weather_place.as_ref().map(|p| p.name.clone()).unwrap_or_default();
        // ORBIT_PREVIEW=fullscreen,islands,calendar,weather: opens by itself, set
        // so for this run only (nothing is saved), for screenshots.
        if let Some(preview) = preview() {
            for option in preview.split(',') {
                match option {
                    "dropdown" => app.prefs.layout = Layout::Dropdown,
                    "side" => app.prefs.layout = Layout::Side,
                    "fullscreen" => app.prefs.layout = Layout::Fullscreen,
                    "islands" => app.prefs.islands = true,
                    "single" => app.prefs.islands = false,
                    "1" | "2" | "3" => app.prefs.columns = option.parse().ok(),
                    "medium" => app.prefs.tile_style = crate::config::TileStyle::Medium,
                    "low" => app.prefs.tile_style = crate::config::TileStyle::Low,
                    "audio" | "system" | "tasks" | "clocks" => {
                        let section = match option {
                            "audio" => Section::Audio,
                            "system" => Section::System,
                            "tasks" => Section::Tasks,
                            _ => Section::Clocks,
                        };
                        if !app.prefs.sections.contains(&section) {
                            app.prefs.sections.push(section);
                        }
                        if section == Section::Clocks && app.prefs.clocks.is_empty() {
                            app.prefs.clocks = vec!["America/New_York".into(), "Asia/Tokyo".into()];
                        }
                    }
                    "calendar" => app.prefs.calendar_open = true,
                    "weather" => app.prefs.weather_open = true,
                    "general" | "tiles" | "cards" => {
                        app.edit_tab = match option {
                            "general" => EditTab::General,
                            "tiles" => EditTab::Tiles,
                            _ => EditTab::Cards,
                        };
                        tasks.push(run(async {
                            tokio::time::sleep(Duration::from_secs(6)).await;
                            Message::Page(Page::Edit)
                        }));
                    }
                    _ => {}
                }
            }
            tasks.push(run(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Message::Toggle
            }));
        }
        tasks.push(run(async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Message::CheckWeather
        }));
        // Not at once: the session is still starting.
        tasks.push(run(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Message::CheckUpdates
        }));
        (app, Task::batch(tasks))
    }

    fn core(&self) -> &app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut app::Core {
        &mut self.core
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::Toggle => return if self.popup.is_some() { self.close() } else { self.open() },
            Message::Close => return self.close(),
            Message::Closed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    if self.page != Page::Password {
                        self.page = Page::Main;
                    }
                }
            }
            Message::Page(page) => {
                self.page = page;
                self.drag = None;
                self.drag_over = None;
                if page == Page::Hotspot {
                    if let Some(path) = self.net.hotspot.as_ref().map(|h| h.path.clone()) {
                        return run(async move { Message::HotspotPassword(sys::hotspot_password(path).await) });
                    }
                }
                if page == Page::Wifi {
                    if let Some(device) = self.net.wifi_device.clone() {
                        return fire(async move { _ = sys::scan_wifi(device).await });
                    }
                }
            }
            Message::Screen(name, size) => {
                if let Some(size) = size {
                    if name == self.core.applet.output_name || self.core.applet.output_name.is_empty() {
                        self.screen = size;
                    }
                }
            }
            Message::Tick => {
                let mut tasks = Vec::new();
                let now = Instant::now();
                for (_, length) in self.timers.iter().filter(|(ends, _)| *ends <= now) {
                    tasks.push(fire(desktop::notify(T.timer_done.to_string(), pomodoro::clock(*length))));
                }
                self.timers.retain(|(ends, _)| *ends > now);
                if let (Some(at), Some(lasts)) = (self.wifi_lost, self.prefs.wifi_badge.lasts()) {
                    if at.elapsed() >= lasts {
                        self.wifi_lost = None;
                    }
                }
                // The user's tiles' states: every few seconds while the popup
                // shows, now and then for the panel; one round at a time.
                let every = Duration::from_secs(if self.popup.is_some() { 3 } else { 30 });
                if !self.custom_polling && self.custom_asked.is_none_or(|t| t.elapsed() >= every) {
                    let checks: Vec<(u32, String)> = self
                        .prefs
                        .custom
                        .iter()
                        .filter(|c| !c.status.trim().is_empty() && self.prefs.tiles.contains(&Tile::Custom(c.id)))
                        .map(|c| (c.id, c.status.clone()))
                        .collect();
                    self.custom_asked = Some(Instant::now());
                    if !checks.is_empty() {
                        self.custom_polling = true;
                        tasks.push(run(async move {
                            let mut on = Vec::new();
                            for (id, status) in checks {
                                // One that hangs counts as off, and is ended.
                                let run = crate::host::command("sh").args(["-c", &status]).kill_on_drop(true).output();
                                let ok = tokio::time::timeout(Duration::from_secs(2), run)
                                    .await
                                    .is_ok_and(|out| out.is_ok_and(|o| o.status.success()));
                                if ok {
                                    on.push(id);
                                }
                            }
                            Message::CustomOn(on)
                        }));
                    }
                }
                // The machine's load every two seconds, while it shows.
                if self.popup.is_some()
                    && self.prefs.sections.contains(&Section::System)
                    && !self.sampling
                    && self.sample.is_none_or(|s| s.at.elapsed() >= Duration::from_millis(1900))
                {
                    self.sampling = true;
                    tasks.push(run(async { Message::System(system::sample().await) }));
                }
                tasks.push(self.tick());
                return Task::batch(tasks);
            }
            Message::Frame => {}
            Message::Changed(source) => return self.refresh(source),
            Message::AudioChanged => {
                // pactl reports in bursts (a slider drag sends dozens): read once.
                if self.audio_pending {
                    self.audio_dirty = true;
                } else {
                    self.audio_pending = true;
                    return run(async {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        Message::Audio(audio::read().await)
                    });
                }
            }
            Message::FlagsChanged => {
                self.dnd = DND.get();
                self.dark = DARK.get();
                // cosmic-comp's: the keyboard layouts, the touchpad.
                self.layouts = keyboard::layouts();
                self.touchpad = desktop::touchpad();
            }
            Message::Network(net) => {
                if let Some(net) = net {
                    self.net = net;
                }
                // Wi-Fi was up and is gone, though not turned off: mark it a while.
                let up = self.net.access_points.iter().any(|a| a.active);
                if up {
                    self.wifi_lost = None;
                } else if self.wifi_was_up && self.net.wifi_enabled && self.prefs.wifi_badge != crate::config::WifiBadge::Off {
                    self.wifi_lost = Some(Instant::now());
                }
                self.wifi_was_up = up;
                // A hotspot made from its page: show its password once it's there.
                if self.page == Page::Hotspot && self.hotspot_password.is_none() {
                    if let Some(path) = self.net.hotspot.as_ref().map(|h| h.path.clone()) {
                        return Task::batch([
                            self.settled(Source::Network),
                            run(async move { Message::HotspotPassword(sys::hotspot_password(path).await) }),
                        ]);
                    }
                }
                return self.settled(Source::Network);
            }
            Message::Bluetooth(bt) => {
                self.bt = bt.unwrap_or_default();
                return self.settled(Source::Bluetooth);
            }
            Message::Battery(battery, peripherals) => {
                self.battery = battery;
                self.peripherals = peripherals;
                return self.settled(Source::Battery);
            }
            Message::Profiles(profiles) => {
                self.profiles = profiles.unwrap_or_default();
                return self.settled(Source::Profiles);
            }
            Message::Brightness(level) => {
                if !self.adjusting() {
                    self.brightness = level;
                }
                return self.settled(Source::Brightness);
            }
            Message::Audio(audio) => {
                self.audio_pending = false;
                let (volume, mic_volume) = (self.audio.volume, self.audio.mic_volume);
                self.audio = audio;
                if self.adjusting() {
                    (self.audio.volume, self.audio.mic_volume) = (volume, mic_volume);
                }
                if std::mem::take(&mut self.audio_dirty) {
                    return self.update(Message::AudioChanged);
                }
            }
            Message::Media(media) => {
                self.media = media;
                return self.settled(Source::Media);
            }
            Message::Events(first, events) => {
                // Today's line keeps today's events, whatever month is shown.
                let today = self.now.date();
                if first == today.first_of_month() {
                    self.today = events.as_ref().map(|days| days.get(&today).cloned().unwrap_or_default());
                }
                if first == self.visible {
                    self.events = events;
                }
            }
            Message::ToggleCalendar => self.pref(|p| p.calendar_open = !p.calendar_open),
            Message::CheckCamera if self.camera_checking => {}
            Message::CheckCamera => {
                self.camera_checking = true;
                return run(async { Message::Camera(audio::camera().await) });
            }
            Message::Camera(apps) => {
                self.camera_checking = false;
                self.camera = apps;
            }
            Message::TileClick(tile) => return self.click(tile),
            Message::Volume(level) => {
                self.audio.volume = level;
                self.adjusting = Some(Instant::now());
                return self.write(Slider::Volume, Some(level));
            }
            Message::MicVolume(level) => {
                self.audio.mic_volume = level;
                self.adjusting = Some(Instant::now());
                return self.write(Slider::Mic, Some(level));
            }
            Message::Picking(inputs) => {
                self.picking = if self.picking == Some(inputs) { None } else { Some(inputs) };
            }
            Message::SetBrightness(level) => {
                self.brightness = Some(level);
                self.adjusting = Some(Instant::now());
                return self.write(Slider::Brightness, Some(level));
            }
            Message::Written(slider) => return self.write(slider, None),
            Message::Released => self.adjusting = None,
            Message::Mute => return fire(audio::toggle_mute(false)),
            Message::Output(name) => return fire(audio::set_output(name)),
            Message::Join(ap) => {
                if let Some(device) = self.net.wifi_device.clone() {
                    return fire(async move { log("wifi", sys::join_wifi(device, ap).await) });
                }
            }
            Message::Connect(path) => return fire(async move { log("connect", sys::connect(path).await) }),
            Message::Disconnect(path) => return fire(async move { log("disconnect", sys::disconnect(path).await) }),
            Message::SetWifi(on) => return fire(async move { log("wifi", sys::set_wifi(on).await) }),
            Message::SetHotspot(on) => {
                let hotspot = self.net.hotspot.clone();
                if !on {
                    if let Some(active) = hotspot.and_then(|h| h.active) {
                        return self.update(Message::Disconnect(active));
                    }
                } else if let Some(device) = self.net.wifi_device.clone() {
                    let saved = hotspot.map(|h| h.path);
                    return fire(async move { log("hotspot", sys::start_hotspot(device, saved).await) });
                }
            }
            Message::HotspotPassword(password) => self.hotspot_password = password,
            Message::Input(name) => return fire(audio::set_input(name)),
            Message::CheckWeather => {
                self.weather_read = Some(Instant::now());
                self.weather_asked += 1;
                let asked = self.weather_asked;
                let (place, language, fahrenheit) = (self.prefs.weather_place.clone(), i18n::language(), i18n::fahrenheit());
                return run(async move { Message::Weather(asked, weather::read(place, language, fahrenheit).await) });
            }
            Message::Weather(asked, weather) => {
                // Only the latest answer (an older one may be another place's);
                // a failed read keeps what was shown.
                if asked == self.weather_asked && (weather.is_some() || self.weather.is_none()) {
                    self.weather = weather;
                }
            }
            Message::City(city) => {
                self.city_input = city;
                // Searched once typing pauses.
                self.city_search += 1;
                let search = self.city_search;
                return run(async move {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    Message::SearchCity(search)
                });
            }
            Message::SearchCity(search) => {
                let query = self.city_input.trim().to_string();
                if search != self.city_search || query.chars().count() < 2 {
                    if query.is_empty() {
                        self.places.clear();
                    }
                    return Task::none();
                }
                let language = i18n::language();
                return run(async move { Message::Places(search, weather::search(query, language).await) });
            }
            Message::Places(search, places) => {
                if search == self.city_search {
                    self.places = places;
                }
            }
            Message::PickPlace(place) => {
                self.city_input = place.name.clone();
                self.places.clear();
                self.prefs.weather_place = Some(place);
                self.prefs.save();
                self.weather = None;
                return self.update(Message::CheckWeather);
            }
            Message::CitySubmit => {
                // Enter takes the first match; an empty field, the time zone's city.
                if self.city_input.trim().is_empty() {
                    self.places.clear();
                    self.prefs.weather_place = None;
                    self.prefs.save();
                    self.weather = None;
                    return self.update(Message::CheckWeather);
                }
                if let Some(place) = self.places.first().cloned() {
                    return self.update(Message::PickPlace(place));
                }
            }
            Message::WeatherPanel(on) => self.pref(|p| p.weather_panel = on),
            Message::ToggleWeather => self.pref(|p| p.weather_open = !p.weather_open),
            Message::Notification(event) => {
                match event {
                    notifications::Event::New(mut n) => {
                        // Only a live one with the id is replaced: kept ones' ids are old.
                        self.notifications.retain(|old| old.restored || old.id != n.id);
                        n.key = self.next_key();
                        self.notifications.insert(0, n);
                        self.notifications.truncate(50);
                    }
                    notifications::Event::Closed(id) => self.notifications.retain(|n| n.restored || n.id != id),
                    // Swiped off its popup: kept here, as phones keep them.
                    notifications::Event::Dismissed(id) => {
                        tracing::debug!("notification {id} swiped off its popup, kept");
                        return Task::none();
                    }
                }
                notifications::save(&self.notifications);
            }
            Message::NotificationOpen(key) => {
                let Some(n) = self.notifications.iter().find(|n| n.key == key).cloned() else { return Task::none() };
                self.notifications.retain(|n| n.key != key);
                notifications::save(&self.notifications);
                let close = self.close();
                // A kept one is the daemon's no more: nothing is sent for it.
                if n.restored {
                    return close;
                }
                if !n.actions.iter().any(|(action, _)| action == "default") {
                    return Task::batch([close, fire(notifications::dismiss(vec![n.id]))]);
                }
                return Task::batch([close, fire(notifications::invoke(n.id, "default".into()))]);
            }
            Message::NotificationDismiss(key) => {
                let live: Vec<u32> = self.notifications.iter().filter(|n| n.key == key && !n.restored).map(|n| n.id).collect();
                self.notifications.retain(|n| n.key != key);
                notifications::save(&self.notifications);
                return fire(notifications::dismiss(live));
            }
            Message::NotificationsClear => {
                let live = self.notifications.drain(..).filter(|n| !n.restored).map(|n| n.id).collect();
                notifications::save(&self.notifications);
                return fire(notifications::dismiss(live));
            }
            Message::NotificationsLoaded(mut kept) => {
                for n in &mut kept {
                    n.key = self.next_key();
                }
                // Live ones that came meanwhile stay first.
                self.notifications.extend(kept);
                self.notifications.truncate(50);
            }
            Message::ToggleNotifications => self.pref(|p| p.notifications_open = !p.notifications_open),
            Message::MoveSection(i, up) => {
                if let Some(j) = self.neighbor(i, up) {
                    self.prefs.sections.swap(i, j);
                    self.prefs.save();
                }
            }
            Message::EditTab(tab) => {
                self.edit_tab = tab;
                if tab == EditTab::Tiles && self.suggestions.is_empty() {
                    return run(async { Message::Suggestions(Suggestion::installed().await) });
                }
            }
            Message::System(sample) => {
                self.sampling = false;
                if let Some(sample) = sample {
                    if let Some(before) = &self.sample {
                        self.load = sample.since(before);
                        self.cpu_history.push_back(self.load.cpu);
                        if self.cpu_history.len() > 40 {
                            self.cpu_history.pop_front();
                        }
                    }
                    self.sample = Some(sample);
                }
            }
            Message::Tasks(mut tasks) => {
                // Ones being ticked off stay off, whatever a read in between says.
                if let Some(tasks) = tasks.as_mut() {
                    tasks.retain(|t| !self.completing.contains(&t.uid));
                }
                self.tasks = tasks;
            }
            Message::CompleteTask(uid) => {
                let Some(list) = self.tasks.as_mut() else { return Task::none() };
                let Some(i) = list.iter().position(|t| t.uid == uid) else { return Task::none() };
                let task = list.remove(i);
                self.completing.push(uid.clone());
                return run(async move {
                    tasks::complete(task).await;
                    Message::TaskDone(uid, tasks::open().await)
                });
            }
            Message::TaskDone(uid, tasks) => {
                self.completing.retain(|u| *u != uid);
                return self.update(Message::Tasks(tasks));
            }
            Message::Extras(dns, charge, light, gamemode) => {
                self.dns = dns;
                self.charge_limit = charge;
                if !self.adjusting() {
                    self.keyboard_light = light;
                }
                self.gamemode = gamemode;
            }
            Message::SetDns(choice) => {
                self.dns = Some(choice);
                return run(async move {
                    log("dns", sys::set_dns(choice).await);
                    extras().await
                });
            }
            Message::SetChargeLimit(on) => {
                if let Some((path, _)) = self.charge_limit.clone() {
                    self.charge_limit = Some((path.clone(), on));
                    return fire(async move { log("charge limit", sys::set_charge_limit(path, on).await) });
                }
            }
            Message::KeyboardLight(level) => {
                if let Some(light) = self.keyboard_light.as_mut() {
                    light.1 = (level * light.2 as f32).round() as u32;
                    self.adjusting = Some(Instant::now());
                    return self.write(Slider::KeyboardLight, Some(level));
                }
            }
            Message::GameModeHeld(held) => {
                self.gamemode_held = held;
                return run(extras());
            }
            Message::RunCustom(id) => {
                if let Some(c) = self.prefs.custom.iter().find(|c| c.id == id) {
                    crate::host::spawn("sh", &["-c", c.command.as_str()]);
                    // Its state, soon after.
                    self.custom_asked = None;
                }
                return self.close();
            }
            Message::CustomOn(on) => {
                self.custom_polling = false;
                self.custom_on = on;
            }
            Message::EditCustom(id) => {
                self.custom_form = Some(match id.and_then(|id| self.prefs.custom.iter().find(|c| c.id == id)) {
                    Some(c) => c.clone(),
                    None => crate::config::Custom::default(),
                });
            }
            Message::CustomForm(field, value) => {
                if let Some(form) = self.custom_form.as_mut() {
                    *field(form) = value;
                }
            }
            Message::CustomConfirm(on) => {
                if let Some(form) = self.custom_form.as_mut() {
                    form.confirm = on;
                }
            }
            Message::SaveCustom => {
                if let Some(mut form) = self.custom_form.take().filter(|f| !f.name.trim().is_empty() && !f.command.trim().is_empty()) {
                    match self.prefs.custom.iter_mut().find(|c| c.id == form.id && form.id != 0) {
                        Some(c) => *c = form,
                        None => {
                            form.id = self.prefs.custom.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                            self.prefs.tiles.push(Tile::Custom(form.id));
                            self.prefs.custom.push(form);
                        }
                    }
                    self.prefs.save_custom();
                    self.prefs.save();
                }
            }
            Message::DeleteCustom => {
                if let Some(form) = self.custom_form.take() {
                    self.prefs.custom.retain(|c| c.id != form.id);
                    self.prefs.tiles.retain(|t| *t != Tile::Custom(form.id));
                    self.prefs.small.retain(|t| *t != Tile::Custom(form.id));
                    self.custom_on.retain(|id| *id != form.id);
                    self.prefs.save_custom();
                    self.prefs.save();
                }
            }
            Message::CancelCustom => self.custom_form = None,
            Message::Suggestions(found) => self.suggestions = found,
            Message::AddSuggestion(s) => {
                let id = self.prefs.custom.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                self.prefs.custom.push(crate::config::Custom {
                    id,
                    name: s.name.to_string(),
                    command: s.command.to_string(),
                    icon: s.icon.to_string(),
                    ..Default::default()
                });
                self.prefs.tiles.push(Tile::Custom(id));
                self.prefs.save_custom();
                self.prefs.save();
            }
            Message::LowBatteryBadge(on) => self.pref(|p| p.low_battery_badge = on),
            Message::TileStyle(style) => self.pref(|p| p.tile_style = style),
            Message::WifiBadge(badge) => self.pref(|p| p.wifi_badge = badge),
            Message::TimerInput(input) => {
                // Digits only, a day at most.
                self.timer_input = input.chars().filter(char::is_ascii_digit).take(4).collect();
            }
            Message::StartTimer(minutes) => {
                self.timer_input.clear();
                let length = Duration::from_secs(minutes * 60);
                self.timers.push((Instant::now() + length, length));
            }
            Message::StopTimer(ends) => self.timers.retain(|(e, _)| *e != ends),
            Message::ClockInput(input) => {
                let wanted = input.trim().to_lowercase().replace(' ', "_");
                self.clock_matches = if wanted.len() < 2 {
                    Vec::new()
                } else {
                    jiff::tz::db()
                        .available()
                        .map(|name| name.as_str().to_string())
                        .filter(|name| name.contains('/') && name.to_lowercase().contains(&wanted))
                        .take(6)
                        .collect()
                };
                self.clock_input = input;
            }
            Message::AddClock(zone) => {
                if !self.prefs.clocks.contains(&zone) {
                    self.prefs.clocks.push(zone);
                    self.prefs.save();
                }
                self.clock_input.clear();
                self.clock_matches.clear();
            }
            Message::RemoveClock(i) => {
                if i < self.prefs.clocks.len() {
                    self.prefs.clocks.remove(i);
                    self.prefs.save();
                }
            }
            Message::WeatherHourly(on) => self.pref(|p| p.weather_hourly = on),
            Message::WeatherHours(n) => self.pref(|p| p.weather_hours = n),
            Message::WeatherDays(n) => self.pref(|p| p.weather_days = n),
            Message::SetColumns(n) => self.pref(|p| p.columns = Some(n)),
            Message::SectionColumn(section, next) => {
                let column = self.prefs.column(section);
                let to = if next { column + 1 } else { column.wrapping_sub(1) };
                if to < self.prefs.columns() {
                    self.prefs.placement.retain(|(s, _)| *s != section);
                    self.prefs.placement.push((section, to as u8));
                    self.prefs.save();
                }
            }
            // Status holds the editor's button: hidden, there'd be no way back.
            Message::ShowSection(Section::Status, false) => {}
            Message::ShowSection(section, on) => {
                self.prefs.sections.retain(|s| *s != section);
                if on {
                    self.prefs.sections.push(section);
                }
                self.prefs.save();
            }
            Message::OpenUrl(url) => {
                crate::host::spawn("xdg-open", &[url.as_str()]);
                return self.close();
            }
            Message::PomodoroToggle => {
                let now = Instant::now();
                if self.pomodoro.running() { self.pomodoro.pause(now) } else { self.pomodoro.start(now) }
            }
            Message::PomodoroSkip => self.pomodoro.skip(Instant::now()),
            Message::PomodoroReset => self.pomodoro.reset(),
            Message::PomodoroPanel(on) => self.pref(|p| p.pomodoro_panel = on),
            // A drag only moves the slider; the level is set once, on release.
            Message::StreamVolume(index, level) => self.stream_drag = Some((index, level)),
            Message::StreamRelease => {
                if let Some((index, level)) = self.stream_drag {
                    if let Some(stream) = self.audio.streams.iter_mut().find(|s| s.index == index) {
                        stream.volume = level;
                    }
                    self.stream_drag = None;
                    return fire(audio::set_stream_volume(index, level));
                }
            }
            Message::StreamMute(index) => return fire(audio::toggle_stream_mute(index)),
            Message::ToggleMixer => self.pref(|p| p.mixer_open = !p.mixer_open),
            Message::CheckUpdates => return run(async { Message::Updates(updates::count().await) }),
            Message::Updates(count) => self.updates = count,
            Message::SetBluetooth(on) => {
                if let Some(adapter) = self.bt.adapter.clone() {
                    return fire(async move { log("bluetooth", sys::set_bluetooth(adapter, on).await) });
                }
            }
            Message::BtConnect(device, connect) => {
                return fire(async move { log("bluetooth", sys::bt_connect(device, connect).await) });
            }
            Message::Profile(profile) => {
                self.profiles.active = profile.clone();
                return fire(async move { log("power profile", sys::set_profile(profile).await) });
            }
            Message::MediaControl(method) => {
                if let Some(media) = &self.media {
                    return fire(media::control(media.bus.clone(), method));
                }
            }
            Message::Day(date) => self.selected = date,
            Message::PrevMonth | Message::NextMonth => {
                let step = jiff::Span::new().months(if matches!(message, Message::PrevMonth) { -1 } else { 1 });
                self.visible = self.visible.saturating_add(step);
                return self.load_events();
            }
            Message::Secret(agent::Event::Cancel(path)) => {
                if self.password.as_ref().is_some_and(|(r, _)| r.path == path) {
                    if let Some((request, _)) = self.password.take() {
                        request.answer(None);
                    }
                    if self.page == Page::Password {
                        self.page = Page::Main;
                    }
                }
            }
            Message::Secret(agent::Event::Ask(request)) => {
                if let Some((old, _)) = self.password.take() {
                    old.answer(None);
                }
                self.password = Some((request, String::new()));
                self.page = Page::Password;
                if self.popup.is_none() {
                    return self.open();
                }
            }
            Message::PasswordInput(value) => {
                if let Some((_, text)) = &mut self.password {
                    *text = value;
                }
            }
            Message::Remember(on) => self.remember = on,
            Message::PasswordSubmit | Message::PasswordCancel => {
                if let Some((request, value)) = self.password.take() {
                    let answer = matches!(message, Message::PasswordSubmit).then_some((value, self.remember));
                    request.answer(answer);
                }
                self.page = Page::Main;
            }
            Message::DragStart(i) => {
                self.drag = Some(i);
                self.drag_over = Some(i);
            }
            Message::DragOver(i) => self.drag_over = Some(i),
            Message::DragEnd => {
                if let (Some(from), Some(to)) = (self.drag.take(), self.drag_over.take()) {
                    let len = self.prefs.tiles.len();
                    if from < len && to < len {
                        if from == to {
                            // A click, not a drag: wide to small and back.
                            let tile = self.prefs.tiles[from];
                            match self.prefs.small.iter().position(|t| *t == tile) {
                                Some(at) => _ = self.prefs.small.remove(at),
                                None => self.prefs.small.push(tile),
                            }
                        } else {
                            let tile = self.prefs.tiles.remove(from);
                            self.prefs.tiles.insert(to, tile);
                        }
                        self.prefs.save();
                    }
                }
            }
            Message::ResetTiles => {
                self.prefs.tiles = Tile::DEFAULT.to_vec();
                self.prefs.small.clear();
                self.prefs.save();
            }
            Message::PanelIcons(icons) => self.pref(|p| p.panel_icons = icons),
            Message::SetSpacing(spacing) => self.pref(|p| p.spacing = spacing),
            Message::SetIslands(on) => self.pref(|p| p.islands = on),
            Message::A11y(desktop::A11y::Ready(tx)) => self.a11y = Some(tx),
            Message::A11y(desktop::A11y::Event(event)) => match event {
                desktop::AccessibilityEvent::Magnifier(on) => self.magnifier = Some(on),
                desktop::AccessibilityEvent::ScreenFilter { inverted, filter } => {
                    self.screen_filter = Some((inverted, filter));
                }
                desktop::AccessibilityEvent::Closed => {
                    self.a11y = None;
                    self.magnifier = None;
                    self.screen_filter = None;
                }
                // Only changes are sent: until one, both are off.
                desktop::AccessibilityEvent::Bound(_) => {
                    self.magnifier.get_or_insert(false);
                    self.screen_filter.get_or_insert((false, None));
                }
            },
            Message::Reader(response) => match response {
                desktop::ReaderResponse::Init(on, tx) => {
                    self.screen_reader = Some(on);
                    self.reader = Some(tx);
                }
                desktop::ReaderResponse::ScreenReader(on) => self.screen_reader = Some(on),
                desktop::ReaderResponse::IsEnabled(_) => {}
                desktop::ReaderResponse::Error(err) => tracing::debug!("screen reader: {err}"),
            },
            Message::Workspaces(workspaces::Event::Ready(tx)) => self.workspaces = Some(tx),
            Message::Workspaces(workspaces::Event::Tiling(on)) => self.tiling = Some(on),
            Message::Workspaces(workspaces::Event::Layout(i)) => self.layout = i,
            Message::KeyboardLayout(i) => {
                if let Some(tx) = &self.workspaces {
                    _ = tx.send(workspaces::Command::Layout(i as u32));
                }
            }
            Message::Inhibited(cookie) => {
                self.inhibiting = false;
                self.inhibit = cookie;
            }
            Message::ShowTile(tile, on) => {
                self.prefs.tiles.retain(|t| *t != tile);
                if on {
                    self.prefs.tiles.push(tile);
                }
                self.prefs.save();
            }
            Message::SetLayout(layout) => {
                self.prefs.layout = layout;
                self.prefs.save();
                // Reopen to take the new shape.
                let close = self.close();
                return Task::batch([close, self.open()]).chain(self.update(Message::Page(Page::Edit)));
            }
            Message::ShowClock(on) => self.pref(|p| p.clock = on),
            Message::Spawn(program, args) => {
                crate::host::spawn(program, args);
                return self.close();
            }
            Message::None => {}
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            sys::changes().map(Message::Changed),
            audio::changes().map(|()| Message::AudioChanged),
            crate::config::changes().map(|()| Message::FlagsChanged),
            // A preview leaves passwords to the Orbit on the panel.
            if preview().is_some() { Subscription::none() } else { agent::subscription().map(Message::Secret) },
            desktop::a11y().map(Message::A11y),
            desktop::screen_reader().map(Message::Reader),
            workspaces::subscription().map(Message::Workspaces),
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            iced::event::listen_with(|event, _, _| {
                use iced::event::wayland::{Event as Wayland, OutputEvent};
                match event {
                    iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(Wayland::Output(
                        OutputEvent::Created(Some(info)) | OutputEvent::InfoUpdate(info),
                        _,
                    ))) => Some(Message::Screen(
                        info.name.clone().unwrap_or_default(),
                        info.logical_size.map(|(w, h)| (w as u32, h as u32)),
                    )),
                    _ => None,
                }
            }),
        ];
        subs.push(notifications::subscription().map(Message::Notification));
        subs.push(iced::time::every(Duration::from_secs(30 * 60)).map(|_| Message::CheckWeather));
        // Updates: hourly; the system's updater refreshes the package lists.
        subs.push(iced::time::every(Duration::from_secs(3600)).map(|_| Message::CheckUpdates));
        // ponytail: polls /proc for cameras every 3 s; inotify on /dev/video* if that costs.
        subs.push(iced::time::every(Duration::from_secs(3)).map(|_| Message::CheckCamera));
        if self.drag.is_some() {
            // Let go anywhere.
            subs.push(iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                    Some(Message::DragEnd)
                }
                _ => None,
            }));
        }
        if self.popup.is_some() && self.prefs.layout == Layout::Side && self.opened.elapsed() < SLIDE {
            subs.push(window::frames().map(|_| Message::Frame));
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        let applet = &self.core.applet;
        let size = applet.suggested_size(true).0;
        let mut icons: Vec<Element<'_, Message>> = Vec::new();
        let push = |icons: &mut Vec<Element<'_, Message>>, name: &str| icons.push(status_icon(named(name), size, Shade::Plain));
        // Do not disturb: no bell either; they still wait in the center.
        if !self.notifications.is_empty() && !self.dnd {
            push(&mut icons, "notification-new-symbolic");
        }
        if !self.camera.is_empty() {
            push(&mut icons, "camera-web-symbolic");
        }
        if !self.audio.recording.is_empty() {
            push(&mut icons, "audio-input-microphone-symbolic");
        }
        if self.vpn_on() {
            push(&mut icons, "network-vpn-symbolic");
        }
        match &self.net.primary {
            Some(p) if p.wired => push(&mut icons, "network-wired-symbolic"),
            Some(_) => push(&mut icons, wifi_icon(
                self.net.access_points.iter().find(|a| a.active).map_or(0, |a| a.strength),
            )),
            None if self.wifi_lost.is_some() => icons.push(status_icon(named("network-wireless-offline-symbolic"), size, Shade::Alert)),
            None if self.net.wifi_enabled => push(&mut icons, "network-wireless-offline-symbolic"),
            None => push(&mut icons, "network-offline-symbolic"),
        }
        if self.bt.devices.iter().any(|d| d.connected) {
            push(&mut icons, "bluetooth-active-symbolic");
        }
        push(&mut icons, if self.audio.muted { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" });
        // The tiles' own icons: the ones on, or all with the off ones faint.
        let mut tile_icons = Vec::new();
        for tile in self.prefs.tiles.iter().filter(|t| !t.has_status_icon() && self.available(**t)) {
            let view = self.tile(*tile);
            let shortcut = matches!(tile, Tile::Screenshot | Tile::Workspaces);
            match self.prefs.panel_icons {
                PanelIcons::Active if view.on => tile_icons.push(status_icon(view.icon, size, Shade::Plain)),
                PanelIcons::All if !shortcut => tile_icons.push(status_icon(view.icon, size, if view.on { Shade::Plain } else { Shade::Faint })),
                _ => {}
            }
        }
        if let Some(b) = &self.battery {
            let handle = icons::battery(b.percent, b.charging);
            let low = self.prefs.low_battery_badge && !b.charging && b.percent <= 20.0;
            icons.push(status_icon(handle, size, if low { Shade::Alert } else { Shade::Plain }));
        }
        // Before the battery and the clock, at the end of the status icons.
        let at = icons.len() - usize::from(self.battery.is_some());
        icons.splice(at..at, tile_icons);
        let horizontal = applet.is_horizontal();
        // Native: COSMIC's interface density (compact 8, standard 12, spacious 16).
        let gap = self.prefs.spacing.gap(cosmic::theme::spacing().space_xs);
        if let Some(w) = self.weather.as_ref().filter(|_| self.prefs.weather_panel) {
            icons.push(
                widget::Row::new()
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .push(status_icon(named(w.condition.icon(w.day)), size, Shade::Plain))
                    .push(applet.text(format!("{:.0}°", w.temperature)))
                    .into(),
            );
        }
        if self.prefs.pomodoro_panel && self.pomodoro.started() {
            icons.push(applet.text(pomodoro::clock(self.pomodoro.remaining(Instant::now()))).into());
        }
        if self.prefs.clock {
            let date = self.now.date();
            let clock = if horizontal {
                format!("{}  {}", i18n::short_date(date), i18n::clock(self.now.time()))
            } else {
                self.now.strftime("%H\n%M").to_string()
            };
            icons.push(applet.text(clock).into());
        }
        let content: Element<'_, Message> = if horizontal {
            widget::row::with_children(icons).spacing(gap).align_y(Alignment::Center).into()
        } else {
            widget::column::with_children(icons).spacing(gap).align_x(Alignment::Center).into()
        };
        let (major, minor) = applet.suggested_padding(true);
        let button = button::custom(content)
            .padding(if horizontal { [minor, major] } else { [major, minor] })
            .class(theme::Button::AppletIcon)
            .on_press_down(Message::Toggle);
        applet.autosize_window(button).into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if self.popup != Some(id) {
            return text::body("").into();
        }
        let layout = self.prefs.layout;
        let main = self.page == Page::Main;
        let width = if main { self.main_width() } else { WIDTH };
        let content = if main {
            self.main_page()
        } else {
            container(self.sub_page()).padding(14).into()
        };
        let body: Element<'_, Message> = if self.prefs.islands && self.page == Page::Main {
            swipe(widget::scrollable(content)).into()
        } else {
            // One surface, outside the scrolling, so a side panel runs the
            // screen's full height.
            let surface = panel(swipe(widget::scrollable(content)).into(), width);
            if layout == Layout::Side { surface.height(Length::Fill).into() } else { surface.into() }
        };
        let body: Element<'_, Message> = if preview().is_some() { preview_frame(body) } else { body };
        match layout {
            Layout::Dropdown => cosmic::widget::autosize::autosize(
                container(body).width(Length::Fixed(width)),
                widget::Id::new("orbit-autosize"),
            )
            // No taller than the popup may be, so what's more scrolls.
            .limits(Limits::NONE.width(width).min_height(1.0).max_height(self.screen.1 as f32 - self.panel_room()))
            .into(),
            Layout::Side => {
                let t = (self.opened.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
                let shift = (1.0 - t).powi(3) * (width + 40.0);
                container(body)
                    .align_right(Length::Fill)
                    .padding(iced::Padding { left: 20.0 + shift, right: 20.0 - shift.min(20.0), top: 0.0, bottom: 0.0 })
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            Layout::Fullscreen => {
                let card = widget::mouse_area(container(body).max_height(self.screen.1 as f32 * 0.9)).on_press(Message::None);
                widget::mouse_area(
                    container(card)
                        .center(Length::Fill)
                        .class(theme::Container::custom(move |_| container::Style {
                            background: Some(Color::BLACK.scale_alpha(0.6).into()),
                            ..Default::default()
                        })),
                )
                .on_press(Message::Close)
                .into()
            }
        }
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::Closed(id))
    }
}
