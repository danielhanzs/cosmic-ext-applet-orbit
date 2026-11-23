// SPDX-License-Identifier: GPL-3.0-only

//! The AI coding assistants' usage limits, as CodexBar shows them: Claude
//! Code's and Codex's session and weekly windows, from their own APIs,
//! with the sign-in each tool already has. Only the ones signed in show.
//!
//! Tokens are read from the tools' own files on the host, sent only to the
//! tool's maker over HTTPS (to curl on its standard input, not its command
//! line), never logged, never kept. An expired one isn't refreshed here:
//! that's the tool's job (refreshing rotates the token it relies on).

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
    /// Near or at the limit, as the service says.
    pub severe: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// Signed in once, but the sign-in has run out: the tool renews it.
    Expired,
    /// Asked and failed (offline, refused…).
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub name: &'static str,
    /// "Max", "Team", "Plus"…
    pub plan: String,
    pub limits: Vec<Limit>,
    pub problem: Option<Problem>,
}

impl Provider {
    /// What the panel shows: the session window, or else the first.
    pub fn headline(&self) -> Option<&Limit> {
        self.limits.iter().find(|l| l.window == Window::Session).or(self.limits.first())
    }
}

/// A file of the user's on the host (a Flatpak doesn't see their home).
async fn home_file(path: &str) -> Option<String> {
    let script = r#"test -f "$HOME/$1" && head -c 65536 "$HOME/$1""#;
    crate::host::output("sh", &["-c", script, "sh", path], 5).await.ok().filter(|t| !t.is_empty())
}

/// A JSON document from `url`, with these headers (the token among them).
async fn get(url: &str, headers: &str) -> Result<Value, String> {
    let out = crate::host::output_with_input(
        "curl",
        &["-sS", "--fail-with-body", "--max-time", "15", "--max-filesize", "1000000", "--proto", "=https", "-H", "@-", url],
        headers.as_bytes(),
        20,
    )
    .await?;
    serde_json::from_str(&out).map_err(|e| e.to_string())
}

/// Every provider the user is signed in to, with its limits.
pub async fn read() -> Vec<Provider> {
    let (claude, codex) = tokio::join!(claude(), codex());
    claude.into_iter().chain(codex).collect()
}

