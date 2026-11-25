//! Network connections (Wi-Fi, cable, mobile data) and reaching a device through a chosen one.
//!
//! For homes with more than one router: the computer keeps its main connection for everything
//! else, while MyCCTV talks to a recorder or camera through another network adapter that is on
//! that device's router (a second Wi-Fi adapter, a network cable, or on a phone, Wi-Fi next to
//! mobile data). Each device can name the connection it is reached through (`Device::network`);
//! every connection to that device's address (and to the cameras behind a recorder) then goes out
//! through that adapter, even when both routers use the same addresses (192.168.1.x).
//!
//! How a connection is tied to an adapter: Windows sends from the adapter that owns the source
//! address (strong host model), so binding to its address is enough; macOS also gets IP_BOUND_IF;
//! Android ties the socket to the Android network (`android_setsocknetwork`).

use crate::model::Device;
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Mobile is only seen on phones
pub enum LinkKind {
    Wifi,
    Wired,
    Mobile,
    Other,
}

/// One network connection of this computer / phone with an IPv4 address.
#[derive(Clone, Debug, PartialEq)]
pub struct NetLink {
    /// Stable name the device setting refers to: "Wi-Fi 2" (Windows), "en1" (macOS), "wlan0" (Android).
    pub name: String,
    pub kind: LinkKind,
    pub ip: Ipv4Addr,
    pub prefix: u8,
    /// The Wi-Fi network (router) it is joined to, when the system tells.
    pub ssid: Option<String>,
    /// Operating-system interface number (macOS).
    pub index: u32,
    /// Android network handle.
    pub handle: u64,
    /// The connection the system uses for the internet.
    pub main: bool,
}

impl NetLink {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            LinkKind::Wifi => "Wi-Fi",
            LinkKind::Wired => "Cable",
            LinkKind::Mobile => "Mobile data",
            LinkKind::Other => "Network",
        }
    }

    /// "Wi-Fi 2 · HomeRouter · 192.168.0.23"
    pub fn label(&self) -> String {
        let lower = self.name.to_ascii_lowercase();
        let mut s = if lower.contains("wi-fi") || lower.contains("wifi") || lower.contains("ethernet") || self.kind == LinkKind::Other {
            self.name.clone()
        } else {
            format!("{} ({})", self.kind_name(), self.name)
        };
        if let Some(ssid) = &self.ssid {
            s.push_str(&format!(" · {ssid}"));
        }
        s.push_str(&format!(" · {}", self.ip));
        s
    }

    /// Whether an address is on this connection's own network segment.
    #[allow(dead_code)]
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = if self.prefix == 0 { 0 } else { u32::MAX << (32 - self.prefix.min(32) as u32) };
        u32::from(ip) & mask == u32::from(self.ip) & mask
    }
}

// ---------------------------------------------------------------- the list of connections

struct Cache {
    at: Option<Instant>,
    links: Vec<NetLink>,
    busy: bool,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache { at: None, links: Vec::new(), busy: false }))
}

const FRESH: Duration = Duration::from_secs(4);

/// The connections as last seen; refreshed in the background when older than a few seconds,
/// so the screen never waits for the system.
pub fn links() -> Vec<NetLink> {
    let mut c = cache().lock().unwrap();
    let stale = c.at.map(|t| t.elapsed() > FRESH).unwrap_or(true);
    if stale && !c.busy {
        c.busy = true;
        std::thread::spawn(|| {
            refresh();
        });
    }
    c.links.clone()
}

/// The connections right now (waits for the system; for background threads).
pub fn links_now() -> Vec<NetLink> {
    refresh()
}

fn refresh() -> Vec<NetLink> {
    let mut l = enumerate();
    let main = main_ip();
    for x in l.iter_mut() {
        x.main = Some(x.ip) == main;
    }
    l.sort_by_key(|x| (!x.main, x.kind != LinkKind::Wifi, x.name.clone()));
    let mut c = cache().lock().unwrap();
    c.links = l.clone();
    c.at = Some(Instant::now());
    c.busy = false;
    l
}

