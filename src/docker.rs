// SPDX-License-Identifier: GPL-3.0-only

//! Docker's containers, through the host's `docker` CLI: grouped into their
//! Compose stacks, each container's log and a shell in it opened in COSMIC's
//! terminal.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub id: String,
    /// Its Compose service, or its own name outside Compose.
    pub service: String,
    pub running: bool,
    /// "Up 2 hours", "Exited (0) 3 days ago"
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    /// The Compose project; empty: containers outside Compose.
    pub name: String,
    pub containers: Vec<Container>,
}

impl Stack {
    pub fn running(&self) -> usize {
        self.containers.iter().filter(|c| c.running).count()
    }
}

/// The stacks, running ones first; None: no Docker to ask (not installed,
/// not running, or not allowed).
pub async fn stacks() -> Option<Vec<Stack>> {
    let out = crate::host::command("docker")
        .args(["ps", "--all", "--no-trunc", "--format", "{{json .}}"])
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(10), out).await.ok()?.ok()?;
    out.status.success().then(|| parse(&String::from_utf8_lossy(&out.stdout)))
}

/// `docker ps --format '{{json .}}'`: one object a line.
fn parse(text: &str) -> Vec<Stack> {
    let mut stacks: BTreeMap<String, Vec<Container>> = BTreeMap::new();
    for line in text.lines() {
        let Ok(c) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let field = |key: &str| c[key].as_str().unwrap_or_default().to_string();
        // "com.docker.compose.project=web,com.docker.compose.service=db,…"
        let labels = field("Labels");
        let label = |key: &str| {
            labels.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')).unwrap_or_default().to_string()
        };
        let project = label("com.docker.compose.project");
        let service = Some(label("com.docker.compose.service")).filter(|s| !s.is_empty()).unwrap_or_else(|| field("Names"));
        stacks.entry(project).or_default().push(Container {
            id: field("ID"),
            service,
            running: field("State") == "running",
            status: field("Status"),
        });
    }
    let mut stacks: Vec<Stack> = stacks
        .into_iter()
        .map(|(name, mut containers)| {
            containers.sort_by(|a, b| a.service.cmp(&b.service));
            Stack { name, containers }
        })
        .collect();
    // Running ones first, then by name; those outside Compose last.
    stacks.sort_by_key(|s| (s.name.is_empty(), s.running() == 0, s.name.clone()));
    stacks
}

/// The container's log, followed, in a terminal.
pub fn logs(id: &str) {
    crate::host::spawn("cosmic-term", &["-e", "docker", "logs", "--follow", "--tail", "500", id]);
}

/// A shell in the container, in a terminal: bash where it has it.
pub fn shell(id: &str) {
    crate::host::spawn(
        "cosmic-term",
        &["-e", "docker", "exec", "-it", id, "sh", "-c", "command -v bash >/dev/null && exec bash || exec sh"],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_into_stacks() {
        let text = r#"{"ID":"a1","Names":"web-db-1","State":"running","Status":"Up 2 hours","Labels":"com.docker.compose.project=web,com.docker.compose.service=db"}
{"ID":"b2","Names":"web-app-1","State":"exited","Status":"Exited (0) 1 day ago","Labels":"com.docker.compose.service=app,com.docker.compose.project=web"}
{"ID":"c3","Names":"lonely","State":"running","Status":"Up 5 minutes","Labels":""}
{"ID":"d4","Names":"old-x-1","State":"exited","Status":"Exited (1)","Labels":"com.docker.compose.project=old,com.docker.compose.service=x"}
"#;
        let stacks = parse(text);
        let names: Vec<&str> = stacks.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["web", "old", ""]);
        assert_eq!(stacks[0].running(), 1);
        assert_eq!(stacks[0].containers[0].service, "app");
        assert_eq!(stacks[2].containers[0].service, "lonely");
    }
}
