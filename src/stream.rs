//! One video connection: pulls a camera stream over RTSP, decodes H.264 to
//! pictures and hands the newest picture to the screen. Each window gets its
//! own worker thread so a slow camera never freezes the others.

use futures::StreamExt;
use retina::client::{Credentials, PlayOptions, Session, SessionOptions, SetupOptions, Transport};
use retina::codec::{CodecItem, FrameFormat};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Connecting,
    Playing,
    Unsupported(String),
    Error(String),
    Ended,
    /// Stopped because nobody is looking (another page, the app minimized or in the background).
    Paused,
}

#[derive(Default)]
pub struct Shared {
    pub picture: Mutex<Option<Picture>>,
    pub state: Mutex<Option<State>>,
    pub info: Mutex<String>,
    pub notice: Mutex<Option<String>>,
    /// Full technical text of the last error.
    pub detail: Mutex<String>,
    /// Which route is in use ("" for the device itself).
    pub route: Mutex<String>,
    /// Playback: seconds since midnight of the picture on screen, plus one (0 = unknown).
    pub position: AtomicU64,
    pub stop: AtomicBool,
    pub frames: AtomicU64,
    /// Live video: how far (ms) the newest picture is behind the camera's own clock, measured from the
    /// first picture. Grows steadily when decoding can't keep up.
    pub lag_ms: AtomicU64,
    record_to: Mutex<Option<PathBuf>>,
    pub recording: AtomicBool,
    snapshot_to: Mutex<Option<PathBuf>>,
    /// Playback speed x1000 (1000 = normal). Only slower speeds are paced locally.
    pub speed_milli: AtomicU64,
    /// Keeps running while the app's screen is not drawn (TV sharing streams).
    pub always: AtomicBool,
}

impl Shared {
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone().unwrap_or(State::Connecting)
    }
    fn set_state(&self, s: State) {
        *self.state.lock().unwrap() = Some(s);
    }
    fn notice(&self, s: String) {
        *self.notice.lock().unwrap() = Some(s);
    }
}

pub struct StreamHandle {
    pub shared: Arc<Shared>,
    pub url: String,
    pub is_playback: bool,
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

impl StreamHandle {
    pub fn start_recording(&self, file: PathBuf) {
        *self.shared.record_to.lock().unwrap() = Some(file);
        self.shared.recording.store(true, Ordering::Relaxed);
    }
    pub fn stop_recording(&self) {
        self.shared.recording.store(false, Ordering::Relaxed);
    }
    pub fn snapshot(&self, file: PathBuf) {
        *self.shared.snapshot_to.lock().unwrap() = Some(file);
    }
    pub fn set_speed(&self, speed: f32) {
        self.shared.speed_milli.store((speed * 1000.0).round().max(1.0) as u64, Ordering::Relaxed);
    }
}

/// One way to reach a picture: address, user name, password, and a short label
/// ("recorder", "camera 192.168.1.111") shown when it is the one that works.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub url: String,
    pub user: String,
    pub pass: String,
    pub label: String,
}

pub fn start(ctx: egui::Context, url: String, user: String, pass: String, is_playback: bool) -> StreamHandle {
    start_routes(ctx, vec![Route { url, user, pass, label: String::new() }], is_playback)
}

/// Starts a window that tries each route in turn until one gives video
/// (for a recorder: the recorder first, then the camera behind that channel).
pub fn start_routes(ctx: egui::Context, routes: Vec<Route>, is_playback: bool) -> StreamHandle {
    start_inner(ctx, routes, is_playback, false)
}

/// Like `start_routes`, but never paused when the app's own screen isn't drawn (TV sharing).
pub fn start_routes_always(ctx: egui::Context, routes: Vec<Route>, is_playback: bool) -> StreamHandle {
    start_inner(ctx, routes, is_playback, true)
}

fn start_inner(ctx: egui::Context, routes: Vec<Route>, is_playback: bool, always: bool) -> StreamHandle {
    let shared = Arc::new(Shared::default());
    shared.speed_milli.store(1000, Ordering::Relaxed);
    shared.always.store(always, Ordering::Relaxed);
    shared.set_state(State::Connecting);
    let key = routes.iter().map(|r| r.url.as_str()).collect::<Vec<_>>().join(" | ");
    let s2 = shared.clone();
    std::thread::Builder::new()
        .name("video".into())
        .spawn(move || worker(ctx, s2, routes, is_playback))
        .ok();
    StreamHandle { shared, url: key, is_playback }
}