/// The connection with this name, if it is connected.
pub fn find(name: &str) -> Option<NetLink> {
    {
        let c = cache().lock().unwrap();
        if c.at.map(|t| t.elapsed() <= FRESH).unwrap_or(false) {
            if let Some(l) = c.links.iter().find(|l| l.name == name) {
                return Some(l.clone());
            }
        }
    }
    links_now().into_iter().find(|l| l.name == name)
}

/// The address the system would use for the internet (no packet is sent).
fn main_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("1.1.1.1:53").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

#[cfg(not(target_os = "android"))]
fn enumerate() -> Vec<NetLink> {
    let mut out: Vec<NetLink> = Vec::new();
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        let if_addrs::IfAddr::V4(v4) = &i.addr else { continue };
        if v4.ip.is_loopback() || v4.ip.is_link_local() || out.iter().any(|o| o.name == i.name) {
            continue;
        }
        let lower = i.name.to_ascii_lowercase();
        // Virtual-machine networks inside this computer can't reach a router.
        if ["default switch", "(wsl", "virtualbox", "vmware network adapter", "loopback", "vmnet", "bridge100", "utun", "awdl", "llw"].iter().any(|v| lower.contains(v)) {
            continue;
        }
        let kind = if lower.contains("wi-fi") || lower.contains("wifi") || lower.contains("wlan") {
            LinkKind::Wifi
        } else if lower.contains("ethernet") {
            LinkKind::Wired
        } else {
            LinkKind::Other
        };
        out.push(NetLink {
            name: i.name.clone(),
            kind,
            ip: v4.ip,
            prefix: u32::from(v4.netmask).count_ones() as u8,
            ssid: None,
            index: i.index.unwrap_or(0),
            handle: 0,
            main: false,
        });
    }
    #[cfg(windows)]
    for w in windows_wifi_adapters() {
        if let Some(l) = out.iter_mut().find(|l| l.name == w.name) {
            l.kind = LinkKind::Wifi;
            l.ssid = w.ssid;
        }
    }
    #[cfg(target_os = "macos")]
    for (port, dev) in mac_hardware_ports() {
        if let Some(l) = out.iter_mut().find(|l| l.name == dev) {
            if port == "Wi-Fi" || port == "AirPort" {
                l.kind = LinkKind::Wifi;
                l.ssid = mac_ssid(&dev);
            } else if port.contains("Ethernet") || port.contains("LAN") || port.contains("Thunderbolt") {
                l.kind = LinkKind::Wired;
            }
        }
    }
    out
}

#[cfg(target_os = "android")]
fn enumerate() -> Vec<NetLink> {
    android::networks().unwrap_or_default()
}

// ---------------------------------------------------------------- which device goes where

fn routes() -> &'static Mutex<HashMap<String, String>> {
    static R: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Remembers which connection each device (and each camera behind a recorder) is reached through.
pub fn sync(devices: &[Device]) {
    let mut m = HashMap::new();
    for d in devices.iter().filter(|d| !d.network.is_empty()) {
        m.insert(d.wire_host().to_ascii_lowercase(), d.network.clone());
        for l in d.camera_links.iter().flatten() {
            m.insert(l.host.trim().to_ascii_lowercase(), d.network.clone());
        }
    }
    let mut r = routes().lock().unwrap();
    if *r != m {
        *r = m;
    }
}

/// The connection chosen for an address, if any.
pub fn via_for(host: &str) -> Option<String> {
    routes().lock().unwrap().get(&host.trim().to_ascii_lowercase()).cloned()
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr, String> {
    let all: Vec<SocketAddr> = (host, port).to_socket_addrs().map_err(|e| format!("cannot find {host}: {e}"))?.collect();
    all.iter().find(|a| a.is_ipv4()).or(all.first()).copied().ok_or_else(|| format!("cannot find {host}"))
}

/// Opens a TCP connection to a device, through its chosen network connection when it has one.
pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let addr = resolve(host, port)?;
    match via_for(host) {
        None => TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("cannot connect: {e}")),
        Some(name) => connect_via(&name, addr, timeout),
    }
}

