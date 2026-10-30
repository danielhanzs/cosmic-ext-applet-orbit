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
    fn get_secrets(&self, setting_name: &str) -> zbus::Result<Settings>;
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
    /// The saved hotspot: a Wi-Fi connection in access point mode.
    pub hotspot: Option<Hotspot>,
}

#[derive(Debug, Clone)]
pub struct Hotspot {
    pub path: OwnedObjectPath,
    pub ssid: String,
    /// The active connection while it's on.
    pub active: Option<OwnedObjectPath>,
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
    let conn = crate::bus::system().await?;
    let nm: NmProxy = proxy(&conn, None).await?;
    let mut net = Network {
        wifi_enabled: nm.wireless_enabled().await?,
        ..Default::default()
    };

    // Saved connections: VPNs, and Wi-Fi networks by SSID.
    let mut saved_wifi = HashMap::new();
    let mut vpns = HashMap::new();
    let list: NmSettingsProxy = proxy(&conn, None).await?;
    // Objects can go between listing and reading them: those are skipped,
    // not the whole snapshot.
    for path in list.list_connections().await? {
        let Ok(connection) = proxy::<NmConnectionProxy>(&conn, Some(path.clone())).await else { continue };
        let Ok(settings) = connection.get_settings().await else { continue };
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
                    .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok())
                    .map(|ssid| String::from_utf8_lossy(&ssid).into_owned());
                let Some(ssid) = ssid else { continue };
                if text(&settings, "802-11-wireless", "mode").as_deref() == Some("ap") {
                    net.hotspot = Some(Hotspot { path, ssid, active: None });
                } else {
                    saved_wifi.insert(ssid, path);
                }
            }
            _ => {}
        }
    }

    for path in nm.active_connections().await? {
        let Ok(active) = proxy::<NmActiveProxy>(&conn, Some(path.clone())).await else { continue };
        let Ok(kind) = active.kind().await else { continue };
        let up = active.state().await.unwrap_or(0) == 2;
        let saved = active.connection().await.ok();
        if let Some(hotspot) = net.hotspot.as_mut().filter(|h| saved.as_ref() == Some(&h.path)) {
            hotspot.active = Some(path);
            continue;
        }
        match kind.as_str() {
            "vpn" | "wireguard" => {
                if let Some(vpn) = saved.as_ref().and_then(|saved| vpns.get_mut(saved)) {
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
        let Ok(device_proxy) = proxy::<NmDeviceProxy>(&conn, Some(device.clone())).await else { continue };
        let Ok(kind) = device_proxy.device_type().await else { continue };
        // 2: Wi-Fi.
        if kind != 2 {
            continue;
        }
        let Ok(wireless) = proxy::<NmWirelessProxy>(&conn, Some(device.clone())).await else { continue };
        let current = wireless.active_access_point().await.ok();
        let mut seen = HashMap::<String, AccessPoint>::new();
        for ap_path in wireless.access_points().await.unwrap_or_default() {
            let Ok(ap) = proxy::<NmAccessPointProxy>(&conn, Some(ap_path.clone())).await else { continue };
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
    let conn = crate::bus::system().await?;
    proxy::<NmProxy>(&conn, None).await?.set_wireless_enabled(on).await
}

pub async fn scan_wifi(device: OwnedObjectPath) -> zbus::Result<()> {
    let conn = crate::bus::system().await?;
    proxy::<NmWirelessProxy>(&conn, Some(device)).await?.request_scan(HashMap::new()).await
}

/// Join a network: its saved connection, or a new one; NetworkManager asks
/// our secret agent for the password if it needs one.
pub async fn join_wifi(device: OwnedObjectPath, ap: AccessPoint) -> zbus::Result<()> {
    let conn = crate::bus::system().await?;
    let nm: NmProxy = proxy(&conn, None).await?;
    match ap.saved {
        Some(saved) => nm.activate_connection(&saved, &device, &ap.path).await.map(drop),
        None => nm.add_and_activate_connection(HashMap::new(), &device, &ap.path).await.map(drop),
    }
}

pub async fn disconnect(active: OwnedObjectPath) -> zbus::Result<()> {
    let conn = crate::bus::system().await?;
    proxy::<NmProxy>(&conn, None).await?.deactivate_connection(&active).await
}

/// Starts the hotspot: the saved one, or a new one named after the computer
/// with a random password, which NetworkManager keeps.
pub async fn start_hotspot(device: OwnedObjectPath, saved: Option<OwnedObjectPath>) -> zbus::Result<()> {
    let conn = crate::bus::system().await?;
    let nm: NmProxy = proxy(&conn, None).await?;
    if let Some(saved) = saved {
        return nm.activate_connection(&saved, &device, &root()).await.map(drop);
    }
    let mut name = [0u8; 64];
    // SAFETY: the buffer is as long as said, and gethostname ends it with a zero.
    let host = if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } == 0 {
        std::ffi::CStr::from_bytes_until_nul(&name).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        String::new()
    };
    // SSIDs are at most 32 bytes: cut a long host name on a character boundary.
    let mut ssid = if host.is_empty() { "Orbit".to_string() } else { host };
    while ssid.len() > 32 {
        ssid.pop();
    }
    let settings = HashMap::from([
        (
            "connection",
            HashMap::from([
                ("id", Value::from("Hotspot")),
                ("type", Value::from("802-11-wireless")),
                ("autoconnect", Value::from(false)),
            ]),
        ),
        (
            "802-11-wireless",
            HashMap::from([("ssid", Value::from(ssid.into_bytes())), ("mode", Value::from("ap"))]),
        ),
        (
            "802-11-wireless-security",
            HashMap::from([("key-mgmt", Value::from("wpa-psk")), ("psk", Value::from(password()))]),
        ),
        ("ipv4", HashMap::from([("method", Value::from("shared"))])),
        ("ipv6", HashMap::from([("method", Value::from("ignore"))])),
    ]);
    nm.add_and_activate_connection(settings, &device, &root()).await.map(drop)
}

/// Twelve letters and digits, from the kernel's random source.
fn password() -> String {
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 12];
    // getrandom fills the buffer, or the fallback below is used.
    let filled = unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) } == bytes.len() as isize;
    if !filled {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        bytes.iter_mut().enumerate().for_each(|(i, b)| *b = (nanos >> (i * 5)) as u8);
    }
    bytes.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect()
}

