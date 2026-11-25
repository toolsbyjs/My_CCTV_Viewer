//! Settings > Networks: the computer's network connections, joining a second Wi-Fi adapter to the
//! cameras' router, and the connection picker used in the device form and the network scan.

use crate::kit;
use crate::netpath::{self, LinkKind, WifiNet};
use crate::theme::Palette;
use egui::{RichText, Ui};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

#[derive(Default)]
pub struct NetUi {
    /// Every Wi-Fi adapter, connected or not (loaded in the background).
    adapters: Vec<netpath::WifiAdapter>,
    adapters_rx: Option<Receiver<(Vec<netpath::WifiAdapter>, Vec<String>)>>,
    /// Cable ports with nothing plugged in.
    cable_ports: Vec<String>,
    adapters_loaded: bool,
    /// The Wi-Fi adapter being joined to a router.
    adapter: String,
    scan_rx: Option<Receiver<Result<Vec<WifiNet>, String>>>,
    nets: Vec<WifiNet>,
    scan_note: Option<String>,
    ssid: String,
    auth: String,
    saved: bool,
    password: String,
    job_rx: Option<Receiver<Result<String, String>>>,
    msg: Option<(bool, String)>,
}

impl NetUi {
    fn load_adapters(&mut self) {
        let (tx, rx) = channel();
        self.adapters_rx = Some(rx);
        self.adapters_loaded = true;
        std::thread::spawn(move || {
            let _ = tx.send((netpath::wifi_adapters(), netpath::free_cable_ports()));
        });
    }