fn worker(ctx: egui::Context, shared: Arc<Shared>, routes: Vec<Route>, is_playback: bool) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            shared.set_state(State::Error(format!("cannot start: {e}")));
            return;
        }
    };
    if routes.is_empty() {
        shared.set_state(State::Error("No camera is connected to this channel".into()));
        return;
    }
    let mut attempt = 0u32;
    let mut current = 0usize;
    'outer: while !shared.stop.load(Ordering::Relaxed) {
        // Try the route that worked last, then the others, before waiting.
        let mut last_error = String::new();
        for k in 0..routes.len() {
            if shared.stop.load(Ordering::Relaxed) {
                break 'outer;
            }
            if ui_asleep(&shared) {
                shared.set_state(State::Paused);
                break 'outer;
            }
            let i = (current + k) % routes.len();
            let r = &routes[i];
            shared.set_state(State::Connecting);
            *shared.route.lock().unwrap() = r.label.clone();
            ctx.request_repaint();
            let result = if r.url.starts_with("dav://") { run_dav(&ctx, &shared, &r.url, &r.user, &r.pass) } else { rt.block_on(run(&ctx, &shared, &r.url, &r.user, &r.pass)) };
            match result {
                Ok(()) => {
                    current = i;
                    if !shared.stop.load(Ordering::Relaxed) {
                        shared.set_state(State::Ended);
                    }
                    attempt = 0;
                    if is_playback {
                        break 'outer;
                    }
                    last_error.clear();
                    break;
                }
                Err(Failure::Unsupported(m)) => {
                    shared.set_state(State::Unsupported(m));
                    break 'outer;
                }
                Err(Failure::Paused) => {
                    // Nobody is looking: disconnect; the screen starts it again when it is shown.
                    shared.set_state(State::Paused);
                    ctx.request_repaint();
                    break 'outer;
                }
                Err(Failure::Other(m)) => {
                    if shared.frames.load(Ordering::Relaxed) > 0 {
                        // It was playing and then stopped: keep this route first.
                        current = i;
                    }
                    last_error = if r.label.is_empty() || routes.len() == 1 { m } else { format!("{m} ({})", r.label) };
                }
            }
        }
        if !last_error.is_empty() {
            shared.set_state(State::Error(last_error));
        }
        ctx.request_repaint();
        if is_playback {
            break;
        }
        // Live video: try again, waiting a little longer each time (2 s .. 10 s).
        attempt += 1;
        let wait = Duration::from_secs((2 * attempt as u64).min(10));
        let until = Instant::now() + wait;
        while Instant::now() < until && !shared.stop.load(Ordering::Relaxed) {
            if ui_asleep(&shared) {
                shared.set_state(State::Paused);
                break 'outer;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

enum Failure {
    Unsupported(String),
    Other(String),
    /// The app's screen isn't being drawn (minimized, in the background): stop using the network and CPU.
    Paused,
}

/// When the app's screen last drew a frame (milliseconds since start; 0 = there is no screen,
/// as in the check tools and the TV server).
static UI_FRAME_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Called by the screen every frame it draws.
pub fn ui_alive() {
    UI_FRAME_MS.store(now_ms(), Ordering::Relaxed);
}

/// No frame for this long means the screen is not shown (minimized, in the background, phone
/// screen off): video windows stop until it is shown again.
const UI_ASLEEP_AFTER: Duration = Duration::from_secs(3);

fn ui_asleep(shared: &Shared) -> bool {
    let last = UI_FRAME_MS.load(Ordering::Relaxed);
    last != 0 && !shared.always.load(Ordering::Relaxed) && !shared.recording.load(Ordering::Relaxed) && now_ms().saturating_sub(last) > UI_ASLEEP_AFTER.as_millis() as u64
}

/// Keeps the full technical message (for troubleshooting) and returns a plain one.
fn fail(shared: &Shared, raw: &str) -> Failure {
    *shared.detail.lock().unwrap() = raw.chars().take(600).collect();
    log::warn!("video error: {raw}");
    Failure::Other(friendly(raw))
}

fn friendly(e: &str) -> String {
    let l = e.to_ascii_lowercase();
    if l.contains("401") || l.contains("unauthorized") {
        "User name or password is wrong".into()
    } else if l.contains("404") || l.contains("not found") {
        "Channel or recording not found".into()
    } else if l.contains("forcibly closed") || l.contains("10054") || l.contains("connection reset") || l.contains("broken pipe") {
        "The device refused the video connection. Turn on its RTSP service (device web page: Network > Connection > RTSP) and check that this computer is not blocked".into()
    } else if l.contains("refused") || l.contains("unreachable") || l.contains("timed out") || l.contains("timeout") {
        "Device is not reachable".into()
    } else {
        e.chars().take(140).collect()
    }
}

async fn run(ctx: &egui::Context, shared: &Shared, url: &str, user: &str, pass: &str) -> Result<(), Failure> {
    // A device reached through a chosen network connection goes through a local relay.
    let (url, _relay) = crate::netpath::rtsp_through(url).map_err(Failure::Other)?;
    let parsed = url::Url::parse(&url).map_err(|e| Failure::Other(format!("bad address: {e}")))?;
    let creds = if user.is_empty() { None } else { Some(Credentials { username: user.into(), password: pass.into() }) };
    let opts = SessionOptions::default().creds(creds).user_agent("MyCCTV/0.1".into());
    let mut session = match tokio::time::timeout(Duration::from_secs(10), Session::describe(parsed, opts)).await {
        Err(_) => return Err(Failure::Other("Device is not reachable".into())),
        Ok(Err(e)) => return Err(fail(shared, &e.to_string())),
        Ok(Ok(s)) => s,
    };
    let Some(vi) = session.streams().iter().position(|s| s.media() == "video") else {
        return Err(Failure::Unsupported("No video in this stream".into()));
    };
    let encoding = session.streams()[vi].encoding_name().to_ascii_lowercase();

    let setup = SetupOptions::default().transport(Transport::Tcp(Default::default())).frame_format(FrameFormat::SIMPLE);
    tokio::time::timeout(Duration::from_secs(10), session.setup(vi, setup))
        .await
        .map_err(|_| Failure::Other("Device is not reachable".into()))?
        .map_err(|e| fail(shared, &e.to_string()))?;
    let playing = tokio::time::timeout(Duration::from_secs(10), session.play(PlayOptions::default()))
        .await
        .map_err(|_| Failure::Other("Device did not start the video".into()))?
        .map_err(|e| fail(shared, &e.to_string()))?;
    let mut demux = Box::pin(playing.demuxed().map_err(|e| Failure::Other(e.to_string()))?);

    let mut decoder = match encoding.as_str() {
        "h264" => Some(VideoDecoder::H264(openh264::decoder::Decoder::new().map_err(|e| Failure::Other(format!("decoder: {e}")))?)),
        "h265" | "hevc" => Some(VideoDecoder::H265(Box::new(rusty_h265::Decoder::new()))),
        _ => None,
    };
    if decoder.is_none() {
        shared.set_state(State::Unsupported(format!(
            "This camera sends {} video. Set its stream to H.264 to watch it here (recording still works).",
            encoding.to_uppercase()
        )));
    }
    *shared.info.lock().unwrap() = encoding.to_uppercase();
    let mut last_data = Instant::now();
    let mut rec: Option<std::fs::File> = None;
    let mut rec_waiting_key = false;
    let mut pace: Option<(Instant, f64)> = None;
    let mut fps_count = 0u32;
    let mut fps_since = Instant::now();
    let mut size: (usize, usize);
    let mut clock0: Option<(Instant, f64)> = None; // (wall, media seconds) of the first picture

    loop {
        if shared.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if ui_asleep(shared) {
            return Err(Failure::Paused);
        }
        let next = match tokio::time::timeout(Duration::from_millis(500), demux.next()).await {
            Err(_) => {
                if last_data.elapsed() > Duration::from_secs(15) {
                    return Err(Failure::Other("Video stopped arriving".into()));
                }
                continue;
            }
            Ok(None) => return Ok(()),
            Ok(Some(Err(e))) => return Err(fail(shared, &e.to_string())),
            Ok(Some(Ok(item))) => item,
        };
        last_data = Instant::now();
        let CodecItem::VideoFrame(frame) = next else { continue };

        // Recording: raw elementary stream, starting at a key frame so it plays back.
        let want_rec = shared.recording.load(Ordering::Relaxed);
        if want_rec && rec.is_none() && !rec_waiting_key {
            let path = shared.record_to.lock().unwrap().clone();
            if let Some(p) = path {
                let p = p.with_extension(if encoding == "hevc" { "h265" } else { encoding.as_str() });
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::File::create(&p) {
                    Ok(f) => {
                        rec = Some(f);
                        rec_waiting_key = true;
                        shared.notice(format!("Recording to {}", p.display()));
                    }
                    Err(e) => {
                        shared.recording.store(false, Ordering::Relaxed);
                        shared.notice(format!("Local record failed: {e}"));
                    }
                }
            }
        }
        if !want_rec && rec.is_some() {
            rec = None;
            rec_waiting_key = false;
            shared.notice("Recording saved".into());
        }
        if let Some(f) = rec.as_mut() {
            if rec_waiting_key && frame.is_random_access_point() {
                rec_waiting_key = false;
            }
            if !rec_waiting_key && f.write_all(frame.data()).is_err() {
                rec = None;
                shared.recording.store(false, Ordering::Relaxed);
                shared.notice("Local record failed: disk write error".into());
            }
        }

        // Slow-motion pacing for playback (speeds below 1X).
        let speed = shared.speed_milli.load(Ordering::Relaxed) as f64 / 1000.0;
        let ts = frame.timestamp().elapsed_secs();
        if speed < 0.999 {
            let (t0, ts0) = *pace.get_or_insert((Instant::now(), ts));
            let target = t0 + Duration::from_secs_f64(((ts - ts0) / speed).max(0.0));
            let now = Instant::now();
            if target > now {
                let wait = (target - now).min(Duration::from_secs(4));
                tokio::time::sleep(wait).await;
            }
        } else {
            pace = None;
        }

        let Some(dec) = decoder.as_mut() else { continue };
        // A damaged frame gives nothing; the next key frame recovers.
        match dec.decode(frame.data()) {
            Some(Picture { w, h, rgba }) => {
                let snap = shared.snapshot_to.lock().unwrap().take();
                if let Some(path) = snap {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let res = image::save_buffer(&path, &rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8);
                    shared.notice(match res {
                        Ok(()) => format!("Picture saved: {}", path.display()),
                        Err(e) => format!("Capture failed: {e}"),
                    });
                }
                size = (w, h);
                let (t0, ts0) = *clock0.get_or_insert((Instant::now(), ts));
                let lag = t0.elapsed().as_secs_f64() - (ts - ts0);
                shared.lag_ms.store((lag.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
                *shared.picture.lock().unwrap() = Some(Picture { w, h, rgba });
                shared.frames.fetch_add(1, Ordering::Relaxed);
                if shared.state() == State::Connecting {
                    shared.set_state(State::Playing);
                }
                fps_count += 1;
                if fps_since.elapsed() >= Duration::from_secs(2) {
                    let fps = fps_count as f32 / fps_since.elapsed().as_secs_f32();
                    *shared.info.lock().unwrap() = format!("{} {}x{} {:.0} fps", if encoding == "h264" { "H.264" } else { "H.265" }, size.0, size.1, fps);
                    fps_count = 0;
                    fps_since = Instant::now();
                }
                ctx.request_repaint();
            }
            None => {}
        }
    }
}

/// The two kinds of camera video we can show.
pub enum VideoDecoder {
    H264(openh264::decoder::Decoder),
    H265(Box<rusty_h265::Decoder>),
}

impl VideoDecoder {
    /// Feeds one frame of Annex B video; returns the newest finished picture, if any.
    pub fn decode(&mut self, data: &[u8]) -> Option<Picture> {
        match self {
            VideoDecoder::H264(d) => decode_rgba(d, data),
            VideoDecoder::H265(d) => {
                if let Err(e) = d.push_annexb(data, None) {
                    log::debug!("h265: {e:?}");
                }
                let mut last = None;
                while let Ok(f) = d.next_frame() {
                    last = Some(f);
                }
                last.and_then(|f| h265_rgba(&f.picture))
            }
        }
    }
}

/// Converts a decoded H.265 picture (4:2:0, 8 or 10 bit) to RGBA, cropped to its display size.
pub fn h265_rgba(p: &rusty_h265::Picture) -> Option<Picture> {
    if p.chroma_format_idc != 1 {
        return None;
    }
    let (cx, cy, w, h) = p.crop;
    if w == 0 || h == 0 {
        return None;
    }
    let shift_y = p.bit_depth_luma.saturating_sub(8) as u32;
    let shift_c = p.bit_depth_chroma.saturating_sub(8) as u32;
    let (yp, up, vp) = (&p.planes[0], &p.planes[1], &p.planes[2]);
    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        let y_row = yp.row(cy + row);
        let c_y = (cy + row) / 2;
        let u_row = up.row(c_y.min(up.height.saturating_sub(1)));
        let v_row = vp.row(c_y.min(vp.height.saturating_sub(1)));
        let out = &mut rgba[row * w * 4..(row + 1) * w * 4];
        for col in 0..w {
            let x = cx + col;
            let yy = (y_row[x] >> shift_y) as i32 - 16;
            let u = (u_row[(x / 2).min(u_row.len() - 1)] >> shift_c) as i32 - 128;
            let v = (v_row[(x / 2).min(v_row.len() - 1)] >> shift_c) as i32 - 128;
            // BT.601 limited range, fixed point (same look as the H.264 path)
            let c = 298 * yy;
            let r = (c + 409 * v + 128) >> 8;
            let g = (c - 100 * u - 208 * v + 128) >> 8;
            let b = (c + 516 * u + 128) >> 8;
            let o = col * 4;
            out[o] = r.clamp(0, 255) as u8;
            out[o + 1] = g.clamp(0, 255) as u8;
            out[o + 2] = b.clamp(0, 255) as u8;
            out[o + 3] = 255;
        }
    }
    Some(Picture { w, h, rgba })
}

/// Decodes one piece of H.264 (Annex B) into an RGBA picture, if it completes one.
pub fn decode_rgba(dec: &mut openh264::decoder::Decoder, data: &[u8]) -> Option<Picture> {
    use openh264::formats::YUVSource;
    let yuv = dec.decode(data).ok()??;
    let (w, h) = yuv.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let mut rgba = vec![0u8; w * h * 4];
    yuv.write_rgba8(&mut rgba);
    Some(Picture { w, h, rgba })
}

#[cfg(test)]
mod tests_h265 {
    /// Decodes an H.265 clip made by ffmpeg (shots/test.h265, 640x360, 50 frames), like the
    /// CP Plus cameras send, one frame at a time.
    #[test]
    fn decodes_h265_clip() {
        let Ok(bytes) = std::fs::read("shots/test.h265") else {
            eprintln!("no test clip; skipped");
            return;
        };
        let mut dec = super::VideoDecoder::H265(Box::new(rusty_h265::Decoder::new()));
        let mut frames = 0;
        let mut last = None;
        // Feed one access unit at a time (split before each first slice / parameter set).
        let mut au: Vec<u8> = Vec::new();
        for nal in openh264::nal_units(&bytes) {
            let body = nal.iter().skip_while(|b| **b == 0).skip(1).copied().collect::<Vec<u8>>();
            let kind = body.first().map(|b| (b >> 1) & 0x3f).unwrap_or(0);
            let first_slice = kind < 32 && body.get(2).map(|b| b & 0x80 != 0).unwrap_or(false);
            if (kind == 32 || first_slice) && !au.is_empty() && !(kind == 32 && au.is_empty()) {
                let starts_new = kind == 32 || !au_has_vps_only(&au);
                if starts_new {
                    if let Some(p) = dec.decode(&au) {
                        frames += 1;
                        last = Some(p);
                    }
                    au.clear();
                }
            }
            au.extend_from_slice(nal);
        }
        if let Some(p) = dec.decode(&au) {
            frames += 1;
            last = Some(p);
        }
        let p = last.expect("no picture decoded");
        assert_eq!((p.w, p.h), (640, 360));
        assert!(frames >= 45, "only {frames} frames");
        let bright = p.rgba.chunks(4).filter(|px| px[0] as u32 + px[1] as u32 + px[2] as u32 > 150).count();
        assert!(bright > p.w * p.h / 4, "picture looks empty");
        image::save_buffer("shots/decoded265.png", &p.rgba, p.w as u32, p.h as u32, image::ExtendedColorType::Rgba8).unwrap();
    }

    /// True when the gathered unit holds only parameter sets so far (no picture yet).
    fn au_has_vps_only(au: &[u8]) -> bool {
        openh264::nal_units(au).all(|n| {
            let body: Vec<u8> = n.iter().skip_while(|b| **b == 0).skip(1).copied().collect();
            body.first().map(|b| (b >> 1) & 0x3f >= 32).unwrap_or(true)
        })
    }
}

#[cfg(test)]
mod pause_tests {
    use super::*;

    /// Without a screen (check tools, TV server) nothing is ever paused.
    #[test]
    fn no_screen_never_pauses() {
        let s = Shared::default();
        if UI_FRAME_MS.load(Ordering::Relaxed) == 0 {
            assert!(!ui_asleep(&s));
        }
    }

    /// Real camera (saved device MYCCTV_PAUSE_DEVICE, 1-based; default 2): plays while the screen
    /// draws, disconnects by itself once it stops drawing.
    #[test]
    #[ignore]
    fn pauses_when_screen_stops() {
        let n: usize = std::env::var("MYCCTV_PAUSE_DEVICE").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let store = crate::model::Store::load();
        let d = store.devices.get(n - 1).expect("device").clone();
        let h = start_routes(egui::Context::default(), d.live_routes(0, 1, &store.devices), false);
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(6) {
            ui_alive();
            std::thread::sleep(Duration::from_millis(100));
        }
        let playing = h.shared.frames.load(Ordering::Relaxed);
        println!("while shown: {:?}, {playing} pictures", h.shared.state());
        assert!(playing > 0, "no video: {:?}", h.shared.state());
        std::thread::sleep(Duration::from_secs(5)); // no more frames drawn
        let state = h.shared.state();
        let a = h.shared.frames.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(2));
        let b = h.shared.frames.load(Ordering::Relaxed);
        println!("after the screen stopped: {state:?}, pictures {a} -> {b}");
        assert_eq!(state, State::Paused);
        assert_eq!(a, b, "still decoding while paused");
    }
}

#[cfg(test)]
mod soak_tests {
    use super::*;

    /// Real camera, many minutes: plays a saved device's channel without a window and prints, every
    /// 10 s, pictures per second and how far the picture is behind the camera. Settings:
    /// MYCCTV_SOAK_DEVICE (1-based, default 1), MYCCTV_SOAK_CHANNEL (1-based, default 1),
    /// MYCCTV_SOAK_STREAM (0 main, 1 sub; default 0), MYCCTV_SOAK_SECS (default 120).
    /// `cargo test --release soak_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn soak_live() {
        let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        let (dev, ch, st, secs) = (env("MYCCTV_SOAK_DEVICE", 1) as usize, env("MYCCTV_SOAK_CHANNEL", 1) as u32, env("MYCCTV_SOAK_STREAM", 0) as u8, env("MYCCTV_SOAK_SECS", 120));
        let store = crate::model::Store::load();
        let d = store.devices.get(dev - 1).expect("device").clone();
        let h = start_routes(egui::Context::default(), d.live_routes(ch - 1, st, &store.devices), false);
        let t = Instant::now();
        let mut last = 0;
        while t.elapsed() < Duration::from_secs(secs) {
            std::thread::sleep(Duration::from_secs(10));
            let f = h.shared.frames.load(Ordering::Relaxed);
            println!(
                "{:>4}s {:?} {:>5.1} fps  behind {:>6} ms  {}",
                t.elapsed().as_secs(),
                h.shared.state(),
                (f - last) as f32 / 10.0,
                h.shared.lag_ms.load(Ordering::Relaxed),
                h.shared.info.lock().unwrap()
            );
            last = f;
        }
        assert!(last > 0, "no video: {:?}", h.shared.state());
    }
}

#[cfg(test)]
mod tests {
    /// Decodes a clip made by ffmpeg (shots/test.h264, 640x360, 50 frames) the same way live video is.
    #[test]
    fn decodes_h264_clip() {
        let Ok(bytes) = std::fs::read("shots/test.h264") else {
            eprintln!("no test clip; skipped");
            return;
        };
        let mut dec = openh264::decoder::Decoder::new().unwrap();
        let mut frames = 0;
        let mut last = None;
        for nal in openh264::nal_units(&bytes) {
            if let Some(p) = super::decode_rgba(&mut dec, nal) {
                frames += 1;
                last = Some(p);
            }
        }
        let p = last.expect("no picture decoded");
        assert_eq!((p.w, p.h), (640, 360));
        assert!(frames >= 45, "only {frames} frames");
        let bright = p.rgba.chunks(4).filter(|px| px[0] as u32 + px[1] as u32 + px[2] as u32 > 150).count();
        assert!(bright > p.w * p.h / 4, "picture looks empty");
        image::save_buffer("shots/decoded.png", &p.rgba, p.w as u32, p.h as u32, image::ExtendedColorType::Rgba8).unwrap();
    }
}

/// Plays recordings straight from the recorder's files over its web interface, for recorders
/// that refuse (or don't offer) RTSP playback. Address form:
/// `dav://host:port/?ch=<0-based>&day=YYYY-MM-DD&from=<seconds since midnight>&end=<seconds>`.
fn run_dav(ctx: &egui::Context, shared: &Shared, url: &str, user: &str, pass: &str) -> Result<(), Failure> {
    use crate::dav::{Codec, DavReader};
    let parsed = url::Url::parse(url).map_err(|e| Failure::Other(format!("bad address: {e}")))?;
    let q: std::collections::HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let ch: u32 = q.get("ch").and_then(|v| v.parse().ok()).unwrap_or(0);
    let day = q.get("day").cloned().unwrap_or_default();
    let from: u32 = q.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    let until: u32 = q.get("end").and_then(|v| v.parse().ok()).unwrap_or(86_399);
    let want_day: Option<(u8, u8)> = chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok().map(|d| {
        use chrono::Datelike;
        (d.day() as u8, d.month() as u8)
    });
    let t = crate::http::Target { host: parsed.host_str().unwrap_or_default().to_string(), port: parsed.port().unwrap_or(80), user: user.into(), pass: pass.into() };
    // Fast way: ask the recorder for the chosen time directly (loadfile.cgi), an hour at a time,
    // so playback starts within a second. Recorders without it: search the recording files.
    let stamp = |s: u32| crate::http::url_escape(&format!("{day} {}", crate::clock::hms(s.min(86_399))));
    let mut by_time: Vec<String> = Vec::new();
    let mut s = from;
    while s <= until {
        let e = (s + 3599).min(until);
        by_time.push(format!("/cgi-bin/loadfile.cgi?action=startLoad&channel={}&startTime={}&endTime={}", ch + 1, stamp(s), stamp(e)));
        s = e + 1;
    }
    let sources: Vec<String>;
    let mut first: Option<Box<dyn std::io::Read + Send>> = None;
    if let Ok(Some(r)) = open_recording(&t, &by_time[0]) {
        first = Some(r);
        sources = by_time;
        *shared.route.lock().unwrap() = "recorder".into();
    } else {
        let files = crate::services::find_records_at(&t, ch, &day, crate::services::RecordFilter::All).map_err(|e| fail(shared, &e))?;
        let mut files: Vec<_> = files.into_iter().filter(|f| f.end > from && f.start <= until && !f.path.is_empty()).collect();
        files.sort_by_key(|f| f.start);
        files.dedup_by(|a, b| a.path == b.path);
        if files.is_empty() {
            return Err(Failure::Other("No recordings from this time on".into()));
        }
        sources = files.into_iter().map(|f| format!("/cgi-bin/RPC_Loadfile{}", f.path)).collect();
        *shared.route.lock().unwrap() = "recorder files".into();
    }
    let mut decoder: Option<VideoDecoder> = None;
    let mut started = false;
    let mut last_secs: u32 = 0;
    let mut clock: Option<(Instant, u64)> = None; // (wall start, media ms at start)
    let mut media_ms: u64 = 0;
    let mut last_ms: Option<u16> = None;
    let mut rec: Option<std::fs::File> = None;
    let mut fps_count = 0u32;
    let mut fps_since = Instant::now();
    for (n, src) in sources.into_iter().enumerate() {
        if std::env::var("MYCCTV_DEBUG").is_ok() {
            eprintln!("source {n}: {src}");
        }
        let reader = match first.take() {
            Some(r) => r,
            None => match open_recording(&t, &src) {
                Ok(Some(r)) => r,
                Ok(None) => continue, // nothing recorded in this hour
                Err(e) => return Err(fail(shared, &e)),
            },
        };
        let mut dav = DavReader::new(reader);
        let mut need_key = n > 0;
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if ui_asleep(shared) {
                return Err(Failure::Paused);
            }
            let frame = match dav.next_frame() {
                Ok(Some(fr)) => fr,
                Ok(None) => break,
                Err(e) => return Err(fail(shared, &format!("recording download broke: {e}"))),
            };
            // Recording files start with left-over data from older, overwritten recordings: skip
            // everything that isn't from the chosen day.
            if want_day.map(|(d, m)| frame.day != d || frame.month != m).unwrap_or(false) {
                continue;
            }
            if frame.secs_of_day > until {
                return Ok(());
            }
            // Start at the first key frame at or after the chosen time.
            if !started {
                if frame.secs_of_day < from || !frame.is_key() {
                    continue;
                }
                started = true;
            } else if frame.secs_of_day < last_secs || (need_key && !frame.is_key()) {
                // The next hour starts a little early: skip what was already shown, resume at a key frame.
                continue;
            }
            need_key = false;
            last_secs = last_secs.max(frame.secs_of_day);
            // Media clock from the block's millisecond counter (wraps at 65536).
            if let Some(prev) = last_ms {
                let d = frame.ms.wrapping_sub(prev) as u64;
                media_ms += if d > 5_000 { 40 } else { d };
            }
            last_ms = Some(frame.ms);
            let speed = (shared.speed_milli.load(Ordering::Relaxed) as f64 / 1000.0).max(0.05);
            // Fast forward above 2X shows key frames only, so decoding keeps up.
            if speed > 2.0 && !frame.is_key() {
                continue;
            }
            let (t0, m0) = *clock.get_or_insert((Instant::now(), media_ms));
            let target = t0 + Duration::from_secs_f64((media_ms - m0) as f64 / 1000.0 / speed);
            let now = Instant::now();
            if target > now {
                std::thread::sleep((target - now).min(Duration::from_secs(4)));
            } else if now - target > Duration::from_secs(3) {
                clock = Some((Instant::now(), media_ms)); // fell behind (slow network): catch up
            }
            shared.position.store(frame.secs_of_day as u64 + 1, Ordering::Relaxed);

            // Time clip (recording) of what is being played.
            if shared.recording.load(Ordering::Relaxed) {
                if rec.is_none() && frame.is_key() {
                    if let Some(p) = shared.record_to.lock().unwrap().clone() {
                        let p = p.with_extension(if frame.codec == Some(Codec::H264) { "h264" } else { "h265" });
                        if let Some(dir) = p.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        rec = std::fs::File::create(&p).ok();
                        shared.notice(format!("Saving clip to {}", p.display()));
                    }
                }
                if let Some(file) = rec.as_mut() {
                    let _ = file.write_all(&frame.data);
                }
            } else if rec.take().is_some() {
                shared.notice("Clip saved".into());
            }

            let dec = decoder.get_or_insert_with(|| match frame.codec {
                Some(Codec::H264) => VideoDecoder::H264(openh264::decoder::Decoder::new().expect("decoder")),
                _ => VideoDecoder::H265(Box::new(rusty_h265::Decoder::new())),
            });
            if let Some(Picture { w, h, rgba }) = dec.decode(&frame.data) {
                if let Some(path) = shared.snapshot_to.lock().unwrap().take() {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let res = image::save_buffer(&path, &rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8);
                    shared.notice(match res {
                        Ok(()) => format!("Picture saved: {}", path.display()),
                        Err(e) => format!("Capture failed: {e}"),
                    });
                }
                *shared.picture.lock().unwrap() = Some(Picture { w, h, rgba });
                shared.frames.fetch_add(1, Ordering::Relaxed);
                if shared.state() != State::Playing {
                    shared.set_state(State::Playing);
                }
                fps_count += 1;
                if fps_since.elapsed() >= Duration::from_secs(2) {
                    let codec = if frame.codec == Some(Codec::H264) { "H.264" } else { "H.265" };
                    *shared.info.lock().unwrap() = format!("{codec} {w}x{h} {:.0} fps · playback from recorder", fps_count as f32 / fps_since.elapsed().as_secs_f32());
                    fps_count = 0;
                    fps_since = Instant::now();
                }
                ctx.request_repaint();
            }
        }
    }
    Ok(())
}

/// Opens a recording download; `Ok(None)` when the recorder answers with an error text
/// instead of video (no recording there, or the request isn't supported).
fn open_recording(t: &crate::http::Target, path: &str) -> Result<Option<Box<dyn std::io::Read + Send>>, String> {
    use std::io::Read;
    let mut r = match crate::http::open_stream(t, path, Duration::from_secs(15)) {
        Ok(r) => r,
        Err(e) if e.contains("answered 4") || e.contains("answered 5") => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut head = vec![0u8; 64];
    let mut got = 0;
    while got < head.len() {
        match r.read(&mut head[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) => return Err(e.to_string()),
        }
    }
    head.truncate(got);
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    if got == 0 || text.starts_with("error") || text.contains("bad request") {
        return Ok(None);
    }
    Ok(Some(Box::new(std::io::Cursor::new(head).chain(r))))
}