/// The hotspot's password, to show for joining it.
pub async fn hotspot_password(saved: OwnedObjectPath) -> Option<String> {
    let conn = crate::bus::system().await.ok()?;
    let secrets = proxy::<NmConnectionProxy>(&conn, Some(saved)).await.ok()?.get_secrets("802-11-wireless-security").await.ok()?;
    text(&secrets, "802-11-wireless-security", "psk")
}

pub async fn connect(saved: OwnedObjectPath) -> zbus::Result<()> {
    let conn = crate::bus::system().await?;
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
    let conn = crate::bus::system().await?;
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
    let conn = crate::bus::system().await?;
    set_property(&conn, "org.bluez", &adapter, "org.bluez.Adapter1", "Powered", on.into()).await
}

pub async fn bt_connect(device: OwnedObjectPath, connect: bool) -> zbus::Result<()> {
    let conn = crate::bus::system().await?;
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
}

#[derive(Debug, Clone)]
pub struct BatteryState {
    pub percent: f64,
    /// Plugged in: charging, or full.
    pub charging: bool,
}

pub async fn battery() -> zbus::Result<Option<BatteryState>> {
    let conn = crate::bus::system().await?;
    let battery: BatteryProxy = proxy(&conn, None).await?;
    if !battery.is_present().await? {
        return Ok(None);
    }
    Ok(Some(BatteryState {
        percent: battery.percentage().await?,
        // 1: charging, 4: fully charged.
        charging: matches!(battery.state().await?, 1 | 4),
    }))
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(interface = "org.freedesktop.UPower.Device", default_service = "org.freedesktop.UPower", default_path = "/")]
trait UPowerDevice {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn model(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;
}

/// A mouse, keyboard, headset, phone… with a battery.
#[derive(Debug, Clone)]
pub struct Peripheral {
    pub name: String,
    pub icon: &'static str,
    pub percent: f64,
}

/// UPower's device types, as icons.
fn peripheral_icon(kind: u32) -> &'static str {
    match kind {
        5 => "input-mouse-symbolic",
        6 => "input-keyboard-symbolic",
        8 => "phone-symbolic",
        12 => "input-gaming-symbolic",
        13 | 10 => "input-tablet-symbolic",
        17 => "audio-headset-symbolic",
        18 => "audio-speakers-symbolic",
        19 => "audio-headphones-symbolic",
        _ => "bluetooth-active-symbolic",
    }
}

