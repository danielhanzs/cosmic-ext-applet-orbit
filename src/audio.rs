// SPDX-License-Identifier: GPL-3.0-only

//! Sound through PipeWire's own tools: wpctl for levels, pactl for the
//! outputs, who records, and to hear about changes.

use cosmic::iced::{Subscription, stream};
use futures::SinkExt;
use tokio::io::AsyncBufReadExt;

#[derive(Debug, Clone, Default)]
pub struct Audio {
    pub volume: f32,
    pub muted: bool,
    pub mic_muted: bool,
    pub outputs: Vec<Output>,
    /// Apps using the microphone right now.
    pub recording: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub name: String,
    pub description: String,
    pub default: bool,
}

async fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = crate::host::command(program).args(args).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// "Volume: 0.45 [MUTED]" → (0.45, true)
fn parse_volume(text: &str) -> Option<(f32, bool)> {
    let level = text.split_whitespace().nth(1)?.parse().ok()?;
    Some((level, text.contains("[MUTED]")))
}

pub async fn read() -> Audio {
    let (sink, source, sinks, default, outputs) = tokio::join!(
        run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]),
        run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]),
        run("pactl", &["-f", "json", "list", "sinks"]),
        run("pactl", &["get-default-sink"]),
        run("pactl", &["-f", "json", "list", "source-outputs"]),
    );
    let (volume, muted) = sink.as_deref().and_then(parse_volume).unwrap_or((0.0, true));
    let mic_muted = source.as_deref().and_then(parse_volume).is_none_or(|(_, m)| m);
    let default = default.unwrap_or_default();
    let json = |text: Option<String>| -> Vec<serde_json::Value> {
        text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    };
    let outputs_list = json(sinks)
        .iter()
        .filter_map(|sink| {
            let name = sink["name"].as_str()?.to_string();
            Some(Output {
                default: name == default.trim(),
                description: sink["description"].as_str().unwrap_or(&name).to_string(),
                name,
            })
        })
        .collect();
    let mut recording: Vec<String> = json(outputs)
        .iter()
        .filter(|o| o["properties"]["stream.monitor"].as_str() != Some("true"))
        .filter_map(|o| o["properties"]["application.name"].as_str().map(str::to_string))
        .collect();
    recording.sort();
    recording.dedup();
    Audio { volume, muted, mic_muted, outputs: outputs_list, recording }
}

pub async fn set_volume(level: f32) {
    _ = run("wpctl", &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{level:.2}")]).await;
}

pub async fn toggle_mute(mic: bool) {
    let target = if mic { "@DEFAULT_AUDIO_SOURCE@" } else { "@DEFAULT_AUDIO_SINK@" };
    _ = run("wpctl", &["set-mute", target, "toggle"]).await;
}

pub async fn set_output(name: String) {
    _ = run("pactl", &["set-default-sink", &name]).await;
}

/// Fires on any change pactl reports.
pub fn changes() -> Subscription<()> {
    Subscription::run(|| {
        stream::channel(4, async |mut out| {
            loop {
                let child = crate::host::command("pactl")
                    .arg("subscribe")
                    .stdout(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn();
                if let Ok(mut child) = child {
                    let mut lines = tokio::io::BufReader::new(child.stdout.take().expect("piped")).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if !line.contains("client") {
                            _ = out.send(()).await;
                        }
                    }
                }
                // PipeWire restarted, or pactl is missing: try again later.
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        })
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn volume() {
        assert_eq!(super::parse_volume("Volume: 0.45\n"), Some((0.45, false)));
        assert_eq!(super::parse_volume("Volume: 1.00 [MUTED]\n"), Some((1.0, true)));
        assert_eq!(super::parse_volume(""), None);
    }
}

/// Apps with a camera open: directly, as /proc shows, or through PipeWire
/// (the camera portal), where PipeWire holds the device for them.
pub async fn camera() -> Vec<String> {
    let holders = if crate::host::in_flatpak() {
        host_camera_holders().await
    } else {
        tokio::task::spawn_blocking(camera_holders).await.unwrap_or_default()
    };
    let mut apps: Vec<String> = holders
        .iter()
        .filter(|name| !matches!(name.as_str(), "pipewire" | "wireplumber"))
        .cloned()
        .collect();
    if holders.len() != apps.len() {
        apps.extend(pipewire_camera_users().await);
    }
    apps.sort();
    apps.dedup();
    apps
}

/// The same, asked of the host: a sandbox sees only its own processes.
async fn host_camera_holders() -> Vec<String> {
    // One find over all fds, not a process per process; then the names.
    const SCRIPT: &str = "find /proc/[0-9]*/fd -lname '/dev/video*' 2>/dev/null | cut -d/ -f3 | sort -u | while read p; do cat /proc/$p/comm; done";
    run("sh", &["-c", SCRIPT])
        .await
        .map(|out| out.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Names of this user's processes holding a /dev/video* open.
fn camera_holders() -> Vec<String> {
    let Ok(procs) = std::fs::read_dir("/proc") else { return Vec::new() };
    procs
        .flatten()
        .filter(|p| p.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter(|p| {
            std::fs::read_dir(p.path().join("fd")).is_ok_and(|fds| {
                fds.flatten().any(|fd| {
                    std::fs::read_link(fd.path())
                        .is_ok_and(|t| t.to_string_lossy().starts_with("/dev/video"))
                })
            })
        })
        .filter_map(|p| std::fs::read_to_string(p.path().join("comm")).ok())
        .map(|name| name.trim().to_string())
        .collect()
}

/// Streams linked, and running, from a camera node.
async fn pipewire_camera_users() -> Vec<String> {
    let Some(dump) = run("pw-dump", &[]).await else { return Vec::new() };
    let objects: Vec<serde_json::Value> = serde_json::from_str(&dump).unwrap_or_default();
    let props = |o: &serde_json::Value| o["info"]["props"].clone();
    let cameras: Vec<u64> = objects
        .iter()
        .filter(|o| props(o)["media.class"] == "Video/Source")
        .filter_map(|o| o["id"].as_u64())
        .collect();
    objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Link" && o["info"]["state"] == "active")
        .filter(|o| o["info"]["output-node-id"].as_u64().is_some_and(|id| cameras.contains(&id)))
        .filter_map(|link| {
            let input = link["info"]["input-node-id"].as_u64()?;
            let node = objects.iter().find(|o| o["id"].as_u64() == Some(input))?;
            let p = props(node);
            p["application.name"].as_str().or(p["node.name"].as_str()).map(str::to_string)
        })
        .collect()
}
