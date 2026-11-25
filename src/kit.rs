//! Small building blocks of the new design: cards, chips, pill buttons,
//! icon buttons, stat tiles and the page header.

use crate::icons::{self, Icon};
use crate::theme::Palette;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Response, RichText, Rounding, Sense, Stroke, Ui};

pub fn card<R>(ui: &mut Ui, pal: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(pal.card)
        .stroke(Stroke::new(1.0f32, pal.border))
        .rounding(Rounding::same(14.0f32))
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, add)
        .inner
}

pub fn page_header(ui: &mut Ui, pal: &Palette, title: &str, subtitle: &str, actions: impl FnOnce(&mut Ui)) {
    if ui.available_width() < 620.0 {
        // Phone: title, then the buttons on their own (wrapping) line.
        ui.label(RichText::new(title).size(22.0).strong().color(pal.text_strong));
        if !subtitle.is_empty() {
            ui.label(RichText::new(subtitle).color(pal.text_dim));
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(actions);
        ui.add_space(10.0);
        return;
    }
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(26.0).strong().color(pal.text_strong));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).color(pal.text_dim));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
    });
    ui.add_space(14.0);
}

/// Filled accent button.
pub fn primary(ui: &mut Ui, pal: &Palette, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(pal.accent).rounding(Rounding::same(10.0f32)).min_size(vec2(0.0, 36.0)))
}

/// Quiet button.
pub fn secondary(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(text).rounding(Rounding::same(10.0f32)).min_size(vec2(0.0, 36.0)))
}

pub fn danger(ui: &mut Ui, pal: &Palette, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(pal.danger).rounding(Rounding::same(10.0f32)).min_size(vec2(0.0, 36.0)))
}

/// Round icon-only button with a tooltip.
pub fn icon_button(ui: &mut Ui, pal: &Palette, icon: Icon, tip: &str, active: bool) -> Response {
    let size = 36.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hot = resp.hovered() || resp.has_focus();
    let fill = if active {
        pal.accent
    } else if hot {
        pal.hover
    } else {
        pal.field
    };
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    if resp.has_focus() {
        ui.painter().circle_stroke(rect.center(), size / 2.0 + 2.0, Stroke::new(2.0f32, pal.accent));
    }
    let c = if active || hot { Color32::WHITE } else { pal.text };
    icons::paint(ui.painter(), rect.shrink(10.0), icon, c);
    resp.on_hover_text(tip)
}

/// Selectable rounded chip (filters, segmented choices).
pub fn chip(ui: &mut Ui, pal: &Palette, text: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), pal.text);
    let size = vec2(galley.size().x + 26.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hot = resp.hovered() || resp.has_focus();
    let (fill, fg) = if selected {
        (pal.accent, Color32::WHITE)
    } else if hot {
        (pal.hover, pal.text_strong)
    } else {
        (pal.field, pal.text)
    };
    ui.painter().rect_filled(rect, Rounding::same(15.0f32), fill);
    if resp.has_focus() && !selected {
        ui.painter().rect_stroke(rect, Rounding::same(15.0f32), Stroke::new(2.0f32, pal.accent));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), fg);
    resp
}

/// Small coloured status pill (Online, Offline, Live...).
pub fn status_pill(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let size = vec2(galley.size().x + 26.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, Rounding::same(11.0f32), color.gamma_multiply(0.16));
    ui.painter().circle_filled(pos2(rect.left() + 10.0, rect.center().y), 3.5, color);
    ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), color);
}

/// Dashboard tile: tinted icon, big number, caption.
pub fn stat_tile(ui: &mut Ui, pal: &Palette, width: f32, icon: Icon, tint: Color32, value: &str, caption: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 104.0), Sense::click());
    let hot = resp.hovered() || resp.has_focus();
    ui.painter().rect(rect, Rounding::same(16.0f32), pal.card, Stroke::new(if resp.has_focus() { 2.0f32 } else { 1.0f32 }, if hot { tint } else { pal.border }));
    let ic = Rect::from_center_size(pos2(rect.left() + 40.0, rect.center().y), vec2(44.0, 44.0));
    ui.painter().circle_filled(ic.center(), 22.0, tint.gamma_multiply(0.18));
    icons::paint(ui.painter(), ic.shrink(11.0), icon, tint);
    ui.painter().text(pos2(rect.left() + 76.0, rect.center().y - 12.0), Align2::LEFT_CENTER, value, FontId::proportional(26.0), pal.text_strong);
    ui.painter().text(pos2(rect.left() + 77.0, rect.center().y + 16.0), Align2::LEFT_CENTER, caption, FontId::proportional(13.0), pal.text_dim);
    resp
}

/// Label above a field, with an optional error underneath.
pub fn field<R>(ui: &mut Ui, pal: &Palette, label: &str, error: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.label(RichText::new(label).size(12.5).color(pal.text_dim));
    let r = add(ui);
    if let Some(e) = error {
        ui.label(RichText::new(e).size(12.0).color(pal.danger));
    }
    ui.add_space(6.0);
    r
}

pub fn text_input(ui: &mut Ui, value: &mut String, hint: &str, limit: usize, password: bool) -> Response {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint).char_limit(limit).password(password).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0)))
}

pub fn empty_state(ui: &mut Ui, pal: &Palette, icon: Icon, title: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(30.0);
        let (r, _) = ui.allocate_exact_size(vec2(84.0, 84.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 42.0, pal.accent_soft);
        icons::paint(ui.painter(), r.shrink(22.0), icon, pal.accent);
        ui.add_space(12.0);
        ui.label(RichText::new(title).size(19.0).strong().color(pal.text_strong));
        ui.label(RichText::new(text).color(pal.text_dim));
        ui.add_space(10.0);
    });
}
