// SPDX-License-Identifier: GPL-3.0-only

//! A NetworkManager secret agent. Connections whose passwords are
//! "agent-owned" (VPNs, Wi-Fi saved for this user only) can only come up
//! while some agent answers NetworkManager's GetSecrets; COSMIC Settings is
//! one, but only while it is open. This one lives as long as the applet: it
//! answers from the keyring, or asks in the popup and may save the answer.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use cosmic::iced::{Subscription, stream};
use futures::{SinkExt, StreamExt, channel::{mpsc, oneshot}};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

/// What the user typed: the password, and whether to keep it in the keyring.
pub type Answer = Option<(String, bool)>;

/// What the agent tells the popup.
#[derive(Clone, Debug)]
pub enum Event {
    /// A password to ask for.
    Ask(Request),
    /// NetworkManager no longer wants the password for this connection path.
    Cancel(String),
}

/// A password the agent waits for, shown in the popup.
#[derive(Clone)]
pub struct Request {
    pub connection: String,
    /// The connection's object path, which a cancel names.
    pub path: String,
    reply: Arc<Mutex<Option<oneshot::Sender<Answer>>>>,
}

impl Request {
    pub fn answer(&self, answer: Answer) {
        if let Some(reply) = self.reply.lock().ok().and_then(|mut r| r.take()) {
            _ = reply.send(answer);
        }
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("connection", &self.connection).finish()
    }
}

struct Agent {
    requests: mpsc::Sender<Event>,
}

/// Anyone on the system bus can call us: NetworkManager only is answered.
async fn from_network_manager(header: &zbus::message::Header<'_>, conn: &zbus::Connection) -> zbus::fdo::Result<()> {
    let nm = zbus::fdo::DBusProxy::new(conn)
        .await?
        .get_name_owner("org.freedesktop.NetworkManager".try_into().map_err(zbus::Error::from)?)
        .await?;
    if header.sender().map(|s| s.as_str()) != Some(nm.as_str()) {
        return Err(zbus::fdo::Error::AccessDenied("only NetworkManager may ask".into()));
    }
    Ok(())
}

// NM_SECRET_AGENT_GET_SECRETS_FLAG_*
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;

fn text(settings: &Settings, group: &str, key: &str) -> Option<String> {
    String::try_from(settings.get(group)?.get(key)?.try_clone().ok()?).ok()
}

/// The one secret this agent knows to give for a setting.
fn secret_key(setting: &str) -> Option<&'static str> {
    match setting {
        "vpn" => Some("password"),
        "802-11-wireless-security" => Some("psk"),
        "802-1x" => Some("password"),
        _ => None,
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        flags: u32,
    ) -> zbus::fdo::Result<Settings> {
        from_network_manager(&header, conn).await?;
        let no = || zbus::fdo::Error::Failed("no secrets".into());
        let key = secret_key(&setting_name).ok_or_else(no)?;
        let uuid = text(&connection, "connection", "uuid").ok_or_else(no)?;
        let name = text(&connection, "connection", "id").unwrap_or_default();

        let mut secret = None;
        if flags & REQUEST_NEW == 0 {
            secret = keyring::find(&uuid, &setting_name, key).await;
        }
        if secret.is_none() && flags & ALLOW_INTERACTION != 0 {
            let (tx, rx) = oneshot::channel();
            let request = Request {
                connection: name.clone(),
                path: connection_path.to_string(),
                reply: Arc::new(Mutex::new(Some(tx))),
            };
            if self.requests.clone().send(Event::Ask(request)).await.is_ok() {
                // NetworkManager gives up after two minutes.
                if let Ok(Ok(Some((password, remember)))) =
                    tokio::time::timeout(Duration::from_secs(110), rx).await
                {
                    if remember {
                        keyring::store(&name, &uuid, &setting_name, key, &password).await;
                    }
                    secret = Some(password);
                }
            }
        }
        let secret = secret.ok_or_else(|| zbus::fdo::Error::Failed("user canceled".into()))?;

        let value: OwnedValue = if setting_name == "vpn" {
            Value::from(HashMap::from([(key.to_string(), secret)])).try_into()
        } else {
            Value::from(secret).try_into()
        }
        .map_err(|_| no())?;
        let field = if setting_name == "vpn" { "secrets" } else { key };
        Ok(HashMap::from([(setting_name, HashMap::from([(field.to_string(), value)]))]))
    }

    async fn cancel_get_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        connection_path: OwnedObjectPath,
        _setting_name: String,
    ) -> zbus::fdo::Result<()> {
        from_network_manager(&header, conn).await?;
        _ = self.requests.clone().send(Event::Cancel(connection_path.to_string())).await;
        Ok(())
    }

    async fn save_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}

    async fn delete_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
}

