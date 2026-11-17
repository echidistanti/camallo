//! Look of the window: colours, text sizes and the few widgets egui does not draw the way this app wants.
//! Icons are drawn with shapes, so they do not depend on which glyphs a font has.

use eframe::egui::{
    self, vec2, Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, TextStyle, Ui,
};

pub const BG: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x1e);
pub const PANEL: Color32 = Color32::from_rgb(0x22, 0x25, 0x27);
pub const CARD: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x30);
pub const FIELD: Color32 = Color32::from_rgb(0x34, 0x38, 0x3b);
pub const LINE: Color32 = Color32::from_rgb(0x3b, 0x40, 0x44);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xec);
pub const MUTED: Color32 = Color32::from_rgb(0x9b, 0xa1, 0xa7);
pub const ACCENT: Color32 = Color32::from_rgb(0x2f, 0xd0, 0x6a);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x1e, 0x3a, 0x2a);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x08, 0x1c, 0x0f);
pub const DANGER: Color32 = Color32::from_rgb(0xf0, 0x5d, 0x5d);
pub const DANGER_DIM: Color32 = Color32::from_rgb(0x41, 0x25, 0x28);

const HOVER: Color32 = Color32::from_rgb(0x42, 0x47, 0x4c);
const PRESSED: Color32 = Color32::from_rgb(0x4c, 0x52, 0x58);

/// Dark theme with one accent colour, rounded shapes and a bit more air than egui's default.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        let v = &mut s.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = CARD;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT_DIM;
        v.selection.stroke = Stroke::new(1.0, ACCENT);

        let r = CornerRadius::same(9);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;
        w.inactive.bg_fill = FIELD;
        w.inactive.weak_bg_fill = FIELD;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;
        w.hovered.bg_fill = HOVER;
        w.hovered.weak_bg_fill = HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, MUTED);
        w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.hovered.corner_radius = r;
        w.active.bg_fill = PRESSED;
        w.active.weak_bg_fill = PRESSED;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.active.corner_radius = r;
        w.open.bg_fill = FIELD;
        w.open.weak_bg_fill = FIELD;
        w.open.bg_stroke = Stroke::new(1.0, MUTED);
        w.open.fg_stroke = Stroke::new(1.0, TEXT);
        w.open.corner_radius = r;

        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.combo_width = 90.0;

        use egui::FontFamily::{Monospace, Proportional};
        s.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, Proportional)),
            (TextStyle::Body, FontId::new(14.5, Proportional)),
            (TextStyle::Button, FontId::new(14.5, Proportional)),
            (TextStyle::Small, FontId::new(12.0, Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, Monospace)),
        ]
        .into();
    });
}

/// Windows ships Segoe UI, which reads better than the bundled font; the bundled ones stay as fallback.
#[cfg_attr(not(windows), allow(unused_variables))]
pub fn fonts(ctx: &egui::Context) {
    #[cfg(windows)]
    {
        use std::sync::Arc;
        let dir = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "C:\\Windows".into())
            .join("Fonts");
        let mut defs = egui::FontDefinitions::default();
        let mut found = false;
        if let Ok(bytes) = std::fs::read(dir.join("segoeui.ttf")) {
            defs.font_data.insert("segoeui".into(), Arc::new(egui::FontData::from_owned(bytes)));
            defs.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "segoeui".into());
            found = true;
        }
        if let Ok(bytes) = std::fs::read(dir.join("seguisym.ttf")) {
            defs.font_data.insert("seguisym".into(), Arc::new(egui::FontData::from_owned(bytes)));
            defs.families.entry(egui::FontFamily::Proportional).or_default().push("seguisym".into());
            found = true;
        }
        if found {
            ctx.set_fonts(defs);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// The one action of the moment.
    Primary,
    Soft,
    /// No background until hovered.
    Ghost,
    /// A choice that is on.
    Selected,
}