/// Opens a TCP connection through the named network connection.
pub fn connect_via(name: &str, addr: SocketAddr, timeout: Duration) -> Result<TcpStream, String> {
    let link = find(name).ok_or_else(|| format!("the network connection \"{name}\" is not connected"))?;
    bound_connect(&link, addr, timeout).map_err(|e| format!("cannot connect through {name}: {e}"))
}

/// Opens a TCP connection that leaves through one network connection only.
pub fn bound_connect(link: &NetLink, addr: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
    use socket2::{Domain, Protocol, Socket, Type};
    if !addr.is_ipv4() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "only IPv4 addresses can use a chosen connection"));
    }
    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    #[cfg(target_os = "android")]
    if link.handle != 0 {
        use std::os::fd::AsRawFd;
        // SAFETY: plain C call on a socket we own.
        if unsafe { android::android_setsocknetwork(link.handle, sock.as_raw_fd()) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(i) = std::num::NonZeroU32::new(link.index) {
        sock.bind_device_by_index_v4(Some(i))?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = sock.bind_device(Some(link.name.as_bytes())); // needs privileges; the bind below still helps
    }
    sock.bind(&SocketAddr::new(IpAddr::V4(link.ip), 0).into())?;
    sock.connect_timeout(&addr.into(), timeout)?;
    let s: TcpStream = sock.into();
    s.set_nodelay(true).ok();
    Ok(s)
}

/// Keeps a local relay running while it lives.
pub struct Forward {
    stop: Arc<AtomicBool>,
    port: u16,
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], self.port)), Duration::from_millis(300));
    }
}

/// For an rtsp:// address of a device reached through a chosen connection: starts a relay on
/// 127.0.0.1 that forwards to the device through that connection, and returns the address rewritten
/// to the relay (the video library opens its own connections). Other addresses come back unchanged.
pub fn rtsp_through(url: &str) -> Result<(String, Option<Forward>), String> {
    let Ok(mut u) = url::Url::parse(url) else { return Ok((url.to_string(), None)) };
    let Some(host) = u.host_str().map(|h| h.trim_matches(['[', ']']).to_string()) else { return Ok((url.to_string(), None)) };
    let Some(name) = via_for(&host) else { return Ok((url.to_string(), None)) };
    let addr = resolve(&host, u.port().unwrap_or(554))?;
    if find(&name).is_none() {
        return Err(format!("the network connection \"{name}\" is not connected"));
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("local relay: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    std::thread::spawn(move || {
        for c in listener.incoming() {
            if st.load(Ordering::Relaxed) {
                break;
            }
            let Ok(c) = c else { continue };
            let name = name.clone();
            std::thread::spawn(move || match connect_via(&name, addr, Duration::from_secs(8)) {
                Ok(up) => pipe(c, up),
                Err(e) => {
                    log::warn!("relay to {addr}: {e}");
                    let _ = c.shutdown(Shutdown::Both);
                }
            });
        }
    });
    let _ = u.set_host(Some("127.0.0.1"));
    let _ = u.set_port(Some(port));
    Ok((u.to_string(), Some(Forward { stop, port })))
}

fn pipe(a: TcpStream, b: TcpStream) {
    let (Ok(mut a2), Ok(mut b2)) = (a.try_clone(), b.try_clone()) else { return };
    let t = std::thread::spawn(move || {
        let _ = io::copy(&mut a2, &mut b2);
        let _ = b2.shutdown(Shutdown::Both);
        let _ = a2.shutdown(Shutdown::Both);
    });
    let (mut a, mut b) = (a, b);
    let _ = io::copy(&mut b, &mut a);
    let _ = a.shutdown(Shutdown::Both);
    let _ = b.shutdown(Shutdown::Both);
    let _ = t.join();
}

// ---------------------------------------------------------------- joining another router

/// A Wi-Fi network (router) a Wi-Fi adapter can see.
#[derive(Clone, Debug)]
pub struct WifiNet {
    pub ssid: String,
    pub auth: String,
    pub signal: Option<u8>,
    pub saved: bool,
}

/// Whether this system lets MyCCTV list the Wi-Fi networks an adapter can see.
pub const CAN_SCAN: bool = cfg!(windows);
/// Whether this system lets MyCCTV join an adapter to a Wi-Fi network (else: open the system's Wi-Fi settings).
pub const CAN_JOIN: bool = cfg!(any(windows, target_os = "macos"));

