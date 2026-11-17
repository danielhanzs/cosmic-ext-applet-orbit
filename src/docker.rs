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
    /// Where a browser reaches it: "http://localhost:8080", if published.
    pub web: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    /// The Compose project; empty: containers outside Compose.
    pub name: String,
    /// The project's folder, where its compose file is.
    pub folder: Option<String>,
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
    let mut stacks: BTreeMap<String, (Option<String>, Vec<Container>)> = BTreeMap::new();
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
        let folder = Some(label("com.docker.compose.project.working_dir")).filter(|f| !f.is_empty());
        let entry = stacks.entry(project).or_default();
        entry.0 = entry.0.take().or(folder);
        entry.1.push(Container {
            id: field("ID"),
            service,
            running: field("State") == "running",
            status: field("Status"),
            web: web(&field("Ports")),
        });
    }
    let mut stacks: Vec<Stack> = stacks
        .into_iter()
        .map(|(name, (folder, mut containers))| {
            containers.sort_by(|a, b| a.service.cmp(&b.service));
            Stack { name, folder, containers }
        })
        .collect();
    // Running ones first, then by name; those outside Compose last.
    stacks.sort_by_key(|s| (s.name.is_empty(), s.running() == 0, s.name.clone()));
    stacks
}

/// The first published port a browser can use, from "0.0.0.0:8080->80/tcp,
/// [::]:8080->80/tcp, 0.0.0.0:1433->1433/tcp": databases and queues left out.
fn web(ports: &str) -> Option<String> {
    const NOT_WEB: [u16; 14] = [22, 25, 53, 1433, 1521, 3306, 5432, 5672, 6379, 9042, 11211, 27017, 9092, 2181];
    ports.split(", ").find_map(|mapping| {
        let (host, inside) = mapping.split_once("->")?;
        let (inside, protocol) = inside.split_once('/')?;
        let (host, inside) = (host.rsplit(':').next()?.parse::<u16>().ok()?, inside.parse::<u16>().ok()?);
        if protocol != "tcp" || NOT_WEB.contains(&inside) {
            return None;
        }
        let scheme = if matches!(inside, 443 | 8443) { "https" } else { "http" };
        Some(format!("{scheme}://localhost:{host}"))
    })
}

/// Starts, stops or restarts a container, by id; done when docker is.
pub async fn container(action: &'static str, id: String) {
    run(&[action, &id]).await;
}

/// Starts, stops or restarts a Compose stack's containers, by its name
/// (its compose file isn't needed: Docker knows them by their labels).
pub async fn stack(action: &'static str, name: String) {
    run(&["compose", "--project-name", &name, action]).await;
}

async fn run(args: &[&str]) {
    let out = crate::host::command("docker").args(args).kill_on_drop(true).output();
    match tokio::time::timeout(std::time::Duration::from_secs(120), out).await {
        Ok(Ok(out)) if out.status.success() => {}
        Ok(Ok(out)) => tracing::warn!("docker {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()),
        _ => tracing::warn!("docker {}: no answer", args.join(" ")),
    }
}

/// A terminal in the stack's folder, for what the card doesn't do.
pub fn terminal_in(folder: &str) {
    crate::host::spawn("cosmic-term", &["--working-directory", folder]);
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
        assert_eq!(web("0.0.0.0:1433->1433/tcp, [::]:1433->1433/tcp, 0.0.0.0:8080->80/tcp"), Some("http://localhost:8080".into()));
        assert_eq!(web("0.0.0.0:8443->443/tcp"), Some("https://localhost:8443".into()));
        assert_eq!(web("9000/tcp"), None);
        assert_eq!(web("0.0.0.0:53->53/udp"), None);
    }
}