impl Kind {
    /// Text colour, fill, fill when hovered, outline.
    fn colors(self) -> (Color32, Color32, Color32, Stroke) {
        match self {
            Kind::Primary => (ON_ACCENT, ACCENT, Color32::from_rgb(0x4a, 0xe0, 0x82), Stroke::NONE),
            Kind::Soft => (TEXT, FIELD, HOVER, Stroke::NONE),
            Kind::Ghost => (MUTED, Color32::TRANSPARENT, FIELD, Stroke::NONE),
            Kind::Selected => (ACCENT, ACCENT_DIM, Color32::from_rgb(0x25, 0x4a, 0x34), Stroke::new(1.0, ACCENT)),
        }
    }
}

/// A rounded button. `size` is the text size, so the main action can be bigger.
pub fn pill(ui: &mut Ui, text: &str, kind: Kind, size: f32) -> Response {
    let (fg, fill, hover, stroke) = kind.colors();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(size), fg);
    let pad = vec2(size * 0.95, size * 0.5);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let bg = if !enabled {
            fill.gamma_multiply(0.45)
        } else if resp.is_pointer_button_down_on() {
            hover.gamma_multiply(0.85)
        } else if resp.hovered() {
            hover
        } else {
            fill
        };
        let text_color = if kind == Kind::Ghost && resp.hovered() { TEXT } else { fg };
        ui.painter().rect(rect, CornerRadius::same(255), bg, stroke, StrokeKind::Inside);
        let at = rect.center() - galley.size() / 2.0;
        let text_color = if enabled { text_color } else { text_color.gamma_multiply(0.5) };
        ui.painter().galley_with_override_text_color(at, galley, text_color);
    }
    if resp.hovered() && ui.is_enabled() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Small button for the usual size of text.
pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    pill(ui, text, kind, 14.0)
}

/// A rounded box for one item of a list.
pub fn card<R>(ui: &mut Ui, fill: Color32, outline: Stroke, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(fill)
        .stroke(outline)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A short label on a dark capsule: format, site, size.
pub fn chip(ui: &mut Ui, text: &str) {
    egui::Frame::new().fill(FIELD).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(12.0).color(MUTED));
    });
}

/// The state of a download as a round picture.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Badge {
    Queued,
    /// 0 to 100.
    Running(f32),
    Done,
    Failed,
    Cancelled,
    /// A video was found.
    Play,
}

pub const BADGE: f32 = 52.0;

