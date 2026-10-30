// SPDX-License-Identifier: GPL-3.0-only

//! The media player to show: MPRIS, the one playing, else the one paused.

use std::{collections::HashMap, path::PathBuf};

use cosmic::widget::image;
use zbus::zvariant::OwnedValue;

const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PATH: &str = "/org/mpris/MediaPlayer2";
/// Covers kept in the cache; older ones go.
const CACHED_COVERS: usize = 100;
/// The largest cover read or downloaded.
const MAX_COVER: u64 = 5_000_000;

#[derive(Debug, Clone)]
pub struct Media {
    pub bus: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub art: Option<image::Handle>,
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
    let mut best: Option<(Media, String)> = None;
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
            art: None,
        };
        if media.title.is_empty() {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| media.playing && !b.playing) {
            best = Some((media, string("mpris:artUrl")));
        }
    }
    // Only the shown player's cover is fetched.
    let (mut media, url) = best?;
    media.art = art(&url).await;
    Some(media)
}

/// The cover, read anew each time: players that rewrite one file for every
/// track show the right one.
async fn art(url: &str) -> Option<image::Handle> {
    let bytes = if let Some(path) = url.strip_prefix("file://") {
        let read = read_cover(percent_decoded(path));
        tokio::time::timeout(std::time::Duration::from_secs(3), read).await.ok()??
    } else if url.starts_with("https://") {
        tokio::fs::read(download(url).await?).await.ok()?
    } else {
        return None;
    };
    Some(image::Handle::from_bytes(bytes))
}

/// A local cover: a regular file of at most MAX_COVER bytes, not whatever a
/// player names (a device, a huge file).
async fn read_cover(path: PathBuf) -> Option<Vec<u8>> {
    let bytes = if crate::host::in_flatpak() {
        // A sandbox doesn't see the player's files (its /tmp, its home).
        let out = crate::host::command("sh")
            .args(["-c", "test -f \"$1\" && head -c \"$2\" -- \"$1\"", "sh"])
            .arg(&path)
            .arg((MAX_COVER + 1).to_string())
            .output()
            .await
            .ok()?;
        out.status.success().then_some(out.stdout)?
    } else {
        use tokio::io::AsyncReadExt;
        let file = tokio::fs::File::open(&path).await.ok()?;
        if !file.metadata().await.ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_COVER + 1).read_to_end(&mut bytes).await.ok()?;
        bytes
    };
    (bytes.len() as u64 <= MAX_COVER).then_some(bytes)
}

/// An HTTPS cover, downloaded once into the cache.
async fn download(url: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
        .join(crate::APP_ID);
    let file = dir.join(format!("{:016x}", fnv1a(url.as_bytes())));
    if file.exists() {
        return Some(file);
    }
    tokio::fs::create_dir_all(&dir).await.ok()?;
    // Into a part file first: a cut-off download never passes for a cover.
    let part = file.with_extension("part");
    // The player is a local app the user runs; its URL is trusted as much
    // as the app, but curl still gets only plain HTTPS, small files.
    let ok = crate::host::command("curl")
        .args(["-sfL", "--globoff", "--proto", "=https", "--proto-redir", "=https"])
        .args(["--max-time", "5", "--max-filesize", &MAX_COVER.to_string(), "-o"])
        .arg(&part)
        .arg(url)
        .status()
        .await
        .is_ok_and(|s| s.success());
    if !ok || tokio::fs::rename(&part, &file).await.is_err() {
        _ = tokio::fs::remove_file(&part).await;
        return None;
    }
    prune(&dir).await;
    Some(file)
}

/// Keeps the newest covers only.
async fn prune(dir: &std::path::Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return };
    let mut files = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Ok(modified) = entry.metadata().await.and_then(|m| m.modified()) {
            files.push((modified, entry.path()));
        }
    }
    if files.len() > CACHED_COVERS {
        files.sort();
        for (_, path) in &files[..files.len() - CACHED_COVERS] {
            _ = tokio::fs::remove_file(path).await;
        }
    }
}

/// A hash that stays the same across Rust versions, for file names.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// "My%20Album/c%C3%B3ver.jpg" → "My Album/cóver.jpg"
fn percent_decoded(path: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

pub async fn control(bus: String, method: &'static str) {
    if let Ok(conn) = zbus::Connection::session().await {
        _ = conn.call_method(Some(bus.as_str()), PATH, Some(PLAYER), method, &()).await;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_paths() {
        assert_eq!(super::percent_decoded("/m/My%20Album/c%C3%B3ver.jpg"), std::path::PathBuf::from("/m/My Album/cóver.jpg"));
        assert_eq!(super::percent_decoded("/a/100%/b%2"), std::path::PathBuf::from("/a/100%/b%2"));
    }

    #[tokio::test]
    async fn covers_are_regular_small_files() {
        assert!(super::read_cover("/dev/zero".into()).await.is_none());
        let dir = std::env::temp_dir().join(format!("orbit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let small = dir.join("cover.jpg");
        std::fs::write(&small, b"jpeg").unwrap();
        assert_eq!(super::read_cover(small).await.as_deref(), Some(&b"jpeg"[..]));
        let big = dir.join("big.jpg");
        std::fs::write(&big, vec![0; super::MAX_COVER as usize + 1]).unwrap();
        assert!(super::read_cover(big).await.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stable_names() {
        assert_eq!(super::fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(super::fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
