// SPDX-License-Identifier: GPL-3.0-only

//! System state over D-Bus: NetworkManager, BlueZ, UPower, power profiles and
//! COSMIC's brightness. Every read is a fresh snapshot; `changes()` tells the
//! app when to take a new one.

use std::collections::HashMap;

use cosmic::iced::{Subscription, stream};
use futures::{SinkExt, StreamExt};
use zbus::{
    MatchRule, MessageStream,
    proxy::{CacheProperties, Defaults},
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

/// A proxy that reads properties fresh instead of caching them.
async fn proxy<T>(conn: &zbus::Connection, path: Option<OwnedObjectPath>) -> zbus::Result<T>
where
    T: From<zbus::Proxy<'static>> + Defaults,
{
    let builder = zbus::proxy::Builder::<T>::new(conn).cache_properties(CacheProperties::No);
    match path {
        Some(path) => builder.path(path.into_inner())?.build().await,
        None => builder.build().await,
    }
}

fn root() -> ObjectPath<'static> {
    ObjectPath::from_static_str_unchecked("/")
}

// ---------------------------------------------------------------- network

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Nm {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;
    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;
    fn deactivate_connection(&self, active_connection: &ObjectPath<'_>) -> zbus::Result<()>;
    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wireless_enabled(&self, value: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait NmSettings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait NmConnection {
    fn get_settings(&self) -> zbus::Result<Settings>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait NmActive {
    #[zbus(property)]
    fn connection(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait NmDevice {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait NmWireless {
    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
    #[zbus(property)]
    fn access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait NmAccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

#[derive(Debug, Clone, Default)]
pub struct Network {
    pub wifi_enabled: bool,
    pub wifi_device: Option<OwnedObjectPath>,
    /// The wired or wireless connection in use.
    pub primary: Option<Primary>,
    pub access_points: Vec<AccessPoint>,
    pub vpns: Vec<Vpn>,
}

#[derive(Debug, Clone)]
pub struct Primary {
    pub name: String,
    pub wired: bool,
}

#[derive(Debug, Clone)]
pub struct AccessPoint {
    pub ssid: String,
    pub path: OwnedObjectPath,
    pub strength: u8,
    pub secure: bool,
    pub active: bool,
    /// The saved connection for this network, if any.
    pub saved: Option<OwnedObjectPath>,
}

#[derive(Debug, Clone)]
pub struct Vpn {
    pub name: String,
    pub path: OwnedObjectPath,
    pub last_used: u64,
    /// The active connection, and whether it is fully up.
    pub active: Option<(OwnedObjectPath, bool)>,
}

fn text(settings: &Settings, group: &str, key: &str) -> Option<String> {
    String::try_from(settings.get(group)?.get(key)?.try_clone().ok()?).ok()
}

pub async fn network() -> zbus::Result<Network> {
    let conn = zbus::Connection::system().await?;
    let nm: NmProxy = proxy(&conn, None).await?;
    let mut net = Network {
        wifi_enabled: nm.wireless_enabled().await?,
        ..Default::default()
    };

    // Saved connections: VPNs, and Wi-Fi networks by SSID.
    let mut saved_wifi = HashMap::new();
    let mut vpns = HashMap::new();
    let list: NmSettingsProxy = proxy(&conn, None).await?;
    for path in list.list_connections().await? {
        let settings = proxy::<NmConnectionProxy>(&conn, Some(path.clone()))
            .await?
            .get_settings()
            .await?;
        let name = text(&settings, "connection", "id").unwrap_or_default();
        let last_used = settings
            .get("connection")
            .and_then(|c| c.get("timestamp"))
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or(0);
        match text(&settings, "connection", "type").as_deref() {
            Some("vpn" | "wireguard") => {
                vpns.insert(path.clone(), Vpn { name, path, last_used, active: None });
            }
            Some("802-11-wireless") => {
                let ssid = settings
                    .get("802-11-wireless")
                    .and_then(|w| w.get("ssid"))
                    .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok());
                if let Some(ssid) = ssid {
                    saved_wifi.insert(String::from_utf8_lossy(&ssid).into_owned(), path);
                }
            }
            _ => {}
        }
    }

    for path in nm.active_connections().await? {
        let active: NmActiveProxy = proxy(&conn, Some(path.clone())).await?;
        let Ok(kind) = active.kind().await else { continue };
        let up = active.state().await.unwrap_or(0) == 2;
        match kind.as_str() {
            "vpn" | "wireguard" => {
                if let Some(vpn) = vpns.get_mut(&active.connection().await?) {
                    vpn.active = Some((path, up));
                }
            }
            "802-3-ethernet" | "802-11-wireless" if net.primary.is_none() && up => {
                net.primary = Some(Primary {
                    name: active.id().await.unwrap_or_default(),
                    wired: kind == "802-3-ethernet",
                });
            }
            _ => {}
        }
    }
    net.vpns = vpns.into_values().collect();
    net.vpns.sort_by_key(|v| (std::cmp::Reverse(v.last_used), v.name.clone()));

    for device in nm.get_devices().await? {
        let kind = proxy::<NmDeviceProxy>(&conn, Some(device.clone())).await?.device_type().await?;
        // 2: Wi-Fi.
        if kind != 2 {
            continue;
        }
        let wireless: NmWirelessProxy = proxy(&conn, Some(device.clone())).await?;
        let current = wireless.active_access_point().await.ok();
        let mut seen = HashMap::<String, AccessPoint>::new();
        for ap_path in wireless.access_points().await.unwrap_or_default() {
            let ap: NmAccessPointProxy = proxy(&conn, Some(ap_path.clone())).await?;
            let Ok(ssid) = ap.ssid().await else { continue };
            if ssid.is_empty() {
                continue;
            }
            let ssid = String::from_utf8_lossy(&ssid).into_owned();
            let strength = ap.strength().await.unwrap_or(0);
            let secure = ap.wpa_flags().await.unwrap_or(0) != 0 || ap.rsn_flags().await.unwrap_or(0) != 0;
            let active = current.as_ref() == Some(&ap_path);
            // One entry per network: the strongest access point, or the one in use.
            if seen.get(&ssid).is_some_and(|old| old.active || old.strength >= strength) && !active {
                continue;
            }
            let saved = saved_wifi.get(&ssid).cloned();
            seen.insert(ssid.clone(), AccessPoint { ssid, path: ap_path, strength, secure, active, saved });
        }
        net.access_points = seen.into_values().collect();
        net.access_points
            .sort_by_key(|ap| (!ap.active, ap.saved.is_none(), std::cmp::Reverse(ap.strength)));
        net.wifi_device = Some(device);
        break;
    }
    Ok(net)
}

pub async fn set_wifi(on: bool) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    proxy::<NmProxy>(&conn, None).await?.set_wireless_enabled(on).await
}

pub async fn scan_wifi(device: OwnedObjectPath) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    proxy::<NmWirelessProxy>(&conn, Some(device)).await?.request_scan(HashMap::new()).await
}

/// Join a network: its saved connection, or a new one; NetworkManager asks
/// our secret agent for the password if it needs one.
pub async fn join_wifi(device: OwnedObjectPath, ap: AccessPoint) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let nm: NmProxy = proxy(&conn, None).await?;
    match ap.saved {
        Some(saved) => nm.activate_connection(&saved, &device, &ap.path).await.map(drop),
        None => nm.add_and_activate_connection(HashMap::new(), &device, &ap.path).await.map(drop),
    }
}

pub async fn disconnect(active: OwnedObjectPath) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    proxy::<NmProxy>(&conn, None).await?.deactivate_connection(&active).await
}

pub async fn connect(saved: OwnedObjectPath) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    proxy::<NmProxy>(&conn, None).await?.activate_connection(&saved, &root(), &root()).await.map(drop)
}

// ---------------------------------------------------------------- bluetooth

#[derive(Debug, Clone, Default)]
pub struct Bluetooth {
    pub adapter: Option<OwnedObjectPath>,
    pub powered: bool,
    pub devices: Vec<BtDevice>,
}

#[derive(Debug, Clone)]
pub struct BtDevice {
    pub path: OwnedObjectPath,
    pub name: String,
    pub icon: String,
    pub connected: bool,
    pub battery: Option<u8>,
}

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

pub async fn bluetooth() -> zbus::Result<Bluetooth> {
    let conn = zbus::Connection::system().await?;
    let objects: Objects = zbus::fdo::ObjectManagerProxy::builder(&conn)
        .destination("org.bluez")?
        .path("/")?
        .build()
        .await?
        .get_managed_objects()
        .await?
        .into_iter()
        .map(|(path, ifaces)| (path, ifaces.into_iter().map(|(k, v)| (k.to_string(), v)).collect()))
        .collect();
    let flag = |props: &HashMap<String, OwnedValue>, key: &str| {
        props.get(key).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
    };
    let string = |props: &HashMap<String, OwnedValue>, key: &str| {
        props.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
    };
    let mut bt = Bluetooth::default();
    for (path, ifaces) in &objects {
        if let Some(adapter) = ifaces.get("org.bluez.Adapter1") {
            if bt.adapter.is_none() {
                bt.adapter = Some(path.clone());
                bt.powered = flag(adapter, "Powered");
            }
        }
        if let Some(device) = ifaces.get("org.bluez.Device1") {
            if !flag(device, "Paired") {
                continue;
            }
            bt.devices.push(BtDevice {
                path: path.clone(),
                name: string(device, "Alias").or_else(|| string(device, "Name")).unwrap_or_default(),
                icon: string(device, "Icon").unwrap_or_else(|| "bluetooth".into()),
                connected: flag(device, "Connected"),
                battery: ifaces
                    .get("org.bluez.Battery1")
                    .and_then(|b| b.get("Percentage"))
                    .and_then(|v| u8::try_from(v).ok()),
            });
        }
    }
    bt.devices.sort_by_key(|d| (!d.connected, d.name.to_lowercase()));
    Ok(bt)
}

async fn set_property(
    conn: &zbus::Connection,
    service: &str,
    path: &ObjectPath<'_>,
    iface: &str,
    name: &str,
    value: Value<'_>,
) -> zbus::Result<()> {
    zbus::fdo::PropertiesProxy::builder(conn)
        .destination(service)?
        .path(path)?
        .build()
        .await?
        .set(iface.try_into()?, name, value)
        .await
        .map_err(Into::into)
}

pub async fn set_bluetooth(adapter: OwnedObjectPath, on: bool) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    set_property(&conn, "org.bluez", &adapter, "org.bluez.Adapter1", "Powered", on.into()).await
}

pub async fn bt_connect(device: OwnedObjectPath, connect: bool) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let method = if connect { "Connect" } else { "Disconnect" };
    conn.call_method(Some("org.bluez"), &device, Some("org.bluez.Device1"), method, &())
        .await
        .map(drop)
}

