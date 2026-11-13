// SPDX-License-Identifier: GPL-3.0-only

mod agent;
mod app;
mod audio;
mod bus;
mod calendar;
mod check;
mod config;
mod desktop;
mod host;
mod i18n;
mod icons;
mod keyboard;
mod media;
mod notifications;
mod pomodoro;
mod style;
mod swipe;
mod sys;
mod system;
mod tasks;
mod updates;
mod weather;
mod workspaces;

pub const APP_ID: &str = "io.github.danielhanzs.CosmicExtAppletOrbit";

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,cosmic_ext_applet_orbit=info".into()),
        )
        .init();
    if std::env::args().any(|a| a == "--check") {
        check::run();
        return Ok(());
    }
    cosmic::applet::run::<app::App>(())
}
