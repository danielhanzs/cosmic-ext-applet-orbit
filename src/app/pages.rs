// SPDX-License-Identifier: GPL-3.0-only

//! The pages behind the tiles' arrows, and the password prompt.

use super::*;

impl App {
    pub(super) fn page_header(&self, title: &str, toggle: Option<(bool, fn(bool) -> Message)>) -> Element<'_, Message> {
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

    pub(super) fn item(
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
    pub(super) fn more(&self, page: &'static [&'static str]) -> Element<'_, Message> {
        button::text(T.more_settings).on_press(Message::Spawn("cosmic-settings", page)).into()
    }

    pub(super) fn sub_page(&self) -> Element<'_, Message> {
        let mut column = widget::Column::new().spacing(4);
        match self.page {
            Page::Wifi => {
                column = column.push(self.page_header(T.wifi, Some((self.net.wifi_enabled, Message::SetWifi))));
                if self.net.wifi_enabled {
                    for ap in &self.net.access_points {
                        let message = match (ap.active, &self.net.wifi_device) {
                            // The one in use: its details.
                            (true, _) => Some(Message::Page(Page::Network)),
                            (_, None) => None,
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
            Page::Displays => {
                column = column.push(self.page_header(T.displays, None));
                let enabled = self.displays.iter().filter(|d| d.enabled).count();
                for d in &self.displays {
                    let name = d.name.clone();
                    // The last one on stays on.
                    let can_switch = !d.enabled || enabled > 1;
                    let mut block = widget::Column::new().spacing(6).padding([8, 10]).push(
                        widget::Row::new()
                            .spacing(8)
                            .align_y(Alignment::Center)
                            .push(icon::from_name(if d.builtin() { "laptop-symbolic" } else { "video-display-symbolic" }).size(20))
                            .push(
                                widget::Column::new()
                                    .push(text::body(if d.model.is_empty() { d.name.clone() } else { d.model.clone() }))
                                    .push(text::caption(d.name.clone()))
                                    .width(Length::Fill),
                            )
                            .push(widget::toggler(d.enabled).on_toggle_maybe(can_switch.then(|| {
                                let name = name.clone();
                                move |on| Message::DisplayOn(name.clone(), on)
                            }))),
                    );
                    // Mirroring another: that, and a way back to a display of its own.
                    if let Some(from) = d.mirroring.as_ref().filter(|_| d.enabled) {
                        block = block.push(
                            widget::Row::new()
                                .spacing(8)
                                .align_y(Alignment::Center)
                                .push(text::caption(format!("{} → {from}", T.mirror)).width(Length::Fill))
                                .push(button::text(T.stop_mirroring).on_press(Message::DisplayOn(name.clone(), true))),
                        );
                    } else if d.enabled {
                        let scales = [1.0f32, 1.25, 1.5, 2.0].into_iter().fold(widget::Row::new().spacing(2), |row, s| {
                            row.push(choice(format!("{:.0}%", s * 100.0), (d.scale - s).abs() < 0.01, Message::DisplayScale(name.clone(), s)))
                        });
                        block = block.push(text::caption(T.scale)).push(scales);
                        let others: Vec<&displays::Display> = self.displays.iter().filter(|o| o.enabled && o.name != d.name).collect();
                        if !others.is_empty() {
                            let mirror = others.into_iter().fold(widget::Row::new().spacing(2), |row, o| {
                                row.push(button::text(o.name.clone()).on_press(Message::Mirror(name.clone(), o.name.clone())))
                            });
                            block = block.push(text::caption(T.mirror)).push(mirror);
                        }
                    }
                    column = column.push(block);
                }
                column = column.push(self.more(&["displays"]));
            }
            Page::Drives => {
                column = column.push(self.page_header(T.drives, None));
                if let Some(err) = &self.eject_error {
                    column = column.push(container(error_text(err)).padding([4, 10]));
                }
                for drive in &self.drives {
                    column = column.push(
                        widget::Row::new()
                            .spacing(8)
                            .padding([4, 10])
                            .align_y(Alignment::Center)
                            .push(icon::from_name("drive-removable-media-symbolic").size(20))
                            .push(
                                button::custom(
                                    widget::Column::new()
                                        .push(text::body(drive.name.clone()))
                                        .push(text::caption(format!("{} · {}", drive.size, drive.mount))),
                                )
                                .class(style::flat())
                                .width(Length::Fill)
                                .on_press(Message::OpenPath(drive.mount.clone())),
                            )
                            .push(widget::tooltip(
                                button::icon(icon::from_name("media-eject-symbolic")).padding(6).on_press(Message::Eject(drive.clone())),
                                text::caption(T.eject),
                                widget::tooltip::Position::Left,
                            )),
                    );
                }
            }
            Page::Network => {
                let name = self.details.as_ref().map_or(String::new(), |d| d.name.clone());
                column = column.push(self.page_header(&name, None));
                if let Some(d) = &self.details {
                    let rows = [("IPv4", d.ipv4.clone()), ("IPv6", d.ipv6.clone()), (T.gateway, d.gateway.clone())]
                        .into_iter()
                        .chain(std::iter::once(("DNS", (!d.dns.is_empty()).then(|| d.dns.join(", ")))));
                    for (label, value) in rows {
                        let Some(value) = value else { continue };
                        column = column.push(
                            widget::Row::new()
                                .spacing(8)
                                .padding([4, 10])
                                .align_y(Alignment::Center)
                                .push(widget::Column::new().push(text::caption(label)).push(text::body(value.clone())).width(Length::Fill))
                                .push(widget::tooltip(
                                    button::icon(icon::from_name("edit-copy-symbolic")).padding(6).on_press(Message::Copy(value)),
                                    text::caption(T.copy),
                                    widget::tooltip::Position::Left,
                                )),
                        );
                    }
                }
                // The Wi-Fi in use, to join from a phone.
                // Only when asked for: it holds the password, for anyone to
                // read off the screen.
                if self.net.access_points.iter().any(|ap| ap.active) {
                    column = column.push(
                        container(button::standard(T.share_wifi).leading_icon(icon::from_name("qr-code-symbolic")).on_press(Message::ShowQr(!self.show_qr)))
                            .padding([8, 10]),
                    );
                    if let Some(qr) = self.wifi_qr.clone().filter(|_| self.show_qr) {
                        column = column.push(qr_view(qr));
                    }
                }
                column = column.push(self.more(if self.net.primary.as_ref().is_some_and(|p| p.wired) { &["wired"] } else { &["wireless"] }));
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
                column = column.push(container(switch(T.focus_dnd, self.prefs.focus_dnd, Message::FocusDnd)).padding([8, 10]));
            }
            Page::KeepAwake => {
                column = column.push(self.page_header(T.keep_awake, None));
                let on = self.inhibit.is_some();
                for (minutes, label) in [(30, "30 min"), (60, "1 h"), (120, "2 h"), (240, "4 h")] {
                    let picked = on && self.awake_until.as_ref().is_some_and(|(_, m)| *m == minutes);
                    column = column.push(self.item(icons::keep_awake(), label, None, picked, Some(Message::KeepAwake(Some(minutes)))));
                }
                column = column.push(self.item(
                    icons::keep_awake(),
                    T.until_off,
                    None,
                    on && self.awake_until.is_none(),
                    Some(Message::KeepAwake(Some(0))),
                ));
                if on {
                    column = column.push(self.item(icons::keep_awake(), T.off, None, false, Some(Message::KeepAwake(None))));
                }
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
                        if let Some(qr) = self.hotspot_qr.clone() {
                            column = column.push(qr_view(qr));
                        }
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

    pub(super) fn password_page(&self) -> Element<'_, Message> {
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