/// Registers the agent, and yields each password it needs asked.
pub fn subscription() -> Subscription<Event> {
    Subscription::run(|| {
        stream::channel(4, async |mut out| {
            let (tx, mut rx) = mpsc::channel(4);
            let conn = async {
                let conn = zbus::connection::Builder::system()?
                    .serve_at("/org/freedesktop/NetworkManager/SecretAgent", Agent { requests: tx })?
                    .build()
                    .await?;
                register(&conn).await?;
                zbus::Result::Ok(conn)
            }
            .await;
            let conn = match conn {
                Ok(conn) => conn,
                Err(err) => {
                    tracing::error!("cannot register the secret agent: {err}");
                    return futures::future::pending().await;
                }
            };
            // A restarted NetworkManager forgets its agents: register again.
            let restarts = match zbus::fdo::DBusProxy::new(&conn).await {
                Ok(dbus) => dbus
                    .receive_name_owner_changed_with_args(&[(0, "org.freedesktop.NetworkManager")])
                    .await
                    .ok(),
                Err(_) => None,
            };
            let mut restarts = match restarts {
                Some(stream) => stream.boxed().fuse(),
                None => futures::stream::pending().boxed().fuse(),
            };
            loop {
                futures::select! {
                    request = rx.next() => match request {
                        Some(request) => _ = out.send(request).await,
                        None => break,
                    },
                    change = restarts.next() => {
                        let back = change
                            .and_then(|c| c.args().ok().map(|a| a.new_owner().is_some()))
                            .unwrap_or(false);
                        if back {
                            if let Err(err) = register(&conn).await {
                                tracing::warn!("cannot register the secret agent again: {err}");
                            }
                        }
                    }
                }
            }
            futures::future::pending().await
        })
    })
}

async fn register(conn: &zbus::Connection) -> zbus::Result<()> {
    conn.call_method(
        Some("org.freedesktop.NetworkManager"),
        "/org/freedesktop/NetworkManager/AgentManager",
        Some("org.freedesktop.NetworkManager.AgentManager"),
        "Register",
        &(crate::APP_ID,),
    )
    .await
    .map(drop)
}

/// The Secret Service (GNOME Keyring), with NetworkManager's own schema, as
/// nm-applet stores these.
mod keyring {
    use super::*;

    const SERVICE: &str = "org.freedesktop.secrets";
    const SCHEMA: &str = "org.freedesktop.NetworkManager.Connection";

