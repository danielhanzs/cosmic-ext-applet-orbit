// SPDX-License-Identifier: GPL-3.0-only

mod agent;
mod app;
mod audio;
mod bus;
mod calendar;
mod config;
mod desktop;
mod host;
mod i18n;
mod icons;
mod media;
mod style;
mod sys;
mod workspaces;

pub const APP_ID: &str = "io.github.danielhanzs.CosmicOrbit";

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,cosmic_orbit=info".into()),
        )
        .init();
    cosmic::applet::run::<app::App>(())
}
