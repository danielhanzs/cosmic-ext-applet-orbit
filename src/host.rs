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

/// Runs a host program to its end, at most `secs` seconds (then it's
/// ended): what it printed, or what went wrong (its error output).
pub async fn output(program: &str, args: &[&str], secs: u64) -> Result<String, String> {
    let run = command(program).args(args).kill_on_drop(true).output();
    match tokio::time::timeout(std::time::Duration::from_secs(secs), run).await {
        Ok(Ok(out)) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(Ok(out)) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            // Silent failures ("not found" from `command -v`) are answers.
            if !err.is_empty() {
                tracing::warn!("{program} {}: {err}", args.join(" "));
            }
            Err(err)
        }
        Ok(Err(err)) => Err(format!("{program}: {err}")),
        Err(_) => {
            tracing::warn!("{program} {}: no answer in {secs} s", args.join(" "));
            Err(format!("{program}: no answer"))
        }
    }
}

/// `N` bytes from the kernel's random source; none without one.
pub fn random<const N: usize>() -> Option<[u8; N]> {
    use std::io::Read;
    let mut bytes = [0u8; N];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).ok()?;
    Some(bytes)
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
