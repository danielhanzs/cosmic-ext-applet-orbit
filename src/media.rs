// SPDX-License-Identifier: GPL-3.0-only

//! The media player to show: MPRIS, the one playing, else the one paused.

use std::{collections::HashMap, hash::{Hash, Hasher}, path::PathBuf};

use zbus::zvariant::OwnedValue;

const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PATH: &str = "/org/mpris/MediaPlayer2";

#[derive(Debug, Clone, Default)]
pub struct Media {
    pub bus: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub art: Option<PathBuf>,
}

async fn props(conn: &zbus::Connection, bus: &str) -> zbus::Result<HashMap<String, OwnedValue>> {
    let reply = conn
        .call_method(Some(bus), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &(PLAYER,))
        .await?;
    Ok(reply.body().deserialize()?)
}

pub async fn read() -> Option<Media> {
    let conn = zbus::Connection::session().await.ok()?;
    let names = zbus::fdo::DBusProxy::new(&conn).await.ok()?.list_names().await.ok()?;
    let mut best: Option<Media> = None;
    for name in names.iter().filter(|n| n.starts_with("org.mpris.MediaPlayer2.")) {
        let Ok(props) = props(&conn, name).await else { continue };
        let status = props
            .get("PlaybackStatus")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default();
        if status == "Stopped" {
            continue;
        }
        let meta: HashMap<String, OwnedValue> = props
            .get("Metadata")
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or_default();
        let string = |key: &str| {
            meta.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
        };
        let artist = meta
            .get("xesam:artist")
            .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
            .map(|a| a.join(", "))
            .unwrap_or_default();
        let media = Media {
            bus: name.to_string(),
            title: string("xesam:title"),
            artist,
            playing: status == "Playing",
            art: art(&string("mpris:artUrl")).await,
        };
        if media.title.is_empty() {
            continue;
        }
        let better = best.as_ref().is_none_or(|b| media.playing && !b.playing);
        if better {
            best = Some(media);
        }
    }
    best
}

/// The cover as a local file: as given, or downloaded once into the cache.
async fn art(url: &str) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        return Some(PathBuf::from(path));
    }
    // The player is a local app the user runs; its URL is trusted as much
    // as the app, but curl still gets only plain HTTPS, small files.
    if !url.starts_with("https://") {
        return None;
    }
    let mut hasher = std::hash::DefaultHasher::new();
    url.hash(&mut hasher);
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
        .join(crate::APP_ID);
    let file = dir.join(format!("{:x}", hasher.finish()));
    if !file.exists() {
        tokio::fs::create_dir_all(&dir).await.ok()?;
        let ok = crate::host::command("curl")
            .args(["-sfL", "--globoff", "--proto", "=https", "--proto-redir", "=https"])
            .args(["--max-time", "5", "--max-filesize", "5000000", "-o"])
            .arg(&file)
            .arg(url)
            .status()
            .await
            .ok()?
            .success();
        if !ok {
            return None;
        }
    }
    Some(file)
}

pub async fn control(bus: String, method: &'static str) {
    if let Ok(conn) = zbus::Connection::session().await {
        _ = conn.call_method(Some(bus.as_str()), PATH, Some(PLAYER), method, &()).await;
    }
}