pub fn badge(ui: &mut Ui, b: Badge) {
    let (rect, _) = ui.allocate_exact_size(vec2(BADGE, BADGE), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    let r = BADGE / 2.0 - 3.0;
    let ink = |color: Color32, w: f32| Stroke::new(w, color);
    match b {
        Badge::Queued => {
            p.circle_stroke(c, r, ink(LINE, 4.0));
            for dx in [-8.0, 0.0, 8.0] {
                p.circle_filled(c + vec2(dx, 0.0), 2.2, MUTED);
            }
        }
        Badge::Running(pct) => {
            p.circle_stroke(c, r, ink(LINE, 4.0));
            let sweep = arc(c, r, pct / 100.0);
            if sweep.len() > 1 {
                p.add(Shape::line(sweep, ink(ACCENT, 4.0)));
            }
            p.text(c, Align2::CENTER_CENTER, format!("{}%", pct.round() as u32), FontId::proportional(13.0), TEXT);
        }
        Badge::Done => {
            p.circle_filled(c, r + 1.0, ACCENT);
            p.add(Shape::line(
                vec![c + vec2(-9.0, 0.5), c + vec2(-2.5, 7.0), c + vec2(9.5, -7.0)],
                ink(ON_ACCENT, 3.5),
            ));
        }
        Badge::Failed => {
            p.circle_filled(c, r + 1.0, DANGER_DIM);
            p.circle_stroke(c, r, ink(DANGER, 2.0));
            p.line_segment([c + vec2(0.0, -9.0), c + vec2(0.0, 2.0)], ink(DANGER, 3.5));
            p.circle_filled(c + vec2(0.0, 9.0), 2.2, DANGER);
        }
        Badge::Cancelled => {
            p.circle_stroke(c, r, ink(LINE, 2.0));
            cross_lines(p, c, 7.0, ink(MUTED, 3.0));
        }
        Badge::Play => {
            p.circle_filled(c, r + 1.0, ACCENT);
            p.add(Shape::convex_polygon(
                vec![c + vec2(-6.0, -10.0), c + vec2(-6.0, 10.0), c + vec2(10.0, 0.0)],
                ON_ACCENT,
                Stroke::NONE,
            ));
        }
    }
}

/// Points of an arc that starts at the top and goes clockwise, `part` of a full circle (0 to 1).
pub fn arc(c: Pos2, r: f32, part: f32) -> Vec<Pos2> {
    let part = part.clamp(0.0, 1.0);
    let steps = ((part * 64.0).ceil() as usize).max(1);
    if part <= 0.0 {
        return vec![];
    }
    (0..=steps)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * part * (i as f32 / steps as f32);
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

fn cross_lines(p: &egui::Painter, c: Pos2, d: f32, stroke: Stroke) {
    p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
    p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
}

/// A round "x" to remove an item from the list.
pub fn close(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered();
        if hot {
            ui.painter().circle_filled(rect.center(), 13.0, FIELD);
        }
        cross_lines(ui.painter(), rect.center(), 4.5, Stroke::new(1.6, if hot { TEXT } else { MUTED }));
        ui.ctx().set_cursor_icon(if hot { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    }
    resp
}

/// Thin bar under the title of a running download.
pub fn bar(ui: &mut Ui, pct: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let r = CornerRadius::same(3);
    ui.painter().rect_filled(rect, r, LINE);
    let w = rect.width() * (pct / 100.0).clamp(0.0, 1.0);
    if w > 0.5 {
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(w.max(6.0), rect.height())), r, ACCENT);
    }
}

/// Big picture for the empty list: a circle with an arrow going down into a tray.
pub fn empty_icon(ui: &mut Ui) {
    let size = 76.0;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    p.circle_stroke(c, size / 2.0 - 2.0, Stroke::new(3.0, LINE));
    let ink = Stroke::new(3.5, MUTED);
    p.line_segment([c + vec2(0.0, -16.0), c + vec2(0.0, 8.0)], ink);
    p.add(Shape::line(vec![c + vec2(-10.0, -2.0), c + vec2(0.0, 9.0), c + vec2(10.0, -2.0)], ink));
    p.add(Shape::line(vec![c + vec2(-14.0, 13.0), c + vec2(-14.0, 18.0), c + vec2(14.0, 18.0), c + vec2(14.0, 13.0)], ink));
}

/// Site name of an address, for the line under the title.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    #[test]
    fn host_is_the_site_without_noise() {
        assert_eq!(host_of("https://www.youtube.com/watch?v=abc"), "youtube.com");
        assert_eq!(host_of("http://user:pw@cdn.site.test:8080/a/b.m3u8"), "cdn.site.test");
        assert_eq!(host_of("https://vimeo.com"), "vimeo.com");
        assert_eq!(host_of("https://a.test?x=1"), "a.test");
        assert_eq!(host_of("not a url"), "not a url");
    }

    #[test]
    fn arc_starts_at_the_top_and_closes_at_the_end() {
        let c = pos2(10.0, 10.0);
        assert!(arc(c, 5.0, 0.0).is_empty());
        let quarter = arc(c, 5.0, 0.25);
        let (a, b) = (quarter[0], *quarter.last().unwrap());
        assert!((a.x - 10.0).abs() < 1e-4 && (a.y - 5.0).abs() < 1e-4, "starts at the top: {a:?}");
        assert!((b.x - 15.0).abs() < 1e-4 && (b.y - 10.0).abs() < 1e-4, "a quarter turn ends on the right: {b:?}");
        let full = arc(c, 5.0, 2.0);
        assert!(full.len() > 60, "values above one are clamped, not wrapped");
        let (f, l) = (full[0], *full.last().unwrap());
        assert!((f.x - l.x).abs() < 1e-3 && (f.y - l.y).abs() < 1e-3);
    }
}