fn title(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------- Claude

async fn claude() -> Option<Provider> {
    let credentials: Value = serde_json::from_str(&home_file(".claude/.credentials.json").await?).ok()?;
    let oauth = &credentials["claudeAiOauth"];
    let token = oauth["accessToken"].as_str().filter(|t| !t.is_empty())?;
    let mut provider =
        Provider { name: "Claude", plan: title(oauth["subscriptionType"].as_str().unwrap_or("")), limits: Vec::new(), problem: None };
    // Milliseconds; renewed by Claude Code itself.
    let expires = oauth["expiresAt"].as_i64().unwrap_or(0);
    if expires <= Timestamp::now().as_millisecond() {
        provider.problem = Some(Problem::Expired);
        return Some(provider);
    }
    let headers = format!(
        "Authorization: Bearer {token}\nanthropic-beta: oauth-2025-04-20\nAccept: application/json\nUser-Agent: claude-cli/2.1.0 (external, cli)\n"
    );
    match get("https://api.anthropic.com/api/oauth/usage", &headers).await {
        Ok(usage) => provider.limits = claude_limits(&usage),
        Err(err) if err.contains("401") => provider.problem = Some(Problem::Expired),
        Err(err) => provider.problem = Some(Problem::Failed(err)),
    }
    Some(provider)
}

/// `limits`: [{"kind": "session" | "weekly_all" | "weekly_scoped", "percent",
/// "severity", "resets_at", "scope": {"model": {"display_name"}}}]; older
/// answers only have "five_hour" and "seven_day": {"utilization", "resets_at"}.
fn claude_limits(usage: &Value) -> Vec<Limit> {
    let resets = |v: &Value| v.as_str().and_then(|t| t.parse::<Timestamp>().ok());
    let listed: Vec<Limit> = usage["limits"]
        .as_array()
        .map(|limits| {
            limits
                .iter()
                .filter_map(|l| {
                    let window = match l["kind"].as_str()? {
                        "session" => Window::Session,
                        "weekly_all" => Window::Weekly,
                        "weekly_scoped" => Window::Model(l["scope"]["model"]["display_name"].as_str().unwrap_or("").to_string()),
                        _ => return None,
                    };
                    Some(Limit {
                        window,
                        percent: l["percent"].as_f64()? as f32,
                        resets: resets(&l["resets_at"]),
                        severe: !matches!(l["severity"].as_str(), None | Some("normal")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if !listed.is_empty() {
        return listed;
    }
    [("five_hour", Window::Session), ("seven_day", Window::Weekly)]
        .into_iter()
        .filter_map(|(key, window)| {
            let w = &usage[key];
            let percent = w["utilization"].as_f64()? as f32;
            Some(Limit { window, percent, resets: resets(&w["resets_at"]), severe: percent >= 90.0 })
        })
        .collect()
}

// ---------------------------------------------------------------- Codex

async fn codex() -> Option<Provider> {
    let auth: Value = serde_json::from_str(&home_file(".codex/auth.json").await?).ok()?;
    let token = auth["tokens"]["access_token"].as_str().filter(|t| !t.is_empty())?;
    let mut provider = Provider { name: "Codex", plan: String::new(), limits: Vec::new(), problem: None };
    let mut headers = format!("Authorization: Bearer {token}\nAccept: application/json\nUser-Agent: Orbit\n");
    if let Some(account) = auth["tokens"]["account_id"].as_str().filter(|a| !a.is_empty() && !a.contains(['\r', '\n'])) {
        headers.push_str(&format!("ChatGPT-Account-Id: {account}\n"));
    }
    match get("https://chatgpt.com/backend-api/wham/usage", &headers).await {
        Ok(usage) => {
            provider.plan = title(usage["plan_type"].as_str().unwrap_or(""));
            provider.limits = codex_limits(&usage);
        }
        Err(err) if err.contains("401") => provider.problem = Some(Problem::Expired),
        Err(err) => provider.problem = Some(Problem::Failed(err)),
    }
    Some(provider)
}

/// `rate_limit`: {"primary_window", "secondary_window"}: {"used_percent",
/// "reset_at" (Unix seconds), "limit_window_seconds"}.
fn codex_limits(usage: &Value) -> Vec<Limit> {
    ["primary_window", "secondary_window"]
        .iter()
        .filter_map(|key| {
            let w = &usage["rate_limit"][key];
            let percent = w["used_percent"].as_f64()? as f32;
            // Up to a day long: the session; longer: the week.
            let window = if w["limit_window_seconds"].as_i64().unwrap_or(0) <= 86_400 { Window::Session } else { Window::Weekly };
            let resets = w["reset_at"].as_i64().and_then(|s| Timestamp::from_second(s).ok());
            Some(Limit { window, percent, resets, severe: percent >= 90.0 })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude() {
        let usage: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":24.0,"resets_at":"2026-11-23T11:39:59.687297+00:00"},
                "limits":[{"kind":"session","percent":24,"severity":"normal","resets_at":"2026-11-23T11:39:59.687297+00:00"},
                          {"kind":"weekly_all","percent":70,"severity":"warning","resets_at":"2026-11-24T23:59:59+00:00"},
                          {"kind":"weekly_scoped","percent":38,"severity":"normal","resets_at":null,"scope":{"model":{"display_name":"Opus"}}},
                          {"kind":"something_new","percent":1}]}"#,
        )
        .unwrap();
        let limits = claude_limits(&usage);
        assert_eq!(limits.len(), 3);
        assert_eq!((limits[0].window.clone(), limits[0].percent, limits[0].severe), (Window::Session, 24.0, false));
        assert!(limits[1].severe && limits[0].resets.is_some() && limits[2].resets.is_none());
        assert_eq!(limits[2].window, Window::Model("Opus".into()));
        // An older answer: the two windows.
        let old: Value = serde_json::from_str(r#"{"five_hour":{"utilization":95,"resets_at":null},"seven_day":null}"#).unwrap();
        let limits = claude_limits(&old);
        assert_eq!((limits.len(), limits[0].severe), (1, true));
    }

    #[test]
    fn reads_codex() {
        let usage: Value = serde_json::from_str(
            r#"{"plan_type":"plus","rate_limit":{"primary_window":{"used_percent":12,"reset_at":1795000000,"limit_window_seconds":18000},
                "secondary_window":{"used_percent":91,"reset_at":1795500000,"limit_window_seconds":604800}}}"#,
        )
        .unwrap();
        let limits = codex_limits(&usage);
        assert_eq!(limits.iter().map(|l| l.window.clone()).collect::<Vec<_>>(), [Window::Session, Window::Weekly]);
        assert!(limits[1].severe && limits[0].resets.is_some());
        assert_eq!(title("plus"), "Plus");
    }
}