// ---------------------------------------------------------------- power

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait Battery {
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
}

#[derive(Debug, Clone)]
pub struct BatteryState {
    pub percent: f64,
    pub icon: String,
}

pub async fn battery() -> zbus::Result<Option<BatteryState>> {
    let conn = zbus::Connection::system().await?;
    let battery: BatteryProxy = proxy(&conn, None).await?;
    if !battery.is_present().await? {
        return Ok(None);
    }
    Ok(Some(BatteryState {
        percent: battery.percentage().await?,
        icon: battery.icon_name().await?,
    }))
}

#[zbus::proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}

#[derive(Debug, Clone, Default)]
pub struct Profiles {
    pub active: String,
    pub available: Vec<String>,
}

pub async fn profiles() -> zbus::Result<Profiles> {
    let conn = zbus::Connection::system().await?;
    let ppd: PowerProfilesProxy = proxy(&conn, None).await?;
    let available = ppd
        .profiles()
        .await?
        .into_iter()
        .filter_map(|p| String::try_from(p.get("Profile")?.try_clone().ok()?).ok())
        .collect();
    Ok(Profiles { active: ppd.active_profile().await?, available })
}

pub async fn set_profile(profile: String) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    proxy::<PowerProfilesProxy>(&conn, None).await?.set_active_profile(&profile).await
}

