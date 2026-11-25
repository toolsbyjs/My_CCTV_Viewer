//! The window: Home, Live View, Playback, Devices, Events and Settings.

use crate::icons::{self, Icon};
use crate::layout::{self, MAX_PLAYBACK_WINDOWS, MAX_WINDOWS};
use crate::model::{DeviceForm, FormField, Settings, Store, ThemeChoice};
use crate::services::{self, AlarmEvent, DeviceInfo, FoundDevice, Ptz, RecordFilter, RecordKind, RecordPiece};
use crate::stream::{self, State, StreamHandle};
use crate::theme::{self, Palette};
use crate::kit;
use crate::{clock, platform, Options};
use chrono::NaiveDate;
use egui::{pos2, vec2, Align, Align2, Color32, FontId, Id, Key, Layout, Rect, RichText, Rounding, Sense, Stroke, TextureHandle, TextureOptions, Ui};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[path = "config_ui.rs"]
mod config_ui;
#[path = "net_ui.rs"]
mod net_ui;

const PTZ_PULSE: Duration = Duration::from_millis(600);
const BURST_PICTURES: u32 = 3;
const BURST_GAP: Duration = Duration::from_secs(1);
const TOAST_TIME: Duration = Duration::from_secs(4);
const MAX_EVENTS: usize = 2000;
/// Largest digital zoom of a video window.
const MAX_ZOOM: f32 = 8.0;
/// Narrowest timeline view (seconds) when zoomed in on Playback.
const TIMELINE_MIN_SPAN: f32 = 120.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Home,
    Live,
    Playback,
    Devices,
    Events,
    Settings,
}

impl Page {
    const ALL: [Page; 6] = [Page::Home, Page::Live, Page::Playback, Page::Devices, Page::Events, Page::Settings];
    fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Live => "Live View",
            Page::Playback => "Playback",
            Page::Devices => "Devices",
            Page::Events => "Events",
            Page::Settings => "Settings",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Page::Home => Icon::Home,
            Page::Live => Icon::Camera,
            Page::Playback => Icon::Play,
            Page::Devices => Icon::Server,
            Page::Events => Icon::Bell,
            Page::Settings => Icon::Gear,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Source {
    device: u64,
    channel: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scale {
    Original,
    Full,
    Wide,
    Tall,
}

impl Scale {
    const ALL: [Scale; 4] = [Scale::Original, Scale::Full, Scale::Wide, Scale::Tall];
    fn label(self) -> &'static str {
        match self {
            Scale::Original => "Original Scale",
            Scale::Full => "Full",
            Scale::Wide => "16:9",
            Scale::Tall => "9:16",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
enum WinMode {
    Live,
    Replay { start: String, end: String },
    Playback { day: NaiveDate, from: u32 },
}

struct PlayWindow {
    source: Option<Source>,
    mode: WinMode,
    stream_override: Option<u8>,
    scale: Scale,
    handle: Option<StreamHandle>,
    tex: Option<TextureHandle>,
    /// The picture last given to the screen; its memory goes back to the video thread for reuse.
    shown: Option<Arc<egui::ColorImage>>,
    shown_frames: u64,
    started: Option<Instant>,
    burst: Option<(u32, Instant)>,
    /// Digital zoom: 1 = whole picture, up to MAX_ZOOM; `center` is the middle of the view (0..1).
    zoom: f32,
    center: [f32; 2],
}

impl PlayWindow {
    fn new() -> Self {
        Self { source: None, mode: WinMode::Live, stream_override: None, scale: Scale::Full, handle: None, tex: None, shown: None, shown_frames: 0, started: None, burst: None, zoom: 1.0, center: [0.5, 0.5] }
    }
    fn clear(&mut self) {
        *self = Self { scale: self.scale, ..Self::new() };
    }
    /// The part of the picture shown (texture coordinates 0..1).
    fn uv_rect(&self) -> Rect {
        let h = 0.5 / self.zoom;
        Rect::from_min_max(pos2(self.center[0] - h, self.center[1] - h), pos2(self.center[0] + h, self.center[1] + h))
    }
    /// Zooms by `factor`, keeping the picture point `at` under the same spot on screen.
    fn zoom_at(&mut self, factor: f32, at: [f32; 2]) {
        let z2 = (self.zoom * factor).clamp(1.0, MAX_ZOOM);
        let k = self.zoom / z2;
        self.center = [at[0] + (self.center[0] - at[0]) * k, at[1] + (self.center[1] - at[1]) * k];
        self.zoom = z2;
        self.keep_inside();
    }
    fn pan(&mut self, d: [f32; 2]) {
        self.center = [self.center[0] + d[0], self.center[1] + d[1]];
        self.keep_inside();
    }
    fn keep_inside(&mut self) {
        let h = 0.5 / self.zoom;
        self.center = [self.center[0].clamp(h, 1.0 - h), self.center[1].clamp(h, 1.0 - h)];
    }
    fn recording(&self) -> bool {
        self.handle.as_ref().map(|h| h.shared.recording.load(Ordering::Relaxed)).unwrap_or(false)
    }
    /// Stops the video (no network or decoding) but keeps the camera and the last picture; playback
    /// remembers where it was, so it carries on from there when shown again.
    fn pause(&mut self) {
        let Some(h) = self.handle.take() else { return };
        let mut pos = h.shared.position.load(Ordering::Relaxed);
        if pos == 0 && h.shared.state() == State::Playing {
            // Device playback over RTSP doesn't tell the time on screen: estimate it, as the timeline does.
            if let (WinMode::Playback { from, .. }, Some(started)) = (&self.mode, self.started) {
                let speed = (h.shared.speed_milli.load(Ordering::Relaxed) as f32 / 1000.0).min(1.0);
                pos = (*from as f32 + started.elapsed().as_secs_f32() * speed).min(86_399.0) as u64 + 1;
            }
        }
        if pos == 0 {
            return;
        }
        let at = (pos - 1) as u32;
        match &mut self.mode {
            WinMode::Live => {}
            WinMode::Playback { from, .. } => *from = at,
            WinMode::Replay { start, end } => {
                if let (Some((day, s)), Some((_, e))) = (clock::parse_stamp(start), clock::parse_stamp(end)) {
                    if at > s && at < e {
                        *start = clock::playback_stamp(day, at);
                    }
                }
            }
        }
    }
    fn paused_by_itself(&self) -> bool {
        self.handle.as_ref().map(|h| h.shared.state() == State::Paused).unwrap_or(false)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum WinAction {
    Select,
    Maximize,
    Close,
    CloseAll,
    Record,
    Capture,
    Burst,
    Replay,
    GoPlayback,
    Scale(Scale),
    Stream(Option<u8>),
    Fullscreen,
    /// Digital zoom by a factor around a picture point (0..1).
    Zoom(f32, [f32; 2]),
    /// Move a zoomed view by this much of the picture.
    Pan([f32; 2]),
    ZoomReset,
}

struct Tour {
    next: usize,
    last: Instant,
}

struct Toast {
    text: String,
    at: Instant,
    error: bool,
}

#[derive(Default)]
struct SearchState {
    open: bool,
    running: bool,
    found: Vec<FoundDevice>,
    rx: Option<Receiver<Option<FoundDevice>>>,
    /// Network connection to search on ("" = the main one).
    via: String,
}

struct Shot {
    path: std::path::PathBuf,
    frame: u32,
    requested: bool,
    started: Instant,
}

pub struct KvmsApp {
    store: Store,
    page: Page,
    pal: Palette,
    applied: (ThemeChoice, f32),

    online: HashMap<u64, bool>,
    online_tx: Sender<(u64, bool)>,
    online_rx: Receiver<(u64, bool)>,
    last_online_check: Option<Instant>,
    info_tx: Sender<(u64, Result<DeviceInfo, String>)>,
    info_rx: Receiver<(u64, Result<DeviceInfo, String>)>,
    logging_in: HashSet<u64>,
    quiet_logins: HashSet<u64>,
    config: Option<config_ui::ConfigView>,
    /// Phone-sized screen: bottom tabs, stacked layouts, panels that open over the video.
    compact: bool,
    /// Phone playback: showing the day / camera choices instead of the video.
    pb_filters_open: bool,
    /// Playback timeline view: first second shown and how many seconds are shown.
    pb_view_start: f32,
    pb_view_span: f32,
    /// Running TV server (Hisense VIDAA and other TVs) and the port it was started on.
    tv: Option<(crate::relay::TvServer, u16)>,

    msg_tx: Sender<String>,
    msg_rx: Receiver<String>,
    toasts: Vec<Toast>,

    // live view
    live: Vec<PlayWindow>,
    split: usize,
    selected: usize,
    maximized: Option<usize>,
    fullscreen: bool,
    tour: Option<Tour>,
    show_ptz: bool,
    ptz_step: u32,
    preset_no: u32,
    ptz_held: Option<(Source, Ptz)>,
    ptz_pulse_stop: Option<(Instant, Source, Ptz)>,
    tree_filter: String,
    show_drawer: bool,

    // playback
    pb: Vec<PlayWindow>,
    pb_split: usize,
    pb_selected: usize,
    pb_day: NaiveDate,
    pb_filter: RecordFilter,
    pb_checked: Vec<Source>,
    pb_results: HashMap<Source, Result<Vec<RecordPiece>, String>>,
    pb_rx: Option<Receiver<(Source, Result<Vec<RecordPiece>, String>)>>,
    pb_pending: usize,
    pb_speed: usize,

    // devices
    form: Option<DeviceForm>,
    selected_devices: HashSet<u64>,
    confirm_delete: Option<Vec<u64>>,
    import_path: String,
    search: SearchState,
    net: net_ui::NetUi,
    more_open: bool,

    // events
    events: Vec<AlarmEvent>,
    unread: usize,
    alarm_tx: Sender<AlarmEvent>,
    alarm_rx: Receiver<AlarmEvent>,
    alarm_stops: HashMap<u64, Arc<AtomicBool>>,
    event_filter: String,
    event_kind: usize,
    alarms_running_for: Option<bool>,

    // app lock
    locked: bool,
    unlock_input: String,
    unlock_error: bool,
    new_password: String,

    shot: Option<Shot>,
    frame_no: u64,
}

pub const SPEEDS: [(&str, f32, bool); 7] = [
    ("1/8X", 0.125, true),
    ("1/4X", 0.25, true),
    ("1/2X", 0.5, true),
    ("1X", 1.0, true),
    ("2X", 2.0, true),
    ("4X", 4.0, true),
    ("8X", 8.0, true),
];

impl KvmsApp {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        let store = Store::load();
        let (online_tx, online_rx) = channel();
        let (info_tx, info_rx) = channel();
        let (msg_tx, msg_rx) = channel();
        let (alarm_tx, alarm_rx) = channel();
        let s = &store.settings;
        theme::apply(&cc.egui_ctx, s.theme, s.ui_scale);
        let split = if layout::COUNTS.contains(&s.last_split) { s.last_split } else { 4 };
        let locked = !s.app_password.is_empty() && opts.screenshot.is_none();
        let opts_page_devcfg: Option<usize> = opts.page.as_deref().and_then(|p| p.strip_prefix("devcfg")).map(|n| n.parse().unwrap_or(1));
        let mut app = Self {
            pal: theme::palette(s.theme),
            applied: (s.theme, s.ui_scale),
            page: match opts.page.as_deref().unwrap_or("").to_ascii_lowercase().as_str() {
                "live" => Page::Live,
                "playback" => Page::Playback,
                "devices" | "add" => Page::Devices,
                "events" => Page::Events,
                "settings" => Page::Settings,
                _ => Page::Home,
            },
            online: HashMap::new(),
            online_tx,
            online_rx,
            last_online_check: None,
            info_tx,
            info_rx,
            logging_in: HashSet::new(),
            quiet_logins: HashSet::new(),
            config: None,
            compact: false,
            pb_filters_open: true,
            pb_view_start: 0.0,
            pb_view_span: 86_400.0,
            tv: None,
            msg_tx,
            msg_rx,
            toasts: Vec::new(),
            live: (0..MAX_WINDOWS).map(|_| PlayWindow::new()).collect(),
            split,
            selected: 0,
            maximized: None,
            fullscreen: false,
            tour: None,
            show_ptz: false,
            ptz_step: 5,
            preset_no: 1,
            ptz_held: None,
            ptz_pulse_stop: None,
            tree_filter: String::new(),
            show_drawer: !store.settings.live_list_hidden,
            pb: (0..MAX_PLAYBACK_WINDOWS).map(|_| PlayWindow::new()).collect(),
            pb_split: 4,
            pb_selected: 0,
            pb_day: clock::today(),
            pb_filter: RecordFilter::All,
            pb_checked: Vec::new(),
            pb_results: HashMap::new(),
            pb_rx: None,
            pb_pending: 0,
            pb_speed: 3,
            form: if opts.page.as_deref() == Some("add") { Some(DeviceForm::default()) } else { None },
            selected_devices: HashSet::new(),
            confirm_delete: None,
            import_path: crate::model::media_dir(&store.settings).join("devices.json").display().to_string(),
            search: SearchState::default(),
            net: net_ui::NetUi::default(),
            more_open: false,
            events: Vec::new(),
            unread: 0,
            alarm_tx,
            alarm_rx,
            alarm_stops: HashMap::new(),
            event_filter: String::new(),
            event_kind: 0,
            alarms_running_for: None,
            locked,
            unlock_input: String::new(),
            unlock_error: false,
            new_password: String::new(),
            shot: opts.screenshot.map(|path| Shot { path, frame: 0, requested: false, started: Instant::now() }),
            frame_no: 0,
            store,
        };
        // Checks only: MYCCTV_TEST_ZOOM=2.5 starts window 1 zoomed in (to look at the zoom display).
        if let Some(z) = std::env::var("MYCCTV_TEST_ZOOM").ok().and_then(|v| v.parse::<f32>().ok()) {
            for w in app.live.iter_mut() {
                w.zoom_at(z, [0.6, 0.6]);
            }
        }
        // Checks only: MYCCTV_TEST_PB=<hh:mm> searches yesterday on the first 2 cameras, zooms the
        // timeline to an hour around that time and starts playing camera 1 there.
        if let Some(at) = std::env::var("MYCCTV_TEST_PB").ok() {
            let p: Vec<u32> = at.split(':').filter_map(|x| x.parse().ok()).collect();
            let secs = p.first().copied().unwrap_or(12) * 3600 + p.get(1).copied().unwrap_or(0) * 60;
            app.pb_checked = app.all_sources().into_iter().take(2).collect();
            app.pb_day = clock::shift_day(clock::today(), -1);
            app.search_records(&cc.egui_ctx);
            app.pb_view_span = 3_600.0;
            app.pb_view_start = (secs as f32 - 1_800.0).max(0.0);
            app.pb_filters_open = false;
            app.play_from(0, secs);
        }
        // Bring back the cameras that were open last time.
        for (i, dev, ch) in app.store.settings.last_view.clone() {
            if i < MAX_WINDOWS && app.store.device(dev).map(|d| ch < d.channels).unwrap_or(false) {
                app.live[i].source = Some(Source { device: dev, channel: ch });
            }
        }
        // `--page devcfg[N]`: open the settings of saved device N (for checks).
        if let Some(n) = opts_page_devcfg {
            if let Some(id) = app.store.devices.get(n.saturating_sub(1)).map(|d| d.id) {
                app.open_config(id, &cc.egui_ctx);
            }
        }
        // Refresh every device's details (model, cameras, which camera is behind each
        // recorder channel) in the background, once per start.
        if app.shot.is_none() || std::env::var("MYCCTV_SHOT_SECS").is_ok() {
            for id in app.store.devices.iter().map(|d| d.id).collect::<Vec<_>>() {
                app.login_quiet(id, &cc.egui_ctx);
            }
        }
        app
    }

    /// Remembers which cameras are open in Live View (saved only when it changes).
    fn remember_view(&mut self) {
        let now: Vec<(usize, u64, u32)> = self.live.iter().enumerate().filter_map(|(i, w)| w.source.map(|s| (i, s.device, s.channel))).collect();
        if now != self.store.settings.last_view && self.tour.is_none() {
            self.store.settings.last_view = now;
            self.save();
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), at: Instant::now(), error: false });
    }
    fn toast_err(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), at: Instant::now(), error: true });
    }

    fn save(&mut self) {
        if let Some((tv, _)) = &self.tv {
            tv.set_devices(&self.store.devices);
        }
        if let Err(e) = self.store.save() {
            self.toast_err(format!("Could not save: {e}"));
        }
    }

    fn source_label(&self, s: Source) -> String {
        match self.store.device(s.device) {
            Some(d) => format!("{} - {}", d.name, d.channel_name(s.channel)),
            None => "Removed device".into(),
        }
    }

    // ------------------------------------------------------------ background work

