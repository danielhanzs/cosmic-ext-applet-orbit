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
    config::{DARK, DND, Layout, Prefs},
    i18n::{self, T},
    media::{self, Media},
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
}

impl Tile {
    pub const ALL: [Tile; 8] = [
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
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Main,
    Wifi,
    Bluetooth,
    Vpn,
    Power,
    Sound,
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
    /// Apps using a camera.
    camera: Vec<String>,
    /// A tile being dragged in the editor, and where it would go.
    drag: Option<usize>,
    drag_over: Option<usize>,
    /// Refreshes on their way: more news of the same is let be.
    pending: HashSet<Source>,
    /// A slider being dragged: refreshes don't move it meanwhile.
    adjusting: Option<Instant>,
    password: Option<(agent::Request, String)>,
    remember: bool,
    show_all: bool,
    screen: (u32, u32),
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
    Battery(Option<BatteryState>),
    Profiles(Option<Profiles>),
    Brightness(Option<f32>),
    Audio(Audio),
    Media(Option<Media>),
    Events(civil::Date, Option<HashMap<civil::Date, Vec<Event>>>),
    CheckCamera,
    Camera(Vec<String>),
    TileClick(Tile),
    Volume(f32),
    SetBrightness(f32),
    Released,
    Mute,
    Output(String),
    Join(AccessPoint),
    Connect(OwnedObjectPath),
    Disconnect(OwnedObjectPath),
    SetWifi(bool),
    SetBluetooth(bool),
    BtConnect(OwnedObjectPath, bool),
    Profile(String),
    MediaControl(&'static str),
    Day(jiff::civil::Date),
    PrevMonth,
    NextMonth,
    Secret(agent::Request),
    PasswordInput(String),
    Remember(bool),
    PasswordSubmit,
    PasswordCancel,
    DragStart(usize),
    DragOver(usize),
    DragEnd,
    ShowTile(Tile, bool),
    SetLayout(Layout),
    OnlyActive(bool),
    ShowCalendar(bool),
    ShowClock(bool),
    ShowAll,
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

/// Starts a program and leaves it be.
fn spawn(program: &str, args: &[&str]) {
    match std::process::Command::new(program).args(args).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => tracing::warn!("cannot run {program}: {err}"),
    }
}

fn profile_name(profile: &str) -> &'static str {
    match profile {
        "performance" => T.performance,
        "power-saver" => T.power_saver,
        _ => T.balanced,
    }
}

fn profile_icon(profile: &str) -> &'static str {
    match profile {
        "performance" => "power-profile-performance-symbolic",
        "power-saver" => "power-profile-power-saver-symbolic",
        _ => "power-profile-balanced-symbolic",
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
    icon: &'static str,
    title: &'static str,
    subtitle: String,
    on: bool,
    page: Option<Page>,
}

impl App {
    fn refresh(&mut self, source: Source) -> app::Task<Message> {
        if !self.pending.insert(source) {
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
                Message::Battery(sys::battery().await.ok().flatten())
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

    fn load_events(&self) -> app::Task<Message> {
        let first = self.visible;
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
                    icon: "network-wired-symbolic",
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
                (icon, self.net.wifi_enabled, subtitle, Some(Page::Wifi))
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
                (icon, self.bt.powered, subtitle, Some(Page::Bluetooth))
            }
            Tile::Vpn => {
                let active = self.net.vpns.iter().find(|v| v.active.is_some());
                let subtitle = match active {
                    Some(v) if v.active.as_ref().is_some_and(|a| a.1) => v.name.clone(),
                    Some(_) => T.connecting.into(),
                    None => T.off.into(),
                };
                ("network-vpn-symbolic", active.is_some(), subtitle, Some(Page::Vpn))
            }
            Tile::Power => {
                let active = &self.profiles.active;
                (
                    profile_icon(active),
                    !active.is_empty() && active != "balanced",
                    profile_name(active).into(),
                    Some(Page::Power),
                )
            }
            Tile::Dnd => ("notification-disabled-symbolic", self.dnd, on_off(self.dnd), None),
            Tile::Dark => ("dark-mode-symbolic", self.dark, on_off(self.dark), None),
            Tile::Airplane => ("airplane-mode-symbolic", self.airplane(), on_off(self.airplane()), None),
            Tile::Mic => {
                let muted = self.audio.mic_muted;
                let icon = if muted { "microphone-disabled-symbolic" } else { "audio-input-microphone-symbolic" };
                let subtitle = if muted { T.muted } else { T.on };
                (icon, muted, subtitle.into(), None)
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
        self.show_all = false;
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
                .push(icon::from_name(b.icon.as_str()).size(16))
                .push(text::body(format!("{:.0}%", b.percent)));
        }
        let action = |name: &'static str, message: Message| {
            button::icon(icon::from_name(name)).padding(8).on_press(message)
        };
        row.push(widget::space().width(Length::Fill))
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
        let label = widget::Column::new()
            .push(text::body(view.title).font(cosmic::font::bold()))
            .push(text::caption(view.subtitle).wrapping(iced::widget::text::Wrapping::None));
        let mut content = widget::Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(icon::from_name(view.icon).size(20))
            .push(widget::container(label).width(Length::Fill).clip(true));
        if let Some(page) = view.page {
            content = content.push(
                button::icon(icon::from_name("go-next-symbolic"))
                    .padding(4)
                    .on_press(Message::Page(page)),
            );
        }
        button::custom(content)
            .class(if view.on { theme::Button::Suggested } else { theme::Button::Standard })
            .padding([10, 12])
            .width(Length::Fill)
            .height(Length::Fixed(60.0))
            .on_press(Message::TileClick(tile))
            .into()
    }

