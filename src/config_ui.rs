//! Device settings screen: friendly sections for the common settings of a recorder or
//! camera, and an "All settings" editor for everything else the device offers.

use super::{KvmsApp, Page};
use crate::devconfig::{self, Snapshot};
use crate::http::Target;
use crate::icons::{self, Icon};
use crate::kit;
use crate::theme::Palette;
use egui::{vec2, Align, Color32, Layout, RichText, Rounding, Sense, Stroke, Ui};
use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Overview,
    Video,
    Picture,
    Overlay,
    Motion,
    Recording,
    Time,
    Network,
    Services,
    All,
}

impl Section {
    const LIST: [Section; 10] = [
        Section::Overview,
        Section::Video,
        Section::Picture,
        Section::Overlay,
        Section::Motion,
        Section::Recording,
        Section::Time,
        Section::Network,
        Section::Services,
        Section::All,
    ];
    fn label(self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Video => "Video quality",
            Section::Picture => "Picture",
            Section::Overlay => "On-screen text",
            Section::Motion => "Detection & alarms",
            Section::Recording => "Recording",
            Section::Time => "Date & time",
            Section::Network => "Network",
            Section::Services => "Ports & services",
            Section::All => "All settings",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Section::Overview => Icon::Server,
            Section::Video => Icon::Camera,
            Section::Picture => Icon::Sun,
            Section::Overlay => Icon::Edit,
            Section::Motion => Icon::Bell,
            Section::Recording => Icon::Record,
            Section::Time => Icon::Refresh,
            Section::Network => Icon::Grid,
            Section::Services => Icon::Joystick,
            Section::All => Icon::Gear,
        }
    }
    /// Changes here can cut the connection, so they are confirmed first.
    fn risky(self) -> bool {
        matches!(self, Section::Network | Section::Services | Section::All)
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Slider(i64, i64),
    Number(i64, i64),
    Text,
    Choice(&'static [(&'static str, &'static str)]),
    /// Choices listed by the device's capabilities (comma separated), with a fallback list.
    CapsChoice(&'static str, &'static [&'static str]),
    /// Number whose limits come from capabilities: "max key" or "min,max key".
    CapsNumber(&'static str, i64, i64),
}

struct Field {
    label: &'static str,
    key: &'static str,
    kind: Kind,
}

const fn f(label: &'static str, key: &'static str, kind: Kind) -> Field {
    Field { label, key, kind }
}

const STREAMS: [(&str, &str); 3] = [("Main stream (sharp)", "MainFormat[0]"), ("Sub stream 1 (light)", "ExtraFormat[0]"), ("Sub stream 2", "ExtraFormat[1]")];

const VIDEO: &[Field] = &[
    f("Video on", "Encode[{c}].{s}.VideoEnable", Kind::Bool),
    f("Codec", "Encode[{c}].{s}.Video.Compression", Kind::CapsChoice("Video.CompressionTypes", &["H.264", "H.265", "MJPG"])),
    f("Resolution", "Encode[{c}].{s}.Video.resolution", Kind::CapsChoice("Video.ResolutionTypes", &[])),
    f("Frame rate (per second)", "Encode[{c}].{s}.Video.FPS", Kind::CapsNumber("Video.FPSMax", 1, 30)),
    f("Bit rate type", "Encode[{c}].{s}.Video.BitRateControl", Kind::Choice(&[("CBR", "Constant"), ("VBR", "Variable")])),
    f("Bit rate (Kb/s)", "Encode[{c}].{s}.Video.BitRate", Kind::CapsNumber("Video.BitRateOptions", 32, 16384)),
    f("Quality (VBR)", "Encode[{c}].{s}.Video.Quality", Kind::Slider(1, 6)),
    f("Key frame interval", "Encode[{c}].{s}.Video.GOP", Kind::Number(1, 250)),
    f("Profile", "Encode[{c}].{s}.Video.Profile", Kind::Choice(&[("Baseline", "Baseline"), ("Main", "Main"), ("High", "High")])),
    f("Sound with video", "Encode[{c}].{s}.AudioEnable", Kind::Bool),
];

const PICTURE: &[Field] = &[
    f("Brightness", "VideoColor[{c}][0].Brightness", Kind::Slider(0, 100)),
    f("Contrast", "VideoColor[{c}][0].Contrast", Kind::Slider(0, 100)),
    f("Saturation", "VideoColor[{c}][0].Saturation", Kind::Slider(0, 100)),
    f("Hue", "VideoColor[{c}][0].Hue", Kind::Slider(0, 100)),
    f("Gamma", "VideoColor[{c}][0].Gamma", Kind::Slider(0, 100)),
    f("Sharpness", "VideoInOptions[{c}].Sharpness", Kind::Slider(0, 100)),
    f("Mirror", "VideoInOptions[{c}].Mirror", Kind::Bool),
    f("Flip", "VideoInOptions[{c}].Flip", Kind::Bool),
    f("Day / night", "VideoInOptions[{c}].DayNightColor", Kind::Choice(&[("0", "Always colour"), ("1", "Automatic"), ("2", "Always black & white")])),
    f("Backlight", "VideoInOptions[{c}].Backlight", Kind::Choice(&[("0", "Off"), ("1", "Backlight compensation"), ("2", "Wide dynamic range"), ("3", "Highlight suppression")])),
    f("Infrared light", "Lighting[{c}][0].Mode", Kind::Choice(&[("Auto", "Automatic"), ("Manual", "Manual"), ("Off", "Off"), ("ZoomPrio", "Follow zoom")])),
];

const OVERLAY: &[Field] = &[
    f("Camera name", "ChannelTitle[{c}].Name", Kind::Text),
    f("Show camera name on video", "VideoWidget[{c}].ChannelTitle.EncodeBlend", Kind::Bool),
    f("Show date and time on video", "VideoWidget[{c}].TimeTitle.EncodeBlend", Kind::Bool),
    f("Show day of the week", "VideoWidget[{c}].TimeTitle.ShowWeek", Kind::Bool),
];

const MOTION: &[Field] = &[
    f("Motion detection", "MotionDetect[{c}].Enable", Kind::Bool),
    f("Sensitivity", "MotionDetect[{c}].Level", Kind::Slider(1, 6)),
    f("Sensitivity (area 1)", "MotionDetect[{c}].MotionDetectWindow[0].Sensitive", Kind::Slider(0, 100)),
    f("Threshold (area 1)", "MotionDetect[{c}].MotionDetectWindow[0].Threshold", Kind::Slider(0, 100)),
    f("On motion: record", "MotionDetect[{c}].EventHandler.RecordEnable", Kind::Bool),
    f("On motion: take pictures", "MotionDetect[{c}].EventHandler.SnapshotEnable", Kind::Bool),
    f("On motion: send e-mail", "MotionDetect[{c}].EventHandler.MailEnable", Kind::Bool),
    f("On motion: alarm output", "MotionDetect[{c}].EventHandler.AlarmOutEnable", Kind::Bool),
    f("On motion: buzzer", "MotionDetect[{c}].EventHandler.BeepEnable", Kind::Bool),
    f("Keep recording after (s)", "MotionDetect[{c}].EventHandler.RecordLatch", Kind::Number(0, 300)),
    f("Tampering detection", "BlindDetect[{c}].Enable", Kind::Bool),
    f("Video loss alarm", "LossDetect[{c}].Enable", Kind::Bool),
];

const RECORDING: &[Field] = &[
    f("Main stream recording", "RecordMode[{c}].Mode", Kind::Choice(&[("0", "By schedule"), ("1", "Always (manual)"), ("2", "Off")])),
    f("Sub stream recording", "RecordMode[{c}].ModeExtra1", Kind::Choice(&[("0", "By schedule"), ("1", "Always (manual)"), ("2", "Off")])),
    f("Record before an event (s)", "Record[{c}].PreRecord", Kind::Number(0, 30)),
    f("Keep a second copy", "Record[{c}].Redundancy", Kind::Bool),
    f("Picture taking", "Snap[{c}].Enable", Kind::Bool),
];

const TIME: &[Field] = &[
    f("Set time from the internet (NTP)", "NTP.Enable", Kind::Bool),
    f("Time server", "NTP.Address", Kind::Text),
    f("Time server port", "NTP.Port", Kind::Number(1, 65535)),
    f("Update every (minutes)", "NTP.UpdatePeriod", Kind::Number(1, 65535)),
    f("Time zone (device's number)", "NTP.TimeZone", Kind::Number(0, 40)),
    f("Time zone description", "NTP.TimeZoneDesc", Kind::Text),
    f("Daylight saving time", "Locales.DSTEnable", Kind::Bool),
    f("Time format", "Locales.TimeFormat", Kind::Text),
];

const NETWORK: &[Field] = &[
    f("Device host name", "Network.Hostname", Kind::Text),
    f("Get address automatically (DHCP)", "Network.eth0.DhcpEnable", Kind::Bool),
    f("IP address", "Network.eth0.IPAddress", Kind::Text),
    f("Subnet mask", "Network.eth0.SubnetMask", Kind::Text),
    f("Gateway", "Network.eth0.DefaultGateway", Kind::Text),
    f("DNS server 1", "Network.eth0.DnsServers[0]", Kind::Text),
    f("DNS server 2", "Network.eth0.DnsServers[1]", Kind::Text),
    f("Packet size (MTU)", "Network.eth0.MTU", Kind::Number(576, 1500)),
    f("Plug and play (UPnP)", "UPnP.Enable", Kind::Bool),
];

const SERVICES: &[Field] = &[
    f("Video streaming (RTSP)", "RTSP.Enable", Kind::Bool),
    f("RTSP port", "RTSP.Port", Kind::Number(1, 65535)),
    f("Web page port (HTTP)", "Web.Port", Kind::Number(1, 65535)),
    f("Secure web page port (HTTPS)", "Web.SSLPort", Kind::Number(1, 65535)),
    f("Vendor app port (TCP)", "DVRIP.TcpPort", Kind::Number(1, 65535)),
    f("Vendor app port (UDP)", "DVRIP.UDPPort", Kind::Number(1, 65535)),
    f("Max. connections", "DVRIP.MaxConnections", Kind::Number(1, 128)),
    f("Allow/deny list", "AccessFilter.Enable", Kind::Bool),
    f("Multicast", "Multicast.RTP[0].Enable", Kind::Bool),
];

enum Msg {
    Loaded(Result<Snapshot, String>),
    Saved(Result<usize, String>),
    Note(Result<String, String>),
}

pub struct ConfigView {
    pub device: u64,
    /// 0: the device itself; n: the camera behind recorder channel link n-1.
    target: usize,
    section: Section,
    channel: u32,
    snap: Option<Snapshot>,
    edits: BTreeMap<String, String>,
    busy: Option<&'static str>,
    error: Option<String>,
    rx: Option<Receiver<Msg>>,
    all_group: Option<String>,
    all_filter: String,
    confirm_save: bool,
    confirm_reboot: bool,
}

impl ConfigView {
    pub fn new(device: u64) -> Self {
        Self {
            device,
            target: 0,
            section: std::env::var("MYCCTV_CFG_SECTION").ok().and_then(|s| Section::LIST.into_iter().find(|x| format!("{x:?}").eq_ignore_ascii_case(&s))).unwrap_or(Section::Overview),
            channel: 0,
            snap: None,
            edits: BTreeMap::new(),
            busy: None,
            error: None,
            rx: None,
            all_group: None,
            all_filter: String::new(),
            confirm_save: false,
            confirm_reboot: false,
        }
    }
    fn value(&self, key: &str) -> Option<String> {
        self.edits.get(key).cloned().or_else(|| self.snap.as_ref().and_then(|s| s.values.get(key).cloned()))
    }
    fn original(&self, key: &str) -> Option<&String> {
        self.snap.as_ref().and_then(|s| s.values.get(key))
    }
    fn set(&mut self, key: &str, v: String) {
        if self.original(key) == Some(&v) {
            self.edits.remove(key);
        } else {
            self.edits.insert(key.to_string(), v);
        }
    }
}

fn expand(key: &str, c: u32, s: &str) -> String {
    key.replace("{c}", &c.to_string()).replace("{s}", s)
}

impl KvmsApp {
    /// Opens the settings screen for one device.
    pub(super) fn open_config(&mut self, id: u64, ctx: &egui::Context) {
        self.config = Some(ConfigView::new(id));
        self.page = Page::Devices;
        self.config_load(ctx);
    }

    /// Who we talk to: the device, or the camera behind one of a recorder's channels.
    fn config_target(&self) -> Option<(Target, String, u32)> {
        let cv = self.config.as_ref()?;
        let d = self.store.device(cv.device)?;
        if cv.target == 0 {
            return Some((d.http(), d.name.clone(), d.channels));
        }
        let link = d.camera_links.get(cv.target - 1)?.as_ref()?;
        let saved = self.store.devices.iter().find(|x| x.id != d.id && x.wire_host() == link.host);
        let (user, pass, port) = match saved {
            Some(s) => (s.user.clone(), s.password.clone(), s.http_port),
            None => (d.user.clone(), d.password.clone(), 80),
        };
        Some((Target { host: link.host.clone(), port, user, pass }, format!("camera {} (channel {})", link.host, cv.target), 1))
    }

    fn config_load(&mut self, ctx: &egui::Context) {
        let Some((t, _, hint)) = self.config_target() else { return };
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Loaded(devconfig::load(&t, hint)));
            c.request_repaint();
        });
        if let Some(cv) = self.config.as_mut() {
            cv.rx = Some(rx);
            cv.busy = Some("Reading settings from the device…");
            cv.error = None;
            cv.snap = None;
            cv.edits.clear();
        }
    }

    fn config_poll(&mut self, ctx: &egui::Context) {
        let mut toast: Option<(String, bool)> = None;
        let mut reload = false;
        if let Some(cv) = self.config.as_mut() {
            if let Some(rx) = &cv.rx {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Msg::Loaded(Ok(s)) => {
                            cv.channel = cv.channel.min(s.channel_count() - 1);
                            cv.snap = Some(s);
                            cv.busy = None;
                        }
                        Msg::Loaded(Err(e)) => {
                            cv.error = Some(e);
                            cv.busy = None;
                        }
                        Msg::Saved(Ok(n)) => {
                            toast = Some((format!("Saved {n} setting(s) on the device"), false));
                            reload = true;
                        }
                        Msg::Saved(Err(e)) => {
                            cv.busy = None;
                            toast = Some((format!("Save failed: {e}"), true));
                        }
                        Msg::Note(r) => {
                            cv.busy = None;
                            toast = Some(match r {
                                Ok(m) => (m, false),
                                Err(e) => (e, true),
                            });
                        }
                    }
                }
            }
            if cv.busy.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        if let Some((m, err)) = toast {
            if err {
                self.toast_err(m);
            } else {
                self.toast(m);
            }
        }
        if reload {
            self.config_load(ctx);
        }
    }

    fn config_run(&mut self, ctx: &egui::Context, busy: &'static str, job: impl FnOnce(&Target) -> Msg + Send + 'static) {
        let Some((t, _, _)) = self.config_target() else { return };
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job(&t));
            c.request_repaint();
        });
        if let Some(cv) = self.config.as_mut() {
            cv.rx = Some(rx);
            cv.busy = Some(busy);
        }
    }

    /// Draws the settings screen in place of the device list. Returns false when closed.
    pub(super) fn config_page(&mut self, ctx: &egui::Context) {
        self.config_poll(ctx);
        let pal = self.pal;
        let Some((_, target_name, _)) = self.config_target() else {
            self.config = None;
            return;
        };
        let dev = self.config.as_ref().and_then(|cv| self.store.device(cv.device).cloned());
        let Some(dev) = dev else {
            self.config = None;
            return;
        };
        let mut close = false;
        let mut reload = false;
        let mut new_target: Option<usize> = None;

        // Section list on the left (on phones: a drop-down at the top instead).
        let compact = self.compact;
        if !compact {
        egui::SidePanel::left("cfgsections")
            .exact_width(230.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(pal.surface).inner_margin(egui::Margin::same(14.0)).stroke(Stroke::new(1.0f32, pal.border)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if kit::icon_button(ui, &pal, Icon::Close, "Back to devices", false).clicked() {
                        close = true;
                    }
                    ui.label(RichText::new("Device settings").size(17.0).strong().color(pal.text_strong));
                });
                ui.add_space(10.0);
                let cv = self.config.as_mut().unwrap();
                for s in Section::LIST {
                    let sel = cv.section == s;
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                    let hot = resp.hovered() || resp.has_focus();
                    if sel {
                        ui.painter().rect_filled(rect, Rounding::same(10.0f32), pal.accent_soft);
                    } else if hot {
                        ui.painter().rect_filled(rect, Rounding::same(10.0f32), pal.hover);
                    }
                    if resp.has_focus() {
                        ui.painter().rect_stroke(rect, Rounding::same(10.0f32), Stroke::new(2.0f32, pal.accent));
                    }
                    let col = if sel { pal.accent } else { pal.text };
                    icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), vec2(18.0, 18.0)), s.icon(), col);
                    ui.painter().text(egui::pos2(rect.left() + 40.0, rect.center().y), egui::Align2::LEFT_CENTER, s.label(), egui::FontId::proportional(14.0), if sel { pal.text_strong } else { pal.text });
                    if resp.clicked() {
                        cv.section = s;
                    }
                }
            });
        }

        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::same(if compact { 12.0 } else { 24.0 }))).show(ctx, |ui| {
            if compact {
                // Phone header: back, name, then the pickers on wrapping lines.
                ui.horizontal(|ui| {
                    if kit::icon_button(ui, &pal, Icon::Close, "Back to devices", false).clicked() {
                        close = true;
                    }
                    ui.label(RichText::new(&dev.name).size(20.0).strong().color(pal.text_strong));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::icon_button(ui, &pal, Icon::Refresh, "Read the settings again", false).clicked() {
                            reload = true;
                        }
                    });
                });
                let cv = self.config.as_mut().unwrap();
                let w = ui.available_width();
                egui::ComboBox::from_id_salt("cfgsection_phone").selected_text(cv.section.label()).width(w - 8.0).show_ui(ui, |ui| {
                    for s in Section::LIST {
                        ui.selectable_value(&mut cv.section, s, s.label());
                    }
                });
                let links: Vec<(usize, String)> = dev.camera_links.iter().enumerate().filter_map(|(i, l)| l.as_ref().map(|l| (i + 1, format!("Camera on channel {} ({})", i + 1, l.host)))).collect();
                if !links.is_empty() {
                    let cur = if cv.target == 0 { "The recorder".to_string() } else { links.iter().find(|(i, _)| *i == cv.target).map(|(_, n)| n.clone()).unwrap_or_default() };
                    egui::ComboBox::from_id_salt("cfgtarget_phone").selected_text(format!("Settings of: {cur}")).width(w - 8.0).show_ui(ui, |ui| {
                        if ui.selectable_label(cv.target == 0, "The recorder").clicked() {
                            new_target = Some(0);
                        }
                        for (i, n) in &links {
                            if ui.selectable_label(cv.target == *i, n).clicked() {
                                new_target = Some(*i);
                            }
                        }
                    });
                }
                if let Some(snap) = &cv.snap {
                    let n = snap.channel_count();
                    if n > 1 {
                        let name = |c: u32| snap.values.get(&format!("ChannelTitle[{c}].Name")).cloned().unwrap_or_else(|| format!("Channel {}", c + 1));
                        egui::ComboBox::from_id_salt("cfgch_phone").selected_text(format!("Channel {}: {}", cv.channel + 1, name(cv.channel))).width(w - 8.0).show_ui(ui, |ui| {
                            for c in 0..n {
                                ui.selectable_value(&mut cv.channel, c, format!("Channel {}: {}", c + 1, name(c)));
                            }
                        });
                    }
                }
            }
            // Header: device, what we configure, which channel.
            if !compact {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&dev.name).size(24.0).strong().color(pal.text_strong));
                    ui.label(RichText::new(format!("Settings of {}", if self.config.as_ref().unwrap().target == 0 { "this device".to_string() } else { target_name.clone() })).color(pal.text_dim));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::icon_button(ui, &pal, Icon::Refresh, "Read the settings again", false).clicked() {
                        reload = true;
                    }
                    let cv = self.config.as_mut().unwrap();
                    // Channel picker (cameras of a recorder, or lenses of a multi-sensor camera).
                    if let Some(snap) = &cv.snap {
                        let n = snap.channel_count();
                        if n > 1 {
                            let name = |c: u32| snap.values.get(&format!("ChannelTitle[{c}].Name")).cloned().unwrap_or_else(|| format!("Channel {}", c + 1));
                            egui::ComboBox::from_id_salt("cfgch").selected_text(format!("Channel {}: {}", cv.channel + 1, name(cv.channel))).width(220.0).show_ui(ui, |ui| {
                                for c in 0..n {
                                    ui.selectable_value(&mut cv.channel, c, format!("Channel {}: {}", c + 1, name(c)));
                                }
                            });
                        }
                    }
                    // Target picker for recorders that told us their cameras.
                    let links: Vec<(usize, String)> = dev.camera_links.iter().enumerate().filter_map(|(i, l)| l.as_ref().map(|l| (i + 1, format!("Camera on channel {} ({})", i + 1, l.host)))).collect();
                    if !links.is_empty() {
                        let cur = if cv.target == 0 { "The recorder".to_string() } else { links.iter().find(|(i, _)| *i == cv.target).map(|(_, n)| n.clone()).unwrap_or_default() };
                        egui::ComboBox::from_id_salt("cfgtarget").selected_text(cur).width(240.0).show_ui(ui, |ui| {
                            if ui.selectable_label(cv.target == 0, "The recorder").clicked() {
                                new_target = Some(0);
                            }
                            for (i, n) in &links {
                                if ui.selectable_label(cv.target == *i, n).clicked() {
                                    new_target = Some(*i);
                                }
                            }
                        });
                        ui.label(RichText::new("Settings of:").color(pal.text_dim));
                    }
                });
            });
            }
            ui.add_space(12.0);

            let cv = self.config.as_ref().unwrap();
            if let Some(b) = cv.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(b);
                });
                if cv.snap.is_none() {
                    return;
                }
            }
            if let Some(e) = &cv.error {
                kit::card(ui, &pal, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Could not read the settings").strong().color(pal.danger));
                    ui.label(e);
                });
                return;
            }
            if cv.snap.is_none() {
                return;
            }
            let pending = cv.edits.len();
            let section = cv.section;
            // Save bar
            egui::TopBottomPanel::bottom("cfgsave").show_inside(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if pending == 0 {
                        ui.label(RichText::new("No changes").color(pal.text_dim));
                    } else {
                        ui.label(RichText::new(format!("{pending} unsaved change(s)")).strong().color(pal.warn));
                        if kit::primary(ui, &pal, "Save to device").clicked() {
                            let cv = self.config.as_mut().unwrap();
                            let risky = section.risky() || cv.edits.keys().any(|k| matches!(devconfig::group_of(k), "Network" | "RTSP" | "Web" | "DVRIP" | "AccessFilter"));
                            if risky {
                                cv.confirm_save = true;
                            } else {
                                self.config_save(ctx);
                            }
                        }
                        if kit::secondary(ui, "Undo changes").clicked() {
                            self.config.as_mut().unwrap().edits.clear();
                        }
                    }
                });
            });
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match section {
                Section::Overview => self.config_overview(ui, ctx),
                Section::All => self.config_all(ui),
                Section::Video => {
                    for (title, s) in STREAMS {
                        let c = self.config.as_ref().unwrap().channel;
                        if self.config.as_ref().unwrap().value(&expand("Encode[{c}].{s}.Video.Compression", c, s)).is_none() {
                            continue;
                        }
                        self.config_fields(ui, &pal, title, VIDEO, s);
                    }
                }
                Section::Picture => self.config_fields(ui, &pal, "Picture", PICTURE, ""),
                Section::Overlay => self.config_fields(ui, &pal, "On-screen text", OVERLAY, ""),
                Section::Motion => self.config_fields(ui, &pal, "Detection & alarms", MOTION, ""),
                Section::Recording => self.config_fields(ui, &pal, "Recording", RECORDING, ""),
                Section::Time => {
                    kit::card(ui, &pal, |ui| {
                        ui.set_width(ui.available_width().min(760.0));
                        ui.label(RichText::new("Device clock").size(16.0).strong().color(pal.text_strong));
                        let clock = self.config.as_ref().unwrap().snap.as_ref().and_then(|s| s.facts.iter().find(|(k, _)| k == "Device clock").map(|(_, v)| v.clone()));
                        ui.label(format!("Device time when read: {}", clock.unwrap_or_else(|| "unknown".into())));
                        if kit::secondary(ui, "Set the device clock to this computer's time").clicked() {
                            self.config_run(ctx, "Setting the clock…", |t| Msg::Note(devconfig::sync_clock(t).map(|n| format!("Device clock set to {n}"))));
                        }
                    });
                    ui.add_space(12.0);
                    self.config_fields(ui, &pal, "Time settings", TIME, "");
                }
                Section::Network => {
                    ui.label(RichText::new("Careful: a wrong address here can make the device unreachable until it is reset.").color(pal.warn));
                    ui.add_space(6.0);
                    self.config_fields(ui, &pal, "Network", NETWORK, "");
                }
                Section::Services => {
                    ui.label(RichText::new("MyCCTV needs video streaming (RTSP) on to show video. After changing a port, update the device in MyCCTV too.").color(pal.text_dim));
                    ui.add_space(6.0);
                    self.config_fields(ui, &pal, "Ports & services", SERVICES, "");
                }
            });
        });

        // Confirmations
        let mut do_save = false;
        let mut do_reboot = false;
        if let Some(cv) = self.config.as_mut() {
            if cv.confirm_save {
                egui::Window::new("Save these changes?").collapsible(false).resizable(false).title_bar(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
                    ui.set_width(380.0f32.min(ctx.screen_rect().width() - 60.0));
                    ui.label(RichText::new("Save these changes?").size(18.0).strong().color(pal.text_strong));
                    ui.label("Some of them change how the device is reached on the network. If they are wrong, the device may stop answering.");
                    for (k, v) in cv.edits.iter().take(12) {
                        ui.label(RichText::new(format!("{k} = {v}")).monospace().size(11.5));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if kit::danger(ui, &pal, "Save").clicked() {
                            do_save = true;
                            cv.confirm_save = false;
                        }
                        if kit::secondary(ui, "Cancel").clicked() {
                            cv.confirm_save = false;
                        }
                    });
                });
            }
            if cv.confirm_reboot {
                egui::Window::new("Restart device?").collapsible(false).resizable(false).title_bar(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
                    ui.set_width(340.0f32.min(ctx.screen_rect().width() - 60.0));
                    ui.label(RichText::new("Restart the device?").size(18.0).strong().color(pal.text_strong));
                    ui.label("Video and recording stop for a minute or two while it starts again.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if kit::danger(ui, &pal, "Restart").clicked() {
                            do_reboot = true;
                            cv.confirm_reboot = false;
                        }
                        if kit::secondary(ui, "Cancel").clicked() {
                            cv.confirm_reboot = false;
                        }
                    });
                });
            }
        }
        if do_save {
            self.config_save(ctx);
        }
        if do_reboot {
            self.config_run(ctx, "Restarting…", |t| Msg::Note(devconfig::reboot(t).map(|_| "The device is restarting".to_string())));
        }
        if let Some(t) = new_target {
            if let Some(cv) = self.config.as_mut() {
                cv.target = t;
                cv.channel = 0;
            }
            reload = true;
        }
        if reload {
            self.config_load(ctx);
        }
        if close {
            self.config = None;
        }
    }

    fn config_save(&mut self, ctx: &egui::Context) {
        let Some(cv) = self.config.as_ref() else { return };
        let changes: Vec<(String, String)> = cv.edits.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        self.config_run(ctx, "Saving…", move |t| Msg::Saved(devconfig::save(t, &changes)));
    }

    fn config_overview(&mut self, ui: &mut Ui, _ctx: &egui::Context) {
        let pal = self.pal;
        let cv = self.config.as_mut().unwrap();
        let Some(snap) = cv.snap.clone() else { return };
        kit::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width().min(760.0));
            ui.label(RichText::new("About this device").size(16.0).strong().color(pal.text_strong));
            egui::Grid::new("facts").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                for (k, v) in &snap.facts {
                    ui.label(RichText::new(k).color(pal.text_dim));
                    ui.label(RichText::new(v).color(pal.text_strong));
                    ui.end_row();
                }
                ui.label(RichText::new("Channels").color(pal.text_dim));
                ui.label(RichText::new(snap.channel_count().to_string()).color(pal.text_strong));
                ui.end_row();
                ui.label(RichText::new("Settings available").color(pal.text_dim));
                ui.label(RichText::new(format!("{} in {} groups", snap.values.len(), snap.groups().len())).color(pal.text_strong));
                ui.end_row();
            });
        });
        ui.add_space(12.0);
        if !snap.storage.is_empty() {
            kit::card(ui, &pal, |ui| {
                ui.set_width(ui.available_width().min(760.0));
                ui.label(RichText::new("Storage").size(16.0).strong().color(pal.text_strong));
                // list.info[0].Detail[0].TotalBytes=...
                let mut disks: BTreeMap<String, (f64, f64, String)> = BTreeMap::new();
                for (k, v) in &snap.storage {
                    let base = k.rsplit_once('.').map(|(b, _)| b.to_string()).unwrap_or_default();
                    let e = disks.entry(base).or_insert((0.0, 0.0, String::new()));
                    if k.ends_with(".TotalBytes") {
                        e.0 = v.parse().unwrap_or(0.0);
                    } else if k.ends_with(".UsedBytes") {
                        e.1 = v.parse().unwrap_or(0.0);
                    } else if k.ends_with(".State") {
                        e.2 = v.clone();
                    }
                }
                for (_, (total, used, state)) in disks.iter().filter(|(_, d)| d.0 > 0.0) {
                    let gb = |b: f64| b / 1_073_741_824.0;
                    ui.horizontal(|ui| {
                        ui.label(format!("{:.0} GB of {:.0} GB used", gb(*used), gb(*total)));
                        if !state.is_empty() {
                            kit::status_pill(ui, state, if state.eq_ignore_ascii_case("Success") || state.eq_ignore_ascii_case("Normal") { pal.live } else { pal.warn });
                        }
                    });
                    ui.add(egui::ProgressBar::new((*used / total.max(1.0)) as f32).desired_width(400.0));
                }
            });
            ui.add_space(12.0);
        }
        kit::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width().min(760.0));
            ui.label(RichText::new("Maintenance").size(16.0).strong().color(pal.text_strong));
            ui.horizontal(|ui| {
                if kit::secondary(ui, "Restart device").clicked() {
                    cv.confirm_reboot = true;
                }
            });
            ui.label(RichText::new("Every setting the device offers is also listed under All settings.").color(pal.text_dim));
        });
    }

    /// One card of friendly fields; fields the device doesn't have are left out.
    fn config_fields(&mut self, ui: &mut Ui, pal: &Palette, title: &str, fields: &[Field], stream: &str) {
        let compact = self.compact;
        // Phone: label above its control; controls as wide as the screen allows.
        let ww = if compact { (ui.available_width() - 48.0).max(160.0) } else { 260.0 };
        let cv = self.config.as_mut().unwrap();
        let c = cv.channel;
        let present: Vec<&Field> = fields.iter().filter(|fl| cv.value(&expand(fl.key, c, stream)).is_some()).collect();
        kit::card(ui, pal, |ui| {
            ui.set_width(ui.available_width().min(760.0));
            ui.label(RichText::new(title).size(16.0).strong().color(pal.text_strong));
            ui.add_space(4.0);
            if present.is_empty() {
                ui.label(RichText::new("This device doesn't offer these settings here. Look under All settings for everything it has.").color(pal.text_dim));
                return;
            }
            egui::Grid::new(("cfg", title, stream)).num_columns(if compact { 1 } else { 2 }).spacing(vec2(24.0, if compact { 4.0 } else { 10.0 })).min_col_width(if compact { 0.0 } else { 220.0 }).show(ui, |ui| {
                for fl in present {
                    let key = expand(fl.key, c, stream);
                    let cur = cv.value(&key).unwrap_or_default();
                    let changed = cv.edits.contains_key(&key);
                    ui.label(RichText::new(fl.label).color(if changed { pal.warn } else { pal.text })).on_hover_text(&key);
                    if compact {
                        ui.end_row();
                    }
                    let caps_key = |suffix: &str| format!("caps{c}.{stream}.{suffix}");
                    let caps = cv.snap.as_ref().map(|s| s.caps.clone()).unwrap_or_default();
                    let mut new_val: Option<String> = None;
                    match fl.kind {
                        Kind::Bool => {
                            let mut b = cur.eq_ignore_ascii_case("true");
                            if ui.add(egui::Checkbox::without_text(&mut b)).changed() {
                                new_val = Some(if b { "true" } else { "false" }.into());
                            }
                        }
                        Kind::Slider(lo, hi) => {
                            let mut n: i64 = cur.trim().parse::<f64>().map(|v| v.round() as i64).unwrap_or(lo);
                            if ui.add(egui::Slider::new(&mut n, lo..=hi)).changed() {
                                new_val = Some(n.to_string());
                            }
                        }
                        Kind::Number(lo, hi) => {
                            let mut n: i64 = cur.trim().parse::<f64>().map(|v| v.round() as i64).unwrap_or(lo);
                            if ui.add(egui::DragValue::new(&mut n).range(lo..=hi)).changed() {
                                new_val = Some(n.to_string());
                            }
                        }
                        Kind::CapsNumber(suffix, lo, hi) => {
                            let (mut lo, mut hi) = (lo, hi);
                            if let Some(v) = caps.get(&caps_key(suffix)) {
                                let parts: Vec<i64> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                                match parts.as_slice() {
                                    [max] => hi = *max,
                                    [a, b] => {
                                        lo = *a;
                                        hi = *b;
                                    }
                                    _ => {}
                                }
                            }
                            let mut n: i64 = cur.trim().parse::<f64>().map(|v| v.round() as i64).unwrap_or(lo);
                            if ui.add(egui::DragValue::new(&mut n).range(lo..=hi.max(lo))).on_hover_text(format!("{lo} to {hi}")).changed() {
                                new_val = Some(n.to_string());
                            }
                        }
                        Kind::Text => {
                            let mut s = cur.clone();
                            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(ww + 40.0).margin(vec2(8.0, 6.0))).changed() {
                                new_val = Some(s);
                            }
                        }
                        Kind::Choice(opts) => {
                            let label = opts.iter().find(|(v, _)| *v == cur).map(|(_, l)| l.to_string()).unwrap_or_else(|| cur.clone());
                            egui::ComboBox::from_id_salt(("ch", &key)).selected_text(label).width(ww).show_ui(ui, |ui| {
                                for (v, l) in opts {
                                    if ui.selectable_label(cur == *v, *l).clicked() {
                                        new_val = Some(v.to_string());
                                    }
                                }
                            });
                        }
                        Kind::CapsChoice(suffix, fallback) => {
                            let mut opts: Vec<String> = caps.get(&caps_key(suffix)).map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_else(|| fallback.iter().map(|s| s.to_string()).collect());
                            if !opts.contains(&cur) {
                                opts.insert(0, cur.clone());
                            }
                            egui::ComboBox::from_id_salt(("cc", &key)).selected_text(cur.clone()).width(ww).show_ui(ui, |ui| {
                                for o in &opts {
                                    if ui.selectable_label(cur == *o, o).clicked() {
                                        new_val = Some(o.clone());
                                    }
                                }
                            });
                        }
                    }
                    if let Some(v) = new_val {
                        cv.set(&key, v);
                    }
                    ui.end_row();
                }
            });
        });
        ui.add_space(12.0);
    }

    /// Every setting the device offers, grouped, editable as text.
    fn config_all(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        let compact = self.compact;
        let cv = self.config.as_mut().unwrap();
        let Some(snap) = cv.snap.clone() else { return };
        if cv.all_group.is_none() {
            cv.all_group = snap.groups().first().map(|(g, _)| g.clone());
        }
        ui.label(RichText::new("Everything the device offers, as the device names it. Change a value and press Save to device.").color(pal.text_dim));
        ui.add(egui::TextEdit::singleline(&mut cv.all_filter).hint_text("Search all settings (name or value)").desired_width(ui.available_width().min(420.0)).margin(vec2(10.0, 8.0)));
        ui.add_space(8.0);
        let filter = cv.all_filter.to_lowercase();
        if compact {
            let groups = snap.groups();
            egui::ComboBox::from_id_salt("allgroups").selected_text(cv.all_group.clone().unwrap_or_default()).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                for (g, n) in groups {
                    if ui.selectable_label(cv.all_group.as_deref() == Some(g.as_str()), format!("{g}  ({n})")).clicked() {
                        cv.all_group = Some(g);
                    }
                }
            });
        }
        ui.horizontal_top(|ui| {
            if !compact {
            // Groups
            ui.allocate_ui_with_layout(vec2(220.0, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("groups").max_height(ui.available_height()).show(ui, |ui| {
                    ui.set_width(210.0);
                    for (g, n) in snap.groups() {
                        let sel = cv.all_group.as_deref() == Some(g.as_str());
                        if ui.selectable_label(sel, format!("{g}  ({n})")).clicked() {
                            cv.all_group = Some(g);
                        }
                    }
                });
            });
            ui.separator();
            }
            // Settings of the chosen group (or the search results)
            let rows: Vec<(String, String)> = snap
                .values
                .iter()
                .filter(|(k, v)| {
                    if !filter.is_empty() {
                        k.to_lowercase().contains(&filter) || v.to_lowercase().contains(&filter)
                    } else {
                        cv.all_group.as_deref().map(|g| devconfig::group_of(k) == g).unwrap_or(false)
                    }
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            ui.vertical(|ui| {
                if rows.is_empty() {
                    ui.label(RichText::new(if filter.is_empty() { "Pick a group on the left, or search." } else { "Nothing matches." }).color(pal.text_dim));
                    return;
                }
                ui.label(RichText::new(format!("{} setting(s)", rows.len())).color(pal.text_dim));
                let key_w = if compact { ui.available_width() * 0.5 } else { (ui.available_width() * 0.55).max(240.0) };
                egui::ScrollArea::vertical().id_salt("allrows").show_rows(ui, 30.0, rows.len(), |ui, range| {
                    for (k, orig) in &rows[range] {
                        ui.horizontal(|ui| {
                            let changed = cv.edits.contains_key(k);
                            ui.add_sized(vec2(key_w, 26.0), egui::Label::new(RichText::new(k).monospace().size(11.5).color(if changed { pal.warn } else { pal.text })).truncate());
                            let mut v = cv.edits.get(k).cloned().unwrap_or_else(|| orig.clone());
                            let r = ui.add(egui::TextEdit::singleline(&mut v).desired_width(ui.available_width() - 8.0).font(egui::TextStyle::Monospace));
                            if r.changed() {
                                cv.set(k, v);
                            }
                        });
                    }
                });
            });
        });
    }
}

/// Settings button drawn on device cards.
pub fn settings_button(ui: &mut Ui, pal: &Palette) -> egui::Response {
    let r = kit::icon_button(ui, pal, Icon::Gear, "Device settings (video, picture, detection, recording, time, network and everything else)", false);
    let _ = Color32::WHITE;
    r
}
