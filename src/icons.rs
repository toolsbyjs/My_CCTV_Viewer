//! Simple drawn icons (no pictures copied from the original).

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Rounding, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Camera,
    Play,
    Server,
    Bell,
    Gear,
    Home,
    Grid,
    Record,
    Snapshot,
    Close,
    Replay,
    Fullscreen,
    Logo,
    Search,
    Plus,
    Edit,
    Trash,
    Refresh,
    Moon,
    Sun,
    Joystick,
    Tour,
    Sidebar,
    Stop,
}

pub fn paint(p: &Painter, r: Rect, icon: Icon, c: Color32) {
    let s = r.width().min(r.height());
    let r = Rect::from_center_size(r.center(), vec2(s, s));
    let w = (s / 14.0).max(1.2);
    let st = Stroke::new(w, c);
    let at = |x: f32, y: f32| -> Pos2 { pos2(r.left() + x * s, r.top() + y * s) };
    match icon {
        Icon::Camera => {
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.3), at(0.68, 0.72)), Rounding::same(s * 0.06), st);
            p.add(Shape::convex_polygon(vec![at(0.7, 0.45), at(0.92, 0.32), at(0.92, 0.7), at(0.7, 0.57)], Color32::TRANSPARENT, st));
            p.circle_filled(at(0.2, 0.42), s * 0.04, c);
        }
        Icon::Play => {
            p.circle_stroke(r.center(), s * 0.42, st);
            p.add(Shape::convex_polygon(vec![at(0.4, 0.3), at(0.72, 0.5), at(0.4, 0.7)], c, Stroke::NONE));
        }
        Icon::Server => {
            for i in 0..3 {
                let y = 0.14 + i as f32 * 0.25;
                p.rect_stroke(Rect::from_min_max(at(0.12, y), at(0.88, y + 0.2)), Rounding::same(s * 0.04), st);
                p.circle_filled(at(0.24, y + 0.1), s * 0.035, c);
                p.line_segment([at(0.55, y + 0.1), at(0.78, y + 0.1)], st);
            }
        }
        Icon::Bell => {
            let pts = vec![at(0.5, 0.12), at(0.7, 0.24), at(0.74, 0.6), at(0.86, 0.72), at(0.14, 0.72), at(0.26, 0.6), at(0.3, 0.24)];
            p.add(Shape::closed_line(pts, st));
            p.circle_filled(at(0.5, 0.82), s * 0.07, c);
        }
        Icon::Gear => {
            p.circle_stroke(r.center(), s * 0.16, st);
            p.circle_stroke(r.center(), s * 0.32, st);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([r.center() + d * s * 0.32, r.center() + d * s * 0.44], Stroke::new(w * 1.8, c));
            }
        }
        Icon::Home => {
            p.add(Shape::closed_line(vec![at(0.5, 0.15), at(0.88, 0.5), at(0.76, 0.5), at(0.76, 0.85), at(0.24, 0.85), at(0.24, 0.5), at(0.12, 0.5)], st));
        }
        Icon::Grid => {
            for i in 0..2 {
                for j in 0..2 {
                    let x = 0.12 + i as f32 * 0.4;
                    let y = 0.12 + j as f32 * 0.4;
                    p.rect_stroke(Rect::from_min_max(at(x, y), at(x + 0.34, y + 0.34)), Rounding::same(1.0), st);
                }
            }
        }
        Icon::Record => {
            p.circle_stroke(r.center(), s * 0.4, st);
            p.circle_filled(r.center(), s * 0.2, c);
        }
        Icon::Snapshot => {
            p.rect_stroke(Rect::from_min_max(at(0.1, 0.28), at(0.9, 0.82)), Rounding::same(s * 0.08), st);
            p.rect_filled(Rect::from_min_max(at(0.34, 0.16), at(0.62, 0.28)), Rounding::same(1.0), c);
            p.circle_stroke(at(0.5, 0.55), s * 0.16, st);
        }
        Icon::Close => {
            p.line_segment([at(0.22, 0.22), at(0.78, 0.78)], st);
            p.line_segment([at(0.78, 0.22), at(0.22, 0.78)], st);
        }
        Icon::Replay => {
            let n = 14;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = std::f32::consts::PI * 0.3 + i as f32 / n as f32 * std::f32::consts::PI * 1.6;
                    r.center() + vec2(a.cos(), -a.sin()) * s * 0.36
                })
                .collect();
            let end = *pts.last().unwrap();
            p.add(Shape::line(pts, st));
            p.add(Shape::convex_polygon(vec![end + vec2(-s * 0.12, -s * 0.02), end + vec2(s * 0.1, -s * 0.06), end + vec2(0.0, s * 0.14)], c, Stroke::NONE));
        }
        Icon::Fullscreen => {
            for (a, b, d) in [((0.15, 0.15), (0.38, 0.15), (0.15, 0.38)), ((0.85, 0.15), (0.62, 0.15), (0.85, 0.38)), ((0.15, 0.85), (0.38, 0.85), (0.15, 0.62)), ((0.85, 0.85), (0.62, 0.85), (0.85, 0.62))] {
                p.line_segment([at(a.0, a.1), at(b.0, b.1)], st);
                p.line_segment([at(a.0, a.1), at(d.0, d.1)], st);
            }
        }
        Icon::Logo => {
            // An "aperture" mark on a blue-to-teal tile.
            let tile = r.shrink(s * 0.04);
            p.rect_filled(tile, Rounding::same(s * 0.26), crate::theme::ACCENT);
            p.rect_filled(Rect::from_min_max(pos2(tile.left(), tile.center().y), tile.max), Rounding { nw: 0.0, ne: 0.0, sw: s * 0.26, se: s * 0.26 }, crate::theme::LIVE.gamma_multiply(0.55));
            let inner = Stroke::new(w * 1.4, Color32::WHITE);
            p.circle_stroke(r.center(), s * 0.24, inner);
            p.circle_filled(r.center(), s * 0.1, Color32::WHITE);
            p.circle_filled(at(0.72, 0.28), s * 0.05, Color32::WHITE);
        }
        Icon::Search => {
            p.circle_stroke(at(0.43, 0.43), s * 0.25, st);
            p.line_segment([at(0.62, 0.62), at(0.86, 0.86)], Stroke::new(w * 1.4, c));
        }
        Icon::Plus => {
            p.line_segment([at(0.5, 0.18), at(0.5, 0.82)], Stroke::new(w * 1.3, c));
            p.line_segment([at(0.18, 0.5), at(0.82, 0.5)], Stroke::new(w * 1.3, c));
        }
        Icon::Edit => {
            p.add(Shape::closed_line(vec![at(0.2, 0.68), at(0.62, 0.26), at(0.74, 0.38), at(0.32, 0.8), at(0.18, 0.82)], st));
        }
        Icon::Trash => {
            p.line_segment([at(0.18, 0.28), at(0.82, 0.28)], st);
            p.add(Shape::closed_line(vec![at(0.26, 0.28), at(0.32, 0.86), at(0.68, 0.86), at(0.74, 0.28)], st));
            p.line_segment([at(0.4, 0.16), at(0.6, 0.16)], st);
        }
        Icon::Refresh => {
            let n = 16;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = 0.4 + i as f32 / n as f32 * std::f32::consts::PI * 1.6;
                    r.center() + vec2(a.cos(), a.sin()) * s * 0.32
                })
                .collect();
            let end = pts[0];
            p.add(Shape::line(pts, st));
            p.add(Shape::convex_polygon(vec![end + vec2(-s * 0.1, -s * 0.12), end + vec2(s * 0.1, -s * 0.04), end + vec2(-s * 0.04, s * 0.1)], c, Stroke::NONE));
        }
        Icon::Moon => {
            // Crescent: outer arc on the left, inner arc back on the right.
            let mut pts = Vec::new();
            let n = 16;
            for i in 0..=n {
                let a = (60.0 + 240.0 * i as f32 / n as f32).to_radians();
                pts.push(r.center() + vec2(a.cos(), a.sin()) * s * 0.34);
            }
            let ic = r.center() + vec2(s * 0.12, -s * 0.06);
            for i in 0..=n {
                let a = (275.0 - 190.0 * i as f32 / n as f32).to_radians();
                pts.push(ic + vec2(a.cos(), a.sin()) * s * 0.27);
            }
            p.add(Shape::closed_line(pts, st));
        }
        Icon::Sun => {
            p.circle_filled(r.center(), s * 0.17, c);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([r.center() + d * s * 0.28, r.center() + d * s * 0.4], st);
            }
        }
        Icon::Joystick => {
            p.circle_stroke(r.center(), s * 0.4, st);
            for a in [0.0f32, 90.0, 180.0, 270.0] {
                let d = vec2(a.to_radians().cos(), a.to_radians().sin());
                let n = vec2(-d.y, d.x);
                let tip = r.center() + d * s * 0.33;
                p.add(Shape::convex_polygon(vec![tip, tip - d * s * 0.1 + n * s * 0.08, tip - d * s * 0.1 - n * s * 0.08], c, Stroke::NONE));
            }
            p.circle_filled(r.center(), s * 0.08, c);
        }
        Icon::Tour => {
            let n = 18;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU * 0.85;
                    r.center() + vec2(a.cos() * s * 0.36, a.sin() * s * 0.22)
                })
                .collect();
            let end = *pts.last().unwrap();
            p.add(Shape::line(pts, st));
            p.circle_filled(end, s * 0.07, c);
        }
        Icon::Sidebar => {
            p.rect_stroke(Rect::from_min_max(at(0.12, 0.18), at(0.88, 0.82)), Rounding::same(s * 0.06), st);
            p.line_segment([at(0.38, 0.18), at(0.38, 0.82)], st);
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_min_max(at(0.26, 0.26), at(0.74, 0.74)), Rounding::same(s * 0.06), c);
        }
    }
}

