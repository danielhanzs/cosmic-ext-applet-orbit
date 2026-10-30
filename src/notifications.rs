// SPDX-License-Identifier: GPL-3.0-only

//! Notifications, as cosmic-notifications hands them to panel applets.
//! Adapted from cosmic-ext-applet-quick-settings.
//!
//! The panel gives applets marked `X-NotificationsApplet` a socket to the
//! daemon in `COSMIC_NOTIFICATIONS`; new notifications come through it. They
//! are dismissed through the usual `org.freedesktop.Notifications`.

use std::{
    collections::{HashMap, HashSet},
    os::unix::io::{FromRawFd, RawFd},
    time::SystemTime,
};

use cosmic::iced::{Subscription, stream};
use futures::{SinkExt, StreamExt, channel::mpsc};
use tokio::sync::OnceCell;
use zbus::{connection::Builder, zvariant::Value};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    /// Pairs of action key and label.
    pub actions: Vec<(String, String)>,
    pub time: SystemTime,
    /// Kept from before the applet started: the daemon has forgotten it, and
    /// its id may be another's now.
    #[serde(default)]
    pub restored: bool,
    /// Asked by its app not to be kept (a transient hint): shown, never saved.
    #[serde(skip)]
    pub transient: bool,
    /// Orbit's own name for it: daemon ids start over when it restarts, so a
    /// kept notification's id may be a live one's.
    #[serde(skip)]
    pub key: u64,
}

/// Notifications older than this are not brought back after a restart.
const KEEP: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

fn history_file() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state"))
        })?;
    Some(dir.join(crate::APP_ID).join("notifications.json"))
}

/// The notifications kept from the last run.
pub fn load() -> Vec<Notification> {
    let Some(file) = history_file() else {
        return Vec::new();
    };
    let Ok(data) = std::fs::read(file) else {
        return Vec::new();
    };
    let mut notifications: Vec<Notification> = serde_json::from_slice(&data).unwrap_or_default();
    notifications.retain(|n| n.time.elapsed().is_ok_and(|age| age < KEEP));
    for notification in &mut notifications {
        notification.restored = true;
        notification.actions.clear();
        // Kept before icons came from the apps' entries, or a file since gone.
        notification.icon = app_icon(&notification.app, None, &notification.icon, None);
        notification.body = plain(&notification.body);
    }
    notifications
}

pub fn save(notifications: &[Notification]) {
    let Some(file) = history_file() else {
        return;
    };
    // Bodies can hold codes and messages: for this user's eyes only.
    let result = (|| {
        use std::{io::Write, os::unix::fs::{DirBuilderExt, OpenOptionsExt}};
        if let Some(dir) = file.parent() {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let kept: Vec<&Notification> = notifications.iter().filter(|n| !n.transient).collect();
        let data = serde_json::to_vec(&kept).map_err(std::io::Error::other)?;
        let part = file.with_extension("part");
        let mut out = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&part)?;
        out.write_all(&data)?;
        std::fs::rename(&part, &file)
    })();
    if let Err(err) = result {
        tracing::warn!("cannot keep the notifications: {err}");
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    New(Notification),
    /// Gone: opened, or closed by its app or from here.
    Closed(u32),
    /// Its popup closed by hand. It stays, as a heads-up swiped away on
    /// Android does, but the daemon has let go of it.
    Dismissed(u32),
}

/// `NotificationClosed`'s reason when its app closed it.
const CLOSED_BY_APP: u32 = 3;

#[zbus::proxy(
    default_service = "com.system76.NotificationsApplet",
    interface = "com.system76.NotificationsApplet",
    default_path = "/com/system76/NotificationsApplet"
)]
trait NotificationsApplet {
    #[zbus(signal)]
    fn notify(
        &self,
        app_name: &str,
        id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<&str>,
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<()>;

    fn invoke_action(&self, id: u32, action: String) -> zbus::Result<()>;
}

#[zbus::proxy(
    default_service = "org.freedesktop.Notifications",
    interface = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: &str) -> zbus::Result<()>;
}

/// What the daemon and the bus tell, before it makes an event.
enum Signal {
    New(Notification),
    Invoked(u32),
    Closed(u32, u32),
}

/// The connection to the daemon, if the panel gave us one.
async fn daemon() -> Option<NotificationsAppletProxy<'static>> {
    static DAEMON: OnceCell<Option<NotificationsAppletProxy<'static>>> = OnceCell::const_new();
    DAEMON
        .get_or_init(|| async {
            let fd: RawFd = std::env::var("COSMIC_NOTIFICATIONS").ok()?.parse().ok()?;
            let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
            stream.set_nonblocking(true).ok()?;
            let stream = tokio::net::UnixStream::from_std(stream).ok()?;
            let conn = Builder::socket(stream).p2p().build().await.ok()?;
            NotificationsAppletProxy::new(&conn).await.ok()
        })
        .await
        .clone()
}

