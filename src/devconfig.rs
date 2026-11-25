//! Reading and changing a recorder's or camera's settings through its web interface
//! (configManager getConfig / setConfig), plus device facts, reboot and clock sync.

use crate::http::{self, Target};
use std::collections::BTreeMap;
use std::time::Duration;

/// Setting groups we ask for one by one when the device can't send "All" at once.
pub const KNOWN_GROUPS: &[&str] = &[
    "General", "Encode", "VideoColor", "VideoInOptions", "ChannelTitle", "VideoWidget", "MotionDetect", "BlindDetect", "LossDetect",
    "Alarm", "AlarmOut", "RecordMode", "Record", "Snap", "NTP", "Locales", "Network", "RTSP", "Web", "DVRIP", "Multicast", "UPnP",
    "DDNS", "Email", "AccessFilter", "RemoteDevice", "RemoteChannel", "Ptz", "StorageGroup", "AudioInput", "VideoInMode", "Lighting",
];

/// Everything read from one device: settings (keys without the "table." prefix),
/// encode capabilities per channel ("caps{ch}.MainFormat[0]...") and device facts.
#[derive(Default, Clone)]
pub struct Snapshot {
    pub values: BTreeMap<String, String>,
    pub caps: BTreeMap<String, String>,
    pub facts: Vec<(String, String)>,
    pub storage: Vec<(String, String)>,
}

impl Snapshot {
    /// How many channels the settings describe (from Encode[n]).
    pub fn channel_count(&self) -> u32 {
        let mut max = 0u32;
        for k in self.values.keys() {
            if let Some(rest) = k.strip_prefix("Encode[") {
                if let Some(n) = rest.split(']').next().and_then(|v| v.parse::<u32>().ok()) {
                    max = max.max(n + 1);
                }
            }
        }
        max.max(1)
    }

    /// Groups ("Encode", "Network"...) and how many settings each holds.
    pub fn groups(&self) -> Vec<(String, usize)> {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for k in self.values.keys() {
            *m.entry(group_of(k).to_string()).or_default() += 1;
        }
        m.into_iter().collect()
    }
}

pub fn group_of(key: &str) -> &str {
    let end = key.find(|c| c == '.' || c == '[').unwrap_or(key.len());
    &key[..end]
}

fn parse_table(body: &str, into: &mut BTreeMap<String, String>) {
    for (k, v) in http::kv_lines(body) {
        let k = k.strip_prefix("table.").unwrap_or(&k);
        // "name=All" answers as table.All.<group>...
        let k = k.strip_prefix("All.").unwrap_or(k).to_string();
        if !k.is_empty() && !k.starts_with("Error") {
            into.insert(k, v);
        }
    }
}

