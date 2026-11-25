//! Devices, settings and where they are saved.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Limits read from the original's "add device" dialog (see MECHANICS.md).
pub const MAX_NAME_LEN: usize = 64;
pub const MAX_HOST_LEN: usize = 50; // setMaxLength(0x32) on the IP/Domain field
pub const MAX_USER_LEN: usize = 50; // setMaxLength(0x32) on the user field
pub const MAX_PASS_LEN: usize = 32; // setMaxLength(0x20) on the password field
pub const MAX_PORT: u32 = 65535; // "Port can not be more than 65535"
pub const MAX_CHANNELS: u32 = 256;
pub const DEFAULT_RTSP_PORT: u16 = 554;
pub const DEFAULT_HTTP_PORT: u16 = 80;
pub const DEFAULT_USER: &str = "admin";
pub const DEFAULT_RTSP_PATH: &str = "/cam/realmonitor?channel={ch}&subtype={stream}";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RegisterMode {
    #[default]
    IpDomain,
    SerialNumber,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub mode: RegisterMode,
    pub host: String,
    pub rtsp_port: u16,
    pub http_port: u16,
    pub user: String,
    pub password: String,
    pub channels: u32,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub device_type: String,
    #[serde(default = "default_path")]
    pub rtsp_path: String,
    #[serde(default)]
    pub channel_names: Vec<String>,
    /// For a recorder: the camera behind each channel (read from the recorder at login).
    #[serde(default)]
    pub camera_links: Vec<Option<CameraLink>>,
    /// Which network connection (adapter) to reach this device through, for a device on another
    /// router than the computer's main connection. Empty = automatic (the system decides).
    #[serde(default)]
    pub network: String,
}

fn default_path() -> String {
    DEFAULT_RTSP_PATH.to_string()
}

/// Where a recorder channel's camera lives on the network.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraLink {
    pub host: String,
    pub rtsp_port: u16,
    /// The camera's own channel (0 for single cameras).
    pub channel: u32,
}

