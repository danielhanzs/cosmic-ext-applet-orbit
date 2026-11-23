// SPDX-License-Identifier: GPL-3.0-only

//! The popup's cards: calendar, weather, notifications, the dynamic
//! island, and the cards turned on in the editor.

use super::*;

impl App {
    pub(super) fn month_grid(&self) -> Element<'_, Message> {
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
                // The months before and after fill the rows, faint.
                let other = day.month() != first.month();
                let selected = day == self.selected;
                let busy = self.events.as_ref().is_some_and(|e| e.contains_key(&day));
                let mut number = text::body(day.day().to_string());
                if day == today && !selected {
                    number = number.class(theme::Text::Accent).font(cosmic::font::bold());
                } else if other && !selected {
                    number = number.class(theme::Text::Custom(|theme| iced::widget::text::Style {
                        color: Some(Color::from(theme.cosmic().background(false).on).scale_alpha(0.4)),
                        ..Default::default()
                    }));
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

    pub(super) fn calendar_view(&self) -> Element<'_, Message> {
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
                // The picked day, and back to today from another.
                column = column.push(
                    widget::Row::new()
                        .align_y(Alignment::Center)
                        .push(text::heading(i18n::long_date(self.selected)).width(Length::Fill))
                        .push(button::text(T.today).on_press_maybe(
                            (self.selected != today || self.visible != today.first_of_month()).then_some(Message::Today),
                        )),
                );
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
                // "14:30 Dentist": an hour then; without a time, all day.
                column = column.push(
                    widget::text_input(T.add_event, self.event_input.as_str())
                        .on_input(Message::EventInput)
                        .on_submit(|_| Message::AddEvent),
                );
                if let Some(err) = &self.event_error {
                    column = column.push(error_text(err));
                }
            }
        }
        widget::Column::new()
            .push(head)
            .push(container(column).padding([0, 12, 12, 12]))
            .into()
    }

    /// What's running that speaks up: the music, who records.
    pub(super) fn island(&self) -> Option<Element<'_, Message>> {
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
            // Where in the track: dragged, it's sought on release.
            if let Some(length) = media.length {
                let at = self.seek_drag.unwrap_or(media.at().as_secs_f32());
                column = column.push(
                    widget::Row::new()
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .push(text::caption(minutes_seconds(Duration::from_secs_f32(at))))
                        .push(
                            widget::slider(0.0..=length.as_secs_f32(), at, Message::SeekDrag)
                                .step(1.0_f32)
                                .on_release(Message::Seek),
                        )
                        .push(text::caption(minutes_seconds(length))),
                );
            }
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

    pub(super) fn weather_view(&self) -> Element<'_, Message> {
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

    pub(super) fn notifications_view(&self) -> Element<'_, Message> {
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

    /// Sound on a card: output and microphone levels, each with its device
    /// (a click lists the others), and the apps playing, folded away.
    pub(super) fn sound_card(&self) -> Element<'_, Message> {
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
    pub(super) fn system_view(&self) -> Element<'_, Message> {
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
            .push(widget::divider::horizontal::light())
            .push(
                widget::Row::new()
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .push(text::heading(T.processes).width(Length::Fill))
                    .push(choice("CPU", !self.processes_by_memory, Message::ProcessesBy(false)))
                    .push(choice("RAM", self.processes_by_memory, Message::ProcessesBy(true))),
            )
            .push(self.processes.iter().fold(widget::Column::new().spacing(2), |column, p| {
                let usage = if self.processes_by_memory {
                    format!("{:.0} MB", p.memory as f64 / 1024.0)
                } else {
                    format!("{:.0}%", p.cpu)
                };
                column.push(
                    widget::Row::new()
                        .spacing(6)
                        .align_y(Alignment::Center)
                        .push(text::body(p.name.clone()).wrapping(iced::widget::text::Wrapping::None).width(Length::Fill))
                        .push(text::caption(usage))
                        .push(self.end_button(p.own.then_some(p.pid))),
                )
            }))
            .into()
    }

    /// An action that (if `ask`) asks first: its button, then "Sure?" and a
    /// way back.
    pub(super) fn guarded(&self, key: String, icon_name: &'static str, tip: &'static str, message: Option<Message>, ask: bool) -> Element<'_, Message> {
        if !ask {
            return tip_button(icon_name, tip, message);
        }
        match message {
            Some(message) if self.confirming.as_deref() == Some(key.as_str()) => widget::Row::new()
                .spacing(2)
                .align_y(Alignment::Center)
                .push(button::destructive(T.sure).on_press(message))
                .push(button::icon(icon::from_name("window-close-symbolic")).padding(4).on_press(Message::Confirm(None)))
                .into(),
            message => tip_button(icon_name, tip, message.map(|_| Message::Confirm(Some(key)))),
        }
    }

    /// Ending a process: asked first; nothing for one that isn't the user's.
    pub(super) fn end_button(&self, pid: Option<u32>) -> Element<'_, Message> {
        match pid {
            Some(pid) => self.guarded(format!("end:{pid}"), "process-stop-symbolic", T.end_process, Some(Message::End(pid)), true),
            None => widget::space().width(Length::Fixed(28.0)).into(),
        }
    }

    /// SSH: a destination typed, the ones used lately, the config's hosts.
    pub(super) fn ssh_view(&self) -> Element<'_, Message> {
        let input = self.ssh_input.trim().to_lowercase();
        let mut column = widget::Column::new()
            .spacing(4)
            .push(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("network-server-symbolic").size(18))
                    .push(text::heading("SSH")),
            )
            .push(
                widget::text_input("user@host", self.ssh_input.as_str())
                    .on_input(Message::SshInput)
                    .on_submit(|text| Message::Ssh(text)),
            );
        let row = |name: String, target: String| {
            button::custom(
                widget::Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("utilities-terminal-symbolic").size(16))
                    .push(widget::Column::new().push(text::body(name.clone())).push(text::caption(target)).width(Length::Fill)),
            )
            .class(style::flat())
            .padding([4, 8])
            .width(Length::Fill)
            .on_press(Message::Ssh(name))
        };
        let mut any = false;
        for recent in self.prefs.recent_ssh.iter().filter(|r| r.to_lowercase().contains(&input)) {
            let target = self.ssh_hosts.iter().find(|h| h.name == *recent).map_or(String::new(), |h| h.target.clone());
            column = column.push(row(recent.clone(), target));
            any = true;
        }
        for host in self.ssh_hosts.iter().filter(|h| {
            !self.prefs.recent_ssh.contains(&h.name) && (h.name.to_lowercase().contains(&input) || h.target.to_lowercase().contains(&input))
        }) {
            column = column.push(row(host.name.clone(), host.target.clone()));
            any = true;
        }
        if !any {
            column = column.push(text::caption(T.ssh_hint));
        }
        column.into()
    }

    /// The ports something listens on: who, to open, to end.
    pub(super) fn ports_view(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(4).push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(icon::from_name("network-transmit-receive-symbolic").size(18))
                .push(text::heading(T.section_ports).width(Length::Fill))
                .push(button::icon(icon::from_name("view-refresh-symbolic")).padding(4).on_press(Message::PortsRefresh)),
        );
        column = column.push(widget::text_input(T.search_ports, self.ports_query.as_str()).on_input(Message::PortsQuery));
        let shown = dev::find_ports(&self.ports, &self.ports_query);
        if shown.is_empty() {
            column = column.push(text::caption(T.no_ports));
        }
        for p in shown {
            // What a well-known one is for, and who listens where it's the user's.
            let what = dev::KNOWN.iter().find(|(n, _)| *n == p.port).map_or(String::new(), |(_, w)| format!("{w} · "));
            let who = p.process.as_ref().map_or(String::new(), |(name, pid)| format!("{name} ({pid}) · "));
            let who = format!("{what}{who}");
            column = column.push(
                widget::Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(text::body(p.port.to_string()).font(cosmic::font::bold()).width(Length::Fixed(56.0)))
                    .push(text::caption(format!("{who}{}", p.address)).wrapping(iced::widget::text::Wrapping::None).width(Length::Fill))
                    // Where it may be a web server.
                    .push(if docker::not_web(p.port) {
                        Element::from(widget::space().width(Length::Fixed(28.0)))
                    } else {
                        widget::tooltip(
                            button::icon(icon::from_name("web-browser-symbolic"))
                                .padding(4)
                                .on_press(Message::OpenUrl(format!("http://localhost:{}", p.port))),
                            text::caption(T.open_port),
                            widget::tooltip::Position::Left,
                        )
                        .into()
                    })
                    .push(self.end_button(p.process.as_ref().map(|(_, pid)| *pid))),
            );
        }
        column.into()
    }

    /// libvirt's machines: their state, started, shut down, their screen.
    pub(super) fn vms_view(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(4).push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(icon::from_name("computer-symbolic").size(18))
                .push(text::heading(T.section_vms)),
        );
        let vms = self.vms.as_deref().unwrap_or_default();
        if vms.is_empty() {
            column = column.push(text::caption(T.no_vms));
        }
        for vm in vms {
            let busy = self.vms_busy.contains(&vm.name);
            let running = vm.running();
            column = column.push(
                widget::Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(widget::Column::new().push(text::body(vm.name.clone())).push(text::caption(vm.state.clone())).width(Length::Fill))
                    .push_maybe(running.then(|| tip_button("video-display-symbolic", T.console, Some(Message::Console(vm.name.clone())))))
                    .push(if running {
                        self.guarded(
                            format!("shutdown:{}", vm.name),
                            "system-shutdown-symbolic",
                            T.shut_down,
                            (!busy).then(|| Message::VmAction("shutdown", vm.name.clone())),
                            true,
                        )
                    } else {
                        tip_button("media-playback-start-symbolic", T.start, (!busy).then(|| Message::VmAction("start", vm.name.clone())))
                    }),
            );
        }
        column.into()
    }

    /// The open to-dos, soonest first: ticked off with a click.
    pub(super) fn tasks_view(&self) -> Element<'_, Message> {
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
        if self.tasks.is_some() {
            column = column.push(
                widget::text_input(T.add_task, self.task_input.as_str())
                    .on_input(Message::TaskInput)
                    .on_submit(|_| Message::AddTask),
            );
            if let Some(err) = &self.task_error {
                column = column.push(error_text(err));
            }
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

    /// systemd's services: searched, or the pinned and failed ones; each
    /// with its state, its journal, and starting, stopping, restarting.
    pub(super) fn services_view(&self) -> Element<'_, Message> {
        let scope = if self.services_user { "user" } else { "system" };
        let mut column = widget::Column::new()
            .spacing(6)
            .push(
                widget::Row::new()
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .push(icon::from_name("system-run-symbolic").size(18))
                    .push(container(text::heading(T.section_services)).padding([0, 4]).width(Length::Fill))
                    .push(choice(T.section_system, !self.services_user, Message::ServicesScope(false)))
                    .push(choice(T.user, self.services_user, Message::ServicesScope(true)))
                    .push(button::icon(icon::from_name("view-refresh-symbolic")).padding(4).on_press(Message::ServicesScope(self.services_user))),
            )
            .push(widget::text_input(T.search_services, self.services_query.as_str()).on_input(Message::ServicesQuery));
        let Some(units) = &self.services else {
            return column.into();
        };
        let key = |u: &services::Unit| format!("{scope}:{}", u.name);
        let shown: Vec<&services::Unit> = if self.services_query.trim().is_empty() {
            // Pinned ones, and any that failed.
            units.iter().filter(|u| u.failed() || self.prefs.pinned_services.contains(&key(u))).collect()
        } else {
            services::search(units, &self.services_query).into_iter().take(15).collect()
        };
        if shown.is_empty() && self.services_query.trim().is_empty() {
            column = column.push(text::caption(T.services_hint));
        }
        if let Some(err) = &self.services_error {
            column = column.push(error_text(err));
        }
        for u in shown {
            let busy = self.services_busy.contains(&key(u));
            let pinned = self.prefs.pinned_services.contains(&key(u));
            let (running, failed) = (u.running(), u.failed());
            let about = if u.description.is_empty() { u.sub.clone() } else { format!("{} · {}", u.sub, u.description) };
            let (icon_name, tip, action) = start_stop(running);
            column = column.push(
                widget::Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(state_dot(running, failed))
                    .push(
                        widget::Column::new()
                            .push(text::body(u.short().to_string()).wrapping(iced::widget::text::Wrapping::None))
                            .push(text::caption(about).wrapping(iced::widget::text::Wrapping::None))
                            .width(Length::Fill)
                            .clip(true),
                    )
                    .push(tip_button(
                        if pinned { "starred-symbolic" } else { "non-starred-symbolic" },
                        if pinned { T.unpin } else { T.pin },
                        Some(Message::PinService(key(u))),
                    ))
                    .push(tip_button("text-x-generic-symbolic", T.logs, Some(Message::ServiceLogs(u.name.clone()))))
                    // The system's: stopping or restarting one asks first
                    // (the session may hang on it).
                    .push(self.guarded(
                        format!("stop:{}", key(u)),
                        icon_name,
                        tip,
                        (!busy).then(|| Message::ServiceAction(action, u.name.clone())),
                        running && !self.services_user,
                    ))
                    .push_maybe(running.then(|| {
                        let restart = (!busy).then(|| Message::ServiceAction("restart", u.name.clone()));
                        self.guarded(format!("restart:{}", key(u)), "system-reboot-symbolic", T.restart, restart, !self.services_user)
                    })),
            );
        }
        column.into()
    }

    /// Docker's Compose stacks, each opening to its containers: their state,
    /// their log, a shell in them.
    pub(super) fn docker_view(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(4).push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(icon::from_name("application-x-executable-symbolic").size(18))
                .push(text::heading("Docker").width(Length::Fill))
                .push(button::icon(icon::from_name("view-refresh-symbolic")).padding(4).on_press(Message::DockerRefresh)),
        );
        let Some(stacks) = &self.docker else {
            return column.push(text::caption(T.docker_unavailable)).into();
        };
        if stacks.is_empty() {
            column = column.push(text::caption(T.no_containers));
        }
        for stack in stacks {
            let open = self.docker_open.contains(&stack.name);
            let name = if stack.name.is_empty() { T.other_containers.to_string() } else { stack.name.clone() };
            let key = format!("stack:{}", stack.name);
            let busy = self.docker_busy.contains(&key);
            let mut row = widget::Row::new().spacing(2).align_y(Alignment::Center).push(
                button::custom(
                    widget::Row::new()
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .push(state_dot(stack.running() > 0, false))
                        .push(text::body(name).width(Length::Fill))
                        .push(text::caption(format!("{}/{}", stack.running(), stack.containers.len())))
                        .push(icon::from_name(if open { "go-up-symbolic" } else { "go-down-symbolic" }).size(16)),
                )
                .class(style::flat())
                .padding([6, 8])
                .width(Length::Fill)
                .on_press(Message::DockerStack(stack.name.clone())),
            );
            // A Compose stack as a whole: its folder, and start, stop, restart.
            if !stack.name.is_empty() {
                if let Some(folder) = &stack.folder {
                    row = row.push(tip_button("folder-open-symbolic", T.project_terminal, Some(Message::DockerFolder(folder.clone()))));
                }
                let (icon_name, tip, action) = start_stop(stack.running() > 0);
                let message = (!busy).then(|| Message::DockerStackAction(action, stack.name.clone()));
                // Stopping a whole stack asks first.
                row = row.push(self.guarded(format!("stop:{key}"), icon_name, tip, message, action == "stop"));
                // Restarting is for what runs.
                if stack.running() > 0 {
                    row = row.push(self.guarded(
                        format!("restart:{key}"),
                        "system-reboot-symbolic",
                        T.restart,
                        (!busy).then(|| Message::DockerStackAction("restart", stack.name.clone())),
                        true,
                    ));
                }
            }
            column = column.push(row);
            if !open {
                continue;
            }
            for c in &stack.containers {
                column = column.push(
                    widget::Row::new()
                        .spacing(8)
                        .padding([0, 8, 0, 24])
                        .align_y(Alignment::Center)
                        .push(state_dot(c.running, false))
                        .push(
                            widget::Column::new()
                                .push(text::body(c.service.as_str()))
                                .push(text::caption(c.status.as_str()).wrapping(iced::widget::text::Wrapping::None))
                                .width(Length::Fill),
                        )
                        .push_maybe(c.web.as_ref().filter(|_| c.running).map(|url| tip_button("web-browser-symbolic", T.open_port, Some(Message::OpenUrl(url.clone())))))
                        .push(tip_button("text-x-generic-symbolic", T.logs, Some(Message::DockerLogs(c.id.clone()))))
                        .push(tip_button("utilities-terminal-symbolic", T.terminal, c.running.then(|| Message::DockerShell(c.id.clone()))))
                        .push({
                            let busy = self.docker_busy.contains(&c.id);
                            let (icon_name, tip, action) = start_stop(c.running);
                            tip_button(icon_name, tip, (!busy).then(|| Message::DockerContainer(action, c.id.clone())))
                        })
                        .push_maybe(c.running.then(|| {
                            tip_button("system-reboot-symbolic", T.restart, (!self.docker_busy.contains(&c.id)).then(|| Message::DockerContainer("restart", c.id.clone())))
                        })),
                );
            }
        }
        column.into()
    }

    /// Other places' time, then timers: started with a click, counted down.
    pub(super) fn clocks_view(&self) -> Element<'_, Message> {
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
    pub(super) fn mixer(&self) -> Element<'_, Message> {
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
    pub(super) fn streams(&self) -> Element<'_, Message> {
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

    /// The AI coding assistants signed in to: each window's share used, a
    /// bar, and when it starts over.
    pub(super) fn usage_view(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(6).push(
            widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(icon::icon(icons::meter(60.0, 30.0)).size(18))
                .push(text::heading(T.section_usage).width(Length::Fill))
                .push(button::icon(icon::from_name("view-refresh-symbolic")).padding(4).on_press_maybe((!self.usage_reading).then_some(Message::UsageRefresh))),
        );
        if self.usage.is_empty() && self.usage_at.is_some() {
            column = column.push(text::caption(T.usage_none));
        }
        for p in &self.usage {
            column = column.push(
                widget::Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(text::body(p.name).font(cosmic::font::bold())),
            );
            for limit in &p.limits {
                let name = window_name(&limit.window);
                let share = format!("{:.0}%{}", limit.percent, self.resets(limit));
                column = column.push(
                    widget::Column::new()
                        .spacing(2)
                        .push(
                            widget::Row::new()
                                .align_y(Alignment::Center)
                                .push(text::caption(name).width(Length::Fill))
                                // Near the limit: in the warning color.
                                .push(if limit.severe { error_text(&share) } else { text::caption(share).into() }),
                        )
                        .push(widget::determinate_linear(limit.percent / 100.0).width(Length::Fill)),
                );
            }
        }
        column.into()
    }
}

