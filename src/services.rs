//! Talking to recorders and cameras over their web interface:
//! camera movement (PTZ), device details, recording search, the alarm feed,
//! the online check and the local network search.

use crate::http::{self, Target};
use crate::model::Device;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------- PTZ

/// The 16 buttons of the original's PTZ panel, in its own numbering
/// (RealMonitorUI 0x180134500: Up 0, Down 1, Left 2, Right 3, LeftUp 4,
/// RightUp 5, LeftDown 6, RightDown 7, Zoom+ 8, Zoom- 9, Focus+ 10, Focus- 11,
/// Aperture+ 12, Aperture- 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ptz {
    Up,
    Down,
    Left,
    Right,
    LeftUp,
    RightUp,
    LeftDown,
    RightDown,
    ZoomIn,
    ZoomOut,
    FocusNear,
    FocusFar,
    IrisOpen,
    IrisClose,
}

impl Ptz {
    pub fn code(self) -> &'static str {
        match self {
            Ptz::Up => "Up",
            Ptz::Down => "Down",
            Ptz::Left => "Left",
            Ptz::Right => "Right",
            Ptz::LeftUp => "LeftUp",
            Ptz::RightUp => "RightUp",
            Ptz::LeftDown => "LeftDown",
            Ptz::RightDown => "RightDown",
            Ptz::ZoomIn => "ZoomTele",
            Ptz::ZoomOut => "ZoomWide",
            Ptz::FocusNear => "FocusNear",
            Ptz::FocusFar => "FocusFar",
            Ptz::IrisOpen => "IrisLarge",
            Ptz::IrisClose => "IrisSmall",
        }
    }
    fn diagonal(self) -> bool {
        matches!(self, Ptz::LeftUp | Ptz::RightUp | Ptz::LeftDown | Ptz::RightDown)
    }
}

pub const PTZ_STEP_MIN: u32 = 1;
pub const PTZ_STEP_MAX: u32 = 8;
pub const PRESET_MIN: u32 = 1;
pub const PRESET_MAX: u32 = 255;

/// Starts or stops one PTZ movement. Runs in the background; errors come back on `reply`.
pub fn ptz(dev: &Device, ch: u32, cmd: Ptz, start: bool, step: u32, reply: Sender<String>) {
    let t = dev.http();
    let step = step.clamp(PTZ_STEP_MIN, PTZ_STEP_MAX);
    let (a1, a2) = if cmd.diagonal() { (step, step) } else { (0, step) };
    let action = if start { "start" } else { "stop" };
    let path = format!("/cgi-bin/ptz.cgi?action={action}&channel={}&code={}&arg1={a1}&arg2={a2}&arg3=0", ch + 1, cmd.code());
    std::thread::spawn(move || {
        if let Err(e) = check_ok(http::get(&t, &path, Duration::from_secs(4))) {
            let _ = reply.send(format!("Camera control failed: {e}"));
        }
    });
}

pub fn preset(dev: &Device, ch: u32, number: u32, set: bool, reply: Sender<String>) {
    let t = dev.http();
    let n = number.clamp(PRESET_MIN, PRESET_MAX);
    let code = if set { "SetPreset" } else { "GotoPreset" };
    let path = format!("/cgi-bin/ptz.cgi?action=start&channel={}&code={code}&arg1=0&arg2={n}&arg3=0", ch + 1);
    std::thread::spawn(move || {
        let msg = match check_ok(http::get(&t, &path, Duration::from_secs(4))) {
            Ok(()) if set => format!("Preset {n} saved"),
            Ok(()) => format!("Going to preset {n}"),
            Err(e) => format!("Preset failed: {e}"),
        };
        let _ = reply.send(msg);
    });
}

