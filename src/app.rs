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
    config::{DARK, DND, Layout, PanelIcons, Prefs, Spacing},
    icons,
    desktop,
    style,
    workspaces,
    i18n::{self, T},
    media::{self, Media},
    pomodoro,
    updates,
    sys::{self, AccessPoint, BatteryState, Bluetooth, Network, Profiles, Source},
};

const WIDTH: f32 = 380.0;
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
}

impl Tile {
    pub const ALL: [Tile; 19] = [
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
    workspaces: Option<cosmic::cctk::sctk::reexports::calloop::channel::Sender<bool>>,
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
    waiting: [Option<f32>; 2],
    writing: [bool; 2],
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
    pomodoro: pomodoro::Pomodoro,
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
    CompChanged,
    Inhibited(Option<u32>),
    ResetTiles,
    PanelIcons(PanelIcons),
    SetIslands(bool),
    SetSpacing(Spacing),
    ToggleCalendar,
    Camera(Vec<String>),
    TileClick(Tile),
    Volume(f32),
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
    ShowCalendar(bool),
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
    title: &'static str,
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
        run(async move {
            match slider {
                Slider::Volume => audio::set_volume(level).await,
                Slider::Brightness => log("brightness", sys::set_brightness(level).await),
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
                    title: T.wired,
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
            Tile::Touchpad => {
                let on = self.touchpad.unwrap_or(false);
                let icon = if on { "input-touchpad-symbolic" } else { "touchpad-disabled-symbolic" };
                (named(icon), on, on_off(on), None)
            }
        };
        TileView { icon, title: tile.name(), subtitle, on, page }
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
                    Some(tx) => _ = tx.send(on),
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
                let panel = applet.suggested_size(true).1 as u32 + 2 * applet.suggested_padding(true).0 as u32 + 16;
                let p = &mut settings.positioner;
                match app.prefs.layout {
                    Layout::Dropdown => {
                        p.size_limits = Limits::NONE
                            .width(WIDTH)
                            .min_height(1.0)
                            .max_height(h.saturating_sub(panel) as f32);
                    }
                    Layout::Side => {
                        let size = (WIDTH as u32 + 40, h.saturating_sub(panel));
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
        let line = |name: &'static str, mute: Option<Message>, slider: Element<'static, Message>, page: Option<Page>| {
            let mut row = widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(button::icon(icon::from_name(name)).padding(6).on_press_maybe(mute))
                .push(slider);
            if let Some(page) = page {
                row = row.push(button::icon(icon::from_name("go-next-symbolic")).padding(6).on_press(Message::Page(page)));
            }
            row
        };
        let volume_icon = match (self.audio.muted, self.audio.volume) {
            (true, _) => "audio-volume-muted-symbolic",
            (_, v) if v < 0.34 => "audio-volume-low-symbolic",
            (_, v) if v < 0.67 => "audio-volume-medium-symbolic",
            _ => "audio-volume-high-symbolic",
        };
        let volume = widget::slider(0.0..=1.0, self.audio.volume, Message::Volume)
            .step(0.01_f32)
            .on_release(Message::Released)
            .into();
        let mut column = widget::Column::new()
            .spacing(4)
            .push(line(volume_icon, Some(Message::Mute), volume, Some(Page::Sound)));
        if let Some(level) = self.brightness {
            let slider = widget::slider(0.05..=1.0, level, Message::SetBrightness)
                .step(0.01_f32)
                .on_release(Message::Released)
                .into();
            column = column.push(line("display-brightness-symbolic", None, slider, None));
        }
        column.into()
    }

    fn tile_view(&self, tile: Tile) -> Element<'_, Message> {
        let view = self.tile(tile);
        let small = self.prefs.small.contains(&tile);
        let content: Element<'_, Message> = if small {
            widget::container(icon::icon(view.icon.clone()).size(20)).center(Length::Fill).into()
        } else {
            let mut label = widget::Column::new().push(text::body(view.title).font(cosmic::font::bold()));
            if !view.subtitle.is_empty() {
                label = label.push(text::caption(view.subtitle).wrapping(iced::widget::text::Wrapping::None));
            }
            let mut row = widget::Row::new()
                .spacing(10)
                .align_y(Alignment::Center)
                .push(icon::icon(view.icon.clone()).size(20))
                .push(widget::container(label).width(Length::Fill).clip(true));
            if let Some(page) = view.page {
                row = row.push(
                    button::icon(icon::from_name("go-next-symbolic"))
                        .class(style::on_tile(view.on))
                        .padding(4)
                        .on_press(Message::Page(page)),
                );
            }
            row.into()
        };
        let button = button::custom(content)
            .class(style::tile(view.on))
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
        let shown = &self.prefs.tiles;
        let mut column = widget::Column::new().spacing(8);
        for row in rows(shown, &self.prefs.small) {
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
                            .push(text::caption(when).width(Length::Fixed(90.0)))
                            .push(info),
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

    /// The popup's parts: on one surface, or each an island of its own.
    fn main_page(&self, wide: bool) -> Element<'_, Message> {
        let islands = self.prefs.islands;
        let width = Length::Fixed(WIDTH);
        let frame = |parts| frame(parts, islands);
        let mut left = vec![self.header(), widget::Column::new().spacing(14).push(self.sliders()).push(self.tiles()).into()];
        let mut right: Vec<Element<'_, Message>> = Vec::new();
        if self.prefs.calendar {
            // Flush to its edges, so the whole line lights on hover.
            let calendar = self.calendar_view();
            right.push(if islands {
                panel(calendar, WIDTH).padding(0).into()
            } else {
                container(calendar).width(Length::Fill).class(theme::Container::Card).into()
            });
        }
        let island = self.island();
        if wide {
            let mut side = widget::Column::new().spacing(if islands { 8 } else { 14 });
            side = side.push(if islands {
                right.into_iter().fold(widget::Column::new().spacing(8), |c, p| c.push(p)).into()
            } else {
                frame(right)
            });
            if let Some(island) = island {
                side = side.push(island);
            }
            return widget::Row::new()
                .spacing(24)
                .push(container(frame(left)).width(width))
                .push(container(side).width(width))
                .into();
        }
        if islands {
            let mut column = widget::Column::new().spacing(8).push(frame(left));
            for part in right {
                column = column.push(part);
            }
            if let Some(island) = island {
                column = column.push(island);
            }
            column.into()
        } else {
            left.extend(right);
            left.extend(island);
            frame(left)
        }
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

    fn more(&self, page: &'static str) -> Element<'_, Message> {
        button::text(T.more_settings)
            .on_press(Message::Spawn("cosmic-settings", match page {
                "wireless" => &["wireless"],
                "bluetooth" => &["bluetooth"],
                "vpn" => &["vpn"],
                "power" => &["power"],
                _ => &["sound"],
            }))
            .into()
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
                column = column.push(self.more("wireless"));
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
                column = column.push(self.more("bluetooth"));
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
                column = column.push(self.more("vpn"));
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
                column = column.push(self.more("power"));
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
                column = column.push(self.more("sound"));
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
                column = column.push(self.more("wireless"));
            }
            Page::Edit => column = column.push(self.edit_page()),
            Page::Password => column = column.push(self.password_page()),
            Page::Main => {}
        }
        column.into()
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
        let mut column = widget::Column::new().spacing(6).push(head);
        if !self.prefs.mixer_open {
            return column.into();
        }
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
        let header = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(button::icon(icon::from_name("go-previous-symbolic")).padding(8).on_press(Message::Page(Page::Main)))
            .push(text::title4(T.edit).width(Length::Fill))
            .push(
                widget::tooltip(
                    button::icon(icon::from_name("edit-undo-symbolic")).padding(8).on_press(Message::ResetTiles),
                    text::caption(T.reset),
                    widget::tooltip::Position::Bottom,
                ),
            );
        let mut column = widget::Column::new()
            .spacing(10)
            .push(header)
            .push(text::caption(T.edit_hint));

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
                        widget::container(text::body(tile.name()).wrapping(iced::widget::text::Wrapping::None))
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
        let hidden: Vec<Tile> = Tile::ALL.into_iter().filter(|t| !self.prefs.tiles.contains(t)).collect();
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
                        .push(text::caption(tile.name()).align_x(iced::alignment::Horizontal::Center));
                    line = line.push(widget::container(cell).center_x(Length::FillPortion(1)));
                }
                for _ in four.len()..4 {
                    line = line.push(widget::space().width(Length::FillPortion(1)));
                }
                column = column.push(line);
            }
        }

        let choice = |label: &'static str, picked: bool, message: Message| {
            let b = button::text(label).on_press(message);
            if picked { b.class(style::picked()) } else { b }
        };
        let mut layouts = widget::Row::new().spacing(4);
        for (layout, name) in [(Layout::Dropdown, T.dropdown), (Layout::Side, T.side), (Layout::Fullscreen, T.fullscreen)] {
            layouts = layouts.push(choice(name, layout == self.prefs.layout, Message::SetLayout(layout)));
        }
        let mut icons = widget::Row::new().spacing(4);
        for (mode, name) in [(PanelIcons::None, T.icons_none), (PanelIcons::Active, T.icons_active), (PanelIcons::All, T.icons_all)] {
            icons = icons.push(choice(name, mode == self.prefs.panel_icons, Message::PanelIcons(mode)));
        }
        let mut spacing = widget::Row::new().spacing(4);
        for (size, name) in [(Spacing::Small, T.small), (Spacing::Medium, T.medium), (Spacing::Large, T.large)] {
            spacing = spacing.push(choice(name, size == self.prefs.spacing, Message::SetSpacing(size)));
        }
        let switch = |label: &'static str, on: bool, message: fn(bool) -> Message| {
            widget::Row::new()
                .align_y(Alignment::Center)
                .push(text::body(label).width(Length::Fill))
                .push(widget::toggler(on).on_toggle(message))
        };
        column
            .push(widget::divider::horizontal::default())
            .push(text::heading(T.layout))
            .push(layouts)
            .push(widget::divider::horizontal::default())
            .push(switch(T.show_calendar, self.prefs.calendar, Message::ShowCalendar))
            .push(switch(T.show_clock, self.prefs.clock, Message::ShowClock))
            .push(text::body(T.panel_icons))
            .push(icons)
            .push(switch(T.islands, self.prefs.islands, Message::SetIslands))
            .push(text::body(T.panel_spacing))
            .push(spacing)
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