/// Runs a system tool without a console window and returns everything it printed.
#[cfg(any(windows, target_os = "macos"))]
fn tool(program: &str, args: &[String]) -> Result<(bool, String), String> {
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        for a in args {
            c.raw_arg(a); // netsh wants name="Wi-Fi 2" exactly
        }
    }
    #[cfg(not(windows))]
    c.args(args);
    let out = c.output().map_err(|e| format!("{program}: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

#[cfg(windows)]
fn quoted(key: &str, value: &str) -> Result<String, String> {
    if value.contains('"') {
        return Err("Names with a \" character can't be used".into());
    }
    Ok(format!("{key}=\"{value}\""))
}

#[cfg(windows)]
const LOCATION_HELP: &str = "Windows only shows Wi-Fi networks to apps when Location is allowed: Settings > Privacy & security > Location > turn on \"Let desktop apps access your location\".";

/// "Name : value" lines of netsh output.
#[cfg(windows)]
fn kv(line: &str) -> Option<(String, String)> {
    let (k, v) = line.split_once(" : ")?;
    Some((k.trim().to_string(), v.trim().to_string()))
}

#[cfg(windows)]
struct WinWifi {
    name: String,
    ssid: Option<String>,
    connected: bool,
}

/// Set when Windows hid the Wi-Fi network names because Location is off for apps.
#[cfg(windows)]
static NAMES_HIDDEN: AtomicBool = AtomicBool::new(false);

/// Whether the system is hiding Wi-Fi network names from MyCCTV, and how to allow them.
pub fn wifi_names_hidden() -> Option<&'static str> {
    #[cfg(windows)]
    if NAMES_HIDDEN.load(Ordering::Relaxed) {
        return Some(LOCATION_HELP);
    }
    None
}

/// Every network adapter Windows has, with or without an address: (name, description, type, up).
#[cfg(windows)]
fn windows_adapters() -> Vec<(String, String, u32, bool)> {
    use windows_sys::Win32::NetworkManagement::IpHelper::*;
    fn wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: Windows gives NUL-terminated UTF-16 strings inside the buffer we own.
        unsafe {
            let mut n = 0;
            while *p.add(n) != 0 && n < 512 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        }
    }
    let mut size: u32 = 64 * 1024;
    for _ in 0..4 {
        let mut buf: Vec<u64> = vec![0; size as usize / 8 + 1];
        let first = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let flags = GAA_FLAG_SKIP_UNICAST | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        // SAFETY: buffer and size describe memory we own; Windows fills a linked list inside it.
        let r = unsafe { GetAdaptersAddresses(0, flags, std::ptr::null(), first, &mut size) };
        if r == 111 {
            continue; // ERROR_BUFFER_OVERFLOW: size now holds what is needed
        }
        if r != 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cur = first;
        while !cur.is_null() {
            // SAFETY: nodes of the list Windows wrote into `buf`, which outlives this loop.
            let a = unsafe { &*cur };
            out.push((wide(a.FriendlyName), wide(a.Description), a.IfType, a.OperStatus == 1));
            cur = a.Next;
        }
        return out;
    }
    Vec::new()
}