fn check_ok(r: Result<(u16, String), String>) -> Result<(), String> {
    match r {
        Ok((200, body)) if !body.to_ascii_lowercase().contains("error") => Ok(()),
        Ok((200, body)) => Err(body.trim().chars().take(80).collect()),
        Ok((code, _)) => Err(format!("device answered {code}")),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------- device details

#[derive(Clone, Debug, Default)]
pub struct DeviceInfo {
    pub device_type: String,
    pub serial: String,
    pub channels: Option<u32>,
    pub channel_names: Vec<String>,
    pub camera_links: Vec<Option<crate::model::CameraLink>>,
}

/// "Login": asks the device for its type, serial number and channel count.
pub fn device_info(t: &Target) -> Result<DeviceInfo, String> {
    let to = Duration::from_secs(5);
    let mut info = DeviceInfo::default();
    let (code, body) = http::get(t, "/cgi-bin/magicBox.cgi?action=getDeviceType", to)?;
    if code != 200 {
        return Err(format!("device answered {code}"));
    }
    info.device_type = value_of(&body, "type").unwrap_or_default();
    if let Ok((200, b)) = http::get(t, "/cgi-bin/magicBox.cgi?action=getSerialNo", to) {
        info.serial = value_of(&b, "sn").unwrap_or_default();
    }
    if let Ok((200, b)) = http::get(t, "/cgi-bin/magicBox.cgi?action=getProductDefinition&name=MaxRemoteInputChannels", to) {
        info.channels = value_of(&b, "table.MaxRemoteInputChannels").and_then(|v| v.parse().ok()).filter(|&c: &u32| c > 0);
    }
    if info.channels.is_none() {
        if let Ok((200, b)) = http::get(t, "/cgi-bin/devVideoInput.cgi?action=getCollect", to) {
            info.channels = value_of(&b, "result").and_then(|v| v.parse().ok()).filter(|&c: &u32| c > 0);
        }
    }
    if let Ok((200, b)) = http::get(t, "/cgi-bin/configManager.cgi?action=getConfig&name=ChannelTitle", to) {
        let mut names: Vec<(usize, String)> = Vec::new();
        for (k, v) in http::kv_lines(&b) {
            // table.ChannelTitle[3].Name=Gate
            if let Some(rest) = k.strip_prefix("table.ChannelTitle[") {
                if let Some((idx, tail)) = rest.split_once(']') {
                    if tail == ".Name" {
                        if let Ok(i) = idx.parse::<usize>() {
                            names.push((i, v));
                        }
                    }
                }
            }
        }
        names.sort();
        info.channel_names = names.into_iter().map(|(_, n)| n).collect();
    }
    info.camera_links = camera_links(t, info.channels.unwrap_or(0) as usize);
    Ok(info)
}

fn value_of(body: &str, key: &str) -> Option<String> {
    http::kv_lines(body).into_iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

/// Online check: can we open the device's video port?
pub fn is_online(dev: &Device) -> bool {
    crate::netpath::connect(&dev.wire_host(), dev.rtsp_port, Duration::from_secs(3)).is_ok()
}

// ---------------------------------------------------------------- recordings

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    General,
    Alarm,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordFilter {
    All,
    General,
    Alarm,
    Motion,
}

impl RecordFilter {
    pub const ALL: [RecordFilter; 4] = [RecordFilter::All, RecordFilter::General, RecordFilter::Alarm, RecordFilter::Motion];
    pub fn label(self) -> &'static str {
        match self {
            RecordFilter::All => "All Records",
            RecordFilter::General => "General Record",
            RecordFilter::Alarm => "Alarm Record",
            RecordFilter::Motion => "Motion Detect Record",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RecordPiece {
    /// Seconds since midnight of the searched day.
    pub start: u32,
    pub end: u32,
    pub kind: RecordKind,
    /// The recording file on the device (for playing it over the web interface).
    pub path: String,
}

/// Lists the recordings of one channel on one day (`day` as YYYY-MM-DD).
pub fn find_records(dev: &Device, ch: u32, day: &str, filter: RecordFilter) -> Result<Vec<RecordPiece>, String> {
    find_records_at(&dev.http(), ch, day, filter)
}

/// Lists recordings of one channel on one day, asking a device by its web address.
pub fn find_records_at(t: &Target, ch: u32, day: &str, filter: RecordFilter) -> Result<Vec<RecordPiece>, String> {
    let t = t.clone();

    let to = Duration::from_secs(8);
    let (code, body) = http::get(&t, "/cgi-bin/mediaFileFind.cgi?action=factory.create", to)?;
    if code != 200 {
        return Err(format!("recording search not available (device answered {code})"));
    }
    let object = value_of(&body, "result").ok_or("recording search not available")?;
    let mut cond = format!(
        "&condition.Channel={}&condition.StartTime={}&condition.EndTime={}&condition.Types[0]=dav",
        ch + 1,
        http::url_escape(&format!("{day} 00:00:00")),
        http::url_escape(&format!("{day} 23:59:59"))
    );
    match filter {
        RecordFilter::All | RecordFilter::General => {}
        RecordFilter::Alarm => cond.push_str("&condition.Flags[0]=Event&condition.Events[0]=AlarmLocal"),
        RecordFilter::Motion => cond.push_str("&condition.Flags[0]=Event&condition.Events[0]=VideoMotion"),
    }
    let mut out = Vec::new();
    let res = (|| -> Result<(), String> {
        let (code, b) = http::get(&t, &format!("/cgi-bin/mediaFileFind.cgi?action=findFile&object={object}{cond}"), to)?;
        if code != 200 || !b.to_ascii_lowercase().contains("ok") {
            return Ok(()); // nothing recorded
        }
        for _ in 0..50 {
            let (code, b) = http::get(&t, &format!("/cgi-bin/mediaFileFind.cgi?action=findNextFile&object={object}&count=100"), to)?;
            if code != 200 {
                break;
            }
            let lines = http::kv_lines(&b);
            let found: usize = lines.iter().find(|(k, _)| k == "found").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
            for i in 0..found {
                let get = |name: &str| lines.iter().find(|(k, _)| k == &format!("items[{i}].{name}")).map(|(_, v)| v.clone());
                let (Some(s), Some(e)) = (get("StartTime"), get("EndTime")) else { continue };
                let flags: String = lines
                    .iter()
                    .filter(|(k, _)| k.starts_with(&format!("items[{i}].Flags")) || k.starts_with(&format!("items[{i}].Events")))
                    .map(|(_, v)| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                let kind = if flags.contains("VideoMotion") || flags.contains("Motion") {
                    RecordKind::Motion
                } else if flags.contains("Event") || flags.contains("Alarm") {
                    RecordKind::Alarm
                } else {
                    RecordKind::General
                };
                if filter == RecordFilter::General && kind != RecordKind::General {
                    continue;
                }
                let (Some(a), Some(b2)) = (secs_of_day(&s, day), secs_of_day(&e, day)) else { continue };
                let path = get("FilePath").unwrap_or_default();
                out.push(RecordPiece { start: a, end: b2.max(a), kind, path });
            }
            if found < 100 {
                break;
            }
        }
        Ok(())
    })();
    let _ = http::get(&t, &format!("/cgi-bin/mediaFileFind.cgi?action=close&object={object}"), to);
    let _ = http::get(&t, &format!("/cgi-bin/mediaFileFind.cgi?action=destroy&object={object}"), to);
    res?;
    out.sort_by_key(|p| p.start);
    Ok(out)
}

/// "2024-01-02 13:04:05" → seconds since midnight of `day` (clamped to the day).
fn secs_of_day(ts: &str, day: &str) -> Option<u32> {
    let (d, t) = ts.trim().split_once(' ')?;
    let mut it = t.split(':').map(|x| x.parse::<u32>().ok());
    let (h, m, s) = (it.next()??, it.next()??, it.next()??);
    let secs = h * 3600 + m * 60 + s;
    let norm = |x: &str| x.split('-').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (norm(d), norm(day));
    Some(match a.cmp(&b) {
        std::cmp::Ordering::Less => 0,
        std::cmp::Ordering::Greater => 86_399,
        std::cmp::Ordering::Equal => secs.min(86_399),
    })
}

// ---------------------------------------------------------------- alarms

#[derive(Clone, Debug)]
pub struct AlarmEvent {
    pub time: String,
    pub device: String,
    pub device_id: u64,
    pub channel: Option<u32>,
    pub code: String,
    pub started: bool,
}

pub fn alarm_label(code: &str) -> String {
    match code {
        "VideoMotion" => "Motion Detection".into(),
        "VideoLoss" => "Video Loss".into(),
        "VideoBlind" => "Video Tampering".into(),
        "AlarmLocal" => "Alarm Input".into(),
        "CrossLineDetection" => "Tripwire".into(),
        "CrossRegionDetection" => "Intrusion".into(),
        "SmartMotionHuman" => "Human Detected".into(),
        "SmartMotionVehicle" => "Vehicle Detected".into(),
        "StorageNotExist" | "StorageFailure" | "StorageLowSpace" => "Storage Problem".into(),
        "NewFile" => "New Recording".into(),
        other => other.to_string(),
    }
}

/// Keeps listening to one device's alarm feed until `stop` is set,
/// reconnecting every 10 s after a failure.
pub fn listen_alarms(dev: Device, stop: Arc<AtomicBool>, out: Sender<AlarmEvent>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let t = dev.http();
        let path = "/cgi-bin/eventManager.cgi?action=attach&codes=%5BAll%5D&heartbeat=5";
        while !stop.load(Ordering::Relaxed) {
            let _ = http::stream_lines(&t, path, &stop, |line| {
                // Code=VideoMotion;action=Start;index=0
                if !line.starts_with("Code=") {
                    return;
                }
                let mut code = String::new();
                let mut action = String::new();
                let mut index = None;
                for part in line.split(';') {
                    if let Some((k, v)) = part.split_once('=') {
                        match k {
                            "Code" => code = v.to_string(),
                            "action" => action = v.to_string(),
                            "index" => index = v.parse::<u32>().ok(),
                            _ => {}
                        }
                    }
                }
                if code.is_empty() || code == "HeartBeat" || action.eq_ignore_ascii_case("pulse") && code == "NewFile" {
                    return;
                }
                let _ = out.send(AlarmEvent {
                    time: crate::clock::now_text(),
                    device: dev.name.clone(),
                    device_id: dev.id,
                    channel: index,
                    code,
                    started: !action.eq_ignore_ascii_case("stop"),
                });
                ctx.request_repaint();
            });
            for _ in 0..100 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
}

// ---------------------------------------------------------------- local search

#[derive(Clone, Debug)]
pub struct FoundDevice {
    pub ip: String,
    pub rtsp: bool,
    pub web: bool,
}

/// The original limits its search to 16 network segments; we search the
/// segment this computer is on (x.x.x.1 - x.x.x.254) for video (554) and web (80) ports.
/// With `via`, searches that network connection's segment, through that connection (a second
/// router the computer is also joined to).
pub fn search_lan(out: Sender<Option<FoundDevice>>, ctx: egui::Context, via: Option<crate::netpath::NetLink>) {
    std::thread::spawn(move || {
        let local = match &via {
            Some(l) => Ok(SocketAddr::V4(std::net::SocketAddrV4::new(l.ip, 0))),
            None => UdpSocket::bind("0.0.0.0:0").and_then(|s| {
                s.connect("10.255.255.255:1")?;
                s.local_addr()
            }),
        };
        let Ok(SocketAddr::V4(local)) = local else {
            let _ = out.send(None);
            return;
        };
        let via = std::sync::Arc::new(via);
        let o = local.ip().octets();
        let ips: Vec<[u8; 4]> = (1..=254u8).filter(|&n| n != o[3]).map(|n| [o[0], o[1], o[2], n]).collect();
        let chunks: Vec<Vec<[u8; 4]>> = ips.chunks(16).map(|c| c.to_vec()).collect();
        let mut handles = Vec::new();
        for chunk in chunks {
            let out = out.clone();
            let ctx = ctx.clone();
            let via = via.clone();
            handles.push(std::thread::spawn(move || {
                for ip in chunk {
                    let a = std::net::Ipv4Addr::from(ip);
                    let to = Duration::from_millis(350);
                    let probe = |port| match via.as_ref() {
                        Some(l) => crate::netpath::bound_connect(l, SocketAddr::from((a, port)), to).is_ok(),
                        None => TcpStream::connect_timeout(&SocketAddr::from((a, port)), to).is_ok(),
                    };
                    let rtsp = probe(554);
                    if rtsp {
                        let web = probe(80);
                        let _ = out.send(Some(FoundDevice { ip: a.to_string(), rtsp, web }));
                        ctx.request_repaint();
                    }
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        let _ = out.send(None);
        ctx.request_repaint();
    });
}

/// For a recorder: which camera (address, video port, camera channel) is behind each channel,
/// from its RemoteChannel and RemoteDevice settings. Empty for a single camera.
pub fn camera_links(t: &Target, channels: usize) -> Vec<Option<crate::model::CameraLink>> {
    let to = Duration::from_secs(6);
    let Ok((200, rc)) = http::get(t, "/cgi-bin/configManager.cgi?action=getConfig&name=RemoteChannel", to) else { return Vec::new() };
    let Ok((200, rd)) = http::get(t, "/cgi-bin/configManager.cgi?action=getConfig&name=RemoteDevice", to) else { return Vec::new() };
    let rc = http::kv_lines(&rc);
    let rd = http::kv_lines(&rd);
    let get = |list: &Vec<(String, String)>, key: String| list.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
    let mut out = Vec::new();
    for i in 0..channels.max(1).min(256) {
        let enabled = get(&rc, format!("table.RemoteChannel[{i}].Enable")).map(|v| v == "true").unwrap_or(false);
        let dev = get(&rc, format!("table.RemoteChannel[{i}].Device"));
        let (Some(dev), true) = (dev, enabled) else {
            out.push(None);
            continue;
        };
        let base = format!("table.RemoteDevice.{dev}");
        let host = get(&rd, format!("{base}.Address")).unwrap_or_default();
        let on = get(&rd, format!("{base}.Enable")).map(|v| v == "true").unwrap_or(false);
        if host.is_empty() || host == "0.0.0.0" || !on {
            out.push(None);
            continue;
        }
        let port = get(&rd, format!("{base}.RtspPort")).and_then(|v| v.parse::<u16>().ok()).filter(|p| *p > 0).unwrap_or(554);
        let channel = get(&rc, format!("table.RemoteChannel[{i}].Channel")).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        out.push(Some(crate::model::CameraLink { host, rtsp_port: port, channel }));
    }
    if out.iter().all(|l| l.is_none()) {
        Vec::new()
    } else {
        out
    }
}
