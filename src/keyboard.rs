// SPDX-License-Identifier: GPL-3.0-only

//! The keyboard layouts set in COSMIC Settings, named as xkb names them.
//! Which one is in use, and switching, go through the compositor (see
//! workspaces.rs, which holds the connection).

use cosmic::cosmic_config::{Config, ConfigGet};

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// "hu", "us"…
    pub code: String,
    pub variant: String,
    /// "Hungarian", "English (Dvorak)"…
    pub name: String,
}

/// What cosmic-comp keeps; only the layouts are read.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Xkb {
    layout: String,
    variant: String,
}

pub fn layouts() -> Vec<Layout> {
    let xkb: Xkb = Config::new("com.system76.CosmicComp", 1)
        .and_then(|c| c.get("xkb_config"))
        .unwrap_or_default();
    let names = std::fs::read_to_string("/usr/share/X11/xkb/rules/base.lst").unwrap_or_default();
    parse(&xkb.layout, &xkb.variant, &names)
}

/// "us,hu" and ",dvorak" (as many, or fewer, variants) → layouts, named from
/// xkb's base.lst; a name it doesn't have: the code.
fn parse(layouts: &str, variants: &str, names: &str) -> Vec<Layout> {
    let mut variants = variants.split(',');
    layouts
        .split(',')
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(|code| {
            let variant = variants.next().unwrap_or("").trim().to_string();
            let name = describe(names, code, &variant).unwrap_or_else(|| {
                if variant.is_empty() { code.to_uppercase() } else { format!("{} ({variant})", code.to_uppercase()) }
            });
            Layout { code: code.to_string(), variant, name }
        })
        .collect()
}

/// base.lst: "! layout" lines are "  hu  Hungarian", "! variant" lines
/// "  dvorak  us: English (Dvorak)".
fn describe(names: &str, code: &str, variant: &str) -> Option<String> {
    let mut section = "";
    for line in names.lines() {
        if let Some(name) = line.strip_prefix("! ") {
            section = name.trim();
            continue;
        }
        let Some((name, rest)) = line.trim().split_once(char::is_whitespace) else { continue };
        let rest = rest.trim();
        match section {
            "layout" if variant.is_empty() && name == code => return Some(rest.to_string()),
            "variant" if name == variant => {
                if let Some(description) = rest.strip_prefix(code).and_then(|r| r.strip_prefix(": ")) {
                    return Some(description.to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_layouts() {
        let names = "! layout\n  us              English (US)\n  hu              Hungarian\n\
                     ! variant\n  dvorak          gb: English (UK, Dvorak)\n  dvorak          us: English (Dvorak)\n";
        let got = parse("hu,us,xx", ",dvorak", names);
        let names: Vec<_> = got.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Hungarian", "English (Dvorak)", "XX"]);
        assert_eq!(got[1].variant, "dvorak");
        assert!(parse("", "", "").is_empty());
    }
}
