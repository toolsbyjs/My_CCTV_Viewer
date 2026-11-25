//! MyCCTV: a viewer for camera recorders (NVR/DVR) and IP cameras.
//! One code base for Windows (desktop window) and Android / Google TV.

pub mod app;
mod clock;
mod dav;
mod devconfig;
mod http;
mod icons;
mod kit;
mod layout;
mod model;
mod netpath;
pub mod platform;
mod relay;
mod services;
mod stream;
mod theme;

use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// `--screenshot <file.png>`: draw about 120 frames, save the window and exit.
    pub screenshot: Option<PathBuf>,
    /// `--page <home|live|playback|devices|events|settings>`: page to open first.
    pub page: Option<String>,
    /// `--size <W>x<H>`: start with this window size (to check the phone layout on a computer).
    pub size: Option<(f32, f32)>,
}

/// `--probe <rtsp-url> [user] [password] [picture.png]`: connect to one camera without a
/// window, report what arrives for up to 15 seconds and save one picture. For troubleshooting.
pub fn probe(url: &str, user: &str, pass: &str, picture: Option<PathBuf>) -> Result<String, String> {
    probe_routes(vec![stream::Route { url: url.into(), user: user.into(), pass: pass.into(), label: String::new() }], picture)
}

fn probe_routes(routes: Vec<stream::Route>, picture: Option<PathBuf>) -> Result<String, String> {
    use std::time::{Duration, Instant};
    let ctx = egui::Context::default();
    let h = stream::start_routes(ctx, routes, false);
    let t0 = Instant::now();
    let mut asked = false;
    while t0.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(200));
        match h.shared.state() {
            stream::State::Error(e) => return Err(format!("{e}  [detail: {}]", h.shared.detail.lock().unwrap())),
            stream::State::Unsupported(e) => return Err(e),
            _ => {}
        }
        let frames = h.shared.frames.load(std::sync::atomic::Ordering::Relaxed);
        if frames > 0 && !asked {
            if let Some(p) = &picture {
                h.snapshot(p.clone());
            }
            asked = true;
        }
        if frames >= 50 {
            std::thread::sleep(Duration::from_millis(2100));
            let notice = h.shared.notice.lock().unwrap().clone().unwrap_or_default();
            return Ok(format!("{} pictures decoded in {:.1} s; {}; {}", h.shared.frames.load(std::sync::atomic::Ordering::Relaxed), t0.elapsed().as_secs_f32(), h.shared.info.lock().unwrap(), notice) + &format!("; via {}", { let v = h.shared.route.lock().unwrap().clone(); if v.is_empty() { "the device itself".to_string() } else { v } }));
        }
    }
    Err(format!("only {} pictures in 15 s", h.shared.frames.load(std::sync::atomic::Ordering::Relaxed)))
}

/// `--probe-device <number in list> [camera] [stream]`: probe a saved device's camera.
pub fn probe_saved(index: usize, channel: u32, stream_type: u8, picture: Option<PathBuf>) -> Result<String, String> {
    let store = model::Store::load();
    let mut d = store.devices.get(index.saturating_sub(1)).ok_or("no such device in the list")?.clone();
    match services::device_info(&d.http()) {
        Ok(info) => {
            println!("login ok: {} cameras, model {}, {} linked camera(s)", info.channels.unwrap_or(0), info.device_type, info.camera_links.iter().flatten().count());
            if !info.camera_links.is_empty() {
                d.camera_links = info.camera_links;
            }
        }
        Err(e) => println!("login failed: {e}"),
    }
    netpath::sync(std::slice::from_ref(&d));
    let url = d.live_url(channel.saturating_sub(1), stream_type);
    println!("probing {} -> {} (network connection: {})", d.name, url, if d.network.is_empty() { "automatic" } else { &d.network });
    let _ = url;
    let r = probe_routes(d.live_routes(channel.saturating_sub(1), stream_type, &store.devices), picture);
    r
}

