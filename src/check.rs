// SPDX-License-Identifier: GPL-3.0-only

//! `cosmic-ext-applet-orbit --check`: what Orbit finds on this system, one
//! line a service, for when a tile or card doesn't show and why isn't clear.
//! Idea from cosmic-control-center-applet.

use crate::{audio, calendar, desktop, keyboard, sys, tasks};

fn line(name: &str, found: Result<String, String>) {
    match found {
        Ok(what) => println!("  ✓ {name:<22} {what}"),
        Err(why) => println!("  ✗ {name:<22} {why}"),
    }
}

fn shown<T, E: std::fmt::Display>(result: Result<T, E>, what: impl FnOnce(T) -> String) -> Result<String, String> {
    result.map(what).map_err(|e| e.to_string())
}

pub fn run() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
    runtime.block_on(async {
        println!("Orbit {}{}", env!("CARGO_PKG_VERSION"), if crate::host::in_flatpak() { " (Flatpak)" } else { "" });
        line(
            "NetworkManager",
            shown(sys::network().await, |n| match n.primary {
                Some(p) => format!("connected: {}{}", p.name, if n.wifi_enabled { "" } else { ", Wi-Fi off" }),
                None => "not connected".into(),
            }),
        );
        line(
            "DNS",
            match sys::dns().await {
                Some(d) => Ok(d.name().unwrap_or("automatic").into()),
                None => Err("the connection's own servers, or none".into()),
            },
        );
        line(
            "BlueZ",
            shown(sys::bluetooth().await, |b| match b.adapter {
                Some(_) => format!("{}, {} device(s)", if b.powered { "on" } else { "off" }, b.devices.len()),
                None => "no adapter".into(),
            }),
        );
        line(
            "UPower battery",
            shown(sys::battery().await, |b| b.map_or("no battery".into(), |b| format!("{:.0}%", b.percent))),
        );
        line(
            "Charge limit",
            match sys::charge_limit().await {
                Some((_, on)) => Ok(if on { "on".into() } else { "off".into() }),
                None => Err("not supported by this battery or UPower".into()),
            },
        );
        line("Power profiles", shown(sys::profiles().await, |p| format!("{} of {}", p.active, p.available.join(", "))));
        line(
            "Screen brightness",
            shown(sys::brightness().await, |b| b.map_or("none".into(), |b| format!("{:.0}%", b * 100.0))),
        );
        line(
            "Keyboard backlight",
            match sys::keyboard_light().await {
                Some((name, level, max)) => Ok(format!("{name}: {level}/{max}")),
                None => Err("none found".into()),
            },
        );
        let sound = audio::read().await;
        line(
            "Sound (wpctl, pactl)",
            if sound.outputs.is_empty() {
                Err("no outputs: is PipeWire running, with wpctl and pactl?".into())
            } else {
                Ok(format!("{} output(s), volume {:.0}%", sound.outputs.len(), sound.volume * 100.0))
            },
        );
        line(
            "Calendars (EDS)",
            match crate::bus::session().await {
                Ok(conn) => shown(calendar::open_all(&conn, "Calendar", "OpenCalendar").await, |c| format!("{} calendar(s)", c.len())),
                Err(err) => Err(err.to_string()),
            },
        );
        line(
            "Task lists (EDS)",
            match tasks::open().await {
                Some(tasks) => Ok(format!("{} open to-do(s)", tasks.len())),
                None => Err("no task service".into()),
            },
        );
        line(
            "Media (MPRIS)",
            Ok(crate::media::read().await.map_or("nothing playing".into(), |m| format!("{} — {}", m.artist, m.title))),
        );
        line(
            "Game Mode",
            match desktop::gamemode().await {
                Some(on) => Ok(if on { "on".into() } else { "off".into() }),
                None => Err("not installed".into()),
            },
        );
        let layouts = keyboard::layouts();
        line(
            "Keyboard layouts",
            if layouts.is_empty() {
                Err("none set in COSMIC".into())
            } else {
                Ok(layouts.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", "))
            },
        );
    });
}
