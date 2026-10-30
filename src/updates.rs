// SPDX-License-Identifier: GPL-3.0-only

//! Updates waiting: system packages, as apt's lists know them (refreshed by
//! the system's own updater), and Flatpaks.

use crate::host;

async fn lines(program: &str, args: &[&str]) -> Vec<String> {
    let Ok(out) = host::command(program).args(args).output().await else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

/// How many updates wait, packages and Flatpaks together.
pub async fn count() -> usize {
    let (apt, flatpak) = tokio::join!(
        lines("apt", &["list", "--upgradable"]),
        lines("flatpak", &["remote-ls", "--updates", "--columns=application"]),
    );
    let packages = apt.iter().filter(|l| l.contains("upgradable from")).count();
    let mut apps: Vec<_> = flatpak.into_iter().filter(|l| !l.trim().is_empty()).collect();
    // The same app in the user's and the system's installation.
    apps.sort();
    apps.dedup();
    packages + apps.len()
}
