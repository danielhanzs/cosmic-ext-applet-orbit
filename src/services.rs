// SPDX-License-Identifier: GPL-3.0-only

//! systemd's services, the system's or the user's, through the host's
//! `systemctl`: their state, starting, stopping and restarting them (polkit
//! asks for a password where the system's need one), and their journal.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// "nginx.service"
    pub name: String,
    pub description: String,
    /// "active", "inactive", "failed", "activating"…
    pub active: String,
    /// "running", "dead", "exited"…
    pub sub: String,
}

impl Unit {
    pub fn running(&self) -> bool {
        matches!(self.active.as_str(), "active" | "activating" | "reloading")
    }

    pub fn failed(&self) -> bool {
        self.active == "failed"
    }

    /// Its name without ".service".
    pub fn short(&self) -> &str {
        self.name.strip_suffix(".service").unwrap_or(&self.name)
    }
}

async fn systemctl(user: bool, args: &[&str]) -> Result<String, String> {
    let mut command = crate::host::command("systemctl");
    if user {
        command.arg("--user");
    }
    let out = command.args(args).arg("--no-pager").kill_on_drop(true).output();
    // Starting one may wait on a password, and on the service.
    match tokio::time::timeout(std::time::Duration::from_secs(120), out).await {
        Ok(Ok(out)) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(Ok(out)) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err("no answer".into()),
    }
}

/// The services: the ones loaded, with their state, and the installed ones
/// that aren't (to start). None: no systemctl to ask.
pub async fn list(user: bool) -> Option<Vec<Unit>> {
    let (loaded, files) = tokio::join!(
        systemctl(user, &["list-units", "--type=service", "--all", "--output=json"]),
        systemctl(user, &["list-unit-files", "--type=service", "--output=json"]),
    );
    Some(merge(&loaded.ok()?, &files.unwrap_or_default()))
}

fn merge(loaded: &str, files: &str) -> Vec<Unit> {
    let mut units: BTreeMap<String, Unit> = BTreeMap::new();
    let rows = |text: &str| serde_json::from_str::<Vec<serde_json::Value>>(text).unwrap_or_default();
    for u in rows(loaded) {
        let field = |key: &str| u[key].as_str().unwrap_or_default().to_string();
        let name = field("unit");
        // "not-found": a name something wants, with nothing installed.
        if name.is_empty() || field("load") == "not-found" {
            continue;
        }
        units.insert(name.clone(), Unit { name, description: field("description"), active: field("active"), sub: field("sub") });
    }
    for f in rows(files) {
        let name = f["unit_file"].as_str().unwrap_or_default();
        let state = f["state"].as_str().unwrap_or_default();
        // Templates need an instance to start; masked ones can't be.
        if name.is_empty() || name.contains("@.") || state == "masked" || units.contains_key(name) {
            continue;
        }
        units.insert(
            name.to_string(),
            Unit { name: name.to_string(), description: String::new(), active: "inactive".into(), sub: state.into() },
        );
    }
    units.into_values().collect()
}

/// "start", "stop" or "restart"; what went wrong, if it did.
pub async fn act(user: bool, action: &'static str, unit: String) -> Result<(), String> {
    systemctl(user, &[action, "--", &unit]).await.map(drop)
}

/// Its journal, followed, in a terminal.
pub fn logs(user: bool, unit: &str) {
    let mut args = vec!["-e", "journalctl"];
    if user {
        args.push("--user");
    }
    args.extend(["--follow", "--lines", "200", "--unit", unit]);
    crate::host::spawn("cosmic-term", &args);
}

/// Those whose name or description has `query` (any case); the failed ones
/// first, then the running.
pub fn search<'a>(units: &'a [Unit], query: &str) -> Vec<&'a Unit> {
    let query = query.trim().to_lowercase();
    let mut found: Vec<&Unit> = units
        .iter()
        .filter(|u| u.name.to_lowercase().contains(&query) || u.description.to_lowercase().contains(&query))
        .collect();
    found.sort_by_key(|u| (!u.failed(), !u.running(), u.name.clone()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_finds() {
        let loaded = r#"[{"unit":"nginx.service","load":"loaded","active":"active","sub":"running","description":"A web server"},
            {"unit":"backup.service","load":"loaded","active":"failed","sub":"failed","description":"Nightly backup"},
            {"unit":"ghost.service","load":"not-found","active":"inactive","sub":"dead","description":"ghost.service"}]"#;
        let files = r#"[{"unit_file":"nginx.service","state":"enabled"},{"unit_file":"redis.service","state":"disabled"},
            {"unit_file":"getty@.service","state":"enabled"},{"unit_file":"old.service","state":"masked"}]"#;
        let units = merge(loaded, files);
        let names: Vec<&str> = units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["backup.service", "nginx.service", "redis.service"]);
        assert!(units[1].running() && units[0].failed() && !units[2].running());
        assert_eq!(units[2].sub, "disabled");
        let found: Vec<&str> = search(&units, "SERV").iter().map(|u| u.short()).collect();
        assert_eq!(found, ["backup", "nginx", "redis"]);
        assert_eq!(search(&units, "web").len(), 1);
    }
}