/// Our app logo as an RGBA picture (blue-to-teal rounded tile with an aperture ring),
/// used for the window icon and the Android launcher icons.
pub fn logo_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let radius = s * 0.24;
    let (cx, cy) = (s / 2.0, s / 2.0);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // rounded square coverage
            let m = s * 0.04;
            let dx = (fx - cx).abs() - (s / 2.0 - m - radius);
            let dy = (fy - cy).abs() - (s / 2.0 - m - radius);
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - radius;
            let a = (0.5 - outside).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let t = (fy / s).clamp(0.0, 1.0);
            let top = [0x4Fu8 as f32, 0x7C as f32, 0xFF as f32];
            let bot = [0x19u8 as f32, 0xC3 as f32, 0xB1 as f32];
            let mut c = [0.0f32; 3];
            for i in 0..3 {
                c[i] = top[i] + (bot[i] - top[i]) * t * 0.7;
            }
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let ring = (1.0 - ((d - s * 0.24).abs() - s * 0.045).clamp(0.0, 1.0)).max(0.0);
            let dot = (s * 0.1 + 0.5 - d).clamp(0.0, 1.0);
            let sd = ((fx - s * 0.72).powi(2) + (fy - s * 0.28).powi(2)).sqrt();
            let spark = (s * 0.05 + 0.5 - sd).clamp(0.0, 1.0);
            let w = ring.max(dot).max(spark);
            for i in 0..3 {
                c[i] = c[i] * (1.0 - w) + 255.0 * w;
            }
            let o = ((y * size + x) * 4) as usize;
            out[o] = c[0] as u8;
            out[o + 1] = c[1] as u8;
            out[o + 2] = c[2] as u8;
            out[o + 3] = (a * 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// Writes the Android launcher icons (run with `cargo test --lib write_launcher_icons -- --ignored`).
    #[test]
    #[ignore]
    fn write_launcher_icons() {
        for (dir, px) in [("mdpi", 48), ("hdpi", 72), ("xhdpi", 96), ("xxhdpi", 144), ("xxxhdpi", 192)] {
            let path = format!("res/mipmap-{dir}/ic_launcher.png");
            std::fs::create_dir_all(format!("res/mipmap-{dir}")).unwrap();
            image::save_buffer(&path, &super::logo_rgba(px), px, px, image::ExtendedColorType::Rgba8).unwrap();
        }
        // Windows program icon: every standard size in one .ico (embedded by build.rs).
        std::fs::create_dir_all("assets").unwrap();
        let frames: Vec<image::codecs::ico::IcoFrame> = [16u32, 20, 24, 32, 40, 48, 64, 96, 128, 256]
            .iter()
            .map(|&s| image::codecs::ico::IcoFrame::as_png(&super::logo_rgba(s), s, s, image::ExtendedColorType::Rgba8).unwrap())
            .collect();
        let file = std::fs::File::create("assets/app.ico").unwrap();
        image::codecs::ico::IcoEncoder::new(file).encode_images(&frames).unwrap();
        // macOS app icon (assets/app.icns), used by the Mac app bundle.
        let mut family = icns::IconFamily::new();
        for s in [16u32, 32, 64, 128, 256, 512, 1024] {
            let img = icns::Image::from_data(icns::PixelFormat::RGBA, s, s, super::logo_rgba(s)).unwrap();
            family.add_icon(&img).unwrap();
        }
        family.write(std::fs::File::create("assets/app.icns").unwrap()).unwrap();
        // The TV web app's icon (Hisense VIDAA and other TVs).
        image::save_buffer("vidaa/icon.png", &super::logo_rgba(256), 256, 256, image::ExtendedColorType::Rgba8).unwrap();
    }
}