impl Device {
    pub fn channel_name(&self, ch: u32) -> String {
        self.channel_names
            .get(ch as usize)
            .filter(|n| !n.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Channel {}", ch + 1))
    }

    /// Host used on the wire: the original turns "Local" into 127.0.0.1.
    pub fn wire_host(&self) -> String {
        if self.host.eq_ignore_ascii_case("local") {
            "127.0.0.1".into()
        } else {
            self.host.trim().to_string()
        }
    }

    pub fn http(&self) -> crate::http::Target {
        crate::http::Target {
            host: self.wire_host(),
            port: self.http_port,
            user: self.user.clone(),
            pass: self.password.clone(),
        }
    }

    /// Live address of one channel. `stream`: 0 main, 1 assist 1, 2 assist 2.
    pub fn live_url(&self, ch: u32, stream: u8) -> String {
        let path = self
            .rtsp_path
            .replace("{ch}", &(ch + 1).to_string())
            .replace("{stream}", &stream.to_string());
        let path = if path.starts_with('/') { path } else { format!("/{path}") };
        format!("rtsp://{}:{}{}", self.wire_host(), self.rtsp_port, path)
    }

    /// Recorded video between two local times (format YYYY_MM_DD_hh_mm_ss).
    /// Ways to show one live channel: through this device first, then (for a recorder)
    /// straight from the camera behind the channel. A camera that is also in the device
    /// list uses its own saved login; otherwise the recorder's login is tried.
    pub fn live_routes(&self, ch: u32, stream: u8, all: &[Device]) -> Vec<crate::stream::Route> {
        // A recorder that told us its camera list: a channel without a camera has nothing to show.
        if !self.camera_links.is_empty() && self.camera_links.get(ch as usize).map(|l| l.is_none()).unwrap_or(true) {
            return Vec::new();
        }
        let mut v = vec![crate::stream::Route { url: self.live_url(ch, stream), user: self.user.clone(), pass: self.password.clone(), label: String::new() }];
        if let Some(Some(link)) = self.camera_links.get(ch as usize) {
            let saved = all.iter().find(|d| d.id != self.id && d.wire_host() == link.host);
            let (user, pass, port) = match saved {
                Some(d) => (d.user.clone(), d.password.clone(), d.rtsp_port),
                None => (self.user.clone(), self.password.clone(), link.rtsp_port),
            };
            v.push(crate::stream::Route {
                url: format!("rtsp://{}:{}/cam/realmonitor?channel={}&subtype={}", link.host, port, link.channel + 1, stream),
                user,
                pass,
                label: format!("camera {}", link.host),
            });
        }
        v
    }

    /// Ways to play recordings of one channel from a time of day: the standard RTSP playback
    /// first, then the recorder's own files over its web interface (for recorders that refuse
    /// RTSP, like the CP-UNR-104F1).
    pub fn playback_routes(&self, ch: u32, day: chrono::NaiveDate, from: u32, until: u32) -> Vec<crate::stream::Route> {
        let start = crate::clock::playback_stamp(day, from);
        let end = crate::clock::playback_stamp(day, until);
        vec![
            crate::stream::Route { url: self.playback_url(ch, &start, &end), user: self.user.clone(), pass: self.password.clone(), label: String::new() },
            crate::stream::Route {
                url: format!("dav://{}:{}/?ch={}&day={}&from={}&end={}", self.wire_host(), self.http_port, ch, day.format("%Y-%m-%d"), from, until),
                user: self.user.clone(),
                pass: self.password.clone(),
                label: "recorder files".into(),
            },
        ]
    }

    pub fn playback_url(&self, ch: u32, start: &str, end: &str) -> String {
        format!(
            "rtsp://{}:{}/cam/playback?channel={}&starttime={}&endtime={}",
            self.wire_host(),
            self.rtsp_port,
            ch + 1,
            start,
            end
        )
    }
}

/// What the person typed in the add/edit dialog, before it becomes a device.
#[derive(Clone, Debug)]
pub struct DeviceForm {
    pub editing: Option<u64>,
    pub name: String,
    pub mode: RegisterMode,
    pub host: String,
    pub rtsp_port: String,
    pub http_port: String,
    pub user: String,
    pub password: String,
    pub channels: String,
    pub rtsp_path: String,
    /// Network connection name ("" = automatic), see `Device::network`.
    pub network: String,
    pub error: Option<(FormField, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormField {
    Name,
    Host,
    Port,
    HttpPort,
    User,
    Password,
    Channels,
}

impl Default for DeviceForm {
    fn default() -> Self {
        Self {
            editing: None,
            name: String::new(),
            mode: RegisterMode::IpDomain,
            host: String::new(),
            rtsp_port: DEFAULT_RTSP_PORT.to_string(),
            http_port: DEFAULT_HTTP_PORT.to_string(),
            user: DEFAULT_USER.into(),
            password: String::new(),
            channels: "4".into(),
            rtsp_path: DEFAULT_RTSP_PATH.into(),
            network: String::new(),
            error: None,
        }
    }
}

impl DeviceForm {
    pub fn from_device(d: &Device) -> Self {
        Self {
            editing: Some(d.id),
            name: d.name.clone(),
            mode: d.mode,
            host: if d.mode == RegisterMode::SerialNumber { d.serial.clone() } else { d.host.clone() },
            rtsp_port: d.rtsp_port.to_string(),
            http_port: d.http_port.to_string(),
            user: d.user.clone(),
            password: d.password.clone(),
            channels: d.channels.to_string(),
            rtsp_path: d.rtsp_path.clone(),
            network: d.network.clone(),
            error: None,
        }
    }

    fn host_chars_ok(s: &str) -> bool {
        // Same characters the original lets you type: ^[a-zA-Z0-9._:\-]*$
        s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
    }

    fn parse_port(s: &str, field: FormField, what: &str) -> Result<u16, (FormField, String)> {
        let t = s.trim();
        if t.is_empty() {
            return Err((field, format!("{what} can not be empty!")));
        }
        match t.parse::<u32>() {
            Ok(p) if p >= 1 && p <= MAX_PORT => Ok(p as u16),
            Ok(_) => Err((field, format!("Port can not be more than {MAX_PORT}"))),
            Err(_) => Err((field, "Port must be a number".into())),
        }
    }

    /// The same checks, in the same order, as the original's dialog.
    pub fn validate(&self, existing: &[Device]) -> Result<Device, (FormField, String)> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err((FormField::Name, "Device name is empty!".into()));
        }
        if name.chars().count() > MAX_NAME_LEN {
            return Err((FormField::Name, format!("Device name can be at most {MAX_NAME_LEN} characters")));
        }
        let host = self.host.trim();
        if host.is_empty() {
            let msg = if self.mode == RegisterMode::SerialNumber { "SN can not be empty!" } else { "IP/Domain name can not be empty!" };
            return Err((FormField::Host, msg.into()));
        }
        if host.len() > MAX_HOST_LEN || !Self::host_chars_ok(host) {
            return Err((FormField::Host, "Use letters, numbers and . _ : - only (50 at most)".into()));
        }
        let rtsp_port = Self::parse_port(&self.rtsp_port, FormField::Port, "Device's port")?;
        let http_port = Self::parse_port(&self.http_port, FormField::HttpPort, "Web port")?;
        let user = self.user.trim();
        if user.is_empty() {
            return Err((FormField::User, "User name can not be empty!".into()));
        }
        if user.len() > MAX_USER_LEN {
            return Err((FormField::User, format!("User name can be at most {MAX_USER_LEN} characters")));
        }
        if self.password.len() > MAX_PASS_LEN {
            return Err((FormField::Password, format!("Password can be at most {MAX_PASS_LEN} characters")));
        }
        let channels = match self.channels.trim().parse::<u32>() {
            Ok(c) if (1..=MAX_CHANNELS).contains(&c) => c,
            _ => return Err((FormField::Channels, format!("Channels must be 1 to {MAX_CHANNELS}"))),
        };
        let others = existing.iter().filter(|d| Some(d.id) != self.editing);
        for d in others {
            if d.name.eq_ignore_ascii_case(name) {
                return Err((FormField::Name, "Device Title has in list!".into()));
            }
            let same_host = if self.mode == RegisterMode::SerialNumber {
                d.serial.eq_ignore_ascii_case(host)
            } else {
                // The same address behind another router is another device.
                d.host.eq_ignore_ascii_case(host) && d.rtsp_port == rtsp_port && d.network == self.network
            };
            if same_host {
                return Err((FormField::Host, "IP/Domain has in list!".into()));
            }
        }
        let (h, serial) = if self.mode == RegisterMode::SerialNumber { (String::new(), host.to_string()) } else { (host.to_string(), String::new()) };
        Ok(Device {
            id: self.editing.unwrap_or(0),
            name: name.to_string(),
            mode: self.mode,
            host: h,
            rtsp_port,
            http_port,
            user: user.to_string(),
            password: self.password.clone(),
            channels,
            serial,
            device_type: String::new(),
            rtsp_path: if self.rtsp_path.trim().is_empty() { DEFAULT_RTSP_PATH.into() } else { self.rtsp_path.trim().to_string() },
            channel_names: Vec::new(),
            camera_links: Vec::new(),
            network: self.network.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Settings.ini AutoStreamType=1: pick main or assist stream by window size.
    pub auto_stream_type: bool,
    /// Windows at or below this count get the main stream when auto is on.
    pub main_stream_max_windows: u32,
    pub tour_interval_secs: u32,
    pub listen_for_alarms: bool,
    pub alarm_sound: bool,
    pub remote_navigation: bool,
    pub ui_scale: f32,
    pub app_password: String,
    pub last_split: usize,
    pub instant_replay_secs: u32,
    pub online_check_secs: u32,
    pub snapshot_dir: String,
    /// Cameras open in Live View: (window, device id, camera), restored on start.
    pub last_view: Vec<(usize, u64, u32)>,
    /// Left menu folded away (a small button brings it back).
    pub nav_collapsed: bool,
    /// Playback search panel folded away.
    pub pb_panel_hidden: bool,
    /// Live View camera list folded away.
    pub live_list_hidden: bool,
    /// TV server for Hisense VIDAA and other TVs that run web apps.
    pub tv_server: bool,
    pub tv_port: u16,
    /// Access code the TV must send.
    pub tv_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            auto_stream_type: true,
            main_stream_max_windows: 4,
            tour_interval_secs: 10,
            listen_for_alarms: true,
            alarm_sound: false,
            remote_navigation: cfg!(target_os = "android"),
            // Android already scales by screen density, so the same 1.0 fits phones and TVs.
            ui_scale: 1.0,
            app_password: String::new(),
            last_split: 1,
            instant_replay_secs: 30,
            online_check_secs: 30,
            snapshot_dir: String::new(),
            last_view: Vec::new(),
            nav_collapsed: false,
            pb_panel_hidden: false,
            live_list_hidden: false,
            tv_server: false,
            tv_port: 8090,
            tv_key: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Store {
    pub devices: Vec<Device>,
    pub settings: Settings,
    pub next_id: u64,
}

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_data_dir(p: PathBuf) {
    let _ = DATA_DIR.set(p);
}

pub fn data_dir() -> PathBuf {
    if let Some(p) = DATA_DIR.get() {
        return p.clone();
    }
    // macOS keeps app data in ~/Library/Application Support.
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library").join("Application Support").join("MyCCTV");
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("MyCCTV")
}

pub fn media_dir(settings: &Settings) -> PathBuf {
    if !settings.snapshot_dir.trim().is_empty() {
        return PathBuf::from(settings.snapshot_dir.trim());
    }
    #[cfg(not(target_os = "android"))]
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join("Pictures").join("MyCCTV");
    }
    data_dir().join("media")
}

impl Store {
    fn path() -> PathBuf {
        data_dir().join("mycctv.json")
    }

    pub fn load() -> Self {
        let text = std::fs::read_to_string(Self::path()).ok().or_else(|| {
            // The app used to be called "My KVMS": bring its device list and settings over once.
            let old = data_dir().parent()?.join("MyKVMS").join("mykvms.json");
            let t = std::fs::read_to_string(&old).ok()?;
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(Self::path(), &t);
            Some(t)
        });
        let store = text.and_then(|s| serde_json::from_str::<Store>(&s).ok()).unwrap_or_default();
        crate::netpath::sync(&store.devices);
        store
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join("mycctv.json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, Self::path()).map_err(|e| e.to_string())
    }

    pub fn add_device(&mut self, mut d: Device) -> u64 {
        self.next_id = self.next_id.max(self.devices.iter().map(|d| d.id).max().unwrap_or(0)) + 1;
        d.id = self.next_id;
        self.devices.push(d);
        self.next_id
    }

    pub fn device(&self, id: u64) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn export_devices(&self, path: &std::path::Path) -> Result<usize, String> {
        let text = serde_json::to_string_pretty(&self.devices).map_err(|e| e.to_string())?;
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        Ok(self.devices.len())
    }

    /// Adds devices from an exported list, skipping names or addresses already present.
    pub fn import_devices(&mut self, path: &std::path::Path) -> Result<(usize, usize), String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let list: Vec<Device> = serde_json::from_str(&text).map_err(|e| format!("not a device list: {e}"))?;
        let (mut added, mut skipped) = (0, 0);
        for d in list {
            let dup = self.devices.iter().any(|x| {
                x.name.eq_ignore_ascii_case(&d.name) || (x.host.eq_ignore_ascii_case(&d.host) && x.rtsp_port == d.rtsp_port && !d.host.is_empty())
            });
            if dup {
                skipped += 1;
            } else {
                self.add_device(d);
                added += 1;
            }
        }
        Ok((added, skipped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(name: &str, host: &str, port: &str, user: &str) -> DeviceForm {
        DeviceForm { name: name.into(), host: host.into(), rtsp_port: port.into(), user: user.into(), ..Default::default() }
    }

    #[test]
    fn add_device_rules_match_the_original() {
        let msg = |r: Result<Device, (FormField, String)>| r.err().map(|e| e.1).unwrap_or_default();
        assert_eq!(msg(form("", "1.2.3.4", "554", "admin").validate(&[])), "Device name is empty!");
        assert_eq!(msg(form("A", "", "554", "admin").validate(&[])), "IP/Domain name can not be empty!");
        assert_eq!(msg(form("A", "1.2.3.4", "", "admin").validate(&[])), "Device's port can not be empty!");
        assert_eq!(msg(form("A", "1.2.3.4", "70000", "admin").validate(&[])), "Port can not be more than 65535");
        assert_eq!(msg(form("A", "1.2.3.4", "554", "").validate(&[])), "User name can not be empty!");
        let first = form("A", "1.2.3.4", "554", "admin").validate(&[]).unwrap();
        assert_eq!(msg(form("a", "5.6.7.8", "554", "admin").validate(&[first.clone()])), "Device Title has in list!");
        assert_eq!(msg(form("B", "1.2.3.4", "554", "admin").validate(&[first.clone()])), "IP/Domain has in list!");
        assert!(form("B", "1.2.3.4", "555", "admin").validate(&[first]).is_ok());
        let local = form("L", "Local", "554", "admin").validate(&[]).unwrap();
        assert_eq!(local.wire_host(), "127.0.0.1");
        assert_eq!(local.live_url(0, 1), "rtsp://127.0.0.1:554/cam/realmonitor?channel=1&subtype=1");
    }
}

/// A new 6-digit access code for TV sharing.
pub fn new_tv_code() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(7);
    format!("{:06}", (n ^ (n >> 17)) % 1_000_000)
}
