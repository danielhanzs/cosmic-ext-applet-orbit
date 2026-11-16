// SPDX-License-Identifier: GPL-3.0-only

//! External monitors' brightness over DDC/CI, through the host's `ddcutil`
//! (it needs the i2c devices: the `i2c` group, or ddcutil's udev rule).
//! Monitors without a backlight COSMIC can set, on a desktop, mostly.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// ddcutil's number for it.
    pub display: u8,
    pub name: String,
    pub level: u16,
    pub max: u16,
}

async fn ddcutil(args: &[&str]) -> Option<String> {
    // Asking a monitor over i2c takes a while; one that hangs, longer.
    let run = crate::host::command("ddcutil").args(args).kill_on_drop(true).output();
    let out = tokio::time::timeout(Duration::from_secs(10), run).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The monitors that answer, with their brightness; none without ddcutil.
pub async fn monitors() -> Vec<Monitor> {
    let Some(found) = ddcutil(&["detect", "--terse"]).await else { return Vec::new() };
    let mut monitors = Vec::new();
    for (display, name) in parse_detect(&found) {
        let number = display.to_string();
        let Some(level) = ddcutil(&["getvcp", "10", "--display", &number, "--terse"]).await else { continue };
        if let Some((level, max)) = parse_level(&level) {
            monitors.push(Monitor { display, name, level, max });
        }
    }
    monitors
}

pub async fn set(display: u8, level: u16) {
    let (number, level) = (display.to_string(), level.to_string());
    if ddcutil(&["setvcp", "10", &level, "--display", &number, "--noverify"]).await.is_none() {
        tracing::warn!("ddcutil could not set display {number}'s brightness");
    }
}

/// "Display 1" blocks, each with a "Monitor: MFG:Model:Serial" line.
fn parse_detect(text: &str) -> Vec<(u8, String)> {
    let mut found = Vec::new();
    let mut display = None;
    for line in text.lines() {
        if let Some(n) = line.strip_prefix("Display ") {
            display = n.trim().parse().ok();
        } else if line.starts_with(|c: char| !c.is_whitespace()) {
            // "Invalid display" and the like.
            display = None;
        } else if let (Some(n), Some(monitor)) = (display, line.trim().strip_prefix("Monitor:")) {
            let model = monitor.trim().split(':').nth(1).unwrap_or("").trim();
            found.push((n, if model.is_empty() { format!("Display {n}") } else { model.to_string() }));
        }
    }
    found
}

/// "VCP 10 C 50 100": now, most.
fn parse_level(text: &str) -> Option<(u16, u16)> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    match fields.as_slice() {
        ["VCP", "10", "C", level, max, ..] => Some((level.parse().ok()?, max.parse().ok().filter(|m: &u16| *m > 0)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ddcutil() {
        let detect = "Display 1\n   I2C bus:  /dev/i2c-5\n   Monitor:  DEL:DELL P2416D:07C536970JDS\n\n\
                      Invalid display\n   I2C bus:  /dev/i2c-7\n   Monitor:  XXX:Broken:1\n\n\
                      Display 2\n   I2C bus:  /dev/i2c-8\n   Monitor:  GSM::123\n";
        assert_eq!(parse_detect(detect), vec![(1, "DELL P2416D".into()), (2, "Display 2".into())]);
        assert_eq!(parse_level("VCP 10 C 50 100\n"), Some((50, 100)));
        assert_eq!(parse_level("VCP 10 ERR"), None);
        assert_eq!(parse_level("VCP 10 C 5 0"), None);
    }
}