pub fn subscription() -> Subscription<Event> {
    Subscription::run_with("notifications", |_| {
        stream::channel(50, |mut output: mpsc::Sender<Event>| async move {
            let Some(daemon) = daemon().await else {
                tracing::info!("no notifications socket from the panel");
                return futures::future::pending().await;
            };
            let Ok(session) = crate::bus::session().await else {
                return futures::future::pending().await;
            };
            let Ok(freedesktop) = NotificationsProxy::new(&session).await else {
                return futures::future::pending().await;
            };
            let (Ok(notify), Ok(closed), Ok(invoked)) = (
                daemon.receive_notify().await,
                freedesktop.receive_notification_closed().await,
                freedesktop.receive_action_invoked().await,
            ) else {
                return futures::future::pending().await;
            };
            let notify = notify.filter_map(|signal| async move {
                let args = signal.args().ok()?;
                let actions = args
                    .actions
                    .chunks(2)
                    .filter_map(|pair| match pair {
                        [key, label] => Some((key.to_string(), label.to_string())),
                        _ => None,
                    })
                    .collect();
                let hint = |key: &str| match args.hints.get(key) {
                    Some(Value::Str(value)) => Some(value.to_string()),
                    _ => None,
                };
                Some(Signal::New(Notification {
                    id: args.id,
                    app: args.app_name.to_owned(),
                    icon: {
                        // Looking through the apps' entries reads files: off the async threads.
                        let (app, entry, icon, image) = (
                            args.app_name.to_owned(),
                            hint("desktop-entry"),
                            args.app_icon.to_owned(),
                            hint("image-path"),
                        );
                        tokio::task::spawn_blocking(move || app_icon(&app, entry.as_deref(), &icon, image.as_deref()))
                            .await
                            .unwrap_or_default()
                    },
                    summary: args.summary.to_owned(),
                    body: plain(args.body),
                    actions,
                    time: SystemTime::now(),
                    restored: false,
                    key: 0,
                    transient: matches!(args.hints.get("transient"), Some(Value::Bool(true))),
                }))
            });
            let closed = closed.filter_map(|signal| async move {
                let args = signal.args().ok()?;
                Some(Signal::Closed(args.id, args.reason))
            });
            let invoked = invoked.filter_map(|signal| async move {
                Some(Signal::Invoked(signal.args().ok()?.id))
            });
            let mut signals =
                futures::stream::select_all([notify.boxed(), closed.boxed(), invoked.boxed()]);
            let mut opened = HashSet::new();
            while let Some(signal) = signals.next().await {
                let Some(event) = event(&mut opened, signal) else {
                    continue;
                };
                if output.send(event).await.is_err() {
                    return;
                }
            }
            futures::future::pending().await
        })
    })
}

/// What a signal means here. A click on a popup runs its action, then
/// closes it as the user would: those opened go, the others stay.
fn event(opened: &mut HashSet<u32>, signal: Signal) -> Option<Event> {
    Some(match signal {
        Signal::New(notification) => Event::New(notification),
        Signal::Invoked(id) => {
            opened.insert(id);
            return None;
        }
        // Gone for good when opened, or closed by its app; a popup that
        // timed out or was swiped away leaves it in the center.
        Signal::Closed(id, reason) if opened.remove(&id) || reason == CLOSED_BY_APP => Event::Closed(id),
        Signal::Closed(id, _) => Event::Dismissed(id),
    })
}

/// The icon a notification shows, by name or as a file: its app's, from the
/// app's desktop entry, as GNOME shows it; else the one it came with.
/// Browsers send theirs as a `file://` URI, often to a file soon gone.
fn app_icon(app: &str, desktop_entry: Option<&str>, icon: &str, image: Option<&str>) -> String {
    if let Some(icon) = desktop_icon(app, desktop_entry) {
        return icon;
    }
    [Some(icon), image]
        .into_iter()
        .flatten()
        .map(|icon| icon.strip_prefix("file://").unwrap_or(icon))
        .find(|icon| {
            !icon.is_empty() && (!icon.starts_with('/') || std::path::Path::new(icon).exists())
        })
        .unwrap_or_default()
        .to_owned()
}