#[cfg(not(target_os = "android"))]
pub fn run_desktop(opts: Options) -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MyCCTV")
        .with_inner_size(opts.size.map(|(w, h)| [w, h]).unwrap_or([1366.0, 768.0]))
        .with_min_inner_size([320.0, 480.0])
        .with_icon(egui::IconData { rgba: icons::logo_rgba(64), width: 64, height: 64 });
    if opts.screenshot.is_some() {
        // A check must never steal focus from what the person is doing.
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    #[allow(unused_mut)]
    let mut native = eframe::NativeOptions { viewport, ..Default::default() };
    // macOS: a check run must not bring the app to the front either (it isn't lowered behind other
    // windows there, because macOS may stop drawing covered windows).
    #[cfg(target_os = "macos")]
    if opts.screenshot.is_some() {
        native.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_activate_ignoring_other_apps(false);
        }));
    }
    eframe::run_native("MyCCTV", native, Box::new(move |cc| Ok(Box::new(app::KvmsApp::new(cc, opts)))))
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    if let Some(p) = android.internal_data_path() {
        model::set_data_dir(p);
    }
    platform::set_android_app(android.clone());
    let native = eframe::NativeOptions {
        event_loop_builder: Some(Box::new(move |builder| {
            use winit::platform::android::EventLoopBuilderExtAndroid;
            builder.with_android_app(android);
        })),
        ..Default::default()
    };
    let _ = eframe::run_native("MyCCTV", native, Box::new(|cc| Ok(Box::new(app::KvmsApp::new(cc, Options::default())))));
}


/// `--config-check <device number> <setting>`: reads one setting and writes the same value
/// back (so nothing changes), to check that saving settings works with this device.
pub fn config_check(index: usize, key: &str) -> Result<String, String> {
    let store = model::Store::load();
    let d = store.devices.get(index.saturating_sub(1)).ok_or("no such device in the list")?;
    let t = d.http();
    let snap = devconfig::load(&t, d.channels)?;
    let v = snap.values.get(key).cloned().ok_or_else(|| format!("{key} not found ({} settings read)", snap.values.len()))?;
    let n = devconfig::save(&t, &[(key.to_string(), v.clone())])?;
    Ok(format!("{} settings read; wrote {key}={v} back ({n} accepted)", snap.values.len()))
}

/// `--playback-check <device number> <camera> <YYYY-MM-DD> <hh:mm:ss>`: plays recordings
/// without a window for a few seconds and saves one picture (troubleshooting).
pub fn playback_check(index: usize, camera: u32, day: &str, time: &str, picture: Option<PathBuf>) -> Result<String, String> {
    let store = model::Store::load();
    let d = store.devices.get(index.saturating_sub(1)).ok_or("no such device in the list")?;
    let day = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let t: Vec<u32> = time.split(':').filter_map(|x| x.parse().ok()).collect();
    let from = t.first().copied().unwrap_or(0) * 3600 + t.get(1).copied().unwrap_or(0) * 60 + t.get(2).copied().unwrap_or(0);
    probe_routes_playback(d.playback_routes(camera.saturating_sub(1), day, from, 86_399), picture)
}

fn probe_routes_playback(routes: Vec<stream::Route>, picture: Option<PathBuf>) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let h = stream::start_routes(egui::Context::default(), routes, true);
    let t0 = Instant::now();
    let mut asked = false;
    while t0.elapsed() < Duration::from_secs(40) {
        std::thread::sleep(Duration::from_millis(200));
        if let stream::State::Error(e) = h.shared.state() {
            return Err(format!("{e} [detail: {}]", h.shared.detail.lock().unwrap()));
        }
        let frames = h.shared.frames.load(Ordering::Relaxed);
        if frames > 0 && !asked {
            if let Some(p) = &picture {
                h.snapshot(p.clone());
            }
            asked = true;
        }
        if frames >= 75 {
            let pos = h.shared.position.load(Ordering::Relaxed).saturating_sub(1);
            return Ok(format!("{frames} pictures in {:.1} s via {}; now at {:02}:{:02}:{:02}; {}", t0.elapsed().as_secs_f32(), h.shared.route.lock().unwrap(), pos / 3600, pos / 60 % 60, pos % 60, h.shared.info.lock().unwrap()));
        }
    }
    Err(format!("only {} pictures in 40 s", h.shared.frames.load(Ordering::Relaxed)))
}

/// `--tv-server`: share the cameras with TVs (Hisense VIDAA and other browsers) without opening
/// the window, for a computer or box that stays on. Uses the port and access code from Settings.
pub fn tv_server_forever() -> Result<(), String> {
    let mut store = model::Store::load();
    if store.settings.tv_key.is_empty() {
        store.settings.tv_key = model::new_tv_code();
        store.save()?;
    }
    let port = store.settings.tv_port;
    let server = relay::start(port, store.settings.tv_key.clone(), &store.devices)?;
    let ip = relay::local_ip().unwrap_or_else(|| "this-computer".into());
    println!("MyCCTV TV sharing is on. On the TV open: http://{ip}:{port}/?key={}", store.settings.tv_key);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        server.set_devices(&model::Store::load().devices);
    }
}
