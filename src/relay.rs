//! "TV server": lets smart TVs that can only run web apps (Hisense VIDAA OS, and any TV or
//! device with a browser) watch the cameras. MyCCTV connects to the cameras as usual and serves:
//!
//! - `/`                      the TV web app (files in `vidaa/`, built into the program)
//! - `/api/devices?key=`      devices and cameras (no passwords)
//! - `/mjpeg?key=&dev=&ch=&s=` live video as a stream of JPEG pictures (works in every browser)
//! - `/api/ptz?key=&dev=&ch=&cmd=&start=` camera control
//!
//! Only this local network can reach it, and every request needs the access code.

use crate::model::Device;
use crate::services::{self, Ptz};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INDEX_HTML: &str = include_str!("../vidaa/index.html");
const APP_JS: &str = include_str!("../vidaa/app.js");
const STYLE_CSS: &str = include_str!("../vidaa/style.css");
const ICON_PNG: &[u8] = include_bytes!("../vidaa/icon.png");

/// Largest picture width sent to a TV (keeps decoding on the TV light).
const MAX_WIDTH: usize = 1280;
/// Pictures per second sent to a TV.
const TV_FPS: f32 = 10.0;

pub struct TvServer {
    pub port: u16,
    stop: Arc<AtomicBool>,
    pub viewers: Arc<AtomicUsize>,
    devices: Arc<Mutex<Vec<Device>>>,
}

impl Drop for TvServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the listener so it notices.
        let _ = TcpStream::connect_timeout(&format!("127.0.0.1:{}", self.port).parse().unwrap(), Duration::from_millis(300));
    }
}

impl TvServer {
    /// Updates the device list the TVs see (call after devices change).
    pub fn set_devices(&self, devices: &[Device]) {
        *self.devices.lock().unwrap() = devices.to_vec();
    }
}

/// This computer's address on the local network (what to type on the TV).
pub fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:1").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