/// An app's icon from its desktop entry: by the entry's name, which apps
/// may say, or by the app's name.
fn desktop_icon(app: &str, desktop_entry: Option<&str>) -> Option<String> {
    use std::{collections::HashMap, sync::Mutex};
    struct Entry {
        id: String,
        name: String,
        wm_class: String,
        icon: String,
    }
    static ENTRIES: Mutex<Option<(std::time::Instant, Vec<Entry>)>> = Mutex::new(None);
    // Only what was found: an app installed later may have its entry then.
    static FOUND: Mutex<Option<HashMap<(String, Option<String>), Option<String>>>> =
        Mutex::new(None);

    let key = (app.to_owned(), desktop_entry.map(str::to_owned));
    if let Some(found) = FOUND.lock().ok()?.get_or_insert_default().get(&key) {
        return found.clone();
    }
    let mut entries = ENTRIES.lock().ok()?;
    // Read again a minute on, for apps installed since.
    if entries.as_ref().is_some_and(|(at, _)| at.elapsed() > std::time::Duration::from_secs(60)) {
        *entries = None;
    }
    let (_, entries) = entries.get_or_insert_with(|| (std::time::Instant::now(), {
        let home = std::env::var("HOME").unwrap_or_default();
        let data_home = std::env::var("XDG_DATA_HOME")
            .unwrap_or_else(|_| format!("{home}/.local/share"));
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
        let dirs = std::iter::once(data_home)
            .chain(data_dirs.split(':').map(str::to_owned))
            .chain([
                format!("{home}/.local/share/flatpak/exports/share"),
                "/var/lib/flatpak/exports/share".to_owned(),
                // the host's, from a Flatpak sandbox
                "/run/host/usr/share".to_owned(),
            ]);
        let mut entries = Vec::new();
        for dir in dirs {
            let Ok(files) = std::fs::read_dir(format!("{dir}/applications")) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_none_or(|e| e != "desktop") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let mut entry = Entry {
                    id: path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase(),
                    name: String::new(),
                    wm_class: String::new(),
                    icon: String::new(),
                };
                // [Desktop Entry] only, not its actions'.
                for line in text
                    .lines()
                    .skip_while(|l| l.trim() != "[Desktop Entry]")
                    .skip(1)
                    .take_while(|l| !l.starts_with('['))
                {
                    match line.split_once('=') {
                        Some(("Name", value)) => entry.name = value.trim().to_lowercase(),
                        Some(("StartupWMClass", value)) => {
                            entry.wm_class = value.trim().to_lowercase()
                        }
                        Some(("Icon", value)) => entry.icon = value.trim().to_owned(),
                        _ => {}
                    }
                }
                if !entry.icon.is_empty() {
                    entries.push(entry);
                }
            }
        }
        entries
    }));
    let app = app.trim().to_lowercase();
    let wanted = desktop_entry.map(|e| e.trim_end_matches(".desktop").to_lowercase());
    let found = wanted
        .and_then(|id| {
            entries
                .iter()
                .find(|e| e.id == id)
                .or_else(|| entries.iter().find(|e| e.id.rsplit('.').next() == Some(&id)))
        })
        .or_else(|| {
            if app.is_empty() {
                return None;
            }
            let dashed = app.replace(' ', "-");
            entries
                .iter()
                .find(|e| e.name == app)
                .or_else(|| entries.iter().find(|e| e.wm_class == app || e.id == dashed))
                .or_else(|| entries.iter().find(|e| e.id.rsplit('.').next() == Some(&dashed)))
        })
        .map(|e| e.icon.clone());
    if found.is_some() {
        FOUND.lock().ok()?.get_or_insert_default().insert(key, found.clone());
    }
    found
}

/// A body as text: without the markup some apps send (links, bold…), and
/// with its entities read.
fn plain(body: &str) -> String {
    let mut text = String::with_capacity(body.len());
    let mut in_tag = false;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // A tag starts with a letter, '/' or '!': "I <3 you" is text.
            '<' if !in_tag && chars.peek().is_some_and(|n| n.is_ascii_alphabetic() || *n == '/' || *n == '!') => {
                in_tag = true
            }
            '>' if in_tag => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}

/// Run an action, `default` being a click on the notification.
pub async fn invoke(id: u32, action: String) {
    if let Some(daemon) = daemon().await {
        if let Err(err) = daemon.invoke_action(id, action).await {
            tracing::warn!("cannot run notification action: {err}");
        }
    }
}

pub async fn dismiss(ids: Vec<u32>) {
    let Ok(session) = crate::bus::session().await else {
        return;
    };
    let Ok(proxy) = NotificationsProxy::new(&session).await else {
        return;
    };
    for id in ids {
        _ = proxy.close_notification(id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup() {
        assert_eq!(
            plain("Ready: <a href=\"https://claude.ai/\">claude.ai</a> &amp; <b>more</b>"),
            "Ready: claude.ai & more"
        );
        assert_eq!(plain("I <3 you, if a < b"), "I <3 you, if a < b");
    }

    #[test]
    fn popups_closed() {
        let mut opened = HashSet::new();
        let mut close = |signal| event(&mut opened, signal);
        // Closed by hand on its popup: kept.
        assert!(matches!(close(Signal::Closed(1, 2)), Some(Event::Dismissed(1))));
        // Clicked: its action, then closed twice, as cosmic-notifications does.
        assert!(close(Signal::Invoked(2)).is_none());
        assert!(matches!(close(Signal::Closed(2, 2)), Some(Event::Closed(2))));
        assert!(matches!(close(Signal::Closed(2, 2)), Some(Event::Dismissed(2))));
        // Closed by its app: gone.
        assert!(matches!(close(Signal::Closed(3, 3)), Some(Event::Closed(3))));
        // Its popup timed out: kept.
        assert!(matches!(close(Signal::Closed(4, 1)), Some(Event::Dismissed(4))));
    }

    #[test]
    fn sent_icon() {
        assert_eq!(app_icon("", None, "dialog-information", None), "dialog-information");
        assert_eq!(app_icon("", None, "file:///nowhere/logo.png", None), "");
    }
}
