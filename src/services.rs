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

/// `systemctl [--user] --no-pager <args>`: the options before the
/// arguments, which may end with `--` and a unit.
fn argv<'a>(user: bool, args: &[&'a str]) -> Vec<&'a str> {
    let mut argv = Vec::with_capacity(args.len() + 2);
    if user {
        argv.push("--user");
    }
    argv.push("--no-pager");
    argv.extend_from_slice(args);
    argv
}

async fn systemctl(user: bool, args: &[&str]) -> Result<String, String> {
    // Starting one may wait on a password, and on the service.
    crate::host::output("systemctl", &argv(user, args), 120).await
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
        // Nothing after the unit: after `--`, systemctl takes all as units.
        assert_eq!(argv(true, &["stop", "--", "a.service"]), ["--user", "--no-pager", "stop", "--", "a.service"]);
    }
}