#[zbus::proxy(
    interface = "com.system76.CosmicSettingsDaemon",
    default_service = "com.system76.CosmicSettingsDaemon",
    default_path = "/com/system76/CosmicSettingsDaemon"
)]
trait SettingsDaemon {
    #[zbus(property)]
    fn display_brightness(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn set_display_brightness(&self, value: i32) -> zbus::Result<()>;
    #[zbus(property)]
    fn max_display_brightness(&self) -> zbus::Result<i32>;
}

/// The screen's brightness, 0 to 1, if it can be set.
pub async fn brightness() -> zbus::Result<Option<f32>> {
    let conn = zbus::Connection::session().await?;
    let daemon: SettingsDaemonProxy = proxy(&conn, None).await?;
    let max = daemon.max_display_brightness().await?;
    if max <= 0 {
        return Ok(None);
    }
    Ok(Some(daemon.display_brightness().await? as f32 / max as f32))
}

pub async fn set_brightness(level: f32) -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    let daemon: SettingsDaemonProxy = proxy(&conn, None).await?;
    let max = daemon.max_display_brightness().await?;
    daemon.set_display_brightness((level * max as f32).round().max(1.0) as i32).await
}

// ---------------------------------------------------------------- changes

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Network,
    Bluetooth,
    Battery,
    Profiles,
    Brightness,
    Media,
}

/// Fires whenever one of the watched services says something changed.
pub fn changes() -> Subscription<Source> {
    Subscription::run(|| {
        stream::channel(16, async |mut out| {
            let (Ok(system), Ok(session)) =
                (zbus::Connection::system().await, zbus::Connection::session().await)
            else {
                tracing::error!("no D-Bus connection");
                return futures::future::pending().await;
            };
            let watch = [
                (&system, "org.freedesktop.NetworkManager", Source::Network),
                (&system, "org.bluez", Source::Bluetooth),
                (&system, "org.freedesktop.UPower", Source::Battery),
                (&system, "net.hadess.PowerProfiles", Source::Profiles),
                (&session, "com.system76.CosmicSettingsDaemon", Source::Brightness),
            ];
            let mut streams = Vec::new();
            for (conn, sender, source) in watch {
                let rule = MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .sender(sender)
                    .expect("valid name")
                    .build();
                match MessageStream::for_match_rule(rule, conn, Some(64)).await {
                    Ok(s) => streams.push(s.map(move |_| source).boxed()),
                    Err(err) => tracing::warn!("cannot watch {sender}: {err}"),
                }
            }
            // MPRIS players have no fixed name: their path is fixed.
            let rule = MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .path("/org/mpris/MediaPlayer2")
                .expect("valid path")
                .build();
            if let Ok(s) = MessageStream::for_match_rule(rule, &session, Some(64)).await {
                streams.push(s.map(|_| Source::Media).boxed());
            }
            let mut all = futures::stream::select_all(streams);
            while let Some(source) = all.next().await {
                _ = out.send(source).await;
            }
            futures::future::pending().await
        })
    })
}