    fn poll_background(&mut self, ctx: &egui::Context) {
        while let Ok((id, on)) = self.online_rx.try_recv() {
            self.online.insert(id, on);
        }
        while let Ok((id, res)) = self.info_rx.try_recv() {
            self.logging_in.remove(&id);
            let name = self.store.device(id).map(|d| d.name.clone()).unwrap_or_default();
            match res {
                Ok(info) => {
                    if let Some(d) = self.store.devices.iter_mut().find(|d| d.id == id) {
                        if !info.device_type.is_empty() {
                            d.device_type = info.device_type.clone();
                        }
                        if !info.serial.is_empty() {
                            d.serial = info.serial.clone();
                        }
                        if let Some(c) = info.channels {
                            d.channels = c.min(crate::model::MAX_CHANNELS);
                        }
                        if !info.camera_links.is_empty() {
                            d.camera_links = info.camera_links.clone();
                        }
                        if !info.channel_names.is_empty() {
                            d.channel_names = info.channel_names.clone();
                        }
                    }
                    self.online.insert(id, true);
                    if !self.quiet_logins.remove(&id) {
                        self.toast(format!("{name}: logged in, {} channels", self.store.device(id).map(|d| d.channels).unwrap_or(0)));
                    }
                    self.save();
                }
                Err(e) => {
                    if !self.quiet_logins.remove(&id) {
                        self.toast_err(format!("{name}: {e}"));
                    }
                }
            }
        }
        while let Ok(m) = self.msg_rx.try_recv() {
            let err = m.contains("failed") || m.contains("wrong");
            self.toasts.push(Toast { text: m, at: Instant::now(), error: err });
        }
        while let Ok(ev) = self.alarm_rx.try_recv() {
            if self.page != Page::Events {
                self.unread += 1;
            }
            self.events.insert(0, ev);
            self.events.truncate(MAX_EVENTS);
        }
        if let Some(rx) = &self.pb_rx {
            while let Ok((src, res)) = rx.try_recv() {
                self.pb_results.insert(src, res);
                self.pb_pending = self.pb_pending.saturating_sub(1);
            }
        }
        if let Some(rx) = &self.search.rx {
            while let Ok(item) = rx.try_recv() {
                match item {
                    Some(f) => self.search.found.push(f),
                    None => self.search.running = false,
                }
            }
        }

        // Online check, every few seconds as set in Settings.
        let every = Duration::from_secs(self.store.settings.online_check_secs.max(5) as u64);
        if self.last_online_check.map(|t| t.elapsed() >= every).unwrap_or(true) {
            self.last_online_check = Some(Instant::now());
            for d in self.store.devices.clone() {
                let tx = self.online_tx.clone();
                let c = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send((d.id, services::is_online(&d)));
                    c.request_repaint();
                });
            }
        }

        // Alarm feeds follow the device list and the setting.
        let want = self.store.settings.listen_for_alarms;
        if !want {
            for (_, s) in self.alarm_stops.drain() {
                s.store(true, Ordering::Relaxed);
            }
        } else {
            let ids: HashSet<u64> = self.store.devices.iter().map(|d| d.id).collect();
            self.alarm_stops.retain(|id, s| {
                let keep = ids.contains(id);
                if !keep {
                    s.store(true, Ordering::Relaxed);
                }
                keep
            });
            for d in &self.store.devices {
                if !self.alarm_stops.contains_key(&d.id) {
                    let stop = Arc::new(AtomicBool::new(false));
                    services::listen_alarms(d.clone(), stop.clone(), self.alarm_tx.clone(), ctx.clone());
                    self.alarm_stops.insert(d.id, stop);
                }
            }
        }
        self.alarms_running_for = Some(want);

        // TV server (Hisense VIDAA and other TVs) follows its setting and port.
        let tv_want = self.store.settings.tv_server && self.shot.is_none();
        let port = self.store.settings.tv_port;
        if !tv_want || self.tv.as_ref().map(|(_, p)| *p != port).unwrap_or(false) {
            self.tv = None;
        }
        if tv_want && self.tv.is_none() {
            if self.store.settings.tv_key.is_empty() {
                self.store.settings.tv_key = crate::model::new_tv_code();
                self.save();
            }
            match crate::relay::start(port, self.store.settings.tv_key.clone(), &self.store.devices) {
                Ok(s) => self.tv = Some((s, port)),
                Err(e) => {
                    self.store.settings.tv_server = false;
                    self.save();
                    self.toast_err(format!("TV sharing could not start: {e}"));
                }
            }
        }

        // PTZ pulse from a remote control press.
        if let Some((at, src, cmd)) = self.ptz_pulse_stop {
            if Instant::now() >= at {
                self.ptz_pulse_stop = None;
                self.send_ptz(src, cmd, false);
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        // Tour: move to the next page of channels.
        if let Some(t) = &self.tour {
            let every = Duration::from_secs(self.store.settings.tour_interval_secs.max(3) as u64);
            if t.last.elapsed() >= every {
                self.tour_step();
            }
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn restart_alarm_listener(&mut self, id: u64) {
        if let Some(s) = self.alarm_stops.remove(&id) {
            s.store(true, Ordering::Relaxed);
        }
    }

    /// Like login, but without a message (used at start-up).
    fn login_quiet(&mut self, id: u64, ctx: &egui::Context) {
        self.quiet_logins.insert(id);
        self.login(id, ctx);
    }

    fn login(&mut self, id: u64, ctx: &egui::Context) {
        let Some(d) = self.store.device(id).cloned() else { return };
        self.logging_in.insert(id);
        let tx = self.info_tx.clone();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((id, services::device_info(&d.http())));
            c.request_repaint();
        });
    }

    // ------------------------------------------------------------ streams

    fn auto_stream(&self, windows: usize, maximized: bool) -> u8 {
        if !self.store.settings.auto_stream_type || maximized || windows as u32 <= self.store.settings.main_stream_max_windows {
            0
        } else {
            1
        }
    }

    /// Makes every visible window's connection match what it should show.
    fn reconcile(&mut self, ctx: &egui::Context) {
        let count = self.split;
        // Only windows someone can see use the network and the processor: another page, a window
        // hidden behind a maximized one, or the app minimized pauses them (a recording keeps going).
        // The app on another monitor, or behind another app's window, keeps playing.
        let shown = !ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        for i in 0..MAX_WINDOWS {
            let visible = shown && i < count && self.page == Page::Live && (self.maximized.is_none() || self.maximized == Some(i));
            if self.live[i].paused_by_itself() || (!visible && !self.live[i].recording()) {
                self.live[i].pause();
            }
            // Windows beyond the current split are closed.
            let keep = i < count;
            let want_stream = self.live[i].stream_override.unwrap_or_else(|| self.auto_stream(count, self.maximized == Some(i)));
            let desired: Option<(Vec<stream::Route>, bool)> = if !keep {
                None
            } else {
                match (self.live[i].source, &self.live[i].mode) {
                    // Live: the device first, then (for a recorder) the camera behind the channel.
                    (Some(s), WinMode::Live) => self
                        .store
                        .device(s.device)
                        .filter(|d| s.channel < d.channels)
                        .map(|d| (d.live_routes(s.channel, want_stream, &self.store.devices), false)),
                    // Instant replay: RTSP playback first, then the recorder's files.
                    (Some(s), WinMode::Replay { start, end }) => match (self.store.device(s.device), clock::parse_stamp(start), clock::parse_stamp(end)) {
                        (Some(d), Some((day, from)), Some((_, until))) if s.channel < d.channels => Some((d.playback_routes(s.channel, day, from, until.max(from)), true)),
                        _ => None,
                    },
                    (Some(s), mode) => self.url_for(mode, s, want_stream).map(|(url, user, pass, pb)| (vec![stream::Route { url, user, pass, label: String::new() }], pb)),
                    (None, _) => None,
                }
            };
            let w = &mut self.live[i];
            match desired {
                None => {
                    if w.handle.is_some() {
                        w.handle = None;
                        w.tex = None;
                    }
                }
                Some(_) if !visible && w.handle.is_none() => {} // paused
                Some((routes, is_pb)) => {
                    let key = routes.iter().map(|r| r.url.as_str()).collect::<Vec<_>>().join(" | ");
                    if w.handle.as_ref().map(|h| h.url != key).unwrap_or(true) {
                        w.handle = Some(stream::start_routes(ctx.clone(), routes, is_pb));
                        w.started = Some(Instant::now());
                        w.shown_frames = 0;
                    }
                }
            }
        }
        for i in 0..MAX_PLAYBACK_WINDOWS {
            let visible = shown && i < self.pb_split && self.page == Page::Playback;
            if self.pb[i].paused_by_itself() || (!visible && !self.pb[i].recording()) {
                self.pb[i].pause();
            }
            let keep = i < self.pb_split;
            let desired: Option<Vec<stream::Route>> = if !keep {
                None
            } else {
                match (self.pb[i].source, &self.pb[i].mode) {
                    (Some(s), WinMode::Playback { day, from }) => self
                        .store
                        .device(s.device)
                        .filter(|d| s.channel < d.channels)
                        .map(|d| d.playback_routes(s.channel, *day, *from, 86_399)),
                    _ => None,
                }
            };
            let speed = SPEEDS[self.pb_speed].1;
            let w = &mut self.pb[i];
            match desired {
                None => {
                    w.handle = None;
                    w.tex = None;
                }
                Some(_) if !visible && w.handle.is_none() => {} // paused
                Some(routes) => {
                    let key = routes.iter().map(|r| r.url.as_str()).collect::<Vec<_>>().join(" | ");
                    if w.handle.as_ref().map(|h| h.url != key).unwrap_or(true) {
                        let h = stream::start_routes(ctx.clone(), routes, true);
                        h.set_speed(speed);
                        w.handle = Some(h);
                        w.started = Some(Instant::now());
                        w.shown_frames = 0;
                    }
                }
            }
        }
    }

    fn url_for(&self, mode: &WinMode, s: Source, stream_type: u8) -> Option<(String, String, String, bool)> {
        let d = self.store.device(s.device)?;
        if s.channel >= d.channels {
            return None;
        }
        Some(match mode {
            WinMode::Live => (d.live_url(s.channel, stream_type), d.user.clone(), d.password.clone(), false),
            WinMode::Replay { start, end } => (d.playback_url(s.channel, start, end), d.user.clone(), d.password.clone(), true),
            WinMode::Playback { day, from } => {
                let start = clock::playback_stamp(*day, *from);
                let end = clock::playback_stamp(*day, 86_399);
                (d.playback_url(s.channel, &start, &end), d.user.clone(), d.password.clone(), true)
            }
        })
    }

    fn pump_windows(&mut self, ctx: &egui::Context) {
        let media = crate::model::media_dir(&self.store.settings);
        let mut notes = Vec::new();
        for (i, w) in self.live.iter_mut().chain(self.pb.iter_mut()).enumerate() {
            let Some(h) = &w.handle else { continue };
            let f = h.shared.frames.load(Ordering::Relaxed);
            if f != w.shown_frames {
                w.shown_frames = f;
                if let Some(pic) = h.shared.picture.lock().unwrap().take() {
                    // No new memory per picture: the previous picture's buffer (uploaded by now) goes
                    // back to the video thread, and this one is handed over as it is, not copied.
                    if let Some(prev) = w.shown.take() {
                        if let Ok(img) = Arc::try_unwrap(prev) {
                            if let Ok(bytes) = bytemuck::allocation::try_cast_vec::<Color32, u8>(img.pixels) {
                                h.shared.recycle(bytes);
                            }
                        }
                    }
                    let size = [pic.w, pic.h];
                    // The pictures are opaque, so straight and premultiplied colors are the same bytes.
                    let pixels = bytemuck::allocation::try_cast_vec::<u8, Color32>(pic.rgba)
                        .unwrap_or_else(|(_, v)| v.chunks_exact(4).map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])).collect());
                    let img = Arc::new(egui::ColorImage { size, pixels });
                    match &mut w.tex {
                        // Same size: update the picture in place on the graphics card.
                        Some(t) if t.size() == size => t.set_partial([0, 0], img.clone(), TextureOptions::LINEAR),
                        Some(t) => t.set(img.clone(), TextureOptions::LINEAR),
                        None => w.tex = Some(ctx.load_texture(format!("video{i}"), img.clone(), TextureOptions::LINEAR)),
                    }
                    w.shown = Some(img);
                }
            }
            if let Some(n) = h.shared.notice.lock().unwrap().take() {
                notes.push(n);
            }
            if let Some((left, at)) = w.burst {
                if Instant::now() >= at {
                    h.snapshot(media.join("Snapshots").join(format!("capture_{}_{}.png", clock::file_stamp(), BURST_PICTURES - left + 1)));
                    w.burst = if left > 1 { Some((left - 1, Instant::now() + BURST_GAP)) } else { None };
                }
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        for n in notes {
            self.toast(n);
        }
    }

    fn open_in_live(&mut self, src: Source) {
        if self.split == 0 {
            self.split = 4;
        }
        let i = self.selected.min(self.split - 1);
        let w = &mut self.live[i];
        w.clear();
        w.source = Some(src);
        self.selected = (i + 1) % self.split;
        self.maximized = None;
    }

    fn open_all(&mut self, dev: u64) {
        let Some(d) = self.store.device(dev) else { return };
        let n = d.channels as usize;
        let need = layout::COUNTS.iter().copied().find(|&c| c >= n).unwrap_or(MAX_WINDOWS);
        if need > self.split {
            self.split = need;
        }
        for ch in 0..n.min(self.split) {
            self.live[ch].clear();
            self.live[ch].source = Some(Source { device: dev, channel: ch as u32 });
        }
        self.selected = 0;
        self.maximized = None;
    }

    fn all_sources(&self) -> Vec<Source> {
        self.store.devices.iter().flat_map(|d| (0..d.channels).map(move |c| Source { device: d.id, channel: c })).collect()
    }

    fn tour_step(&mut self) {
        let all = self.all_sources();
        let Some(t) = self.tour.as_mut() else { return };
        if all.is_empty() {
            self.tour = None;
            return;
        }
        let start = t.next % all.len();
        t.next = start + self.split;
        t.last = Instant::now();
        for i in 0..self.split {
            let w = &mut self.live[i];
            w.clear();
            if start + i < all.len() {
                w.source = Some(all[start + i]);
            }
        }
        if start + self.split >= all.len() {
            if let Some(t) = self.tour.as_mut() {
                t.next = 0;
            }
        }
    }

    fn send_ptz(&self, src: Source, cmd: Ptz, start: bool) {
        if let Some(d) = self.store.device(src.device) {
            services::ptz(d, src.channel, cmd, start, self.ptz_step, self.msg_tx.clone());
        }
    }

    fn apply_window_action(&mut self, playback: bool, i: usize, a: WinAction, ctx: &egui::Context) {
        let media = crate::model::media_dir(&self.store.settings);
        let label = {
            let wins = if playback { &self.pb } else { &self.live };
            wins[i].source.map(|s| self.source_label(s)).unwrap_or_default()
        };
        let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        match a {
            WinAction::Select => {
                if playback {
                    self.pb_selected = i
                } else {
                    self.selected = i
                }
            }
            WinAction::Maximize => {
                if playback {
                    self.pb_selected = i;
                } else {
                    self.selected = i;
                    self.maximized = if self.maximized == Some(i) { None } else { Some(i) };
                }
            }
            WinAction::Close => {
                let w = if playback { &mut self.pb[i] } else { &mut self.live[i] };
                w.clear();
                if playback {
                    // keep the checked list; just stop this window
                } else if self.maximized == Some(i) {
                    self.maximized = None;
                }
            }
            WinAction::CloseAll => {
                let wins = if playback { &mut self.pb } else { &mut self.live };
                for w in wins.iter_mut() {
                    w.clear();
                }
                if !playback {
                    self.maximized = None;
                    self.tour = None;
                }
            }
            WinAction::Record => {
                let w = if playback { &mut self.pb[i] } else { &mut self.live[i] };
                if let Some(h) = &w.handle {
                    if h.shared.recording.load(Ordering::Relaxed) {
                        h.stop_recording();
                    } else {
                        h.start_recording(media.join("Records").join(format!("{safe}_{}", clock::file_stamp())));
                        self.toast(format!("Window{} local record start.", i + 1));
                    }
                } else {
                    self.toast_err(format!("Window{} local record failed.", i + 1));
                }
            }
            WinAction::Capture => {
                let w = if playback { &self.pb[i] } else { &self.live[i] };
                match &w.handle {
                    Some(h) if h.shared.state() == State::Playing => h.snapshot(media.join("Snapshots").join(format!("{safe}_{}.png", clock::file_stamp()))),
                    _ => self.toast_err("Capture failed: no picture in this window"),
                }
            }
            WinAction::Burst => {
                let w = if playback { &mut self.pb[i] } else { &mut self.live[i] };
                if w.handle.is_some() {
                    w.burst = Some((BURST_PICTURES, Instant::now()));
                }
            }
            WinAction::Replay => {
                if playback {
                    return;
                }
                let secs = self.store.settings.instant_replay_secs.clamp(5, 600);
                let w = &mut self.live[i];
                w.mode = match w.mode {
                    WinMode::Replay { .. } => WinMode::Live,
                    _ => {
                        let (start, end) = clock::last_seconds(secs);
                        WinMode::Replay { start, end }
                    }
                };
            }
            WinAction::GoPlayback => {
                let src = if playback { None } else { self.live[i].source };
                if let Some(s) = src {
                    if !self.pb_checked.contains(&s) {
                        if self.pb_checked.len() >= MAX_PLAYBACK_WINDOWS {
                            self.toast_err(format!("Max checked channels cannot more than {MAX_PLAYBACK_WINDOWS}"));
                            return;
                        }
                        self.pb_checked.push(s);
                    }
                    self.page = Page::Playback;
                    self.search_records(ctx);
                }
            }
            WinAction::Scale(s) => {
                let w = if playback { &mut self.pb[i] } else { &mut self.live[i] };
                w.scale = s;
            }
            WinAction::Stream(s) => {
                if !playback {
                    self.live[i].stream_override = s;
                }
            }
            WinAction::Zoom(f, at) => {
                let w = if playback { &mut self.pb[i] } else { &mut self.live[i] };
                w.zoom_at(f, at);
            }
            WinAction::Pan(d) => {
                let w = if playback { &mut self.pb[i] } else { &mut self.live[i] };
                w.pan(d);
            }
            WinAction::ZoomReset => {
                let w = if playback { &mut self.pb[i] } else { &mut self.live[i] };
                w.zoom = 1.0;
                w.center = [0.5, 0.5];
            }
            WinAction::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
        }
    }

    // ------------------------------------------------------------ playback search

    fn search_records(&mut self, ctx: &egui::Context) {
        if self.pb_checked.is_empty() {
            self.toast_err("Please select cells to search!");
            return;
        }
        let (tx, rx) = channel();
        self.pb_rx = Some(rx);
        self.pb_results.clear();
        self.pb_pending = self.pb_checked.len();
        let day = clock::day_text(self.pb_day);
        for src in self.pb_checked.clone() {
            let Some(d) = self.store.device(src.device).cloned() else { continue };
            let tx = tx.clone();
            let day = day.clone();
            let filter = self.pb_filter;
            let c = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send((src, services::find_records(&d, src.channel, &day, filter)));
                c.request_repaint();
            });
        }
        // Pick the smallest layout that holds every checked channel (36 at most).
        let n = self.pb_checked.len();
        self.pb_split = [1usize, 4, 9, 16, 25, 36].into_iter().find(|&c| c >= n).unwrap_or(36);
        for w in self.pb.iter_mut() {
            w.clear();
        }
    }

    fn play_from(&mut self, row: usize, secs: u32) {
        let Some(src) = self.pb_checked.get(row).copied() else { return };
        if row >= MAX_PLAYBACK_WINDOWS {
            return;
        }
        let w = &mut self.pb[row];
        let scale = w.scale;
        w.clear();
        w.scale = scale;
        w.source = Some(src);
        w.mode = WinMode::Playback { day: self.pb_day, from: secs };
        self.pb_selected = row;
    }

    fn playing_position(w: &PlayWindow, speed: f32) -> Option<u32> {
        let WinMode::Playback { from, .. } = w.mode else { return None };
        let h = w.handle.as_ref()?;
        if h.shared.state() != State::Playing {
            return Some(from);
        }
        // The recorder-file player knows the exact time of the picture on screen.
        let exact = h.shared.position.load(Ordering::Relaxed);
        if exact > 0 {
            return Some((exact - 1) as u32);
        }
        let el = w.started.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0) * speed.min(1.0);
        Some((from as f32 + el).min(86_399.0) as u32)
    }
}

