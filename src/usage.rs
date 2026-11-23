// SPDX-License-Identifier: GPL-3.0-only

//! The AI coding assistants' usage limits, as CodexBar shows them: Claude
//! Code's and Codex's session and weekly windows. Read only from what the
//! tools themselves write on this computer: no sign-in of theirs is read,
//! nothing is asked of their makers' servers.
//!
//! - Codex writes its limits into its session logs as it works.
//! - Claude Code hands them to its status line command; one line in its
//!   settings keeps a copy for Orbit (see the README):
//!   `tee ~/.claude/orbit-status.json | <the status line as it was>`.

use jiff::Timestamp;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Window {
    /// The rolling few hours (5 for both).
    Session,
    Weekly,
    /// A weekly allowance for one model ("Opus").
    Model(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    pub window: Window,
    /// 0..100
    pub percent: f32,
    pub resets: Option<Timestamp>,
    /// Near the limit.
    pub severe: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub name: &'static str,
    pub limits: Vec<Limit>,
}

impl Provider {
    /// What the panel shows: the session window, or else the first.
    pub fn headline(&self) -> Option<&Limit> {
        self.limits.iter().find(|l| l.window == Window::Session).or(self.limits.first())
    }
}

/// The output of a script run on the host, in the user's home (a Flatpak
/// doesn't see it); nothing for no output.
async fn home(script: &str, arg: &str) -> Option<String> {
    crate::host::output("sh", &["-c", script, "sh", arg], 5).await.ok().filter(|t| !t.trim().is_empty())
}

/// Every assistant that has told its limits, with them.
pub async fn read() -> Vec<Provider> {
    let (claude, codex) = tokio::join!(claude(), codex());
    claude.into_iter().chain(codex).filter(|p| !p.limits.is_empty()).collect()
}

/// A window as last told; one whose reset has passed since starts over.
fn limit(window: Window, percent: f64, resets: Option<Timestamp>) -> Limit {
    let over = resets.is_some_and(|r| r <= Timestamp::now());
    let percent = if over { 0.0 } else { percent.clamp(0.0, 100.0) as f32 };
    Limit { window, percent, resets: resets.filter(|_| !over), severe: percent >= 90.0 }
}

fn title(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------- Claude

async fn claude() -> Option<Provider> {
    let status = home(r#"test -f "$HOME/$1" && head -c 65536 "$HOME/$1""#, ".claude/orbit-status.json").await?;
    let status: Value = serde_json::from_str(&status).ok()?;
    Some(Provider { name: "Claude", limits: claude_limits(&status["rate_limits"]) })
}

/// The status line's `rate_limits`: {"five_hour", "seven_day", (a model's
/// own week) "seven_day_opus"…: {"used_percentage", "resets_at" (Unix
/// seconds)}}; a window is left out until used.
fn claude_limits(limits: &Value) -> Vec<Limit> {
    let mut found: Vec<Limit> = limits
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, w)| {
            let window = match key.as_str() {
                "five_hour" => Window::Session,
                "seven_day" => Window::Weekly,
                other => Window::Model(title(other.strip_prefix("seven_day_")?)),
            };
            let resets = w["resets_at"].as_i64().and_then(|s| Timestamp::from_second(s).ok());
            Some(limit(window, w["used_percentage"].as_f64()?, resets))
        })
        .collect();
    found.sort_by_key(|l| match l.window {
        Window::Session => 0,
        Window::Weekly => 1,
        Window::Model(_) => 2,
    });
    found
}

// ---------------------------------------------------------------- Codex

async fn codex() -> Option<Provider> {
    // The latest of the recent sessions that has told its limits.
    let script = r#"for f in $(ls -t "$HOME"/.codex/sessions/*/*/*/rollout-*.jsonl 2>/dev/null | head -n 5); do
        line=$(grep -F '"rate_limits":{' "$f" | tail -n 1)
        [ -n "$line" ] && { printf '%s' "$line" | head -c 65536; exit 0; }
    done"#;
    let line: Value = serde_json::from_str(&home(script, "").await?).ok()?;
    Some(Provider { name: "Codex", limits: codex_limits(&line) })
}

/// A session log's line: {"timestamp", "payload": {"rate_limits":
/// {"primary", "secondary": {"used_percent", "window_minutes", "resets_at"
/// (Unix seconds), or before that "resets_in_seconds" (from the line's
/// time)}}}}.
fn codex_limits(line: &Value) -> Vec<Limit> {
    let at = line["timestamp"].as_str().and_then(|t| t.parse::<Timestamp>().ok());
    ["primary", "secondary"]
        .iter()
        .filter_map(|key| {
            let w = &line["payload"]["rate_limits"][key];
            let percent = w["used_percent"].as_f64()?;
            // Up to a day long: the session; longer: the week.
            let window = if w["window_minutes"].as_i64().unwrap_or(0) <= 1440 { Window::Session } else { Window::Weekly };
            let resets = match w["resets_at"].as_i64() {
                Some(s) => Timestamp::from_second(s).ok(),
                None => at.zip(w["resets_in_seconds"].as_i64()).and_then(|(at, s)| at.checked_add(jiff::SignedDuration::from_secs(s)).ok()),
            };
            Some(limit(window, percent, resets))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude() {
        let status: Value = serde_json::from_str(
            r#"{"model":{"id":"x"},"rate_limits":{"seven_day":{"used_percentage":91.5,"resets_at":4102444800},
                "five_hour":{"used_percentage":23.5,"resets_at":4102444800},
                "seven_day_opus":{"used_percentage":38,"resets_at":1000},
                "spend_limit":{"used_percentage":62.8,"resets_at":4102444800}}}"#,
        )
        .unwrap();
        let limits = claude_limits(&status["rate_limits"]);
        assert_eq!(limits.iter().map(|l| l.window.clone()).collect::<Vec<_>>(), [Window::Session, Window::Weekly, Window::Model("Opus".into())]);
        assert_eq!((limits[0].percent, limits[0].severe, limits[1].severe), (23.5, false, true));
        // Its reset long past: started over.
        assert_eq!((limits[2].percent, limits[2].resets), (0.0, None));
        assert!(claude_limits(&Value::Null).is_empty());
    }

    #[test]
    fn reads_codex() {
        let line: Value = serde_json::from_str(
            r#"{"timestamp":"2099-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{
                "primary":{"used_percent":12,"window_minutes":300,"resets_in_seconds":600},
                "secondary":{"used_percent":95,"window_minutes":10080,"resets_at":4102444800}}}}"#,
        )
        .unwrap();
        let limits = codex_limits(&line);
        assert_eq!(limits.iter().map(|l| l.window.clone()).collect::<Vec<_>>(), [Window::Session, Window::Weekly]);
        assert_eq!(limits[0].resets, Some("2099-01-01T00:10:00Z".parse().unwrap()));
        assert!(limits[1].severe && !limits[0].severe);
    }
}
