// SPDX-License-Identifier: GPL-3.0-only

//! The editor: its tabs, the tiles, the cards and their settings.

use super::*;

impl App {
    pub(super) fn edit_page(&self) -> Element<'_, Message> {
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
            [
                (EditTab::Appearance, T.appearance),
                (EditTab::Panel, T.panel),
                (EditTab::Tiles, T.tiles),
                (EditTab::Cards, T.cards),
                (EditTab::About, T.about),
            ]
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
                EditTab::Appearance => self.edit_appearance(),
                EditTab::Panel => self.edit_panel(),
                EditTab::Tiles => self.edit_tiles(),
                EditTab::Cards => self.edit_cards(),
                EditTab::About => self.edit_about(),
            })
            .into()
    }

    /// How the popup opens and sits, and the tiles' look.
    pub(super) fn edit_appearance(&self) -> Element<'_, Message> {
        use crate::config::TileStyle;
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
            .push(group(T.tile_style, T.tile_style_hint, looks.into()))
            .into()
    }

    /// What the button on the panel shows, and the marks it puts up.
    pub(super) fn edit_panel(&self) -> Element<'_, Message> {
        use crate::config::WifiBadge;
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
            // Everything else the panel can show, in one place.
            .push(group(
                T.on_panel,
                T.on_panel_hint,
                widget::Column::new()
                    .spacing(10)
                    .push(switch(T.weather, self.prefs.weather_panel, Message::WeatherPanel))
                    .push(switch(T.pomodoro, self.prefs.pomodoro_panel, Message::PomodoroPanel))
                    .push(switch(T.section_usage, self.prefs.usage_panel, Message::UsagePanel))
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
    pub(super) fn edit_about(&self) -> Element<'_, Message> {
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
    pub(super) fn edit_tiles(&self) -> Element<'_, Message> {
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
    pub(super) fn edit_custom(&self) -> Element<'_, Message> {
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
    pub(super) fn edit_sections(&self) -> Element<'_, Message> {
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

    /// Which cards show and where, then each card's own settings.
    pub(super) fn edit_cards(&self) -> Element<'_, Message> {
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
            .push(group(T.cards, T.cards_hint, self.edit_sections()))
            .push(group(T.weather, T.weather_hint, weather.into()))
            .push(group(T.section_clocks, T.clocks_hint, clocks.into()))
            .into()
    }
}