pub async fn peripherals() -> zbus::Result<Vec<Peripheral>> {
    let conn = crate::bus::system().await?;
    let mut found = Vec::new();
    for path in proxy::<UPowerProxy>(&conn, None).await?.enumerate_devices().await? {
        let Ok(device) = proxy::<UPowerDeviceProxy>(&conn, Some(path)).await else { continue };
        // The computer's own batteries and power lines supply it; these don't.
        let (Ok(false), Ok(kind)) = (device.power_supply().await, device.kind().await) else { continue };
        // 1: line power, 2: battery, 3: UPS, 4: monitor.
        if kind <= 4 {
            continue;
        }
        let percent = device.percentage().await.unwrap_or(0.0);
        if percent <= 0.0 {
            continue;
        }
        found.push(Peripheral {
            name: device.model().await.unwrap_or_default(),
            icon: peripheral_icon(kind),
            percent,
        });
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
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
    let conn = crate::bus::system().await?;
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
    let conn = crate::bus::system().await?;
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
    let conn = crate::bus::session().await?;
    let daemon: SettingsDaemonProxy = proxy(&conn, None).await?;
    let max = daemon.max_display_brightness().await?;
    if max <= 0 {
        return Ok(None);
    }
    Ok(Some(daemon.display_brightness().await? as f32 / max as f32))
}

pub async fn set_brightness(level: f32) -> zbus::Result<()> {
    let conn = crate::bus::session().await?;
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
                (crate::bus::system().await, crate::bus::session().await)
            else {
                tracing::error!("no D-Bus connection");
                return futures::future::pending().await;
            };
            // By path: zbus can't tell a well-known sender apart on its side,
            // so a sender rule would let every signal on the bus through.
            let watch: [(&zbus::Connection, &str, Source); 7] = [
                (&system, "/org/freedesktop/NetworkManager", Source::Network),
                (&system, "/org/bluez", Source::Bluetooth),
                (&system, "/org/freedesktop/UPower/devices", Source::Battery),
                (&system, "/org/freedesktop/UPower/PowerProfiles", Source::Profiles),
                (&system, "/net/hadess/PowerProfiles", Source::Profiles),
                (&session, "/com/system76/CosmicSettingsDaemon", Source::Brightness),
                // Bluetooth devices come and go through BlueZ's object manager at /.
                (&system, "/", Source::Bluetooth),
            ];
            let mut streams = Vec::new();
            for (conn, path, source) in watch {
                let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal);
                let rule = if path == "/" {
                    rule.path("/").and_then(|r| r.interface("org.freedesktop.DBus.ObjectManager"))
                } else {
                    rule.path_namespace(path)
                };
                let Ok(rule) = rule.map(|r| r.build()) else { continue };
                match MessageStream::for_match_rule(rule, conn, Some(64)).await {
                    Ok(s) => streams.push(s.map(move |_| source).boxed()),
                    Err(err) => tracing::warn!("cannot watch {path}: {err}"),
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