/// Network cable ports with nothing plugged in (Windows): a cable from the cameras' router there
/// gives a second connection.
pub fn free_cable_ports() -> Vec<String> {
    #[cfg(windows)]
    {
        windows_adapters()
            .into_iter()
            .filter(|(_, desc, t, up)| {
                let d = desc.to_ascii_lowercase();
                *t == 6 && !up && !["virtual", "tap", "bluetooth", "hyper-v", "vpn", "miniport", "tunnel", "loopback"].iter().any(|v| d.contains(v))
            })
            .map(|(name, desc, _, _)| format!("{name} ({desc})"))
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn windows_wifi_adapters() -> Vec<WinWifi> {
    // The adapters themselves from Windows' network list (works without any permission)...
    let mut out: Vec<WinWifi> = windows_adapters()
        .into_iter()
        .filter(|(name, desc, t, _)| *t == 71 && !desc.contains("Wi-Fi Direct") && !desc.contains("Virtual") && !name.starts_with("Local Area Connection*"))
        .map(|(name, _, _, up)| WinWifi { name, ssid: None, connected: up })
        .collect();
    // ...and the router each one is joined to from netsh (Windows hides it while Location is off).
    if let Ok((_, text)) = tool("netsh", &["wlan".into(), "show".into(), "interfaces".into()]) {
        let mut current: Option<String> = None;
        for line in text.lines() {
            let Some((k, v)) = kv(line) else { continue };
            match k.as_str() {
                "Name" => current = Some(v),
                "SSID" if !v.is_empty() => {
                    if let Some(w) = out.iter_mut().find(|w| Some(&w.name) == current.as_ref()) {
                        w.ssid = Some(v);
                    }
                }
                _ => {}
            }
        }
        NAMES_HIDDEN.store(!out.is_empty() && text.to_ascii_lowercase().contains("location"), Ordering::Relaxed);
    }
    out
}

/// A Wi-Fi adapter and the router it is joined to (None: not joined).
#[derive(Clone, Debug, PartialEq)]
pub struct WifiAdapter {
    pub name: String,
    pub ssid: Option<String>,
    /// Joined to a router (also when the system hides the router's name).
    pub connected: bool,
}

/// Every Wi-Fi adapter (connected or not), so a second adapter can be joined to a router.
pub fn wifi_adapters() -> Vec<WifiAdapter> {
    #[cfg(windows)]
    {
        windows_wifi_adapters().into_iter().map(|w| WifiAdapter { name: w.name, ssid: w.ssid, connected: w.connected }).collect()
    }
    #[cfg(target_os = "macos")]
    {
        mac_hardware_ports()
            .into_iter()
            .filter(|(p, _)| p == "Wi-Fi" || p == "AirPort")
            .map(|(_, d)| {
                let ssid = mac_ssid(&d);
                WifiAdapter { connected: ssid.is_some(), ssid, name: d }
            })
            .collect()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Vec::new()
    }
}

/// The Wi-Fi networks one adapter can see (Windows).
pub fn wifi_scan(_adapter: &str) -> Result<Vec<WifiNet>, String> {
    #[cfg(windows)]
    {
        let iface = quoted("interface", _adapter)?;
        let (_, profiles) = tool("netsh", &["wlan".into(), "show".into(), "profiles".into(), iface.clone()])?;
        let saved: Vec<String> = profiles.lines().filter(|l| l.contains("Profile")).filter_map(kv).map(|(_, v)| v).collect();
        let (_, text) = tool("netsh", &["wlan".into(), "show".into(), "networks".into(), iface, "mode=bssid".into()])?;
        if !text.contains("SSID 1") && text.to_ascii_lowercase().contains("location") {
            return Err(LOCATION_HELP.into());
        }
        let mut out: Vec<WifiNet> = Vec::new();
        for line in text.lines() {
            let Some((k, v)) = kv(line) else { continue };
            if k.starts_with("SSID ") {
                if !v.is_empty() && !out.iter().any(|n| n.ssid == v) {
                    out.push(WifiNet { saved: saved.contains(&v), ssid: v, auth: String::new(), signal: None });
                } else {
                    out.push(WifiNet { ssid: String::new(), auth: String::new(), signal: None, saved: false }); // hidden or repeat: dropped below
                }
            } else if let Some(n) = out.last_mut() {
                if k == "Authentication" && n.auth.is_empty() {
                    n.auth = v;
                } else if k == "Signal" && n.signal.is_none() {
                    n.signal = v.trim_end_matches('%').trim().parse().ok();
                }
            }
        }
        out.retain(|n| !n.ssid.is_empty());
        out.sort_by_key(|n| std::cmp::Reverse(n.signal.unwrap_or(0)));
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        Err("Type the Wi-Fi network's name below.".into())
    }
}

#[cfg(windows)]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Joins one Wi-Fi adapter to a Wi-Fi network; the other connections stay as they are.
/// `auth` is what the scan reported ("WPA2-Personal", "Open"...), or empty.
pub fn wifi_connect(_adapter: &str, _ssid: &str, _password: &str, _auth: &str, _saved: bool) -> Result<String, String> {
    #[cfg(windows)]
    {
        let iface = quoted("interface", _adapter)?;
        let name = quoted("name", _ssid)?;
        let ssid_arg = quoted("ssid", _ssid)?;
        if !_saved || !_password.is_empty() {
            let open = _auth.eq_ignore_ascii_case("open") || (_password.is_empty() && !_saved);
            let (a, e) = if open {
                ("open", "none")
            } else if _auth.contains("WPA3") {
                ("WPA3SAE", "AES")
            } else if _auth.contains("WPA-") {
                ("WPAPSK", "AES")
            } else {
                ("WPA2PSK", "AES")
            };
            let key = if open {
                String::new()
            } else {
                format!("<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>", xml_escape(_password))
            };
            // connectionMode manual: Windows won't join this router by itself (on any adapter).
            let xml = format!(
                "<?xml version=\"1.0\"?>\r\n<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{n}</name><SSIDConfig><SSID><name>{n}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>manual</connectionMode><MSM><security><authEncryption><authentication>{a}</authentication><encryption>{e}</encryption><useOneX>false</useOneX></authEncryption>{key}</security></MSM></WLANProfile>",
                n = xml_escape(_ssid)
            );
            let path = std::env::temp_dir().join(format!("mycctv-wifi-{}.xml", std::process::id()));
            std::fs::write(&path, xml).map_err(|e| format!("cannot write the Wi-Fi profile: {e}"))?;
            let file = quoted("filename", &path.to_string_lossy())?;
            let r = tool("netsh", &["wlan".into(), "add".into(), "profile".into(), file, iface.clone(), "user=current".into()]);
            let _ = std::fs::remove_file(&path);
            let (ok, text) = r?;
            if !ok {
                return Err(text.trim().to_string());
            }
        }
        let (ok, text) = tool("netsh", &["wlan".into(), "connect".into(), name, ssid_arg, iface])?;
        if !ok {
            return Err(text.trim().to_string());
        }
        // Wait for the adapter to get an address from the router.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(750));
            if let Some(l) = links_now().into_iter().find(|l| l.name == _adapter) {
                if l.ssid.as_deref() == Some(_ssid) {
                    return Ok(format!("{} joined {} ({})", _adapter, _ssid, l.ip));
                }
            }
        }
        Ok(format!("{_adapter} is joining {_ssid}; it can take a few more seconds to get an address"))
    }
    #[cfg(target_os = "macos")]
    {
        let mut args = vec!["-setairportnetwork".to_string(), _adapter.to_string(), _ssid.to_string()];
        if !_password.is_empty() {
            args.push(_password.to_string());
        }
        let (ok, text) = tool("networksetup", &args)?;
        if !ok || text.to_ascii_lowercase().contains("error") || text.to_ascii_lowercase().contains("could not") {
            return Err(text.trim().to_string());
        }
        Ok(format!("{_adapter} joined {_ssid}"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("Join the network in the system's Wi-Fi settings".into())
    }
}

/// Disconnects one Wi-Fi adapter (Windows).
pub fn wifi_disconnect(_adapter: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let (ok, text) = tool("netsh", &["wlan".into(), "disconnect".into(), quoted("interface", _adapter)?])?;
        if ok {
            Ok(())
        } else {
            Err(text.trim().to_string())
        }
    }
    #[cfg(not(windows))]
    {
        Err("Disconnect it in the system's Wi-Fi settings".into())
    }
}

/// Windows: makes a connection the last choice for the internet (interface metric 9000), so the
/// computer keeps using its main connection after another adapter joins the cameras' router.
/// Windows asks the person for permission (administrator).
pub fn keep_internet_off(_adapter: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let inner = format!("Set-NetIPInterface -InterfaceAlias '{}' -InterfaceMetric 9000", _adapter.replace('\'', "''"));
        let utf16: Vec<u8> = inner.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let outer = format!(
            "$p = Start-Process powershell -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ArgumentList '-NoProfile','-EncodedCommand','{}'; exit $p.ExitCode",
            base64(&utf16)
        );
        let (ok, text) = tool("powershell", &["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), format!("\"{outer}\"")])?;
        if ok {
            Ok(())
        } else if text.to_ascii_lowercase().contains("cancel") {
            Err("Windows didn't get permission".into())
        } else {
            Err(text.trim().chars().take(300).collect())
        }
    }
    #[cfg(not(windows))]
    {
        Err("Change the order of connections in the system's network settings".into())
    }
}

