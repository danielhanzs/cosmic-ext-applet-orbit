// SPDX-License-Identifier: GPL-3.0-only

//! The system's programs and files, from wherever Orbit runs: directly, or
//! from a Flatpak sandbox, which has neither, through `flatpak-spawn --host`.

use std::{path::PathBuf, sync::LazyLock};

static FLATPAK: LazyLock<bool> = LazyLock::new(|| std::path::Path::new("/.flatpak-info").exists());

pub fn in_flatpak() -> bool {
    *FLATPAK
}

/// A host program, to give arguments to. In a sandbox it ends with Orbit.
pub fn command(program: &str) -> tokio::process::Command {
    if in_flatpak() {
        let mut command = tokio::process::Command::new("flatpak-spawn");
        command.args(["--host", "--watch-bus", program]);
        command
    } else {
        tokio::process::Command::new(program)
    }
}

/// Starts a host program and leaves it be.
pub fn spawn(program: &str, args: &[&str]) {
    let result = if in_flatpak() {
        std::process::Command::new("flatpak-spawn").arg("--host").arg(program).args(args).spawn()
    } else {
        std::process::Command::new(program).args(args).spawn()
    };
    match result {
        // Reaped by a thread, not left a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => tracing::warn!("cannot run {program}: {err}"),
    }
}

/// The user's configuration directory on the host: in a sandbox,
/// XDG_CONFIG_HOME points into ~/.var/app instead.
pub fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if in_flatpak() {
        return home.map(|h| h.join(".config"));
    }
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))
}