/// Reads all settings, capabilities and facts. Slow devices can take several seconds.
pub fn load(t: &Target, channels_hint: u32) -> Result<Snapshot, String> {
    let to = Duration::from_secs(20);
    let mut s = Snapshot::default();
    let (code, body) = http::get(t, "/cgi-bin/configManager.cgi?action=getConfig&name=All", to)?;
    if code == 401 {
        return Err("user name or password is wrong".into());
    }
    if code == 200 && body.lines().count() > 20 {
        parse_table(&body, &mut s.values);
    } else {
        for g in KNOWN_GROUPS {
            if let Ok((200, b)) = http::get(t, &format!("/cgi-bin/configManager.cgi?action=getConfig&name={g}"), Duration::from_secs(8)) {
                parse_table(&b, &mut s.values);
            }
        }
    }
    if s.values.is_empty() {
        return Err("the device did not share its settings (it may not support this kind of setup)".into());
    }
    let channels = s.channel_count().max(channels_hint).min(64);
    for ch in 0..channels {
        if let Ok((200, b)) = http::get(t, &format!("/cgi-bin/encode.cgi?action=getConfigCaps&channel={}", ch + 1), Duration::from_secs(8)) {
            for (k, v) in http::kv_lines(&b) {
                if let Some(rest) = k.strip_prefix("caps[0].") {
                    s.caps.insert(format!("caps{ch}.{rest}"), v);
                }
            }
        }
    }
    for (label, path) in [
        ("Model", "/cgi-bin/magicBox.cgi?action=getDeviceType"),
        ("Serial number", "/cgi-bin/magicBox.cgi?action=getSerialNo"),
        ("Firmware", "/cgi-bin/magicBox.cgi?action=getSoftwareVersion"),
        ("Hardware", "/cgi-bin/magicBox.cgi?action=getHardwareVersion"),
        ("Device name", "/cgi-bin/magicBox.cgi?action=getMachineName"),
        ("Device clock", "/cgi-bin/global.cgi?action=getCurrentTime"),
    ] {
        if let Ok((200, b)) = http::get(t, path, Duration::from_secs(6)) {
            let v = http::kv_lines(&b).into_iter().map(|(_, v)| v).collect::<Vec<_>>().join(" ");
            if !v.is_empty() {
                s.facts.push((label.to_string(), v));
            }
        }
    }
    if let Ok((200, b)) = http::get(t, "/cgi-bin/storageDevice.cgi?action=getDeviceAllInfo", Duration::from_secs(8)) {
        for (k, v) in http::kv_lines(&b) {
            if k.ends_with(".State") || k.ends_with(".TotalBytes") || k.ends_with(".UsedBytes") || k.ends_with(".Name") {
                s.storage.push((k, v));
            }
        }
    }
    Ok(s)
}

/// Writes changed settings, a few per request. Returns how many were accepted.
pub fn save(t: &Target, changes: &[(String, String)]) -> Result<usize, String> {
    let mut done = 0;
    for chunk in changes.chunks(8) {
        let mut path = String::from("/cgi-bin/configManager.cgi?action=setConfig");
        for (k, v) in chunk {
            path.push('&');
            // Names keep their [ ] and dots; only characters that would break the address are escaped.
            for ch in k.chars() {
                match ch {
                    '&' => path.push_str("%26"),
                    '=' => path.push_str("%3D"),
                    ' ' => path.push_str("%20"),
                    '#' => path.push_str("%23"),
                    '+' => path.push_str("%2B"),
                    '%' => path.push_str("%25"),
                    c => path.push(c),
                }
            }
            path.push('=');
            path.push_str(&http::url_escape(v));
        }
        let (code, body) = http::get(t, &path, Duration::from_secs(15))?;
        if code != 200 || !body.to_ascii_uppercase().contains("OK") {
            let what = chunk.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", ");
            return Err(format!("the device refused {what} ({})", body.trim().chars().take(80).collect::<String>()));
        }
        done += chunk.len();
    }
    Ok(done)
}

pub fn reboot(t: &Target) -> Result<(), String> {
    let (code, body) = http::get(t, "/cgi-bin/magicBox.cgi?action=reboot", Duration::from_secs(10))?;
    if code == 200 && body.to_ascii_uppercase().contains("OK") {
        Ok(())
    } else {
        Err(format!("device answered {code}"))
    }
}

/// Sets the device clock to this computer's local time.
pub fn sync_clock(t: &Target) -> Result<String, String> {
    let now = crate::clock::now_text();
    let (code, body) = http::get(t, &format!("/cgi-bin/global.cgi?action=setCurrentTime&time={}", http::url_escape(&now)), Duration::from_secs(10))?;
    if code == 200 && body.to_ascii_uppercase().contains("OK") {
        Ok(now)
    } else {
        Err(format!("device answered {code}: {}", body.trim().chars().take(60).collect::<String>()))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn groups_and_channels() {
        let mut s = super::Snapshot::default();
        super::parse_table("table.Encode[0].MainFormat[0].Video.FPS=25\ntable.Encode[3].MainFormat[0].Video.FPS=25\ntable.NTP.Enable=true\n", &mut s.values);
        assert_eq!(s.channel_count(), 4);
        assert_eq!(s.groups(), vec![("Encode".to_string(), 2), ("NTP".to_string(), 1)]);
        assert_eq!(super::group_of("VideoColor[0][0].Hue"), "VideoColor");
    }
}