#[cfg(windows)]
fn base64(b: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Opens the system's own Wi-Fi settings.
pub fn open_wifi_settings() {
    #[cfg(windows)]
    {
        let _ = tool("explorer", &["ms-settings:network-wifi".into()]);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = tool("open", &["x-apple.systempreferences:com.apple.wifi-settings-extension".into()]);
    }
    #[cfg(target_os = "android")]
    {
        if let Err(e) = android::open_wifi_panel() {
            log::warn!("Wi-Fi settings: {e}");
        }
    }
}

#[cfg(target_os = "macos")]
fn mac_hardware_ports() -> Vec<(String, String)> {
    let Ok((_, text)) = tool("networksetup", &["-listallhardwareports".into()]) else { return Vec::new() };
    let mut out = Vec::new();
    let mut port = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("Hardware Port: ") {
            port = Some(p.trim().to_string());
        } else if let (Some(d), Some(p)) = (line.strip_prefix("Device: "), port.take()) {
            out.push((p, d.trim().to_string()));
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn mac_ssid(dev: &str) -> Option<String> {
    let (_, text) = tool("networksetup", &["-getairportnetwork".into(), dev.into()]).ok()?;
    text.split_once("Network: ").map(|(_, s)| s.trim().to_string()).filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------- Android

#[cfg(target_os = "android")]
mod android {
    use super::{LinkKind, NetLink};
    use jni::objects::{JObject, JObjectArray, JString, JValue};
    use jni::JNIEnv;
    use std::net::Ipv4Addr;

    #[link(name = "android")]
    extern "C" {
        /// libandroid (API 23+): ties a socket to one Android network.
        pub fn android_setsocknetwork(network: u64, fd: std::os::raw::c_int) -> std::os::raw::c_int;
    }

    fn with_env<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Result<R, String> {
        let app = crate::platform::android_app().ok_or("no Android activity")?;
        // SAFETY: pointers come from the running activity and stay valid while the app runs.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }.map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let r = f(&mut env, &activity);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        r.map_err(|e| e.to_string())
    }

    pub fn networks() -> Result<Vec<NetLink>, String> {
        with_env(|env, activity| {
            let svc = env.new_string("connectivity")?;
            let cm = env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&svc)])?.l()?;
            let nets = JObjectArray::from(env.call_method(&cm, "getAllNetworks", "()[Landroid/net/Network;", &[])?.l()?);
            let count = env.get_array_length(&nets)?;
            let mut out = Vec::new();
            for i in 0..count {
                let net = env.get_object_array_element(&nets, i)?;
                let caps = env.call_method(&cm, "getNetworkCapabilities", "(Landroid/net/Network;)Landroid/net/NetworkCapabilities;", &[JValue::Object(&net)])?.l()?;
                let lp = env.call_method(&cm, "getLinkProperties", "(Landroid/net/Network;)Landroid/net/LinkProperties;", &[JValue::Object(&net)])?.l()?;
                if caps.is_null() || lp.is_null() {
                    continue;
                }
                let mut has = |t: i32| -> jni::errors::Result<bool> { env.call_method(&caps, "hasTransport", "(I)Z", &[JValue::Int(t)])?.z() };
                // TRANSPORT_CELLULAR 0, WIFI 1, ETHERNET 3, VPN 4
                if has(4)? {
                    continue;
                }
                let kind = if has(1)? {
                    LinkKind::Wifi
                } else if has(3)? {
                    LinkKind::Wired
                } else if has(0)? {
                    LinkKind::Mobile
                } else {
                    LinkKind::Other
                };
                let ifname = env.call_method(&lp, "getInterfaceName", "()Ljava/lang/String;", &[])?.l()?;
                let name: String = if ifname.is_null() { format!("network{i}") } else { env.get_string(&JString::from(ifname))?.into() };
                let handle = env.call_method(&net, "getNetworkHandle", "()J", &[])?.j()? as u64;
                let addrs = env.call_method(&lp, "getLinkAddresses", "()Ljava/util/List;", &[])?.l()?;
                let n = env.call_method(&addrs, "size", "()I", &[])?.i()?;
                for j in 0..n {
                    let la = env.call_method(&addrs, "get", "(I)Ljava/lang/Object;", &[JValue::Int(j)])?.l()?;
                    let ia = env.call_method(&la, "getAddress", "()Ljava/net/InetAddress;", &[])?.l()?;
                    let raw = jni::objects::JByteArray::from(env.call_method(&ia, "getAddress", "()[B", &[])?.l()?);
                    let bytes = env.convert_byte_array(&raw)?;
                    if bytes.len() != 4 {
                        continue;
                    }
                    let prefix = env.call_method(&la, "getPrefixLength", "()I", &[])?.i()? as u8;
                    out.push(NetLink {
                        name: name.clone(),
                        kind,
                        ip: Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]),
                        prefix,
                        ssid: None,
                        index: 0,
                        handle,
                        main: false,
                    });
                    break;
                }
            }
            Ok(out)
        })
    }

    pub fn open_wifi_panel() -> Result<(), String> {
        with_env(|env, activity| {
            for action in ["android.settings.panel.action.WIFI", "android.settings.WIFI_SETTINGS"] {
                let a = env.new_string(action)?;
                let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&a)])?;
                if env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]).is_ok() {
                    return Ok(());
                }
                let _ = env.exception_clear();
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments() {
        let l = NetLink { name: "Wi-Fi 2".into(), kind: LinkKind::Wifi, ip: Ipv4Addr::new(192, 168, 1, 40), prefix: 24, ssid: Some("Cams".into()), index: 0, handle: 0, main: false };
        assert!(l.contains(Ipv4Addr::new(192, 168, 1, 33)));
        assert!(!l.contains(Ipv4Addr::new(192, 168, 0, 33)));
        assert_eq!(l.label(), "Wi-Fi 2 · Cams · 192.168.1.40");
    }

    /// A pretend device on this computer's own address, reached through a chosen connection: the
    /// relay must carry bytes both ways and the device must see the chosen connection's address.
    #[test]
    fn relay_through_chosen_connection() {
        use std::io::{Read, Write};
        let Some(link) = links_now().into_iter().next() else { return };
        let server = TcpListener::bind((link.ip, 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let host = link.ip.to_string();
        let form = crate::model::DeviceForm { name: "Test".into(), host: host.clone(), network: link.name.clone(), ..Default::default() };
        let dev = form.validate(&[]).unwrap();
        sync(&[dev]);
        assert_eq!(via_for(&host).as_deref(), Some(link.name.as_str()));
        let t = std::thread::spawn(move || {
            let (mut c, from) = server.accept().unwrap();
            let mut b = [0u8; 5];
            c.read_exact(&mut b).unwrap();
            c.write_all(b"pong!").unwrap();
            (from.ip(), b)
        });
        let (url, relay) = rtsp_through(&format!("rtsp://{host}:{port}/cam?channel=1")).unwrap();
        assert!(relay.is_some() && url.starts_with("rtsp://127.0.0.1:"), "{url}");
        let u = url::Url::parse(&url).unwrap();
        let mut c = TcpStream::connect(("127.0.0.1", u.port().unwrap())).unwrap();
        c.write_all(b"ping!").unwrap();
        let mut b = [0u8; 5];
        c.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"pong!");
        let (from, got) = t.join().unwrap();
        assert_eq!(&got, b"ping!");
        assert_eq!(from, IpAddr::V4(link.ip));
        sync(&[]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_adapter_list() {
        let all = windows_adapters();
        assert!(!all.is_empty());
        for (n, d, t, up) in all {
            println!("adapter {n} | {d} | type {t} | up {up}");
        }
    }

    #[test]
    fn lists_this_computer() {
        // Whatever this machine has: every entry must have a usable address.
        for l in links_now() {
            assert!(!l.ip.is_loopback(), "{l:?}");
            println!("{}{}", l.label(), if l.main { "  (main)" } else { "" });
        }
    }
}
