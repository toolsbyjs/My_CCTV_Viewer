//! "Aurora" look: a fresh design of our own (navigation rail, cards, floating
//! controls), in a Midnight (dark) and a Daylight (light) variant.

use crate::model::ThemeChoice;
use egui::{Color32, Rounding, Stroke};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub card: Color32,
    pub field: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub live: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub video_bg: Color32,
    pub dark: bool,
}

pub const ACCENT: Color32 = Color32::from_rgb(0x4F, 0x7C, 0xFF);
pub const LIVE: Color32 = Color32::from_rgb(0x19, 0xC3, 0xB1);
pub const DANGER: Color32 = Color32::from_rgb(0xFF, 0x54, 0x70);
pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xB0, 0x20);

pub fn palette(t: ThemeChoice) -> Palette {
    match t {
        ThemeChoice::Dark => Palette {
            bg: Color32::from_rgb(0x0B, 0x0F, 0x14),
            surface: Color32::from_rgb(0x12, 0x18, 0x21),
            card: Color32::from_rgb(0x18, 0x20, 0x2B),
            field: Color32::from_rgb(0x22, 0x2C, 0x3A),
            hover: Color32::from_rgb(0x2A, 0x36, 0x47),
            border: Color32::from_rgb(0x26, 0x31, 0x42),
            text: Color32::from_rgb(0xC3, 0xCD, 0xDA),
            text_strong: Color32::from_rgb(0xF4, 0xF7, 0xFB),
            text_dim: Color32::from_rgb(0x6F, 0x7E, 0x94),
            accent: ACCENT,
            accent_soft: Color32::from_rgb(0x1E, 0x2C, 0x52),
            live: LIVE,
            warn: WARN,
            danger: DANGER,
            video_bg: Color32::from_rgb(0x07, 0x09, 0x0C),
            dark: true,
        },
        ThemeChoice::Light => Palette {
            bg: Color32::from_rgb(0xF2, 0xF4, 0xF8),
            surface: Color32::WHITE,
            card: Color32::WHITE,
            field: Color32::from_rgb(0xEE, 0xF1, 0xF6),
            hover: Color32::from_rgb(0xE3, 0xE8, 0xF0),
            border: Color32::from_rgb(0xDC, 0xE2, 0xEA),
            text: Color32::from_rgb(0x2B, 0x35, 0x45),
            text_strong: Color32::from_rgb(0x0E, 0x16, 0x21),
            text_dim: Color32::from_rgb(0x84, 0x90, 0xA2),
            accent: Color32::from_rgb(0x3D, 0x63, 0xF0),
            accent_soft: Color32::from_rgb(0xE4, 0xEA, 0xFF),
            live: Color32::from_rgb(0x0E, 0xA5, 0x95),
            warn: Color32::from_rgb(0xE0, 0x8A, 0x00),
            danger: Color32::from_rgb(0xE5, 0x3E, 0x5C),
            video_bg: Color32::from_rgb(0x14, 0x18, 0x1F),
            dark: false,
        },
    }
}

pub fn apply(ctx: &egui::Context, t: ThemeChoice, scale: f32) {
    let p = palette(t);
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.surface;
    v.window_fill = p.card;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.field;
    v.code_bg_color = p.field;
    v.override_text_color = Some(p.text);
    v.window_stroke = Stroke::new(1.0f32, p.border);
    v.window_rounding = Rounding::same(14.0f32);
    v.menu_rounding = Rounding::same(10.0f32);
    v.window_shadow = egui::epaint::Shadow { offset: egui::vec2(0.0, 10.0), blur: 30.0, spread: 0.0, color: Color32::from_black_alpha(if p.dark { 140 } else { 40 }) };
    v.popup_shadow = egui::epaint::Shadow { offset: egui::vec2(0.0, 6.0), blur: 18.0, spread: 0.0, color: Color32::from_black_alpha(if p.dark { 120 } else { 30 }) };
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0f32, Color32::WHITE);
    v.hyperlink_color = p.accent;
    v.slider_trailing_fill = true;
    let r = Rounding::same(8.0f32);
    v.widgets.noninteractive.bg_fill = p.card;
    v.widgets.noninteractive.weak_bg_fill = p.card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0f32, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0f32, p.text);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.bg_fill = p.field;
    v.widgets.inactive.weak_bg_fill = p.field;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0f32, p.text);
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.fg_stroke = Stroke::new(1.5f32, p.text_strong);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0f32, p.accent);
    v.widgets.hovered.rounding = r;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.weak_bg_fill = p.accent;
    v.widgets.active.fg_stroke = Stroke::new(1.5f32, Color32::WHITE);
    v.widgets.active.rounding = r;
    v.widgets.active.expansion = 0.0;
    v.widgets.open.bg_fill = p.hover;
    v.widgets.open.weak_bg_fill = p.hover;
    v.widgets.open.rounding = r;
    ctx.set_theme(if p.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.slider_width = 150.0;
        s.spacing.menu_margin = egui::Margin::same(8.0);
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
    });
    ctx.set_zoom_factor(scale.clamp(0.75, 3.0));
}