// ================================================================== drawing (Aurora design)

const NAV_W: f32 = 92.0;
const GAP: f32 = 6.0;

const EVENT_GROUPS: [&str; 6] = ["All", "Motion", "Alarm input", "Video problems", "Smart detection", "Other"];

fn event_group(code: &str) -> usize {
    match code {
        "VideoMotion" => 1,
        "AlarmLocal" | "AlarmOutput" => 2,
        "VideoLoss" | "VideoBlind" | "VideoAbnormalDetection" => 3,
        "CrossLineDetection" | "CrossRegionDetection" | "SmartMotionHuman" | "SmartMotionVehicle" | "FaceDetection" | "LeftDetection" | "TakenAwayDetection" => 4,
        _ => 5,
    }
}

fn event_color(pal: &Palette, code: &str) -> Color32 {
    match event_group(code) {
        1 => pal.warn,
        2 => pal.danger,
        3 => Color32::from_rgb(0xB0, 0x7C, 0xFF),
        4 => pal.accent,
        _ => pal.text_dim,
    }
}

fn layout_glyph(p: &egui::Painter, rect: Rect, count: usize, color: Color32) {
    let s = layout::split(count);
    let unit = rect.width() / s.grid as f32;
    let w = if s.grid > 5 { 0.6f32 } else { 1.2f32 };
    for cell in &s.cells {
        let r = Rect::from_min_size(pos2(rect.left() + cell.x as f32 * unit, rect.top() + cell.y as f32 * unit), vec2(cell.w as f32 * unit, cell.h as f32 * unit));
        p.rect_stroke(r.shrink(0.5), Rounding::same(1.0f32), Stroke::new(w, color));
    }
}

fn wall_rects(area: Rect, count: usize, maximized: Option<usize>) -> Vec<Option<Rect>> {
    let split = layout::split(count);
    let unit = vec2((area.width() + GAP) / split.grid as f32, (area.height() + GAP) / split.grid as f32);
    split
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| match maximized {
            Some(m) if m == i => Some(area),
            Some(_) => None,
            None => Some(Rect::from_min_size(
                pos2(area.left() + cell.x as f32 * unit.x, area.top() + cell.y as f32 * unit.y),
                vec2(cell.w as f32 * unit.x - GAP, cell.h as f32 * unit.y - GAP),
            )),
        })
        .collect()
}

fn round_text_button(ui: &mut Ui, pal: &Palette, text: &str, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), if enabled { Sense::click() } else { Sense::hover() });
    let hot = enabled && (resp.hovered() || resp.has_focus());
    let held = enabled && resp.is_pointer_button_down_on();
    ui.painter().circle_filled(rect.center(), 20.0, if held { pal.accent } else if hot { pal.hover } else { pal.field });
    if resp.has_focus() {
        ui.painter().circle_stroke(rect.center(), 22.0, Stroke::new(2.0f32, pal.accent));
    }
    let c = if !enabled { pal.text_dim } else if held { Color32::WHITE } else { pal.text_strong };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(20.0), c);
    resp
}

impl Page {
    fn short(self) -> &'static str {
        match self {
            Page::Live => "Live",
            other => other.label(),
        }
    }
}