pub fn start(port: u16, key: String, devices: &[Device]) -> Result<TvServer, String> {
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("cannot open port {port}: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let viewers = Arc::new(AtomicUsize::new(0));
    let devs = Arc::new(Mutex::new(devices.to_vec()));
    let (s2, v2, d2) = (stop.clone(), viewers.clone(), devs.clone());
    std::thread::Builder::new()
        .name("tv-server".into())
        .spawn(move || {
            for conn in listener.incoming() {
                if s2.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                if !is_local(&conn) {
                    continue;
                }
                let (k, v, d, s) = (key.clone(), v2.clone(), d2.clone(), s2.clone());
                std::thread::spawn(move || {
                    let _ = handle(conn, &k, &v, &d, &s);
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(TvServer { port, stop, viewers, devices: devs })
}

/// Only private / local addresses may connect.
fn is_local(c: &TcpStream) -> bool {
    match c.peer_addr().map(|a| a.ip()) {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

fn query(path: &str) -> (String, HashMap<String, String>) {
    let (p, q) = path.split_once('?').unwrap_or((path, ""));
    let mut m = HashMap::new();
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = url::form_urlencoded::parse(format!("x={v}").as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default();
            m.insert(k.to_string(), v);
        }
    }
    (p.to_string(), m)
}

fn respond(c: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(c, "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n", body.len())?;
    c.write_all(body)
}

fn handle(mut c: TcpStream, key: &str, viewers: &AtomicUsize, devices: &Mutex<Vec<Device>>, stop: &AtomicBool) -> std::io::Result<()> {
    c.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(c.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
    }
    let (p, q) = query(&path);
    // The web app's own files need no code (they hold no camera data).
    match p.as_str() {
        "/" | "/index.html" => return respond(&mut c, "200 OK", "text/html; charset=utf-8", INDEX_HTML.as_bytes()),
        "/app.js" => return respond(&mut c, "200 OK", "application/javascript; charset=utf-8", APP_JS.as_bytes()),
        "/style.css" => return respond(&mut c, "200 OK", "text/css; charset=utf-8", STYLE_CSS.as_bytes()),
        "/icon.png" => return respond(&mut c, "200 OK", "image/png", ICON_PNG),
        // The Android / Fire TV package, when it sits next to this program (for the Downloader app).
        "/apk" | "/MyCCTV.apk" => {
            let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
            let apk = dir.and_then(|d| std::fs::read_dir(d).ok()).and_then(|rd| {
                rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("apk")).unwrap_or(false)).max()
            });
            return match apk.and_then(|p| std::fs::read(p).ok()) {
                Some(bytes) => {
                    write!(c, "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.android.package-archive\r\nContent-Disposition: attachment; filename=\"MyCCTV.apk\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len())?;
                    c.write_all(&bytes)
                }
                None => respond(&mut c, "404 Not Found", "text/plain", b"Put the MyCCTV .apk file next to mycctv.exe to share it here."),
            };
        }
        _ => {}
    }
    if q.get("key").map(|k| k.as_str()) != Some(key) {
        return respond(&mut c, "403 Forbidden", "application/json", br#"{"error":"wrong access code"}"#);
    }
    let find = |q: &HashMap<String, String>| -> Option<(Device, u32)> {
        let id: u64 = q.get("dev")?.parse().ok()?;
        let ch: u32 = q.get("ch")?.parse().ok()?;
        let d = devices.lock().unwrap().iter().find(|d| d.id == id).cloned()?;
        (ch < d.channels).then_some((d, ch))
    };
    match p.as_str() {
        "/api/devices" => {
            let list: Vec<serde_json::Value> = devices
                .lock()
                .unwrap()
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        "name": d.name,
                        "cameras": (0..d.channels).map(|c| serde_json::json!({"ch": c, "name": d.channel_name(c)})).collect::<Vec<_>>(),
                    })
                })
                .collect();
            respond(&mut c, "200 OK", "application/json", serde_json::to_string(&list).unwrap_or_default().as_bytes())
        }
        "/api/ptz" => {
            let Some((d, ch)) = find(&q) else { return respond(&mut c, "404 Not Found", "application/json", b"{}") };
            let cmd = match q.get("cmd").map(|s| s.as_str()).unwrap_or("") {
                "up" => Ptz::Up,
                "down" => Ptz::Down,
                "left" => Ptz::Left,
                "right" => Ptz::Right,
                "zoomin" => Ptz::ZoomIn,
                "zoomout" => Ptz::ZoomOut,
                _ => return respond(&mut c, "400 Bad Request", "application/json", b"{}"),
            };
            let (tx, _rx) = std::sync::mpsc::channel();
            services::ptz(&d, ch, cmd, q.get("start").map(|s| s == "1").unwrap_or(false), 5, tx);
            respond(&mut c, "200 OK", "application/json", br#"{"ok":true}"#)
        }
        "/mjpeg" => {
            let Some((d, ch)) = find(&q) else { return respond(&mut c, "404 Not Found", "text/plain", b"no such camera") };
            let stream_type: u8 = q.get("s").and_then(|s| s.parse().ok()).unwrap_or(1);
            let all = devices.lock().unwrap().clone();
            viewers.fetch_add(1, Ordering::Relaxed);
            let r = mjpeg(&mut c, d.live_routes(ch, stream_type, &all), stop);
            viewers.fetch_sub(1, Ordering::Relaxed);
            r
        }
        _ => respond(&mut c, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Sends live video as multipart JPEG until the TV disconnects.
fn mjpeg(c: &mut TcpStream, routes: Vec<crate::stream::Route>, stop: &AtomicBool) -> std::io::Result<()> {
    c.set_write_timeout(Some(Duration::from_secs(10)))?;
    write!(c, "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=mycctvframe\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n")?;
    let h = crate::stream::start_routes_always(egui::Context::default(), routes, false);
    let gap = Duration::from_secs_f32(1.0 / TV_FPS);
    let mut last = Instant::now() - gap;
    let mut seen = 0u64;
    let mut idle = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(15));
        let f = h.shared.frames.load(Ordering::Relaxed);
        // No camera / refused: end the stream so the TV page shows "No video".
        if f == 0 && matches!(h.shared.state(), crate::stream::State::Error(_) | crate::stream::State::Unsupported(_)) {
            return Ok(());
        }
        if f == seen || last.elapsed() < gap {
            // Keep the connection alive while waiting (and notice a closed TV page).
            if idle.elapsed() > Duration::from_secs(5) {
                c.write_all(b"\r\n")?;
                idle = Instant::now();
            }
            continue;
        }
        seen = f;
        let Some(pic) = h.shared.picture.lock().unwrap().take() else { continue };
        let jpeg = encode(&pic);
        h.shared.recycle(pic.rgba);
        write!(c, "--mycctvframe\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len())?;
        c.write_all(&jpeg)?;
        c.write_all(b"\r\n")?;
        last = Instant::now();
        idle = Instant::now();
    }
}

/// RGBA picture → JPEG, scaled down to MAX_WIDTH.
pub fn encode(p: &crate::stream::Picture) -> Vec<u8> {
    let step = p.w.div_ceil(MAX_WIDTH).max(1);
    let (w, h) = (p.w / step, p.h / step);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = &p.rgba[(y * step) * p.w * 4..];
        for x in 0..w {
            let o = x * step * 4;
            rgb.extend_from_slice(&row[o..o + 3]);
        }
    }
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 65);
    let _ = enc.encode(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8);
    out
}