    fn attributes<'a>(uuid: &'a str, setting: &'a str, key: &'a str) -> HashMap<&'a str, &'a str> {
        HashMap::from([
            ("connection-uuid", uuid),
            ("setting-name", setting),
            ("setting-key", key),
            ("xdg:schema", SCHEMA),
        ])
    }

    async fn session(conn: &zbus::Connection) -> zbus::Result<OwnedObjectPath> {
        let reply = conn
            .call_method(
                Some(SERVICE),
                "/org/freedesktop/secrets",
                Some("org.freedesktop.Secret.Service"),
                "OpenSession",
                &("plain", Value::from("")),
            )
            .await?;
        let (_, session): (OwnedValue, OwnedObjectPath) = reply.body().deserialize()?;
        Ok(session)
    }

    /// Unlocks the items, asking the user for the keyring's password if
    /// needed; gives back those now unlocked.
    async fn unlock(
        conn: &zbus::Connection,
        items: Vec<OwnedObjectPath>,
    ) -> zbus::Result<Vec<OwnedObjectPath>> {
        let reply = conn
            .call_method(
                Some(SERVICE),
                "/org/freedesktop/secrets",
                Some("org.freedesktop.Secret.Service"),
                "Unlock",
                &(&items,),
            )
            .await?;
        let (unlocked, prompt_path): (Vec<OwnedObjectPath>, OwnedObjectPath) = reply.body().deserialize()?;
        if prompt_path.as_str() == "/" {
            return Ok(unlocked);
        }
        Ok(if prompt(conn, prompt_path).await? { items } else { Vec::new() })
    }

    /// Shows the keyring's own dialog; true unless the user dismissed it.
    async fn prompt(conn: &zbus::Connection, path: OwnedObjectPath) -> zbus::Result<bool> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.Secret.Prompt")?
            .member("Completed")?
            .path(path.clone())?
            .build();
        let mut completed = zbus::MessageStream::for_match_rule(rule, conn, Some(1)).await?;
        conn.call_method(Some(SERVICE), &path, Some("org.freedesktop.Secret.Prompt"), "Prompt", &("",))
            .await?;
        let wait = tokio::time::timeout(Duration::from_secs(100), completed.next()).await;
        let Ok(Some(Ok(message))) = wait else { return Ok(false) };
        let (dismissed, _): (bool, OwnedValue) = message.body().deserialize()?;
        Ok(!dismissed)
    }

    pub async fn find(uuid: &str, setting: &str, key: &str) -> Option<String> {
        let result = async {
            let conn = crate::bus::session().await?;
            let mut attrs = attributes(uuid, setting, key);
            // Some stores leave the schema out.
            attrs.remove("xdg:schema");
            let reply = conn
                .call_method(
                    Some(SERVICE),
                    "/org/freedesktop/secrets",
                    Some("org.freedesktop.Secret.Service"),
                    "SearchItems",
                    &(attrs,),
                )
                .await?;
            let (mut unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
                reply.body().deserialize()?;
            if unlocked.is_empty() && !locked.is_empty() {
                unlocked = unlock(&conn, locked).await?;
            }
            if unlocked.is_empty() {
                return Ok(None);
            }
            let session = session(&conn).await?;
            let reply = conn
                .call_method(
                    Some(SERVICE),
                    "/org/freedesktop/secrets",
                    Some("org.freedesktop.Secret.Service"),
                    "GetSecrets",
                    &(unlocked, &session),
                )
                .await?;
            let secrets: HashMap<OwnedObjectPath, (OwnedObjectPath, Vec<u8>, Vec<u8>, String)> =
                reply.body().deserialize()?;
            zbus::Result::Ok(
                secrets
                    .into_values()
                    .next()
                    .and_then(|(_, _, value, _)| String::from_utf8(value).ok()),
            )
        }
        .await;
        result.unwrap_or_else(|err| {
            tracing::warn!("keyring: {err}");
            None
        })
    }

    pub async fn store(label: &str, uuid: &str, setting: &str, key: &str, secret: &str) {
        let result = async {
            let conn = crate::bus::session().await?;
            let session = session(&conn).await?;
            let props = HashMap::from([
                (
                    "org.freedesktop.Secret.Item.Label",
                    Value::from(format!("Network secret for {label}/{setting}/{key}")),
                ),
                (
                    "org.freedesktop.Secret.Item.Attributes",
                    Value::from(attributes(uuid, setting, key)),
                ),
            ]);
            let secret = (
                ObjectPath::from(&session),
                Vec::<u8>::new(),
                secret.as_bytes().to_vec(),
                "text/plain",
            );
            let reply = conn
                .call_method(
                    Some(SERVICE),
                    "/org/freedesktop/secrets/aliases/default",
                    Some("org.freedesktop.Secret.Collection"),
                    "CreateItem",
                    &(props, secret, true),
                )
                .await?;
            // A locked keyring asks to be unlocked first.
            let (_, prompt_path): (OwnedObjectPath, OwnedObjectPath) = reply.body().deserialize()?;
            if prompt_path.as_str() != "/" {
                prompt(&conn, prompt_path).await?;
            }
            zbus::Result::Ok(())
        }
        .await;
        if let Err(err) = result {
            tracing::warn!("cannot save to the keyring: {err}");
        }
    }
}