    fn tiles(&self) -> Element<'_, Message> {
        let mut shown: Vec<Tile> = self.prefs.tiles.clone();
        let hidden_inactive = self.prefs.only_active && !self.show_all;
        if hidden_inactive {
            shown.retain(|t| self.tile(*t).on);
        }
        let mut column = widget::Column::new().spacing(8);
        for pair in shown.chunks(2) {
            let mut row = widget::Row::new().spacing(8);
            for tile in pair {
                row = row.push(self.tile_view(*tile));
            }
            if pair.len() == 1 {
                row = row.push(widget::space().width(Length::Fill));
            }
            column = column.push(row);
        }
        if self.prefs.only_active {
            let label = if self.show_all { T.done } else { T.show_all };
            column = column.push(
                widget::Row::new()
                    .push(widget::space().width(Length::Fill))
                    .push(button::text(label).on_press(Message::ShowAll)),
            );
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
                        .class(if selected { theme::Button::Suggested } else { theme::Button::Text })
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
        let mut column = widget::Column::new()
            .spacing(8)
            .push(text::title4(i18n::long_date(self.now.date())))
            .push(self.month_grid());
        match &self.events {
            None => {
                column = column.push(
                    widget::Row::new()
                        .align_y(Alignment::Center)
                        .push(text::caption(T.no_calendar).width(Length::Fill))
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
        card(column)
    }

    /// What's running that speaks up: the music, who records.
    fn island(&self) -> Option<Element<'_, Message>> {
        let mut column = widget::Column::new().spacing(8);
        let mut any = false;
        if let Some(media) = &self.media {
            any = true;
            let art: Element<'_, Message> = match &media.art {
                Some(path) => widget::image(path.as_path()).width(44).height(44).into(),
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
        // In the colors phones use: green camera, orange microphone.
        let privacy = [
            (&self.camera, "camera-web-symbolic", T.filming, Color::from_rgb(0.3, 0.85, 0.4)),
            (&self.audio.recording, "audio-input-microphone-symbolic", T.recording, Color::from_rgb(1.0, 0.65, 0.2)),
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
                    .push(text::caption(format!("{} {what}", apps.join(", "))).class(theme::Text::Color(color))),
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
                        border: iced::Border { radius: 28.0.into(), ..Default::default() },
                        ..Default::default()
                    }
                }))
                .into()
        })
    }

    fn main_page(&self, wide: bool) -> Element<'_, Message> {
        let controls = widget::Column::new()
            .spacing(14)
            .push(self.header())
            .push(self.sliders())
            .push(self.tiles());
        let mut side = widget::Column::new().spacing(14);
        if self.prefs.calendar {
            side = side.push(self.calendar_view());
        }
        if let Some(island) = self.island() {
            side = side.push(island);
        }
        if wide {
            widget::Row::new()
                .spacing(24)
                .push(widget::container(controls).width(Length::Fixed(WIDTH)))
                .push(widget::container(side).width(Length::Fixed(WIDTH)))
                .into()
        } else {
            controls.push(side).into()
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
        icon_name: impl Into<String>,
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
            .push(icon::from_name(icon_name.into()).size(20))
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
                        column = column.push(self.item(wifi_icon(ap.strength), name, subtitle, ap.active, message));
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
                            icon_name,
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
                        "network-vpn-symbolic",
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
                        profile_icon(profile),
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
                        "audio-speakers-symbolic",
                        output.description.clone(),
                        None,
                        output.default,
                        Some(Message::Output(output.name.clone())),
                    ));
                }
                column = column.push(self.more("sound"));
            }
            Page::Edit => column = column.push(self.edit_page()),
            Page::Password => column = column.push(self.password_page()),
            Page::Main => {}
        }
        column.into()
    }

    fn edit_page(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new()
            .spacing(6)
            .push(self.page_header(T.edit, None))
            .push(text::heading(T.tiles));
        let shown = &self.prefs.tiles;
        let hidden = Tile::ALL.iter().filter(|t| !shown.contains(t));
        // Grab a row and drop it on another to move it there.
        for (i, tile) in shown.iter().enumerate() {
            let row = widget::Row::new()
                .spacing(10)
                .align_y(Alignment::Center)
                .push(icon::from_name("list-drag-handle-symbolic").size(16))
                .push(icon::from_name(self.tile(*tile).icon).size(16))
                .push(text::body(tile.name()).width(Length::Fill))
                .push(widget::toggler(true).on_toggle(|on| Message::ShowTile(*tile, on)));
            let target = self.drag.is_some() && self.drag_over == Some(i) && self.drag != Some(i);
            let class = match (self.drag == Some(i), target) {
                (true, _) => theme::Container::Primary,
                (_, true) => theme::Container::custom(|theme| container::Style {
                    border: iced::Border {
                        color: theme.cosmic().accent_color().into(),
                        width: 2.0,
                        radius: theme.cosmic().corner_radii.radius_s.into(),
                    },
                    ..Default::default()
                }),
                _ => theme::Container::Transparent,
            };
            column = column.push(
                widget::mouse_area(container(row).padding([6, 8]).class(class))
                    .interaction(iced::mouse::Interaction::Grab)
                    .on_press(Message::DragStart(i))
                    .on_enter(Message::DragOver(i)),
            );
        }
        for tile in hidden {
            column = column.push(
                widget::Row::new()
                    .align_y(Alignment::Center)
                    .push(text::body(tile.name()).width(Length::Fill))
                    .push(widget::toggler(false).on_toggle(|on| Message::ShowTile(*tile, on))),
            );
        }
        let layouts = [
            (Layout::Dropdown, T.dropdown),
            (Layout::Side, T.side),
            (Layout::Fullscreen, T.fullscreen),
        ];
        let mut choices = widget::Row::new().spacing(4);
        for (layout, name) in layouts {
            let b = button::text(name).on_press(Message::SetLayout(layout));
            choices = choices.push(if layout == self.prefs.layout { b.class(theme::Button::Suggested) } else { b });
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
            .push(choices)
            .push(widget::divider::horizontal::default())
            .push(switch(T.only_active, self.prefs.only_active, Message::OnlyActive))
            .push(switch(T.show_calendar, self.prefs.calendar, Message::ShowCalendar))
            .push(switch(T.show_clock, self.prefs.clock, Message::ShowClock))
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

/// A rounded block in the theme's component color.
fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(12)
        .width(Length::Fill)
        .class(theme::Container::Card)
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
            camera: Vec::new(),
            drag: None,
            drag_over: None,
            pending: HashSet::new(),
            adjusting: None,
            password: None,
            remember: true,
            show_all: false,
            screen: (1920, 1080),
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
                return run(async { Message::Audio(audio::read().await) });
            }
            Message::FlagsChanged => {
                self.dnd = DND.get();
                self.dark = DARK.get();
            }
            Message::Network(net) => {
                self.pending.remove(&Source::Network);
                if let Some(net) = net {
                    self.net = net;
                }
            }
            Message::Bluetooth(bt) => {
                self.pending.remove(&Source::Bluetooth);
                self.bt = bt.unwrap_or_default();
            }
            Message::Battery(battery) => {
                self.pending.remove(&Source::Battery);
                self.battery = battery;
            }
            Message::Profiles(profiles) => {
                self.pending.remove(&Source::Profiles);
                self.profiles = profiles.unwrap_or_default();
            }
            Message::Brightness(level) => {
                self.pending.remove(&Source::Brightness);
                if !self.adjusting() {
                    self.brightness = level;
                }
            }
            Message::Audio(audio) => {
                let volume = self.audio.volume;
                self.audio = audio;
                if self.adjusting() {
                    self.audio.volume = volume;
                }
            }
            Message::Media(media) => {
                self.pending.remove(&Source::Media);
                self.media = media;
            }
            Message::Events(first, events) => {
                if first == self.visible {
                    self.events = events;
                }
            }
            Message::CheckCamera => return run(async { Message::Camera(audio::camera().await) }),
            Message::Camera(apps) => self.camera = apps,
            Message::TileClick(tile) => return self.click(tile),
            Message::Volume(level) => {
                self.audio.volume = level;
                self.adjusting = Some(Instant::now());
                return fire(audio::set_volume(level));
            }
            Message::SetBrightness(level) => {
                self.brightness = Some(level);
                self.adjusting = Some(Instant::now());
                return fire(async move { log("brightness", sys::set_brightness(level).await) });
            }
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
            Message::Secret(request) => {
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
                    if from != to && from < self.prefs.tiles.len() && to < self.prefs.tiles.len() {
                        let tile = self.prefs.tiles.remove(from);
                        self.prefs.tiles.insert(to, tile);
                        self.prefs.save();
                    }
                }
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
            Message::OnlyActive(on) => {
                self.prefs.only_active = on;
                self.prefs.save();
            }
            Message::ShowCalendar(on) => {
                self.prefs.calendar = on;
                self.prefs.save();
            }
            Message::ShowClock(on) => {
                self.prefs.clock = on;
                self.prefs.save();
            }
            Message::ShowAll => self.show_all = !self.show_all,
            Message::Spawn(program, args) => {
                spawn(program, args);
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
        let mut push = |name: &str| icons.push(icon::from_name(name.to_string()).size(size).symbolic(true).into());
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
        if let Some(b) = &self.battery {
            push(b.icon.as_str());
        }
        let horizontal = applet.is_horizontal();
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
            widget::row::with_children(icons).spacing(8).align_y(Alignment::Center).into()
        } else {
            widget::column::with_children(icons).spacing(8).align_x(Alignment::Center).into()
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
        let content = if self.page == Page::Main { self.main_page(wide) } else { self.sub_page() };
        let body = widget::scrollable(widget::container(content).padding(14));
        match layout {
            Layout::Dropdown => self
                .core
                .applet
                .popup_container(container(body).width(Length::Fixed(WIDTH)))
                .limits(Limits::NONE.width(WIDTH).min_height(1.0).max_height(self.screen.1 as f32))
                .into(),
            Layout::Side => {
                let t = (self.opened.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
                let shift = (1.0 - t).powi(3) * (WIDTH + 40.0);
                container(panel(body.into(), WIDTH).height(Length::Fill))
                    .padding(iced::Padding { left: 20.0 + shift, right: 20.0 - shift.min(20.0), top: 0.0, bottom: 0.0 })
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            Layout::Fullscreen => {
                let width = if wide { WIDTH * 2.0 + 60.0 } else { WIDTH };
                let card = widget::mouse_area(panel(body.into(), width).max_height(self.screen.1 as f32 * 0.9))
                    .on_press(Message::None);
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