impl KvmsApp {
    /// Phone navigation: a tab bar along the bottom edge, easy to reach with a thumb.
    fn bottom_nav(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        egui::TopBottomPanel::bottom("bottomnav")
            .exact_height(62.0)
            .frame(egui::Frame::none().fill(pal.surface).stroke(Stroke::new(1.0f32, pal.border)))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                let w = full.width() / Page::ALL.len() as f32;
                for (i, p) in Page::ALL.into_iter().enumerate() {
                    let rect = Rect::from_min_size(pos2(full.left() + i as f32 * w, full.top()), vec2(w, full.height()));
                    let resp = ui.interact(rect, Id::new(("tab", i)), Sense::click());
                    let sel = self.page == p;
                    let painter = ui.painter();
                    if sel {
                        painter.rect_filled(Rect::from_center_size(pos2(rect.center().x, rect.top() + 21.0), vec2(52.0, 28.0)), Rounding::same(14.0f32), pal.accent_soft);
                    }
                    if resp.has_focus() {
                        painter.rect_stroke(rect.shrink(3.0), Rounding::same(10.0f32), Stroke::new(2.0f32, pal.accent));
                    }
                    let col = if sel { pal.accent } else { pal.text_dim };
                    let ir = Rect::from_center_size(pos2(rect.center().x, rect.top() + 21.0), vec2(20.0, 20.0));
                    icons::paint(painter, ir, p.icon(), col);
                    painter.text(pos2(rect.center().x, rect.bottom() - 12.0), Align2::CENTER_CENTER, p.short(), FontId::proportional(11.0), if sel { pal.text_strong } else { pal.text_dim });
                    if p == Page::Events && self.unread > 0 {
                        let c = pos2(ir.right() + 5.0, ir.top());
                        painter.circle_filled(c, 8.0, pal.danger);
                        painter.text(c, Align2::CENTER_CENTER, if self.unread > 99 { "99+".to_string() } else { self.unread.to_string() }, FontId::proportional(9.0), Color32::WHITE);
                    }
                    if resp.clicked() {
                        self.page = p;
                        if p == Page::Events {
                            self.unread = 0;
                        }
                    }
                }
            });
    }

    /// When the left menu is folded away: one small round button (bottom-left) brings it back.
    fn nav_handle(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        egui::Area::new(Id::new("navhandle")).anchor(Align2::LEFT_BOTTOM, vec2(10.0, -10.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            if kit::icon_button(ui, &pal, Icon::Sidebar, &format!("Show the menu ({})", self.page.label()), false).clicked() {
                self.store.settings.nav_collapsed = false;
                self.save();
            }
        });
    }

    fn nav_rail(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        egui::SidePanel::left("nav")
            .exact_width(NAV_W)
            .resizable(false)
            .frame(egui::Frame::none().fill(pal.surface).inner_margin(egui::Margin::symmetric(10.0, 16.0)).stroke(Stroke::new(1.0f32, pal.border)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    icons::paint(ui.painter(), r, Icon::Logo, Color32::WHITE);
                    ui.add_space(20.0);
                    for p in Page::ALL {
                        let sel = self.page == p;
                        let (rect, resp) = ui.allocate_exact_size(vec2(72.0, 62.0), Sense::click());
                        let hot = resp.hovered() || resp.has_focus();
                        let painter = ui.painter();
                        if sel {
                            painter.rect_filled(rect, Rounding::same(14.0f32), pal.accent_soft);
                        } else if hot {
                            painter.rect_filled(rect, Rounding::same(14.0f32), pal.hover);
                        }
                        if resp.has_focus() {
                            painter.rect_stroke(rect, Rounding::same(14.0f32), Stroke::new(2.0f32, pal.accent));
                        }
                        let col = if sel { pal.accent } else if hot { pal.text_strong } else { pal.text_dim };
                        let ir = Rect::from_center_size(pos2(rect.center().x, rect.top() + 23.0), vec2(22.0, 22.0));
                        icons::paint(painter, ir, p.icon(), col);
                        painter.text(pos2(rect.center().x, rect.bottom() - 13.0), Align2::CENTER_CENTER, p.short(), FontId::proportional(11.5), if sel { pal.text_strong } else { col });
                        if p == Page::Events && self.unread > 0 {
                            let c = pos2(ir.right() + 4.0, ir.top() + 1.0);
                            painter.circle_filled(c, 8.5, pal.danger);
                            painter.text(c, Align2::CENTER_CENTER, if self.unread > 99 { "99+".to_string() } else { self.unread.to_string() }, FontId::proportional(9.0), Color32::WHITE);
                        }
                        if resp.on_hover_text(p.label()).clicked() {
                            self.page = p;
                            if p == Page::Events {
                                self.unread = 0;
                            }
                        }
                        ui.add_space(2.0);
                    }
                    ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                        ui.label(RichText::new(clock::short_time()).size(13.0).color(pal.text_dim));
                        if kit::icon_button(ui, &pal, Icon::Sidebar, "Hide this menu (more room for video)", false).clicked() {
                            self.store.settings.nav_collapsed = true;
                            self.save();
                        }
                        let dark = self.store.settings.theme == ThemeChoice::Dark;
                        if kit::icon_button(ui, &pal, if dark { Icon::Sun } else { Icon::Moon }, if dark { "Switch to Daylight" } else { "Switch to Midnight" }, false).clicked() {
                            self.store.settings.theme = if dark { ThemeChoice::Light } else { ThemeChoice::Dark };
                            self.save();
                        }
                        if kit::icon_button(ui, &pal, Icon::Fullscreen, "Full screen (F11)", self.fullscreen).clicked() {
                            self.fullscreen = !self.fullscreen;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                        }
                    });
                });
            });
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.at.elapsed() < TOAST_TIME);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        let pal = self.pal;
        egui::Area::new(Id::new("toasts")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -86.0)).order(egui::Order::Foreground).interactable(false).show(ctx, |ui| {
            for t in self.toasts.iter().rev().take(4) {
                egui::Frame::none()
                    .fill(if pal.dark { Color32::from_rgb(0x26, 0x30, 0x40) } else { Color32::from_rgb(0x1C, 0x24, 0x31) })
                    .rounding(Rounding::same(22.0f32))
                    .inner_margin(egui::Margin::symmetric(18.0, 10.0))
                    .shadow(egui::epaint::Shadow { offset: vec2(0.0, 6.0), blur: 16.0, spread: 0.0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.5, if t.error { pal.danger } else { pal.live });
                            ui.label(RichText::new(&t.text).color(Color32::WHITE));
                        });
                    });
                ui.add_space(6.0);
            }
        });
    }

    // ------------------------------------------------------------ dashboard

    fn home(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let mut go: Option<Page> = None;
        let mut watch: Option<Source> = None;
        let mut add = false;
        let mut scan = false;
        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::same(28.0))).show(ctx, |ui| {
            kit::page_header(ui, &pal, clock::greeting(), &clock::long_date(), |ui| {
                if kit::primary(ui, &pal, "+  Add device").clicked() {
                    add = true;
                }
            });
            let total = self.store.devices.len();
            let online = self.store.devices.iter().filter(|d| self.online.get(&d.id).copied().unwrap_or(false)).count();
            let cams: u32 = self.store.devices.iter().map(|d| d.channels).sum();
            let today = self.events.iter().filter(|e| clock::is_today(&e.time)).count();
            let rec = self.live.iter().chain(self.pb.iter()).filter(|w| w.recording()).count();
            let avail = ui.available_width();
            let tile = ((avail - 3.0 * 14.0) / 4.0).max(170.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
                if kit::stat_tile(ui, &pal, tile, Icon::Server, pal.live, &format!("{online}/{total}"), "Devices online").clicked() {
                    go = Some(Page::Devices);
                }
                if kit::stat_tile(ui, &pal, tile, Icon::Camera, pal.accent, &cams.to_string(), "Cameras").clicked() {
                    go = Some(Page::Live);
                }
                if kit::stat_tile(ui, &pal, tile, Icon::Bell, pal.warn, &today.to_string(), "Events today").clicked() {
                    go = Some(Page::Events);
                }
                if kit::stat_tile(ui, &pal, tile, Icon::Record, pal.danger, &rec.to_string(), "Recording now").clicked() {
                    go = Some(Page::Live);
                }
            });
            ui.add_space(18.0);
            if total == 0 {
                kit::card(ui, &pal, |ui| {
                    ui.set_width(ui.available_width());
                    kit::empty_state(ui, &pal, Icon::Camera, "Let's add your first recorder", "MyCCTV shows live video, recordings and alarms from recorders (NVR/DVR) and IP cameras.");
                    ui.vertical_centered(|ui| {
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 330.0).max(0.0) / 2.0);
                            if kit::primary(ui, &pal, "+  Add device").clicked() {
                                add = true;
                            }
                            if kit::secondary(ui, "Scan my network").clicked() {
                                scan = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
                return;
            }
            let ncols = if self.compact { 1 } else { 2 };
            ui.columns(ncols, |cols| {
                let last = cols.len() - 1;
                kit::card(&mut cols[0], &pal, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Cameras").size(17.0).strong().color(pal.text_strong));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.link("Open Live").clicked() {
                                go = Some(Page::Live);
                            }
                        });
                    });
                    ui.add_space(6.0);
                    for s in self.all_sources().into_iter().take(8) {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                        let hot = resp.hovered() || resp.has_focus();
                        if hot {
                            ui.painter().rect_filled(rect, Rounding::same(10.0f32), pal.hover);
                        }
                        let ic = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(32.0, 32.0));
                        ui.painter().circle_filled(ic.center(), 16.0, pal.accent_soft);
                        icons::paint(ui.painter(), ic.shrink(8.0), Icon::Camera, pal.accent);
                        let d = self.store.device(s.device);
                        let name = d.map(|d| d.channel_name(s.channel)).unwrap_or_default();
                        let dev = d.map(|d| d.name.clone()).unwrap_or_default();
                        ui.painter().text(pos2(rect.left() + 46.0, rect.center().y - 8.0), Align2::LEFT_CENTER, name, FontId::proportional(14.0), pal.text_strong);
                        ui.painter().text(pos2(rect.left() + 46.0, rect.center().y + 10.0), Align2::LEFT_CENTER, dev, FontId::proportional(12.0), pal.text_dim);
                        let on = self.online.get(&s.device).copied().unwrap_or(false);
                        ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, if hot { "Watch  ›" } else if on && rect.width() > 300.0 { "Online" } else { "" }, FontId::proportional(12.5), if hot { pal.accent } else { pal.live });
                        if resp.clicked() {
                            watch = Some(s);
                        }
                    }
                });
                if last == 0 {
                    cols[0].add_space(12.0);
                }
                kit::card(&mut cols[last], &pal, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Recent events").size(17.0).strong().color(pal.text_strong));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.link("See all").clicked() {
                                go = Some(Page::Events);
                            }
                        });
                    });
                    ui.add_space(6.0);
                    if self.events.is_empty() {
                        ui.label(RichText::new("All quiet. Motion and alarms from your devices will show up here.").color(pal.text_dim));
                    }
                    for e in self.events.iter().take(7) {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(12.0, 36.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 5.0, event_color(&pal, &e.code));
                            ui.vertical(|ui| {
                                ui.label(RichText::new(services::alarm_label(&e.code)).color(pal.text_strong));
                                ui.label(RichText::new(format!("{}{}", e.device, e.channel.map(|c| format!(" · Camera {}", c + 1)).unwrap_or_default())).size(12.0).color(pal.text_dim));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(e.time.get(11..16).unwrap_or(&e.time)).color(pal.text_dim));
                            });
                        });
                    }
                });
            });
        });
        if add {
            self.page = Page::Devices;
            self.form = Some(DeviceForm::default());
            self.search.open = false;
        }
        if scan {
            self.page = Page::Devices;
            self.form = None;
            self.search.open = true;
            if !self.search.running {
                self.start_search(ctx);
            }
        }
        if let Some(p) = go {
            self.page = p;
            if p == Page::Events {
                self.unread = 0;
            }
        }
        if let Some(s) = watch {
            self.page = Page::Live;
            self.tour = None;
            self.open_in_live(s);
        }
    }

    // ------------------------------------------------------------ live view

    fn camera_drawer(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            if !self.compact && kit::icon_button(ui, &pal, Icon::Sidebar, "Hide the camera list (more room for video)", false).clicked() {
                self.show_drawer = false;
            }
            ui.label(RichText::new("Cameras").size(19.0).strong().color(pal.text_strong));
            let n: u32 = self.store.devices.iter().map(|d| d.channels).sum();
            ui.label(RichText::new(n.to_string()).color(pal.text_dim));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::icon_button(ui, &pal, Icon::Plus, "Add device", false).clicked() {
                    self.page = Page::Devices;
                    self.form = Some(DeviceForm::default());
                }
            });
        });
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.tree_filter).hint_text("Search cameras").desired_width(f32::INFINITY).margin(vec2(10.0, 8.0)));
        ui.add_space(8.0);
        if self.store.devices.is_empty() {
            kit::empty_state(ui, &pal, Icon::Server, "No devices yet", "Add a recorder to see its cameras here.");
            return;
        }
        // Touch screens and TV remotes: one tap / OK opens a camera.
        let quick_open = self.store.settings.remote_navigation || platform::IS_ANDROID || self.compact;
        let filter = self.tree_filter.to_lowercase();
        let mut open: Option<Source> = None;
        let mut open_all: Option<u64> = None;
        let mut login: Option<u64> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for d in &self.store.devices {
                let matches_dev = filter.is_empty() || d.name.to_lowercase().contains(&filter);
                let chans: Vec<u32> = (0..d.channels).filter(|&c| matches_dev || d.channel_name(c).to_lowercase().contains(&filter)).collect();
                if chans.is_empty() {
                    continue;
                }
                egui::Frame::none().fill(pal.card).rounding(Rounding::same(12.0f32)).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let id = ui.make_persistent_id(("dev", d.id));
                    let on = self.online.get(&d.id).copied();
                    egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, true)
                        .show_header(ui, |ui| {
                            let c = match on {
                                Some(true) => pal.live,
                                Some(false) => pal.text_dim,
                                None => pal.warn,
                            };
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, c);
                            let resp = ui.add(egui::Label::new(RichText::new(&d.name).strong().color(pal.text_strong)).sense(Sense::click()));
                            if resp.double_clicked() {
                                open_all = Some(d.id);
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Open all cameras").clicked() {
                                    open_all = Some(d.id);
                                    ui.close_menu();
                                }
                                if ui.button("Refresh details").clicked() {
                                    login = Some(d.id);
                                    ui.close_menu();
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Open all").clicked() {
                                    open_all = Some(d.id);
                                }
                            });
                        })
                        .body(|ui| {
                            for c in chans {
                                let src = Source { device: d.id, channel: c };
                                let showing = self.live.iter().take(self.split).any(|w| w.source == Some(src));
                                let playing = self.live.iter().take(self.split).any(|w| w.source == Some(src) && w.handle.as_ref().map(|h| h.shared.state() == State::Playing).unwrap_or(false));
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                                let hot = resp.hovered() || resp.has_focus();
                                if hot {
                                    ui.painter().rect_filled(rect, Rounding::same(8.0f32), pal.hover);
                                }
                                if resp.has_focus() {
                                    ui.painter().rect_stroke(rect, Rounding::same(8.0f32), Stroke::new(2.0f32, pal.accent));
                                }
                                let ir = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(18.0, 18.0));
                                icons::paint(ui.painter(), ir, Icon::Camera, if showing { pal.live } else { pal.text });
                                ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, truncate(&d.channel_name(c), 24), FontId::proportional(13.5), if showing { pal.text_strong } else { pal.text });
                                if playing {
                                    let pr = Rect::from_center_size(pos2(rect.right() - 26.0, rect.center().y), vec2(38.0, 18.0));
                                    ui.painter().rect_filled(pr, Rounding::same(9.0f32), pal.live.gamma_multiply(0.18));
                                    ui.painter().text(pr.center(), Align2::CENTER_CENTER, "LIVE", FontId::proportional(10.0), pal.live);
                                }
                                if resp.double_clicked() || (quick_open && resp.clicked()) {
                                    open = Some(src);
                                }
                                resp.on_hover_text(if quick_open { "Press to show in the selected window" } else { "Double-click to show in the selected window" });
                            }
                        });
                });
                ui.add_space(8.0);
            }
        });
        if let Some(s) = open {
            self.tour = None;
            self.open_in_live(s);
        }
        if let Some(d) = open_all {
            self.tour = None;
            self.open_all(d);
        }
        if let Some(d) = login {
            self.login(d, ctx);
        }
    }

    fn ptz_panel(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        let target = self.live.get(self.selected).and_then(|w| if w.mode == WinMode::Live { w.source } else { None });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Camera control").size(19.0).strong().color(pal.text_strong));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::icon_button(ui, &pal, Icon::Close, "Hide", false).clicked() {
                    self.show_ptz = false;
                }
            });
        });
        match target {
            Some(s) => ui.label(RichText::new(self.source_label(s)).color(pal.text_dim)),
            None => ui.label(RichText::new("Select a window that shows a camera. Works with cameras that can turn and zoom (PTZ).").color(pal.text_dim)),
        };
        ui.add_space(10.0);
        let enabled = target.is_some();
        let mut held: Option<Ptz> = None;
        let mut pulsed: Option<Ptz> = None;

        // Round direction pad
        let size = 210.0;
        let (pad, _) = ui.allocate_exact_size(vec2(ui.available_width(), size), Sense::hover());
        let c = pad.center();
        ui.painter().circle_filled(c, size / 2.0 - 2.0, pal.card);
        ui.painter().circle_stroke(c, size / 2.0 - 2.0, Stroke::new(1.0f32, pal.border));
        let dirs = [
            (Ptz::Right, 0.0f32, "Right"),
            (Ptz::RightDown, 45.0, "Right Down"),
            (Ptz::Down, 90.0, "Down"),
            (Ptz::LeftDown, 135.0, "Left Down"),
            (Ptz::Left, 180.0, "Left"),
            (Ptz::LeftUp, 225.0, "Left Up"),
            (Ptz::Up, 270.0, "Up"),
            (Ptz::RightUp, 315.0, "Right Up"),
        ];
        for (cmd, deg, tip) in dirs {
            let a = deg.to_radians();
            let d = vec2(a.cos(), a.sin());
            let bc = c + d * 70.0;
            let r = Rect::from_center_size(bc, vec2(44.0, 44.0));
            let resp = ui.interact(r, Id::new(("ptzdir", cmd.code())), if enabled { Sense::click() } else { Sense::hover() });
            let down = enabled && resp.is_pointer_button_down_on();
            let hot = enabled && (resp.hovered() || resp.has_focus());
            let fill = if down || self.ptz_held.map(|h| h.1 == cmd).unwrap_or(false) { pal.accent } else if hot { pal.hover } else { pal.field };
            ui.painter().circle_filled(bc, 22.0, fill);
            if resp.has_focus() {
                ui.painter().circle_stroke(bc, 24.0, Stroke::new(2.0f32, pal.accent));
            }
            let n = vec2(-d.y, d.x);
            let col = if !enabled { pal.text_dim } else if down { Color32::WHITE } else { pal.text_strong };
            let tip_pt = bc + d * 8.0;
            ui.painter().add(egui::Shape::convex_polygon(vec![tip_pt, bc - d * 5.0 + n * 7.0, bc - d * 5.0 - n * 7.0], col, Stroke::NONE));
            if down {
                held = Some(cmd);
            } else if enabled && resp.clicked() {
                pulsed = Some(cmd);
            }
            resp.on_hover_text(tip);
        }
        ui.painter().circle_filled(c, 30.0, pal.field);
        ui.painter().text(c - vec2(0.0, 7.0), Align2::CENTER_CENTER, self.ptz_step.to_string(), FontId::proportional(20.0), pal.text_strong);
        ui.painter().text(c + vec2(0.0, 13.0), Align2::CENTER_CENTER, "speed", FontId::proportional(10.0), pal.text_dim);
        ui.add_space(8.0);

        for (minus, label, plus) in [(Ptz::ZoomOut, "Zoom", Ptz::ZoomIn), (Ptz::FocusNear, "Focus", Ptz::FocusFar), (Ptz::IrisClose, "Iris", Ptz::IrisOpen)] {
            ui.horizontal(|ui| {
                ui.add_sized(vec2(76.0, 40.0), egui::Label::new(RichText::new(label).color(pal.text_strong)));
                let r1 = round_text_button(ui, &pal, "−", enabled).on_hover_text(format!("{label} -"));
                if r1.is_pointer_button_down_on() && enabled {
                    held = Some(minus);
                } else if r1.clicked() {
                    pulsed = Some(minus);
                }
                let r2 = round_text_button(ui, &pal, "+", enabled).on_hover_text(format!("{label} +"));
                if r2.is_pointer_button_down_on() && enabled {
                    held = Some(plus);
                } else if r2.clicked() {
                    pulsed = Some(plus);
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_sized(vec2(76.0, 30.0), egui::Label::new(RichText::new("Speed").color(pal.text_strong)));
            ui.add(egui::Slider::new(&mut self.ptz_step, services::PTZ_STEP_MIN..=services::PTZ_STEP_MAX).show_value(false));
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Preset positions").strong().color(pal.text_strong));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.preset_no).range(services::PRESET_MIN..=services::PRESET_MAX).prefix("No. "));
            if ui.add_enabled(enabled, egui::Button::new("Go to")).clicked() {
                if let (Some(s), Some(d)) = (target, target.and_then(|s| self.store.device(s.device))) {
                    services::preset(d, s.channel, self.preset_no, false, self.msg_tx.clone());
                }
            }
            if ui.add_enabled(enabled, egui::Button::new("Save here")).on_hover_text("Remember the camera's current position under this number").clicked() {
                if let (Some(s), Some(d)) = (target, target.and_then(|s| self.store.device(s.device))) {
                    services::preset(d, s.channel, self.preset_no, true, self.msg_tx.clone());
                }
            }
        });

        // Press-and-hold moves the camera; a keyboard or remote press gives a short move.
        let now_held = match (target, held) {
            (Some(s), Some(c)) => Some((s, c)),
            _ => None,
        };
        if now_held != self.ptz_held {
            if let Some((s, c)) = self.ptz_held {
                self.send_ptz(s, c, false);
            }
            if let Some((s, c)) = now_held {
                self.send_ptz(s, c, true);
            }
            self.ptz_held = now_held;
        }
        if let (Some(s), Some(c), None) = (target, pulsed, now_held) {
            self.send_ptz(s, c, true);
            self.ptz_pulse_stop = Some((Instant::now() + PTZ_PULSE, s, c));
        }
    }

    fn live_view(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let phone = self.compact && !self.fullscreen;
        let portrait = phone && ctx.screen_rect().height() > ctx.screen_rect().width() * 1.1;
        if !self.fullscreen && self.show_drawer && !portrait {
            egui::SidePanel::left("cams")
                .resizable(true)
                .default_width(280.0)
                .width_range(220.0..=(if phone { (ctx.screen_rect().width() * 0.45).max(220.0) } else { 420.0 }))
                .frame(egui::Frame::none().fill(pal.surface).inner_margin(egui::Margin::same(14.0)).stroke(Stroke::new(1.0f32, pal.border)))
                .show(ctx, |ui| self.camera_drawer(ui, ctx));
        }
        if !self.fullscreen && self.show_ptz && phone {
            // Phone: camera control slides up from the bottom.
            egui::TopBottomPanel::bottom("ptzphone")
                .resizable(false)
                .exact_height((ctx.screen_rect().height() * 0.48).clamp(260.0, 520.0))
                .frame(egui::Frame::none().fill(pal.surface).inner_margin(egui::Margin::same(12.0)).stroke(Stroke::new(1.0f32, pal.border)))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.ptz_panel(ui));
                });
        } else if !self.fullscreen && self.show_ptz {
            egui::SidePanel::right("ptz")
                .resizable(false)
                .exact_width(262.0)
                .frame(egui::Frame::none().fill(pal.surface).inner_margin(egui::Margin::same(14.0)).stroke(Stroke::new(1.0f32, pal.border)))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.ptz_panel(ui));
                });
        }
        let margin = if self.fullscreen { 4.0 } else if phone { 8.0 } else { 12.0 };
        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::same(margin))).show(ctx, |ui| {
            let full = ui.max_rect();
            let dock_h = 52.0;
            // Phone held upright: a 16:9 video area on top, the controls, then the camera list.
            let portrait_here = portrait && !self.show_ptz;
            let wall = if portrait_here {
                Rect::from_min_size(full.min, vec2(full.width(), (full.width() * 9.0 / 16.0 * if self.split >= 4 && self.maximized.is_none() { 1.25 } else { 1.0 }).min(full.height() * 0.55)))
            } else {
                Rect::from_min_max(full.min, pos2(full.max.x, full.max.y - dock_h - 10.0))
            };
            let mut actions: Vec<(usize, WinAction)> = Vec::new();
            for (i, r) in wall_rects(wall, self.split, self.maximized).into_iter().enumerate() {
                let Some(rect) = r else { continue };
                let mut title = self.live[i].source.map(|s| self.source_label(s)).unwrap_or_default();
                if let Some(h) = &self.live[i].handle {
                    let via = h.shared.route.lock().unwrap().clone();
                    if !via.is_empty() && h.shared.state() == State::Playing {
                        title.push_str(&format!("  · via {via}"));
                    }
                }
                let acts = video_window(ui, rect, &self.live[i], i, &title, self.selected == i, &pal, false);
                actions.extend(acts.into_iter().map(|a| (i, a)));
            }
            // Floating control dock
            let dock_w = (if portrait_here { 300.0f32 } else { 350.0 }).min(full.width());
            let dock_y = if portrait_here { wall.bottom() + 10.0 + dock_h / 2.0 } else { full.max.y - dock_h / 2.0 };
            let dock = Rect::from_center_size(pos2(full.center().x, dock_y), vec2(dock_w, dock_h));
            ui.painter().rect(dock, Rounding::same(26.0f32), pal.card, Stroke::new(1.0f32, pal.border));
            let mut touring = self.tour.is_some();
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(dock.shrink2(vec2(14.0, 8.0))), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // layout picker
                    let (lr, lresp) = ui.allocate_exact_size(vec2(84.0, 36.0), Sense::click());
                    let hot = lresp.hovered() || lresp.has_focus();
                    ui.painter().rect_filled(lr, Rounding::same(18.0f32), if hot { pal.hover } else { pal.field });
                    if lresp.has_focus() {
                        ui.painter().rect_stroke(lr, Rounding::same(18.0f32), Stroke::new(2.0f32, pal.accent));
                    }
                    layout_glyph(ui.painter(), Rect::from_center_size(pos2(lr.left() + 22.0, lr.center().y), vec2(18.0, 18.0)), self.split, pal.text_strong);
                    ui.painter().text(pos2(lr.left() + 40.0, lr.center().y), Align2::LEFT_CENTER, self.split.to_string(), FontId::proportional(14.0), pal.text_strong);
                    let cv = pos2(lr.right() - 16.0, lr.center().y + 2.0);
                    ui.painter().add(egui::Shape::convex_polygon(vec![cv + vec2(-5.0, -4.0), cv + vec2(5.0, -4.0), cv + vec2(0.0, 2.0)], pal.text_dim, Stroke::NONE));
                    let popup = Id::new("layout_popup");
                    if lresp.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(popup));
                    }
                    let lresp = lresp.on_hover_text("Window layout");
                    let mut pick: Option<usize> = None;
                    egui::popup::popup_above_or_below_widget(ui, popup, &lresp, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                        ui.set_min_width(250.0);
                        ui.label(RichText::new("Window layout").strong().color(pal.text_strong));
                        egui::Grid::new("layouts").spacing(vec2(8.0, 8.0)).show(ui, |ui| {
                            for (n, c) in layout::COUNTS.iter().enumerate() {
                                let (r, resp) = ui.allocate_exact_size(vec2(52.0, 58.0), Sense::click());
                                let sel = self.split == *c;
                                let hot = resp.hovered() || resp.has_focus();
                                ui.painter().rect_filled(r, Rounding::same(10.0f32), if sel { pal.accent } else if hot { pal.hover } else { pal.field });
                                layout_glyph(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.top() + 22.0), vec2(28.0, 28.0)), *c, if sel { Color32::WHITE } else { pal.text_strong });
                                ui.painter().text(pos2(r.center().x, r.bottom() - 10.0), Align2::CENTER_CENTER, c.to_string(), FontId::proportional(11.0), if sel { Color32::WHITE } else { pal.text_dim });
                                if resp.clicked() {
                                    pick = Some(*c);
                                }
                                if n % 4 == 3 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
                    if let Some(c) = pick {
                        ui.memory_mut(|m| m.close_popup());
                        self.split = c;
                        self.maximized = None;
                        self.selected = self.selected.min(c - 1);
                        self.store.settings.last_split = c;
                        let _ = self.store.save();
                        if self.tour.is_some() {
                            self.tour_step();
                        }
                    }
                    if kit::icon_button(ui, &pal, Icon::Tour, if touring { "Stop tour" } else { "Tour: show every camera in turn" }, touring).clicked() {
                        touring = !touring;
                    }
                    if kit::icon_button(ui, &pal, Icon::Stop, "Close all video", false).clicked() {
                        actions.push((0, WinAction::CloseAll));
                    }
                    let (sep, _) = ui.allocate_exact_size(vec2(1.0, 26.0), Sense::hover());
                    ui.painter().rect_filled(sep, Rounding::ZERO, pal.border);
                    if !portrait_here && kit::icon_button(ui, &pal, Icon::Sidebar, "Camera list", self.show_drawer && !self.fullscreen).clicked() {
                        self.show_drawer = !self.show_drawer;
                    }
                    if kit::icon_button(ui, &pal, Icon::Joystick, "Camera control (PTZ)", self.show_ptz && !self.fullscreen).clicked() {
                        self.show_ptz = !self.show_ptz;
                    }
                    if kit::icon_button(ui, &pal, Icon::Fullscreen, if self.fullscreen { "Exit full screen" } else { "Full screen" }, self.fullscreen).clicked() {
                        actions.push((0, WinAction::Fullscreen));
                    }
                    if let Some(h) = self.live.get(self.selected).and_then(|w| w.handle.as_ref()) {
                        ui.label(RichText::new(truncate(&h.shared.info.lock().unwrap().clone(), 22)).size(11.0).color(pal.text_dim));
                    }
                });
            });
            if portrait_here {
                // The camera list fills the rest of the phone screen.
                let list = Rect::from_min_max(pos2(full.left(), dock.bottom() + 12.0), full.max);
                if list.height() > 80.0 {
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(list), |ui| self.camera_drawer(ui, ctx));
                }
            }
            if touring != self.tour.is_some() {
                if touring {
                    self.tour = Some(Tour { next: 0, last: Instant::now() });
                    self.tour_step();
                } else {
                    self.tour = None;
                }
            }
            for (i, a) in actions {
                self.apply_window_action(false, i, a, ctx);
            }
        });
    }

    // ------------------------------------------------------------ playback

    fn playback(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let phone = self.compact && !self.fullscreen;
        // Phone: the search choices take the whole screen until Search is pressed.
        let panel_open = if phone { self.pb_filters_open } else { !self.store.settings.pb_panel_hidden };
        if !self.fullscreen && panel_open {
            egui::SidePanel::left("pbleft")
                .resizable(false)
                .exact_width(if phone { ctx.screen_rect().width() - 30.0 } else { 290.0 })
                .frame(egui::Frame::none().fill(pal.surface).inner_margin(egui::Margin::same(14.0)).stroke(Stroke::new(1.0f32, pal.border)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Playback").size(19.0).strong().color(pal.text_strong));
                        if !phone {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if kit::icon_button(ui, &pal, Icon::Sidebar, "Hide this panel (more room for video)", false).clicked() {
                                    self.store.settings.pb_panel_hidden = true;
                                    let _ = self.store.save();
                                }
                            });
                        }
                    });
                    ui.label(RichText::new("Pick a day and cameras, then search.").color(pal.text_dim));
                    ui.add_space(10.0);
                    kit::card(ui, &pal, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if ui.button(" ◀ ").on_hover_text("Previous day").clicked() {
                                self.pb_day = clock::shift_day(self.pb_day, -1);
                            }
                            ui.add_sized(vec2(ui.available_width() - 46.0, 30.0), egui::Label::new(RichText::new(clock::nice_day(self.pb_day)).strong().color(pal.text_strong)));
                            if ui.button(" ▶ ").on_hover_text("Next day").clicked() {
                                self.pb_day = clock::shift_day(self.pb_day, 1);
                            }
                        });
                        ui.horizontal(|ui| {
                            let today = clock::today();
                            if kit::chip(ui, &pal, "Today", self.pb_day == today).clicked() {
                                self.pb_day = today;
                            }
                            let y = clock::shift_day(today, -1);
                            if kit::chip(ui, &pal, "Yesterday", self.pb_day == y).clicked() {
                                self.pb_day = y;
                            }
                        });
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new("Recording type").size(12.5).color(pal.text_dim));
                    ui.horizontal_wrapped(|ui| {
                        for f in RecordFilter::ALL {
                            let label = match f {
                                RecordFilter::All => "All",
                                RecordFilter::General => "Continuous",
                                RecordFilter::Alarm => "Alarm",
                                RecordFilter::Motion => "Motion",
                            };
                            if kit::chip(ui, &pal, label, self.pb_filter == f).on_hover_text(f.label()).clicked() {
                                self.pb_filter = f;
                            }
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new(format!("Cameras  ·  {} of {} chosen", self.pb_checked.len(), MAX_PLAYBACK_WINDOWS)).size(12.5).color(pal.text_dim));
                    let mut toggle: Option<Source> = None;
                    egui::ScrollArea::vertical().max_height((ui.available_height() - 56.0).max(60.0)).auto_shrink([false, false]).show(ui, |ui| {
                        for d in &self.store.devices {
                            ui.label(RichText::new(&d.name).strong().color(pal.text_strong));
                            for c in 0..d.channels {
                                let src = Source { device: d.id, channel: c };
                                let mut on = self.pb_checked.contains(&src);
                                if ui.checkbox(&mut on, d.channel_name(c)).changed() {
                                    toggle = Some(src);
                                }
                            }
                            ui.add_space(6.0);
                        }
                        if self.store.devices.is_empty() {
                            ui.label(RichText::new("Add a device first.").color(pal.text_dim));
                        }
                    });
                    if let Some(s) = toggle {
                        if let Some(pos) = self.pb_checked.iter().position(|x| *x == s) {
                            self.pb_checked.remove(pos);
                        } else if self.pb_checked.len() >= MAX_PLAYBACK_WINDOWS {
                            self.toast_err(format!("At most {MAX_PLAYBACK_WINDOWS} cameras can be played back together"));
                        } else {
                            self.pb_checked.push(s);
                        }
                    }
                    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                        let label = if self.pb_pending > 0 { "Searching..." } else { "Search recordings" };
                        let r = ui.add_sized(vec2(ui.available_width(), 40.0), egui::Button::new(RichText::new(label).strong().color(Color32::WHITE)).fill(pal.accent).rounding(Rounding::same(12.0f32)));
                        if r.clicked() {
                            self.search_records(ctx);
                            if self.compact && !self.pb_checked.is_empty() {
                                self.pb_filters_open = false;
                            }
                        }
                    });
                });
        }
        if phone && self.pb_filters_open {
            return;
        }
        let rows = self.pb_checked.len().max(1);
        let bar_h = if phone { (210.0 + rows as f32 * 28.0).min(ctx.screen_rect().height() * 0.55) } else { (156.0 + rows as f32 * 28.0).min(360.0) };
        let mut play: Option<(usize, u32)> = None;
        egui::TopBottomPanel::bottom("pbbar").exact_height(bar_h).frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin { left: 12.0, right: 12.0, top: 0.0, bottom: 12.0 })).show(ctx, |ui| {
            kit::card(ui, &pal, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());
                if phone && kit::secondary(ui, "Change day or cameras").clicked() {
                    self.pb_filters_open = true;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if !phone && self.store.settings.pb_panel_hidden && kit::icon_button(ui, &pal, Icon::Sidebar, "Show the search panel", false).clicked() {
                        self.store.settings.pb_panel_hidden = false;
                        let _ = self.store.save();
                    }
                    for (i, (label, speed, ok)) in SPEEDS.iter().enumerate() {
                        let tip = if *speed == 1.0 {
                            "Normal speed".to_string()
                        } else if *speed > 1.0 {
                            format!("Fast forward {label} (when playing from the recorder's files; above 2X only key pictures are shown)")
                        } else if *ok {
                            format!("Slow motion {label}")
                        } else {
                            "Fast forward needs a feature standard video streaming doesn't offer".into()
                        };
                        if !*ok {
                            ui.add_enabled(false, egui::Button::new(*label).rounding(Rounding::same(15.0f32))).on_disabled_hover_text(tip);
                            continue;
                        }
                        if kit::chip(ui, &pal, label, self.pb_speed == i).on_hover_text(tip).clicked() {
                            self.pb_speed = i;
                            for w in &self.pb {
                                if let Some(h) = &w.handle {
                                    h.set_speed(*speed);
                                }
                            }
                        }
                    }
                    ui.add_space(10.0);
                    let sel = self.pb_selected;
                    let rec = self.pb.get(sel).map(|w| w.recording()).unwrap_or(false);
                    if kit::icon_button(ui, &pal, Icon::Stop, "Stop this window", false).clicked() {
                        self.apply_window_action(true, sel, WinAction::Close, ctx);
                    }
                    if kit::icon_button(ui, &pal, Icon::Close, "Stop all windows", false).clicked() {
                        self.apply_window_action(true, sel, WinAction::CloseAll, ctx);
                    }
                    if kit::icon_button(ui, &pal, Icon::Record, if rec { "Stop saving clip" } else { "Save a clip of this window" }, rec).clicked() {
                        self.apply_window_action(true, sel, WinAction::Record, ctx);
                    }
                    if kit::icon_button(ui, &pal, Icon::Snapshot, "Take a picture", false).clicked() {
                        self.apply_window_action(true, sel, WinAction::Capture, ctx);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        for (c, label) in [(pal.warn, "Motion"), (pal.danger, "Alarm"), (pal.live, "Continuous")] {
                            ui.label(RichText::new(label).size(12.0).color(pal.text_dim));
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.5, c);
                        }
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let label_w = if phone { 92.0 } else { 170.0 };
                    let full = ui.available_width();
                    let bar_w = (full - label_w - 8.0).max(100.0);
                    // Zoomable timeline: the visible part of the day is [v0, v0 + span] seconds.
                    let (mut v0, mut span) = (self.pb_view_start, self.pb_view_span);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new("Timeline").color(pal.text_dim));
                        let center = v0 + span / 2.0;
                        if kit::chip(ui, &pal, "  −  ", false).on_hover_text("Zoom out (or mouse wheel / pinch over the timeline)").clicked() {
                            span = (span * 2.0).min(86_400.0);
                            v0 = center - span / 2.0;
                        }
                        if kit::chip(ui, &pal, "  +  ", false).on_hover_text("Zoom in").clicked() {
                            span = (span / 2.0).max(TIMELINE_MIN_SPAN);
                            v0 = center - span / 2.0;
                        }
                        for (label, s) in [("Day", 86_400.0f32), ("6 h", 21_600.0), ("1 h", 3_600.0), ("10 min", 600.0), ("2 min", 120.0)] {
                            if kit::chip(ui, &pal, label, (span - s).abs() < 1.0).clicked() {
                                span = s;
                                v0 = center - span / 2.0;
                            }
                        }
                        if kit::chip(ui, &pal, " ◀ ", false).on_hover_text("Earlier").clicked() {
                            v0 -= span * 0.5;
                        }
                        if kit::chip(ui, &pal, " ▶ ", false).on_hover_text("Later").clicked() {
                            v0 += span * 0.5;
                        }
                        v0 = v0.clamp(0.0, 86_400.0 - span);
                        ui.label(RichText::new(format!("{} – {}", clock::hms(v0 as u32), clock::hms((v0 + span).min(86_399.0) as u32))).monospace().color(pal.text_strong));
                    });
                    v0 = v0.clamp(0.0, 86_400.0 - span);
                    let to_x = |t: f32, left: f32| left + (t - v0) / span * bar_w;
                    let to_t = |x: f32, left: f32| (v0 + (x - left) / bar_w * span).clamp(0.0, 86_399.0);
                    // Ticks: about 8–12 across the visible part.
                    let step = [60.0f32, 120.0, 300.0, 600.0, 900.0, 1_800.0, 3_600.0, 7_200.0, 10_800.0].into_iter().find(|s| span / s <= 12.0).unwrap_or(10_800.0);
                    let mut ticks: Vec<f32> = Vec::new();
                    let mut tk = (v0 / step).ceil() * step;
                    while tk <= v0 + span {
                        ticks.push(tk);
                        tk += step;
                    }
                    let (ruler, _) = ui.allocate_exact_size(vec2(full, 16.0), Sense::hover());
                    for t in &ticks {
                        let x = to_x(*t, ruler.left() + label_w);
                        let secs = *t as u32;
                        let label = if step >= 3_600.0 { format!("{:02}:00", secs / 3600) } else { format!("{:02}:{:02}", secs / 3600, secs / 60 % 60) };
                        if x < ruler.right() - 30.0 {
                            ui.painter().text(pos2(x, ruler.top()), Align2::LEFT_TOP, label, FontId::proportional(10.5), pal.text_dim);
                        }
                    }
                    if self.pb_checked.is_empty() {
                        ui.label(RichText::new("Tick cameras on the left and press Search recordings. Then click the timeline to play from that moment.").color(pal.text_dim));
                    }
                    let speed = SPEEDS[self.pb_speed].1;
                    let mut zoom_at: Option<(f32, f32)> = None; // (factor, time under the pointer)
                    let mut pan_by = 0.0f32;
                    let mut follow: Option<u32> = None;
                    for (row, src) in self.pb_checked.iter().enumerate() {
                        let (rr, resp) = ui.allocate_exact_size(vec2(full, 24.0), Sense::click_and_drag());
                        let bar = Rect::from_min_size(pos2(rr.left() + label_w, rr.top() + 3.0), vec2(bar_w, 18.0));
                        let p = ui.painter_at(rr.expand(4.0));
                        if resp.has_focus() {
                            p.rect_stroke(rr, Rounding::same(6.0f32), Stroke::new(2.0f32, pal.accent));
                        }
                        let name = self.source_label(*src);
                        p.text(pos2(rr.left() + 4.0, rr.center().y), Align2::LEFT_CENTER, truncate(&name, 24), FontId::proportional(12.0), if self.pb_selected == row { pal.text_strong } else { pal.text });
                        p.rect_filled(bar, Rounding::same(6.0f32), pal.field);
                        for t in &ticks {
                            let x = to_x(*t, bar.left());
                            p.line_segment([pos2(x, bar.top() + 6.0), pos2(x, bar.bottom() - 6.0)], Stroke::new(1.0f32, pal.border));
                        }
                        let bp = ui.painter_at(bar);
                        match self.pb_results.get(src) {
                            Some(Ok(pieces)) => {
                                for piece in pieces.iter().filter(|pc| (pc.end as f32) >= v0 && (pc.start as f32) <= v0 + span) {
                                    let x0 = to_x((piece.start as f32).max(v0), bar.left());
                                    let x1 = to_x((piece.end as f32).min(v0 + span), bar.left());
                                    let c = match piece.kind {
                                        RecordKind::General => pal.live,
                                        RecordKind::Alarm => pal.danger,
                                        RecordKind::Motion => pal.warn,
                                    };
                                    bp.rect_filled(Rect::from_min_max(pos2(x0, bar.top() + 2.0), pos2(x1.max(x0 + 1.5), bar.bottom() - 2.0)), Rounding::same(3.0f32), c);
                                }
                                if pieces.is_empty() {
                                    p.text(bar.center(), Align2::CENTER_CENTER, "No recordings this day", FontId::proportional(11.0), pal.text_dim);
                                }
                            }
                            Some(Err(e)) => {
                                p.text(bar.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, truncate(e, 90), FontId::proportional(11.0), pal.danger);
                            }
                            None if self.pb_pending > 0 => {
                                p.text(bar.center(), Align2::CENTER_CENTER, "Searching...", FontId::proportional(11.0), pal.text_dim);
                            }
                            None => {}
                        }
                        if let Some(pos) = self.pb.get(row).and_then(|w| Self::playing_position(w, speed)) {
                            let t = pos as f32;
                            if t >= v0 && t <= v0 + span {
                                let x = to_x(t, bar.left());
                                bp.line_segment([pos2(x, bar.top()), pos2(x, bar.bottom())], Stroke::new(2.5f32, pal.accent));
                                p.circle_filled(pos2(x, bar.top() - 2.0), 4.0, pal.accent);
                            }
                            if row == self.pb_selected {
                                follow = Some(pos);
                            }
                        }
                        // Wheel / pinch zooms at the pointer; dragging moves the timeline.
                        if let Some(hp) = resp.hover_pos() {
                            if bar.contains(hp) {
                                let secs = to_t(hp.x, bar.left()) as u32;
                                bp.line_segment([pos2(hp.x, bar.top()), pos2(hp.x, bar.bottom())], Stroke::new(1.0f32, pal.text_strong));
                                resp.clone().on_hover_text(format!("Play from {}", clock::hms(secs)));
                                let (wheel, pinch) = ui.input(|i| (i.raw_scroll_delta.y, i.zoom_delta()));
                                let mut f = pinch;
                                if wheel != 0.0 {
                                    f *= (1.0 + wheel * 0.002).clamp(0.6, 1.6);
                                }
                                if (f - 1.0).abs() > 0.001 {
                                    zoom_at = Some((f, secs as f32));
                                }
                            }
                        }
                        if resp.dragged() {
                            pan_by -= resp.drag_delta().x / bar_w * span;
                        }
                        if resp.clicked() {
                            let first = self.pb_results.get(src).and_then(|r| r.as_ref().ok()).and_then(|v| v.first()).map(|p| p.start).unwrap_or(0);
                            let secs = resp.interact_pointer_pos().filter(|cp| bar.contains(*cp)).map(|cp| to_t(cp.x, bar.left()) as u32).unwrap_or(first);
                            play = Some((row, secs));
                        }
                    }
                    if let Some((f, at)) = zoom_at {
                        let new_span = (span / f).clamp(TIMELINE_MIN_SPAN, 86_400.0);
                        v0 = at - (at - v0) * (new_span / span);
                        span = new_span;
                    }
                    v0 += pan_by;
                    // Keep the playing moment in view when zoomed in.
                    if let Some(pos) = follow {
                        let t = pos as f32;
                        if pan_by == 0.0 && zoom_at.is_none() && span < 86_400.0 && t > v0 + span && t < v0 + span * 1.5 {
                            v0 = t - span * 0.1;
                        }
                    }
                    self.pb_view_span = span;
                    self.pb_view_start = v0.clamp(0.0, 86_400.0 - span);
                });
            });
        });
        if let Some((row, secs)) = play {
            self.play_from(row, secs);
        }
        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::same(12.0))).show(ctx, |ui| {
            let area = ui.max_rect();
            let mut actions = Vec::new();
            for (i, r) in wall_rects(area, self.pb_split.min(MAX_PLAYBACK_WINDOWS), None).into_iter().enumerate() {
                let Some(rect) = r else { continue };
                let title = match (&self.pb[i].source, &self.pb[i].mode) {
                    (Some(s), WinMode::Playback { day, .. }) => format!("{}  ·  {}", self.source_label(*s), clock::nice_day(*day)),
                    _ => self.pb_checked.get(i).map(|s| self.source_label(*s)).unwrap_or_default(),
                };
                let acts = video_window(ui, rect, &self.pb[i], i, &title, self.pb_selected == i, &pal, true);
                actions.extend(acts.into_iter().map(|a| (i, a)));
            }
            for (i, a) in actions {
                if a == WinAction::Maximize && self.pb[i].source.is_none() && i < self.pb_checked.len() {
                    let first = self.pb_results.get(&self.pb_checked[i]).and_then(|r| r.as_ref().ok()).and_then(|v| v.first()).map(|p| p.start).unwrap_or(0);
                    self.play_from(i, first);
                    continue;
                }
                self.apply_window_action(true, i, a, ctx);
            }
        });
    }

    // ------------------------------------------------------------ devices

    fn devices(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        if self.config.is_some() {
            self.config_page(ctx);
            return;
        }
        let mut configure: Option<u64> = None;
        if self.form.is_some() || self.search.open {
            self.device_sheet(ctx);
        }
        let mut edit: Option<u64> = None;
        let mut delete: Option<u64> = None;
        let mut login: Option<u64> = None;
        let mut watch: Option<u64> = None;
        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::same(28.0))).show(ctx, |ui| {
            let total = self.store.devices.len();
            let online = self.store.devices.iter().filter(|d| self.online.get(&d.id).copied().unwrap_or(false)).count();
            kit::page_header(ui, &pal, "Devices", &format!("{total} device(s)  ·  {online} online"), |ui| {
                if kit::primary(ui, &pal, "+  Add device").clicked() {
                    self.form = Some(DeviceForm::default());
                    self.search.open = false;
                }
                if kit::secondary(ui, "Scan network").on_hover_text("Find recorders and cameras on this network").clicked() {
                    self.form = None;
                    self.search.open = true;
                    if !self.search.running {
                        self.start_search(ctx);
                    }
                }
                if kit::secondary(ui, "Import / Export").clicked() {
                    self.more_open = !self.more_open;
                }
                if kit::icon_button(ui, &pal, Icon::Refresh, "Check which devices are online", false).clicked() {
                    self.last_online_check = None;
                    self.toast("Checking devices...");
                }
            });
            if self.more_open {
                kit::card(ui, &pal, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Move your device list to another computer, phone or TV").strong().color(pal.text_strong));
                    ui.label(RichText::new("Export saves the list (with passwords) to this file; Import adds the devices from it.").color(pal.text_dim));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.import_path).desired_width(ui.available_width() - 200.0).margin(vec2(10.0, 8.0)));
                        if kit::secondary(ui, "Export").clicked() {
                            let path = std::path::PathBuf::from(self.import_path.trim());
                            match self.store.export_devices(&path) {
                                Ok(n) => self.toast(format!("Exported {n} device(s) to {}", path.display())),
                                Err(e) => self.toast_err(format!("Export failed: {e}")),
                            }
                        }
                        if kit::secondary(ui, "Import").clicked() {
                            let path = std::path::PathBuf::from(self.import_path.trim());
                            match self.store.import_devices(&path) {
                                Ok((a, s)) => {
                                    self.save();
                                    self.toast(format!("Imported {a} device(s); {s} were already in the list"));
                                }
                                Err(e) => self.toast_err(format!("Import failed: {e}")),
                            }
                        }
                    });
                });
                ui.add_space(14.0);
            }
            if total == 0 {
                kit::card(ui, &pal, |ui| {
                    ui.set_width(ui.available_width());
                    kit::empty_state(ui, &pal, Icon::Server, "No devices yet", "Press Add device and type your recorder's address, or Scan network to find it.");
                });
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // Cards in a grid that fits the window: as many columns as fit, one per row on narrow screens.
                let gap = 14.0;
                let avail = ui.available_width();
                let cols = (((avail + gap) / (330.0 + gap)).floor() as usize).max(1);
                let card_w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
                let ids: Vec<u64> = self.store.devices.iter().map(|d| d.id).collect();
                for row in ids.chunks(cols) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for id in row {
                            let Some(d) = self.store.device(*id).cloned() else { continue };
                            ui.allocate_ui_with_layout(vec2(card_w, 10.0), Layout::top_down(Align::Min), |ui| {
                                egui::Frame::none().fill(pal.card).stroke(Stroke::new(1.0f32, pal.border)).rounding(Rounding::same(16.0f32)).inner_margin(egui::Margin::same(16.0)).show(ui, |ui| {
                                    ui.set_width(card_w - 34.0);
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(vec2(42.0, 42.0), Sense::hover());
                                        ui.painter().circle_filled(r.center(), 21.0, pal.accent_soft);
                                        icons::paint(ui.painter(), r.shrink(11.0), if d.channels > 1 { Icon::Server } else { Icon::Camera }, pal.accent);
                                        ui.vertical(|ui| {
                                            let name = ui.add(egui::Label::new(RichText::new(truncate(&d.name, 24)).size(16.0).strong().color(pal.text_strong)).sense(Sense::click()));
                                            if name.on_hover_text("Click to edit").clicked() {
                                                edit = Some(d.id);
                                            }
                                            ui.label(RichText::new(&d.host).color(pal.text_dim));
                                        });
                                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                            if self.logging_in.contains(&d.id) {
                                                kit::status_pill(ui, "Connecting", pal.warn);
                                            } else {
                                                match self.online.get(&d.id) {
                                                    Some(true) => kit::status_pill(ui, "Online", pal.live),
                                                    Some(false) => kit::status_pill(ui, "Offline", pal.text_dim),
                                                    None => kit::status_pill(ui, "Checking", pal.warn),
                                                }
                                            }
                                        });
                                    });
                                    ui.add_space(8.0);
                                    egui::Grid::new(("devinfo", d.id)).num_columns(2).spacing(vec2(14.0, 4.0)).show(ui, |ui| {
                                        let linked = d.camera_links.iter().flatten().count();
                                        for (k, v) in [
                                            ("Cameras", if linked > 0 { format!("{} ({} linked)", d.channels, linked) } else { d.channels.to_string() }),
                                            ("Ports", format!("video {} · web {}", d.rtsp_port, d.http_port)),
                                            ("Network", if d.network.is_empty() { "Automatic".into() } else { truncate(&d.network, 26) }),
                                            ("Model", if d.device_type.is_empty() { "—".into() } else { truncate(&d.device_type, 26) }),
                                            ("Serial", if d.serial.is_empty() { "—".into() } else { truncate(&d.serial, 22) }),
                                        ] {
                                            ui.label(RichText::new(k).size(12.5).color(pal.text_dim));
                                            ui.label(RichText::new(v).size(12.5).color(pal.text));
                                            ui.end_row();
                                        }
                                    });
                                    ui.add_space(10.0);
                                    ui.horizontal(|ui| {
                                        if kit::primary(ui, &pal, "Watch").clicked() {
                                            watch = Some(d.id);
                                        }
                                        if kit::secondary(ui, "Edit").on_hover_text("Name, address, ports, user name and password").clicked() {
                                            edit = Some(d.id);
                                        }
                                        if kit::secondary(ui, "Settings").on_hover_text("The device's own settings: video, picture, detection, recording, time, network and everything else").clicked() {
                                            configure = Some(d.id);
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        if ui.small_button("Refresh details").on_hover_text("Read model, serial number and cameras from the device").clicked() {
                                            login = Some(d.id);
                                        }
                                        if ui.small_button(RichText::new("Delete").color(pal.danger)).clicked() {
                                            delete = Some(d.id);
                                        }
                                    });
                                });
                            });
                        }
                    });
                    ui.add_space(gap);
                }
            });
        });
        if let Some(id) = edit {
            if let Some(d) = self.store.device(id) {
                self.form = Some(DeviceForm::from_device(d));
                self.search.open = false;
            }
        }
        if let Some(id) = delete {
            self.confirm_delete = Some(vec![id]);
        }
        if let Some(id) = login {
            self.login(id, ctx);
        }
        if let Some(id) = watch {
            self.tour = None;
            self.open_all(id);
            self.page = Page::Live;
        }
        if let Some(id) = configure {
            self.form = None;
            self.search.open = false;
            self.open_config(id, ctx);
        }
        self.delete_dialog(ctx);
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        self.search.found.clear();
        self.search.running = true;
        self.search.rx = Some(rx);
        let via = if self.search.via.is_empty() { None } else { crate::netpath::links().into_iter().find(|l| l.name == self.search.via) };
        services::search_lan(tx, ctx.clone(), via);
    }

    /// The right-hand sheet: add/edit form, or network scan results.
    fn device_sheet(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        egui::SidePanel::right("sheet")
            .resizable(false)
            .exact_width(if self.compact { ctx.screen_rect().width() - 46.0 } else { 410.0 })
            .frame(egui::Frame::none().fill(pal.surface).inner_margin(egui::Margin::same(22.0)).stroke(Stroke::new(1.0f32, pal.border)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if let Some(mut form) = self.form.take() {
                        let keep = self.form_contents(ui, ctx, &mut form);
                        if keep {
                            self.form = Some(form);
                        }
                    } else if self.search.open {
                        self.search_contents(ui, ctx);
                    }
                });
            });
    }

    fn form_contents(&mut self, ui: &mut Ui, ctx: &egui::Context, form: &mut DeviceForm) -> bool {
        let pal = self.pal;
        let mut keep = true;
        let mut submit: Option<bool> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(if form.editing.is_some() { "Edit device" } else { "Add device" }).size(21.0).strong().color(pal.text_strong));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::icon_button(ui, &pal, Icon::Close, "Close", false).clicked() {
                    keep = false;
                }
            });
        });
        ui.label(RichText::new("Your recorder or camera must offer standard video streaming (RTSP). Most do; it is usually on.").color(pal.text_dim));
        ui.add_space(12.0);
        let err = form.error.clone();
        let e = |f: FormField| err.as_ref().filter(|(ef, _)| *ef == f).map(|(_, m)| m.clone());
        kit::field(ui, &pal, "Name", e(FormField::Name).as_deref(), |ui| kit::text_input(ui, &mut form.name, "e.g. Home recorder", crate::model::MAX_NAME_LEN, false));
        kit::field(ui, &pal, "Address (IP or domain)", e(FormField::Host).as_deref(), |ui| kit::text_input(ui, &mut form.host, "e.g. 192.168.1.108", crate::model::MAX_HOST_LEN, false));
        kit::field(ui, &pal, "Network connection", None, |ui| net_ui::network_picker(ui, "devnet", &mut form.network, "Automatic (the system decides)"));
        ui.label(RichText::new("Only needed when the device is on another router than this computer's main connection: pick the connection that goes to that router (Settings > Networks).").size(12.0).color(pal.text_dim));
        ui.add_space(6.0);
        ui.columns(2, |c| {
            kit::field(&mut c[0], &pal, "Video port", e(FormField::Port).as_deref(), |ui| kit::text_input(ui, &mut form.rtsp_port, "554", 5, false));
            kit::field(&mut c[1], &pal, "Web port", e(FormField::HttpPort).as_deref(), |ui| kit::text_input(ui, &mut form.http_port, "80", 5, false));
        });
        kit::field(ui, &pal, "User name", e(FormField::User).as_deref(), |ui| kit::text_input(ui, &mut form.user, "admin", crate::model::MAX_USER_LEN, false));
        kit::field(ui, &pal, "Password", e(FormField::Password).as_deref(), |ui| kit::text_input(ui, &mut form.password, "", crate::model::MAX_PASS_LEN, true));
        kit::field(ui, &pal, "Number of cameras", e(FormField::Channels).as_deref(), |ui| kit::text_input(ui, &mut form.channels, "4", 3, false))
            .on_hover_text("Filled in from the device automatically when it allows");
        egui::CollapsingHeader::new(RichText::new("Advanced").color(pal.text_dim)).show(ui, |ui| {
            ui.label(RichText::new("Video address path: {ch} is the camera number, {stream} is 0 for the main stream and 1 for the smaller one.").size(12.0).color(pal.text_dim));
            kit::text_input(ui, &mut form.rtsp_path, crate::model::DEFAULT_RTSP_PATH, 200, false);
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if kit::primary(ui, &pal, if form.editing.is_some() { "Save changes" } else { "Add device" }).clicked() {
                submit = Some(false);
            }
            if form.editing.is_none() && kit::secondary(ui, "Add & add another").clicked() {
                submit = Some(true);
            }
        });
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            keep = false;
        }
        if let Some(cont) = submit {
            match form.validate(&self.store.devices) {
                Err(e) => form.error = Some(e),
                Ok(dev) => {
                    let id = if let Some(id) = form.editing {
                        if let Some(slot) = self.store.devices.iter_mut().find(|d| d.id == id) {
                            let mut dev = dev;
                            dev.id = id;
                            dev.device_type = slot.device_type.clone();
                            if dev.serial.is_empty() {
                                dev.serial = slot.serial.clone();
                            }
                            dev.channel_names = slot.channel_names.clone();
                            *slot = dev;
                        }
                        self.restart_alarm_listener(id);
                        id
                    } else {
                        self.store.add_device(dev)
                    };
                    self.save();
                    self.online.remove(&id);
                    self.last_online_check = None;
                    self.login(id, ctx);
                    self.toast(if form.editing.is_some() { "Changes saved" } else { "Device added — connecting..." });
                    if cont {
                        let mut f = DeviceForm::default();
                        f.user = form.user.clone();
                        f.rtsp_port = form.rtsp_port.clone();
                        f.http_port = form.http_port.clone();
                        *form = f;
                    } else {
                        keep = false;
                    }
                }
            }
        }
        keep
    }

    fn search_contents(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let pal = self.pal;
        let mut add: Option<String> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Scan network").size(21.0).strong().color(pal.text_strong));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::icon_button(ui, &pal, Icon::Close, "Close", false).clicked() {
                    self.search.open = false;
                }
            });
        });
        ui.label(RichText::new("Looks on this network for anything that offers camera video.").color(pal.text_dim));
        ui.add_space(6.0);
        let before = self.search.via.clone();
        ui.horizontal(|ui| {
            ui.label("Search on");
            net_ui::network_picker(ui, "scannet", &mut self.search.via, "Main connection");
        });
        if self.search.via != before {
            self.start_search(ctx);
        }
        ui.add_space(10.0);
        if self.search.running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Searching...  {} found so far", self.search.found.len()));
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(format!("Found {}.", self.search.found.len()));
                if kit::secondary(ui, "Scan again").clicked() {
                    self.start_search(ctx);
                }
            });
        }
        ui.add_space(8.0);
        for f in self.search.found.clone() {
            let known = self.store.devices.iter().any(|d| d.host == f.ip);
            kit::card(ui, &pal, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 17.0, pal.accent_soft);
                    icons::paint(ui.painter(), r.shrink(9.0), Icon::Camera, pal.accent);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&f.ip).strong().color(pal.text_strong));
                        ui.label(RichText::new(if f.web { "Video and web page" } else { "Video" }).size(12.0).color(pal.text_dim));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if known {
                            kit::status_pill(ui, "Added", pal.text_dim);
                        } else if kit::primary(ui, &pal, "Add").clicked() {
                            add = Some(f.ip.clone());
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        if let Some(ip) = add {
            let mut f = DeviceForm::default();
            f.name = format!("Device {}", ip.rsplit('.').next().unwrap_or(&ip));
            f.host = ip;
            f.network = self.search.via.clone();
            self.form = Some(f);
            self.search.open = false;
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let Some(ids) = self.confirm_delete.clone() else { return };
        let mut close = false;
        let names: Vec<String> = ids.iter().filter_map(|id| self.store.device(*id).map(|d| d.name.clone())).collect();
        egui::Window::new("Delete device?").collapsible(false).resizable(false).title_bar(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.set_width(340.0f32.min(ctx.screen_rect().width() - 60.0));
            ui.label(RichText::new("Delete device?").size(19.0).strong().color(pal.text_strong));
            ui.label(RichText::new(format!("{} will be removed from MyCCTV. Recordings on the device itself are not touched.", names.join(", "))).color(pal.text));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if kit::danger(ui, &pal, "Delete").clicked() {
                    self.store.devices.retain(|d| !ids.contains(&d.id));
                    for w in self.live.iter_mut().chain(self.pb.iter_mut()) {
                        if w.source.map(|s| ids.contains(&s.device)).unwrap_or(false) {
                            w.clear();
                        }
                    }
                    self.pb_checked.retain(|s| !ids.contains(&s.device));
                    for id in &ids {
                        self.selected_devices.remove(id);
                        self.restart_alarm_listener(*id);
                    }
                    self.save();
                    close = true;
                }
                if kit::secondary(ui, "Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    close = true;
                }
            });
        });
        if close {
            self.confirm_delete = None;
        }
    }

    // ------------------------------------------------------------ events

    fn events_page(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        self.unread = 0;
        let mut go: Option<Source> = None;
        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::same(28.0))).show(ctx, |ui| {
            let listening = self.store.settings.listen_for_alarms;
            kit::page_header(ui, &pal, "Events", &format!("{} event(s)  ·  {}", self.events.len(), if listening { "listening for alarms" } else { "alarms are off" }), |ui| {
                if kit::secondary(ui, "Clear list").clicked() {
                    self.events.clear();
                }
                if kit::chip(ui, &pal, if listening { "Alarms on" } else { "Alarms off" }, listening).on_hover_text("Receive motion and alarm messages from your devices").clicked() {
                    self.store.settings.listen_for_alarms = !listening;
                    self.save();
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (i, g) in EVENT_GROUPS.iter().enumerate() {
                    let n = if i == 0 { self.events.len() } else { self.events.iter().filter(|e| event_group(&e.code) == i).count() };
                    if kit::chip(ui, &pal, &format!("{g}  {n}"), self.event_kind == i).clicked() {
                        self.event_kind = i;
                    }
                }
            });
            ui.add_space(12.0);
            if self.events.is_empty() {
                kit::card(ui, &pal, |ui| {
                    ui.set_width(ui.available_width());
                    kit::empty_state(ui, &pal, Icon::Bell, "All quiet", "Motion, alarm inputs, video loss and smart detections appear here while MyCCTV is open.");
                });
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let kind = self.event_kind;
                for e in self.events.iter().filter(|e| kind == 0 || event_group(&e.code) == kind).take(400) {
                    let col = event_color(&pal, &e.code);
                    egui::Frame::none().fill(pal.card).rounding(Rounding::same(12.0f32)).inner_margin(egui::Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 18.0, col.gamma_multiply(0.18));
                            icons::paint(ui.painter(), r.shrink(10.0), Icon::Bell, col);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(services::alarm_label(&e.code)).strong().color(pal.text_strong));
                                ui.label(RichText::new(format!("{}{}", e.device, e.channel.map(|c| format!("  ·  Camera {}", c + 1)).unwrap_or_default())).size(12.5).color(pal.text_dim));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if let Some(c) = e.channel {
                                    if kit::secondary(ui, "Watch").clicked() {
                                        go = Some(Source { device: e.device_id, channel: c });
                                    }
                                }
                                kit::status_pill(ui, if e.started { "Started" } else { "Ended" }, if e.started { col } else { pal.text_dim });
                                ui.label(RichText::new(&e.time).color(pal.text_dim));
                            });
                        });
                    });
                    ui.add_space(6.0);
                }
            });
        });
        if let Some(s) = go {
            self.page = Page::Live;
            self.open_in_live(s);
        }
    }

    // ------------------------------------------------------------ settings

    fn settings_page(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let mut changed = false;
        let mut restart_tv = false;
        let tv_viewers = self.tv.as_ref().map(|(t, _)| t.viewers.load(Ordering::Relaxed));
        let tv_ip = if self.store.settings.tv_server { crate::relay::local_ip() } else { None };
        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::same(28.0))).show(ctx, |ui| {
            kit::page_header(ui, &pal, "Settings", "Make MyCCTV yours", |_| {});
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let s: &mut Settings = &mut self.store.settings;
                section(ui, &pal, "Appearance", |ui| {
                    ui.horizontal(|ui| {
                        setting_label(ui, "Theme");
                        if kit::chip(ui, &pal, "Midnight", s.theme == ThemeChoice::Dark).clicked() {
                            s.theme = ThemeChoice::Dark;
                            changed = true;
                        }
                        if kit::chip(ui, &pal, "Daylight", s.theme == ThemeChoice::Light).clicked() {
                            s.theme = ThemeChoice::Light;
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        setting_label(ui, "Text and button size");
                        changed |= ui.add(egui::Slider::new(&mut s.ui_scale, 0.8..=2.5).step_by(0.1).suffix("×")).changed();
                    });
                });
                section(ui, &pal, "Live video", |ui| {
                    changed |= ui.checkbox(&mut s.auto_stream_type, "Pick the sharp or the light video stream automatically").changed();
                    ui.horizontal(|ui| {
                        ui.label("Use the sharp (main) stream when showing");
                        changed |= ui.add(egui::DragValue::new(&mut s.main_stream_max_windows).range(1..=64)).changed();
                        ui.label("windows or fewer");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Tour: change cameras every");
                        changed |= ui.add(egui::DragValue::new(&mut s.tour_interval_secs).range(3..=3600).suffix(" s")).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Instant replay goes back");
                        changed |= ui.add(egui::DragValue::new(&mut s.instant_replay_secs).range(5..=600).suffix(" s")).changed();
                    });
                });
                section(ui, &pal, "Devices and alarms", |ui| {
                    changed |= ui.checkbox(&mut s.listen_for_alarms, "Receive alarms from devices").changed();
                    ui.horizontal(|ui| {
                        ui.label("Check which devices are online every");
                        changed |= ui.add(egui::DragValue::new(&mut s.online_check_secs).range(5..=3600).suffix(" s")).changed();
                    });
                });
                section(ui, &pal, "Networks (more than one router)", |ui| net_ui::networks_section(ui, &pal, &mut self.net));
                let s: &mut Settings = &mut self.store.settings;
                section(ui, &pal, "TV and remote control", |ui| {
                    changed |= ui
                        .checkbox(&mut s.remote_navigation, "Arrow keys move between buttons (TV remote)")
                        .on_hover_text("Arrows move the highlight, OK presses. One press on a camera opens it.")
                        .changed();
                });
                section(ui, &pal, "TV sharing (Hisense VIDAA and other smart TVs)", |ui| {
                    ui.label(RichText::new("For TVs that can't install this app (like Hisense TVs with VIDAA OS): MyCCTV shares your cameras with the TV's browser over your home network. Keep MyCCTV running on this computer or box while watching.").color(pal.text_dim));
                    changed |= ui.checkbox(&mut s.tv_server, "Share cameras with TVs on this network").changed();
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        changed |= ui.add(egui::DragValue::new(&mut s.tv_port).range(1024..=65535)).changed();
                        ui.label("Access code");
                        ui.label(RichText::new(if s.tv_key.is_empty() { "(made when sharing starts)" } else { s.tv_key.as_str() }).strong().monospace().color(pal.text_strong));
                        if kit::secondary(ui, "New code").on_hover_text("TVs using the old code will need the new one").clicked() {
                            s.tv_key = crate::model::new_tv_code();
                            changed = true;
                            restart_tv = true;
                        }
                    });
                    if let (true, Some(viewers)) = (s.tv_server, tv_viewers) {
                        let addr = format!("http://{}:{}", tv_ip.as_deref().unwrap_or("this-computer"), s.tv_port);
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("On the TV, open the Browser and go to:  {addr}")).strong().color(pal.live));
                        ui.label(RichText::new(format!("Or with the code included:  {addr}/?key={}", s.tv_key)).monospace().color(pal.text));
                        ui.label(RichText::new(format!("TVs watching now: {viewers} video stream(s)")).color(pal.text_dim));
                    }
                });
                section(ui, &pal, "Pictures and recordings", |ui| {
                    ui.label("Save to folder (leave empty for the default):");
                    changed |= ui.add(egui::TextEdit::singleline(&mut s.snapshot_dir).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).changed();
                    ui.label(RichText::new(format!("Now saving to {}", crate::model::media_dir(s).display())).size(12.0).color(pal.text_dim));
                    ui.label(RichText::new(format!("Device list kept in {}", crate::model::data_dir().display())).size(12.0).color(pal.text_dim));
                });
                section(ui, &pal, "App lock", |ui| {
                    ui.label(if s.app_password.is_empty() { "Ask for a password when MyCCTV opens:" } else { "A password is set. Type a new one to change it, or remove it." });
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.new_password).password(true).char_limit(crate::model::MAX_PASS_LEN).desired_width(220.0).margin(vec2(10.0, 8.0)));
                        if kit::secondary(ui, "Set password").clicked() && !self.new_password.is_empty() {
                            s.app_password = self.new_password.clone();
                            self.new_password.clear();
                            changed = true;
                        }
                        if !s.app_password.is_empty() && kit::secondary(ui, "Remove").clicked() {
                            s.app_password.clear();
                            changed = true;
                        }
                    });
                });
                section(ui, &pal, "About", |ui| {
                    ui.label(format!("MyCCTV {} — live view, playback, camera control and alarms for recorders and IP cameras with standard RTSP video (H.264). Runs on Windows, macOS, Android phones and tablets, and Google TV.", env!("CARGO_PKG_VERSION")));
                });
            });
        });
        if restart_tv {
            self.tv = None;
        }
        if changed {
            self.save();
        }
    }

    fn lock_screen(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg)).show(ctx, |ui| {
            let r = Rect::from_center_size(ui.max_rect().center(), vec2(360.0, 330.0));
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r), |ui| {
                kit::card(ui, &pal, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.add_space(8.0);
                        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Logo, Color32::WHITE);
                        ui.add_space(6.0);
                        ui.label(RichText::new("MyCCTV").size(22.0).strong().color(pal.text_strong));
                        ui.label(RichText::new("Enter your password to continue").color(pal.text_dim));
                        ui.add_space(12.0);
                        let r = ui.add(egui::TextEdit::singleline(&mut self.unlock_input).password(true).hint_text("Password").desired_width(260.0).margin(vec2(10.0, 9.0)));
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if self.unlock_error {
                            ui.label(RichText::new("That password is not right").color(pal.danger));
                        }
                        ui.add_space(8.0);
                        if ui.add_sized(vec2(260.0, 40.0), egui::Button::new(RichText::new("Unlock").strong().color(Color32::WHITE)).fill(pal.accent).rounding(Rounding::same(12.0f32))).clicked() || enter {
                            if self.unlock_input == self.store.settings.app_password {
                                self.locked = false;
                                self.unlock_input.clear();
                            } else {
                                self.unlock_error = true;
                            }
                        }
                        ui.add_space(8.0);
                    });
                });
            });
        });
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = self.shot.as_mut() else { return };
        shot.frame += 1;
        ctx.request_repaint();
        let events = ctx.input(|i| i.raw.events.clone());
        for e in events {
            if let egui::Event::Screenshot { image, .. } = e {
                let [w, h] = image.size;
                let mut rgba = Vec::with_capacity(w * h * 4);
                for p in &image.pixels {
                    rgba.extend_from_slice(&[p.r(), p.g(), p.b(), 255]);
                }
                if let Some(dir) = shot.path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = image::save_buffer(&shot.path, &rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8) {
                    eprintln!("could not save screenshot: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        let wait_frames: u32 = std::env::var("MYCCTV_SHOT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
        let wait_secs: u64 = std::env::var("MYCCTV_SHOT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        // Only once the window really has a size (a minimised or not-yet-shown window has none).
        let has_size = ctx.screen_rect().width() >= 64.0 && ctx.screen_rect().height() >= 64.0;
        if has_size && shot.frame >= wait_frames && shot.started.elapsed() >= Duration::from_secs(wait_secs) && !shot.requested {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        if shot.frame > wait_frames + 600 && shot.started.elapsed() > Duration::from_secs(wait_secs + 20) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn section(ui: &mut Ui, pal: &Palette, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::none().fill(pal.card).stroke(Stroke::new(1.0f32, pal.border)).rounding(Rounding::same(14.0f32)).inner_margin(egui::Margin::same(18.0)).show(ui, |ui| {
        ui.set_width(ui.available_width().min(820.0));
        ui.label(RichText::new(title).size(16.0).strong().color(pal.text_strong));
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(12.0);
}

/// Draws one video tile and returns what the person asked it to do.
#[allow(clippy::too_many_arguments)]
fn video_window(ui: &mut Ui, rect: Rect, w: &PlayWindow, index: usize, title: &str, selected: bool, pal: &Palette, playback: bool) -> Vec<WinAction> {
    let mut acts = Vec::new();
    let id = Id::new(("videowin", playback, index));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let round = Rounding::same(if rect.width() > 160.0 { 10.0f32 } else { 4.0f32 });
    let p = ui.painter_at(rect.expand(3.0));
    p.rect_filled(rect, round, pal.video_bg);

    let uv = w.uv_rect();
    let mut shown: Option<Rect> = None;
    if let (Some(tex), Some(h)) = (&w.tex, &w.handle) {
        if matches!(h.shared.state(), State::Playing | State::Connecting) {
            let ts = tex.size_vec2();
            let target = match w.scale {
                Scale::Full => rect,
                Scale::Original => fit(rect, ts.x / ts.y.max(1.0)),
                Scale::Wide => fit(rect, 16.0 / 9.0),
                Scale::Tall => fit(rect, 9.0 / 16.0),
            };
            let mut img = egui::epaint::RectShape::filled(target, round, Color32::WHITE);
            img.fill_texture_id = tex.id();
            img.uv = uv;
            p.add(egui::Shape::Rect(img));
            shown = Some(target);
        }
    }

    // Digital zoom: mouse wheel or pinch zooms at the pointer, dragging moves a zoomed picture.
    if let Some(target) = shown {
        let to_uv = |pos: egui::Pos2| -> [f32; 2] {
            let t = (pos - target.min) / target.size();
            [uv.min.x + t.x.clamp(0.0, 1.0) * uv.width(), uv.min.y + t.y.clamp(0.0, 1.0) * uv.height()]
        };
        if ui.rect_contains_pointer(rect) {
            let (wheel, pinch, pointer) = ui.input(|i| (i.raw_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
            let mut f = pinch;
            if wheel != 0.0 {
                f *= (1.0 + wheel * 0.002).clamp(0.6, 1.6);
            }
            if (f - 1.0).abs() > 0.001 {
                if let Some(pt) = pointer {
                    acts.push(WinAction::Zoom(f, to_uv(pt)));
                }
            }
        }
        if w.zoom > 1.001 && resp.dragged() {
            let d = resp.drag_delta();
            acts.push(WinAction::Pan([-d.x / target.width() * uv.width(), -d.y / target.height() * uv.height()]));
        }
        if w.zoom > 1.01 && rect.width() > 120.0 {
            // Zoom level and a small map of where the view is.
            let label = format!("{:.1}×", w.zoom);
            let pill = Rect::from_min_size(pos2(rect.left() + 8.0, rect.bottom() - 30.0), vec2(48.0, 22.0));
            p.rect_filled(pill, Rounding::same(11.0f32), Color32::from_black_alpha(150));
            p.text(pill.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), Color32::WHITE);
            if rect.width() > 260.0 {
                let map = Rect::from_min_size(pos2(pill.right() + 8.0, rect.bottom() - 38.0), vec2(48.0, 30.0));
                p.rect_filled(map, Rounding::same(3.0f32), Color32::from_black_alpha(150));
                let v = Rect::from_min_max(map.min + vec2(uv.min.x * map.width(), uv.min.y * map.height()), map.min + vec2(uv.max.x * map.width(), uv.max.y * map.height()));
                p.rect_stroke(v, Rounding::ZERO, Stroke::new(1.5f32, pal.accent));
            }
        }
    }

    let small = rect.height() < 110.0 || rect.width() < 170.0;
    let status: Option<(String, Color32)> = match (&w.source, &w.handle) {
        (None, _) => None,
        (Some(_), None) => Some(("Not connected".into(), pal.text_dim)),
        (Some(_), Some(h)) => match h.shared.state() {
            State::Connecting if w.tex.is_none() => Some(("Connecting…".into(), pal.text)),
            State::Error(e) => Some((e, pal.danger)),
            State::Unsupported(e) => Some((e, pal.warn)),
            State::Ended => Some(("End of recording".into(), pal.text)),
            State::Paused => Some(("Paused".into(), pal.text_dim)),
            _ => None,
        },
    };
    if let Some((msg, c)) = status {
        if !small {
            if msg.starts_with("Connecting") {
                let t = ui.input(|i| i.time) as f32;
                for k in 0..3 {
                    let a = ((t * 3.0 - k as f32 * 0.6).sin() * 0.5 + 0.5).clamp(0.2, 1.0);
                    p.circle_filled(rect.center() + vec2((k as f32 - 1.0) * 14.0, -14.0), 4.0, pal.accent.gamma_multiply(a));
                }
                ui.ctx().request_repaint_after(Duration::from_millis(60));
            }
            let galley = p.layout(msg, FontId::proportional(13.0), c, (rect.width() - 40.0).max(40.0));
            let pos = rect.center() + vec2(-galley.size().x / 2.0, 2.0);
            p.galley(pos, galley, c);
        } else {
            p.circle_filled(rect.center(), 4.0, c);
        }
    }
    if w.source.is_none() && !small {
        let c = rect.center();
        p.circle_stroke(c - vec2(0.0, 10.0), 18.0, Stroke::new(1.5f32, pal.border));
        p.line_segment([c - vec2(0.0, 18.0), c - vec2(0.0, 2.0)], Stroke::new(1.5f32, pal.text_dim));
        p.line_segment([c - vec2(8.0, 10.0), c + vec2(8.0, -10.0)], Stroke::new(1.5f32, pal.text_dim));
        let hint = if playback { "Click the timeline to play" } else { "Choose a camera" };
        p.text(c + vec2(0.0, 22.0), Align2::CENTER_CENTER, hint, FontId::proportional(12.0), pal.text_dim);
    }

    // Badge + title (top-left)
    if w.source.is_some() && !small {
        let (badge, bc) = match &w.mode {
            WinMode::Live => ("LIVE", pal.live),
            WinMode::Replay { .. } => ("REPLAY", pal.warn),
            WinMode::Playback { .. } => ("PLAYBACK", pal.accent),
        };
        let mut t = title.to_string();
        if let (WinMode::Live, Some(s)) = (&w.mode, w.stream_override) {
            t.push_str(if s == 0 { " · sharp" } else { " · light" });
        }
        let max_chars = ((rect.width() - 110.0) / 7.0).max(6.0) as usize;
        let text = truncate(&t, max_chars);
        let galley = p.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
        let bw = badge.len() as f32 * 7.0 + 16.0;
        let pill = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(bw + galley.size().x + 16.0, 24.0));
        p.rect_filled(pill, Rounding::same(12.0f32), Color32::from_black_alpha(150));
        let br = Rect::from_min_size(pill.min + vec2(3.0, 3.0), vec2(bw, 18.0));
        p.rect_filled(br, Rounding::same(9.0f32), bc);
        p.text(br.center(), Align2::CENTER_CENTER, badge, FontId::proportional(10.0), Color32::WHITE);
        p.galley(pos2(br.right() + 8.0, pill.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    } else if small && w.source.is_some() {
        p.text(rect.left_top() + vec2(4.0, 3.0), Align2::LEFT_TOP, (index + 1).to_string(), FontId::proportional(9.0), Color32::from_gray(180));
    }
    if w.recording() {
        let c = pos2(rect.right() - 14.0, rect.bottom() - 14.0);
        p.circle_filled(c, 6.0, pal.danger);
        if !small {
            p.text(c - vec2(10.0, 0.0), Align2::RIGHT_CENTER, "REC", FontId::proportional(11.0), pal.danger);
        }
    }

    // Hover tools (top-right)
    let hovered = ui.rect_contains_pointer(rect);
    if (hovered || selected) && w.source.is_some() && !small {
        let tools: &[(Icon, WinAction, &str)] = if playback {
            &[(Icon::Snapshot, WinAction::Capture, "Take a picture"), (Icon::Record, WinAction::Record, "Save a clip"), (Icon::Close, WinAction::Close, "Close")]
        } else {
            &[
                (Icon::Replay, WinAction::Replay, "Instant replay"),
                (Icon::Snapshot, WinAction::Capture, "Take a picture"),
                (Icon::Record, WinAction::Record, "Record"),
                (Icon::Close, WinAction::Close, "Close"),
            ]
        };
        let mut x = rect.right() - 8.0;
        for (icon, act, tip) in tools.iter().rev() {
            let c = pos2(x - 14.0, rect.top() + 20.0);
            x -= 32.0;
            let r = Rect::from_center_size(c, vec2(28.0, 28.0));
            let br = ui.interact(r, id.with(tip), Sense::click());
            let active = (*act == WinAction::Record && w.recording()) || (*act == WinAction::Replay && matches!(w.mode, WinMode::Replay { .. }));
            ui.painter().circle_filled(c, 14.0, if br.hovered() { pal.accent } else if active { pal.danger } else { Color32::from_black_alpha(150) });
            icons::paint(ui.painter(), r.shrink(7.0), *icon, Color32::WHITE);
            if br.on_hover_text(*tip).clicked() {
                acts.push(*act);
            }
        }
        // Zoom buttons (right edge, middle): +, −, and back to the whole picture.
        if shown.is_some() && rect.height() > 150.0 {
            let mut y = rect.center().y - if w.zoom > 1.01 { 48.0 } else { 32.0 };
            let mut zoom_buttons: Vec<(&str, WinAction, &str)> = vec![("+", WinAction::Zoom(1.5, w.center), "Zoom in (or use the mouse wheel / pinch)"), ("−", WinAction::Zoom(1.0 / 1.5, w.center), "Zoom out")];
            if w.zoom > 1.01 {
                zoom_buttons.push(("1×", WinAction::ZoomReset, "Show the whole picture"));
            }
            for (label, act, tip) in zoom_buttons {
                let c = pos2(rect.right() - 22.0, y + 14.0);
                y += 32.0;
                let r = Rect::from_center_size(c, vec2(28.0, 28.0));
                let br = ui.interact(r, id.with(label), Sense::click());
                ui.painter().circle_filled(c, 14.0, if br.hovered() { pal.accent } else { Color32::from_black_alpha(150) });
                ui.painter().text(c, Align2::CENTER_CENTER, label, FontId::proportional(if label == "1×" { 11.0 } else { 17.0 }), Color32::WHITE);
                if br.on_hover_text(tip).clicked() {
                    acts.push(act);
                }
            }
        }
    }

    let (bw, bc) = if resp.has_focus() {
        (2.5f32, pal.warn)
    } else if selected {
        (2.0f32, pal.accent)
    } else {
        (1.0f32, if pal.dark { Color32::from_rgb(0x1A, 0x22, 0x2E) } else { pal.border })
    };
    ui.painter().rect_stroke(rect, round, Stroke::new(bw, bc));

    if resp.clicked() {
        acts.push(WinAction::Select);
        if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            acts.push(WinAction::Maximize);
        }
    }
    if resp.double_clicked() {
        acts.push(WinAction::Maximize);
    }
    resp.context_menu(|ui| {
        ui.set_min_width(210.0);
        let has = w.source.is_some();
        if ui.add_enabled(has, egui::Button::new("Close video")).clicked() {
            acts.push(WinAction::Close);
            ui.close_menu();
        }
        if ui.button("Close all video").clicked() {
            acts.push(WinAction::CloseAll);
            ui.close_menu();
        }
        ui.separator();
        if ui.add_enabled(has, egui::Button::new(if w.recording() { "Stop recording" } else { "Start recording" })).clicked() {
            acts.push(WinAction::Record);
            ui.close_menu();
        }
        if ui.add_enabled(has, egui::Button::new("Take a picture")).clicked() {
            acts.push(WinAction::Capture);
            ui.close_menu();
        }
        if ui.add_enabled(has, egui::Button::new("Take 3 pictures (1 per second)")).clicked() {
            acts.push(WinAction::Burst);
            ui.close_menu();
        }
        if !playback {
            let replaying = matches!(w.mode, WinMode::Replay { .. });
            if ui.add_enabled(has, egui::Button::new(if replaying { "Back to live" } else { "Instant replay" })).clicked() {
                acts.push(WinAction::Replay);
                ui.close_menu();
            }
            if ui.add_enabled(has, egui::Button::new("Open in Playback")).clicked() {
                acts.push(WinAction::GoPlayback);
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.add_enabled(has && w.zoom < MAX_ZOOM, egui::Button::new("Zoom in")).clicked() {
            acts.push(WinAction::Zoom(1.5, w.center));
            ui.close_menu();
        }
        if ui.add_enabled(has && w.zoom > 1.0, egui::Button::new("Zoom out")).clicked() {
            acts.push(WinAction::Zoom(1.0 / 1.5, w.center));
            ui.close_menu();
        }
        if ui.add_enabled(has && w.zoom > 1.0, egui::Button::new("Show whole picture")).clicked() {
            acts.push(WinAction::ZoomReset);
            ui.close_menu();
        }
        ui.menu_button("Picture shape", |ui| {
            for s in Scale::ALL {
                let label = match s {
                    Scale::Original => "Keep camera shape",
                    Scale::Full => "Fill the window",
                    Scale::Wide => "16:9",
                    Scale::Tall => "9:16",
                };
                if ui.radio(w.scale == s, label).clicked() {
                    acts.push(WinAction::Scale(s));
                    ui.close_menu();
                }
            }
        });
        if !playback {
            ui.menu_button("Video quality", |ui| {
                for (v, l) in [(None, "Automatic"), (Some(0u8), "Sharp (main stream)"), (Some(1), "Light (sub stream 1)"), (Some(2), "Light (sub stream 2)")] {
                    if ui.radio(w.stream_override == v, l).clicked() {
                        acts.push(WinAction::Stream(v));
                        ui.close_menu();
                    }
                }
            });
        }
        ui.separator();
        if ui.button("Full screen on / off").clicked() {
            acts.push(WinAction::Fullscreen);
            ui.close_menu();
        }
    });
    acts
}

fn fit(rect: Rect, aspect: f32) -> Rect {
    let ra = rect.width() / rect.height().max(1.0);
    let size = if ra > aspect { vec2(rect.height() * aspect, rect.height()) } else { vec2(rect.width(), rect.width() / aspect) };
    Rect::from_center_size(rect.center(), size)
}

impl eframe::App for KvmsApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if !self.store.settings.remote_navigation || ctx.wants_keyboard_input() {
            return;
        }
        // TV remote: arrows move between controls like Tab / Shift+Tab.
        for e in raw.events.iter_mut() {
            if let egui::Event::Key { key, pressed, modifiers, physical_key, repeat } = e {
                let (new_key, shift) = match key {
                    Key::ArrowDown | Key::ArrowRight => (Key::Tab, false),
                    Key::ArrowUp | Key::ArrowLeft => (Key::Tab, true),
                    _ => continue,
                };
                *e = egui::Event::Key { key: new_key, physical_key: *physical_key, pressed: *pressed, repeat: *repeat, modifiers: egui::Modifiers { shift, ..*modifiers } };
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_no += 1;
        let s = &self.store.settings;
        if self.applied != (s.theme, s.ui_scale) {
            theme::apply(ctx, s.theme, s.ui_scale);
            self.applied = (s.theme, s.ui_scale);
            self.pal = theme::palette(s.theme);
        }
        self.handle_screenshot(ctx);
        platform::keyboard_wanted(ctx.wants_keyboard_input());

        // Tells the video threads the screen is being drawn; when it stops (minimized, Android in the
        // background) they stop by themselves within a few seconds.
        stream::ui_alive();
        if self.live.iter().chain(self.pb.iter()).any(|w| w.handle.is_some()) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if self.locked {
            // Nothing plays behind the lock screen (recordings keep going).
            for w in self.live.iter_mut().chain(self.pb.iter_mut()) {
                if !w.recording() {
                    w.pause();
                }
            }
            self.lock_screen(ctx);
            return;
        }
        self.poll_background(ctx);
        crate::netpath::sync(&self.store.devices);

        let (esc, f11) = ctx.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::F11)));
        if esc && self.form.is_none() && self.confirm_delete.is_none() && !self.search.open {
            if self.maximized.is_some() {
                self.maximized = None;
            } else if self.fullscreen {
                self.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        }
        if f11 {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if (self.form.is_some() || self.search.open) && self.page != Page::Devices {
            self.page = Page::Devices;
        }

        self.remember_view();
        self.reconcile(ctx);
        self.pump_windows(ctx);

        // + / − / 0 keys (keyboard or TV remote) zoom the selected video window.
        if !ctx.wants_keyboard_input() && matches!(self.page, Page::Live | Page::Playback) {
            let (zin, zout, zreset) = ctx.input(|i| {
                let plain = !i.modifiers.ctrl && !i.modifiers.command;
                (
                    plain && (i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)),
                    plain && i.key_pressed(Key::Minus),
                    plain && i.key_pressed(Key::Num0),
                )
            });
            let playback = self.page == Page::Playback;
            let i = if playback { self.pb_selected } else { self.selected };
            let center = if playback { self.pb.get(i).map(|w| w.center) } else { self.live.get(i).map(|w| w.center) };
            if let Some(c) = center {
                if zin {
                    self.apply_window_action(playback, i, WinAction::Zoom(1.5, c), ctx);
                }
                if zout {
                    self.apply_window_action(playback, i, WinAction::Zoom(1.0 / 1.5, c), ctx);
                }
                if zreset {
                    self.apply_window_action(playback, i, WinAction::ZoomReset, ctx);
                }
            }
        }
        // Phones (and narrow windows) get the compact layout.
        self.compact = ctx.screen_rect().width() < 720.0;
        if !self.fullscreen {
            if self.compact {
                self.bottom_nav(ctx);
            } else if self.store.settings.nav_collapsed {
                self.nav_handle(ctx);
            } else {
                self.nav_rail(ctx);
            }
        }
        // Remember whether the Live View camera list is folded away.
        if !self.compact && self.store.settings.live_list_hidden == self.show_drawer {
            self.store.settings.live_list_hidden = !self.show_drawer;
            self.save();
        }
        match self.page {
            Page::Home => self.home(ctx),
            Page::Live => self.live_view(ctx),
            Page::Playback => self.playback(ctx),
            Page::Devices => self.devices(ctx),
            Page::Events => self.events_page(ctx),
            Page::Settings => self.settings_page(ctx),
        }
        self.toasts(ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for (_, s) in self.alarm_stops.drain() {
            s.store(true, Ordering::Relaxed);
        }
        for w in self.live.iter_mut().chain(self.pb.iter_mut()) {
            w.handle = None;
        }
        let _ = self.store.save();
    }
}

fn setting_label(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(vec2(170.0, 30.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(170.0);
        ui.label(text);
    });
}