/// Padded parts, framed together on one surface or each as an island.
fn frame<'a>(parts: Vec<Element<'a, Message>>, islands: bool) -> Element<'a, Message> {
    if islands {
        parts
            .into_iter()
            .fold(widget::Column::new().spacing(8), |c, part| c.push(panel(container(part).padding(14).into(), WIDTH)))
            .into()
    } else {
        // One surface: view_window puts it around everything.
        let column = parts.into_iter().fold(widget::Column::new().spacing(14), |c, p| c.push(p));
        container(column).padding(14).into()
    }
}

/// An icon from the icon theme.
fn named(name: &str) -> icon::Handle {
    icon::from_name(name.to_string()).symbolic(true).handle()
}

/// An icon on the panel, in the panel's color; faint for a function that's off.
fn status_icon<'a>(handle: icon::Handle, size: u16, faint: bool) -> Element<'a, Message> {
    icon::icon(handle)
        .size(size)
        .class(theme::Svg::Custom(std::rc::Rc::new(move |theme| iced::widget::svg::Style {
            color: Some(Color::from(theme.cosmic().background(theme.transparent).on).scale_alpha(if faint { 0.4 } else { 1.0 })),
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
            tiling: None,
            touchpad: desktop::touchpad(),
            drag: None,
            drag_over: None,
            pending: HashSet::new(),
            dirty: HashSet::new(),
            adjusting: None,
            waiting: [None; 2],
            writing: [false; 2],
            audio_pending: false,
            audio_dirty: false,
            events_read: None,
            password: None,
            remember: true,
            screen: (1920, 1080),
            hotspot_password: None,
            updates: 0,
            stream_drag: None,
            pomodoro: pomodoro::Pomodoro::default(),
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
            }
            Message::Network(net) => {
                if let Some(net) = net {
                    self.net = net;
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
                let volume = self.audio.volume;
                self.audio = audio;
                if self.adjusting() {
                    self.audio.volume = volume;
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
            Message::ToggleCalendar => {
                self.prefs.calendar_open = !self.prefs.calendar_open;
                self.prefs.save();
            }
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
            Message::PomodoroToggle => {
                let now = Instant::now();
                if self.pomodoro.running() { self.pomodoro.pause(now) } else { self.pomodoro.start(now) }
            }
            Message::PomodoroSkip => self.pomodoro.skip(Instant::now()),
            Message::PomodoroReset => self.pomodoro.reset(),
            Message::PomodoroPanel(on) => {
                self.prefs.pomodoro_panel = on;
                self.prefs.save();
            }
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
            Message::ToggleMixer => {
                self.prefs.mixer_open = !self.prefs.mixer_open;
                self.prefs.save();
            }
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
            Message::PanelIcons(icons) => {
                self.prefs.panel_icons = icons;
                self.prefs.save();
            }
            Message::SetSpacing(spacing) => {
                self.prefs.spacing = spacing;
                self.prefs.save();
            }
            Message::SetIslands(on) => {
                self.prefs.islands = on;
                self.prefs.save();
            }
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
            Message::CompChanged => self.touchpad = desktop::touchpad(),
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
            Message::ShowCalendar(on) => {
                self.prefs.calendar = on;
                self.prefs.save();
            }
            Message::ShowClock(on) => {
                self.prefs.clock = on;
                self.prefs.save();
            }
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
            agent::subscription().map(Message::Secret),
            desktop::a11y().map(Message::A11y),
            desktop::screen_reader().map(Message::Reader),
            desktop::comp_changes().map(|()| Message::CompChanged),
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
        let mut push = |name: &str| icons.push(status_icon(named(name), size, false));
        if !self.camera.is_empty() {
            push("camera-web-symbolic");
        }
        if !self.audio.recording.is_empty() {
            push("audio-input-microphone-symbolic");
        }
        if self.vpn_on() {
            push("network-vpn-symbolic");
        }
        match &self.net.primary {
            Some(p) if p.wired => push("network-wired-symbolic"),
            Some(_) => push(wifi_icon(
                self.net.access_points.iter().find(|a| a.active).map_or(0, |a| a.strength),
            )),
            None if self.net.wifi_enabled => push("network-wireless-offline-symbolic"),
            None => push("network-offline-symbolic"),
        }
        if self.bt.devices.iter().any(|d| d.connected) {
            push("bluetooth-active-symbolic");
        }
        push(if self.audio.muted { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" });
        // The tiles' own icons: the ones on, or all with the off ones faint.
        let mut tile_icons = Vec::new();
        for tile in self.prefs.tiles.iter().filter(|t| !t.has_status_icon()) {
            let view = self.tile(*tile);
            let shortcut = matches!(tile, Tile::Screenshot | Tile::Workspaces);
            match self.prefs.panel_icons {
                PanelIcons::Active if view.on => tile_icons.push(status_icon(view.icon, size, false)),
                PanelIcons::All if !shortcut => tile_icons.push(status_icon(view.icon, size, !view.on)),
                _ => {}
            }
        }
        if let Some(b) = &self.battery {
            icons.push(status_icon(icons::battery(b.percent, b.charging), size, false));
        }
        // Before the battery and the clock, at the end of the status icons.
        let at = icons.len() - usize::from(self.battery.is_some());
        icons.splice(at..at, tile_icons);
        let horizontal = applet.is_horizontal();
        let gap = self.prefs.spacing.gap();
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
        let wide = layout == Layout::Fullscreen && self.page == Page::Main;
        let content = if self.page == Page::Main {
            self.main_page(wide)
        } else {
            container(self.sub_page()).padding(14).into()
        };
        let body: Element<'_, Message> = if self.prefs.islands && self.page == Page::Main {
            widget::scrollable(content).into()
        } else {
            // One surface, outside the scrolling, so a side panel runs the
            // screen's full height.
            let surface = panel(widget::scrollable(content).into(), if wide { WIDTH * 2.0 + 60.0 } else { WIDTH });
            if layout == Layout::Side { surface.height(Length::Fill).into() } else { surface.into() }
        };
        match layout {
            Layout::Dropdown => cosmic::widget::autosize::autosize(
                container(body).width(Length::Fixed(WIDTH)),
                widget::Id::new("orbit-autosize"),
            )
            .limits(Limits::NONE.width(WIDTH).min_height(1.0).max_height(self.screen.1 as f32))
            .into(),
            Layout::Side => {
                let t = (self.opened.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
                let shift = (1.0 - t).powi(3) * (WIDTH + 40.0);
                container(body)
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
