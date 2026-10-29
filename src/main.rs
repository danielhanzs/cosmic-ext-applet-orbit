// SPDX-License-Identifier: GPL-3.0-only

mod agent;
mod app;
mod audio;
mod calendar;
mod config;
mod i18n;
mod media;
mod sys;

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
