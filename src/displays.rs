// SPDX-License-Identifier: GPL-3.0-only

//! The displays, through the host's `cosmic-randr`: on or off, their scale,
//! one mirroring another.

#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// The connector: "eDP-1", "DP-3".
    pub name: String,
    /// Its make and model, or nothing.
    pub model: String,
    pub enabled: bool,
    pub scale: f32,
    /// The current mode: width, height, refresh in mHz.
    pub mode: Option<(u32, u32, u32)>,
}

impl Display {
    /// A laptop's own screen, by its connector.
    pub fn builtin(&self) -> bool {
        ["eDP", "LVDS", "DSI"].iter().any(|p| self.name.starts_with(p))
    }
}

async fn randr(args: &[&str]) -> Option<String> {
    let out = crate::host::command("cosmic-randr").args(args).kill_on_drop(true).output().await.ok()?;
    if !out.status.success() {
        tracing::warn!("cosmic-randr {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub async fn list() -> Vec<Display> {
    randr(&["list", "--kdl"]).await.map(|kdl| parse(&kdl)).unwrap_or_default()
}

pub async fn set_enabled(name: String, on: bool) {
    randr(&[if on { "enable" } else { "disable" }, &name]).await;
}

pub async fn set_scale(display: Display, scale: f32) {
    let Some((width, height, refresh)) = display.mode else { return };
    let (width, height) = (width.to_string(), height.to_string());
    let refresh = format!("{:.3}", refresh as f32 / 1000.0);
    let scale = format!("{scale:.2}");
    randr(&["mode", &display.name, &width, &height, "--refresh", &refresh, "--scale", &scale]).await;
}

/// `name` shows what `from` shows.
pub async fn mirror(name: String, from: String) {
    randr(&["mirror", &name, &from]).await;
}

/// `cosmic-randr list --kdl`:
/// ```text
/// output "DP-3" enabled=#true {
///   description make="Dell Inc." model="DELL P2416D"
///   scale 1.00
///   modes {
///     mode 2560 1440 59951 current=#true preferred=#true
/// ```
fn parse(kdl: &str) -> Vec<Display> {
    let quoted = |text: &str, key: &str| -> Option<String> {
        let start = text.find(&format!("{key}=\""))? + key.len() + 2;
        Some(text[start..].split('"').next()?.to_string())
    };
    let mut displays: Vec<Display> = Vec::new();
    for line in kdl.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("output ") {
            let name = rest.split('"').nth(1).unwrap_or_default().to_string();
            displays.push(Display { name, model: String::new(), enabled: rest.contains("enabled=#true"), scale: 1.0, mode: None });
            continue;
        }
        let Some(display) = displays.last_mut() else { continue };
        if line.starts_with("description ") {
            let parts: Vec<String> = ["make", "model"].iter().filter_map(|k| quoted(line, k)).filter(|s| !s.is_empty()).collect();
            display.model = parts.join(" ");
        } else if let Some(scale) = line.strip_prefix("scale ") {
            display.scale = scale.trim().parse().unwrap_or(1.0);
        } else if line.starts_with("mode ") && line.contains("current=#true") {
            let n: Vec<u32> = line.split_whitespace().skip(1).take(3).filter_map(|f| f.parse().ok()).collect();
            if let [w, h, r] = n[..] {
                display.mode = Some((w, h, r));
            }
        }
    }
    displays
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_randr() {
        let kdl = r#"output "eDP-1" enabled=#true {
  description model=""
  scale 1.25
  modes {
    mode 2560 1600 165001 current=#true preferred=#true
    mode 2560 1600 60000
  }
}
output "DP-3" enabled=#false {
  description make="Dell Inc." model="DELL P2416D"
  scale 1.00
}
"#;
        let d = parse(kdl);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].name.as_str(), d[0].model.as_str(), d[0].enabled, d[0].scale), ("eDP-1", "", true, 1.25));
        assert_eq!(d[0].mode, Some((2560, 1600, 165001)));
        assert_eq!((d[1].model.as_str(), d[1].enabled, d[1].mode), ("Dell Inc. DELL P2416D", false, None));
    }
}