/// An icon button with its name on hover; greyed out without a message.
pub(super) fn tip_button(icon_name: &'static str, tip: &'static str, message: Option<Message>) -> Element<'static, Message> {
    widget::tooltip(button::icon(icon::from_name(icon_name)).padding(4).on_press_maybe(message), text::caption(tip), widget::tooltip::Position::Left)
        .into()
}

/// The icon, name and action that start or (running) stop something.
fn start_stop(running: bool) -> (&'static str, &'static str, &'static str) {
    if running { ("media-playback-stop-symbolic", T.stop, "stop") } else { ("media-playback-start-symbolic", T.start, "start") }
}

/// A small dot: red failed, green running, faint stopped.
fn state_dot(running: bool, failed: bool) -> Element<'static, Message> {
    container(widget::space())
        .width(Length::Fixed(8.0))
        .height(Length::Fixed(8.0))
        .class(theme::Container::custom(move |theme| {
            let cosmic = theme.cosmic();
            let color: Color = if failed {
                cosmic.destructive_color().into()
            } else if running {
                cosmic.success_color().into()
            } else {
                Color::from(cosmic.button.on).scale_alpha(0.3)
            };
            container::Style {
                background: Some(color.into()),
                border: iced::Border { radius: 4.0.into(), ..Default::default() },
                ..Default::default()
            }
        }))
        .into()
}