    fn run_job(&mut self, job: impl FnOnce() -> Result<String, String> + Send + 'static) {
        let (tx, rx) = channel();
        self.job_rx = Some(rx);
        self.msg = None;
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    fn poll(&mut self) {
        if let Some(Ok((a, cables))) = self.adapters_rx.as_ref().map(|r| r.try_recv()) {
            self.cable_ports = cables;
            self.adapters = a;
            self.adapters_rx = None;
            if !self.adapters.iter().any(|a| a.name == self.adapter) {
                // Prefer a Wi-Fi adapter that isn't the main connection.
                let main = netpath::links().into_iter().find(|l| l.main).map(|l| l.name);
                self.adapter = self.adapters.iter().map(|a| &a.name).find(|a| Some(*a) != main.as_ref()).or(self.adapters.first().map(|a| &a.name)).cloned().unwrap_or_default();
            }
        }
        if let Some(Ok(r)) = self.scan_rx.as_ref().map(|r| r.try_recv()) {
            self.scan_rx = None;
            match r {
                Ok(n) => {
                    self.scan_note = if n.is_empty() { Some("No Wi-Fi networks found by this adapter.".into()) } else { None };
                    self.nets = n;
                }
                Err(e) => {
                    self.nets.clear();
                    self.scan_note = Some(e);
                }
            }
        }
        if let Some(Ok(r)) = self.job_rx.as_ref().map(|r| r.try_recv()) {
            self.job_rx = None;
            self.msg = Some(match r {
                Ok(m) => (true, m),
                Err(e) => (false, e),
            });
            self.password.clear();
            self.load_adapters();
        }
    }

    fn busy(&self) -> bool {
        self.adapters_rx.is_some() || self.scan_rx.is_some() || self.job_rx.is_some()
    }
}

/// A drop-down of the connections, with `auto_text` for "" (no choice).
pub fn network_picker(ui: &mut Ui, id: &str, value: &mut String, auto_text: &str) {
    let links = netpath::links();
    let text = |l: &netpath::NetLink| format!("{}{}", l.label(), if l.main { "  (main)" } else { "" });
    let current = if value.is_empty() {
        auto_text.to_string()
    } else {
        links.iter().find(|l| l.name == *value).map(text).unwrap_or_else(|| format!("{value} (not connected now)"))
    };
    let width = (ui.available_width() - 8.0).clamp(160.0, 380.0);
    egui::ComboBox::from_id_salt(id).selected_text(current).width(width).show_ui(ui, |ui| {
        ui.selectable_value(value, String::new(), auto_text);
        for l in &links {
            ui.selectable_value(value, l.name.clone(), text(l));
        }
        if !value.is_empty() && !links.iter().any(|l| l.name == *value) {
            let v = value.clone();
            ui.selectable_value(value, v.clone(), format!("{v} (not connected now)"));
        }
    });
}

pub fn networks_section(ui: &mut Ui, pal: &Palette, st: &mut NetUi) {
    if !st.adapters_loaded {
        st.load_adapters();
    }
    st.poll();
    let links = netpath::links();
    if st.busy() || links.is_empty() {
        ui.ctx().request_repaint_after(Duration::from_millis(400));
    } else {
        ui.ctx().request_repaint_after(Duration::from_secs(4)); // connections come and go
    }

    ui.label(
        RichText::new(
            "If your recorder or cameras are on another router than the one this device uses for the internet, MyCCTV can reach them through a second connection while everything else stays on your main connection. \
             Join the cameras' router with a second connection, then pick that connection for the device (Devices > Edit > Network connection).",
        )
        .color(pal.text_dim),
    );
    ui.add_space(6.0);
    ui.label(RichText::new("Connections now").strong().color(pal.text_strong));
    if links.is_empty() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Looking...");
        });
    }
    let mut keep_off: Option<String> = None;
    let mut disconnect: Option<String> = None;
    for l in &links {
        ui.horizontal_wrapped(|ui| {
            kit::status_pill(ui, l.kind_name(), if l.main { pal.live } else { pal.accent });
            ui.label(RichText::new(l.label()).color(pal.text));
            if l.main {
                ui.label(RichText::new("main · internet").size(12.0).color(pal.text_dim));
            } else if cfg!(windows) && !st.busy() {
                if ui
                    .small_button("Keep internet on main")
                    .on_hover_text("Windows will keep using your main connection for the internet and only use this one for devices that are set to it. Windows asks for permission.")
                    .clicked()
                {
                    keep_off = Some(l.name.clone());
                }
                if l.kind == LinkKind::Wifi && ui.small_button("Disconnect").clicked() {
                    disconnect = Some(l.name.clone());
                }
            }
        });
    }
    // Wi-Fi adapters without an address of their own: not joined, or shared through a virtual
    // switch (the address then belongs to that switch, listed above).
    for a in st.adapters.iter().filter(|a| !links.iter().any(|l| l.name == a.name)) {
        ui.horizontal_wrapped(|ui| {
            kit::status_pill(ui, "Wi-Fi", pal.text_dim);
            let text = match (&a.ssid, a.connected) {
                (Some(s), _) => format!("{} · joined {s} (used through another connection above)", a.name),
                (None, true) => format!("{} · joined a router (used through another connection above)", a.name),
                (None, false) => format!("{} · not connected", a.name),
            };
            ui.label(RichText::new(text).color(pal.text_dim));
        });
    }
    for p in &st.cable_ports {
        ui.horizontal_wrapped(|ui| {
            kit::status_pill(ui, "Cable", pal.text_dim);
            ui.label(RichText::new(format!("{p} · nothing plugged in")).color(pal.text_dim));
        });
    }
    if !st.cable_ports.is_empty() {
        ui.label(
            RichText::new("Tip: a network cable from the cameras' router into a free cable port gives this computer a second connection right away; then pick it for the device.")
                .size(12.5)
                .color(pal.text_dim),
        );
    }
    if let Some(a) = keep_off {
        st.run_job(move || netpath::keep_internet_off(&a).map(|_| format!("{a} is now only a second connection; the internet stays on your main connection")));
    }
    if let Some(a) = disconnect {
        st.run_job(move || netpath::wifi_disconnect(&a).map(|_| format!("{a} disconnected")));
    }

    ui.add_space(10.0);
    ui.label(RichText::new("Join another router").strong().color(pal.text_strong));
    if netpath::CAN_JOIN && !st.adapters.is_empty() {
        let main = links.iter().find(|l| l.main).map(|l| l.name.clone());
        ui.horizontal_wrapped(|ui| {
            ui.label("With Wi-Fi adapter");
            let before = st.adapter.clone();
            egui::ComboBox::from_id_salt("joinadapter").selected_text(st.adapter.clone()).show_ui(ui, |ui| {
                for a in &st.adapters {
                    let text = match (&a.ssid, a.connected) {
                        (Some(s), _) => format!("{} (on {s})", a.name),
                        (None, true) => format!("{} (in use)", a.name),
                        (None, false) => format!("{} (free)", a.name),
                    };
                    ui.selectable_value(&mut st.adapter, a.name.clone(), text);
                }
            });
            if st.adapter != before {
                st.nets.clear();
                st.scan_note = None;
            }
            if netpath::CAN_SCAN && st.scan_rx.is_none() && kit::secondary(ui, "Find networks").clicked() {
                let a = st.adapter.clone();
                let (tx, rx) = channel();
                st.scan_rx = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send(netpath::wifi_scan(&a));
                });
            }
            if st.scan_rx.is_some() {
                ui.spinner();
            }
        });
        // The main connection itself, or an adapter that is joined but shared through a virtual switch
        // (which is then most likely the main connection).
        let shared = st.adapters.iter().any(|a| a.name == st.adapter && a.connected && !links.iter().any(|l| l.name == a.name));
        if main.as_ref() == Some(&st.adapter) || shared {
            ui.label(
                RichText::new("This adapter is your main connection: joining another router with it moves this whole computer to that router. To keep both, add a second adapter (a small USB Wi-Fi adapter, or a network cable to the cameras' router).")
                    .size(12.5)
                    .color(pal.warn),
            );
        }
        if let (Some(help), None) = (netpath::wifi_names_hidden(), &st.scan_note) {
            ui.label(RichText::new(help).size(12.5).color(pal.text_dim));
        }
        if let Some(n) = &st.scan_note {
            ui.label(RichText::new(n).size(12.5).color(pal.text_dim));
        }
        if !st.nets.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for n in st.nets.clone() {
                    let text = format!("{}{}{}", n.ssid, n.signal.map(|s| format!("  {s}%")).unwrap_or_default(), if n.saved { "  (saved)" } else { "" });
                    if kit::chip(ui, pal, &text, st.ssid == n.ssid).on_hover_text(&n.auth).clicked() {
                        st.ssid = n.ssid.clone();
                        st.auth = n.auth.clone();
                        st.saved = n.saved;
                    }
                }
            });
        }
        egui::Grid::new("joinform").num_columns(2).spacing(egui::vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label("Router (Wi-Fi name)");
            if kit::text_input(ui, &mut st.ssid, "e.g. Cameras-WiFi", 32, false).changed() {
                st.saved = st.nets.iter().any(|n| n.ssid == st.ssid && n.saved);
                st.auth = st.nets.iter().find(|n| n.ssid == st.ssid).map(|n| n.auth.clone()).unwrap_or_default();
            }
            ui.end_row();
            ui.label("Password");
            kit::text_input(ui, &mut st.password, if st.saved { "saved (leave empty)" } else { "" }, 63, true);
            ui.end_row();
        });
        let ready = !st.ssid.trim().is_empty() && !st.adapter.is_empty() && st.job_rx.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, egui::Button::new(RichText::new("Join").strong())).clicked() {
                let (a, s, p, au, sv) = (st.adapter.clone(), st.ssid.trim().to_string(), st.password.clone(), st.auth.clone(), st.saved);
                st.run_job(move || netpath::wifi_connect(&a, &s, &p, &au, sv));
            }
            if st.job_rx.is_some() {
                ui.spinner();
                ui.label("Working...");
            }
        });
    } else {
        ui.label(
            RichText::new(if crate::platform::IS_ANDROID {
                "On a phone or tablet, mobile data keeps the internet while Wi-Fi joins the cameras' router. Join it in Android's Wi-Fi settings (if Android asks, choose to stay connected without internet)."
            } else {
                "Join the cameras' router with your second adapter in the system's Wi-Fi settings."
            })
            .color(pal.text_dim),
        );
    }
    if !netpath::CAN_JOIN || st.adapters.is_empty() {
        if kit::secondary(ui, "Open Wi-Fi settings").clicked() {
            netpath::open_wifi_settings();
        }
    }
    if let Some((ok, m)) = &st.msg {
        ui.add_space(4.0);
        ui.label(RichText::new(m).color(if *ok { pal.live } else { pal.danger }));
    }
    if !crate::platform::IS_ANDROID && st.adapters.len() <= 1 && links.iter().filter(|l| !l.main).count() == 0 {
        ui.add_space(4.0);
        ui.label(
            RichText::new("This computer has one connection. One Wi-Fi adapter can only be on one router at a time, so to stay on your main router while watching cameras on another, add a second adapter (a small USB Wi-Fi adapter) or connect a network cable to the cameras' router.")
                .size(12.5)
                .color(pal.text_dim),
        );
    }
}
