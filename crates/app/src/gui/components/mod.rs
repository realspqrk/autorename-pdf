//! Custom widgets (spec §5). Every size/color comes from `theme`.
//! All interactive components report `WidgetInfo` for AccessKit (§8).
//! Part B (Settings/Companies/History) uses the remaining components.
#![allow(dead_code, unused_imports)]

pub mod badge;
pub mod banner;
pub mod button;
pub mod dialog;
pub mod drop_zone;
pub mod field;
pub mod plan_row;
pub mod progress;
pub mod segmented;
pub mod select;
pub mod stepper;
pub mod toggle;

use std::sync::Arc;

use egui::epaint::Galley;
use egui::{Align2, Color32, Id, Pos2, Rect, Response, Sense, StrokeKind, Ui, WidgetInfo};

use super::theme::{self, Palette, Type, space, stroke};

pub use badge::{Badge, chip};
pub use banner::Banner;
pub use button::{BSize, Button, Variant};
pub use field::Field;
pub use plan_row::{PlanRow, RowView};
pub use progress::Progress;
pub use segmented::segmented;
pub use select::Select;
pub use stepper::Stepper;
pub use toggle::toggle;

const KBD_ID: &str = "arpdf.keyboard_focus";
const REDUCE_ID: &str = "arpdf.reduce_motion";

/// Keyboard focus rings are shown only after keyboard input (§2.6).
pub fn set_keyboard_mode(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::unique(KBD_ID), on));
}

pub fn keyboard_mode(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.get_temp(Id::unique(KBD_ID)).unwrap_or(false))
}

pub fn set_reduce_motion(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::unique(REDUCE_ID), on));
    ctx.all_styles_mut(|s| {
        s.animation_time = if on { 0.0 } else { theme::ANIMATION_TIME };
        s.scroll_animation = if on {
            egui::style::ScrollAnimation::none()
        } else {
            egui::style::ScrollAnimation::default()
        };
    });
}

pub fn reduce_motion(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.get_temp(Id::unique(REDUCE_ID)).unwrap_or(false))
}

/// Motion token → seconds (0 under reduce motion).
pub fn dur(ctx: &egui::Context, secs: f32) -> f32 {
    if reduce_motion(ctx) { 0.0 } else { secs }
}

/// 0..1 hover/press blend with `motion.fast`.
pub fn anim(ui: &Ui, id: Id, on: bool) -> f32 {
    let t = dur(ui.ctx(), theme::motion::FAST);
    ui.ctx().animate_bool_with_time(id, on, t)
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t)
}

/// Galley laid out with a placeholder color (painted with `fallback`).
pub fn galley(ui: &Ui, text: &str, t: Type) -> Arc<Galley> {
    ui.painter()
        .layout_no_wrap(text.to_string(), t.font(), Color32::PLACEHOLDER)
}

pub fn wrapped(ui: &Ui, text: &str, t: Type, width: f32) -> Arc<Galley> {
    ui.painter()
        .layout(text.to_string(), t.font(), Color32::PLACEHOLDER, width)
}

pub fn text_width(ui: &Ui, text: &str, t: Type) -> f32 {
    galley(ui, text, t).size().x
}

/// Paint `g` with its left edge at `x`, vertically centered on `cy`.
pub fn paint_galley_v(ui: &Ui, x: f32, cy: f32, g: Arc<Galley>, color: Color32) -> Rect {
    let size = g.size();
    let pos = Pos2::new(x, cy - size.y / 2.0).round();
    ui.painter().galley(pos, g, color);
    Rect::from_min_size(pos, size)
}

/// Paint a single line of text; returns its rect.
pub fn paint_text(ui: &Ui, x: f32, cy: f32, text: &str, t: Type, color: Color32) -> Rect {
    paint_galley_v(ui, x, cy, galley(ui, text, t), color)
}

/// Paint text centered in `rect`.
pub fn paint_text_center(ui: &Ui, rect: Rect, text: &str, t: Type, color: Color32) {
    let g = galley(ui, text, t);
    let x = rect.center().x - g.size().x / 2.0;
    paint_galley_v(ui, x, rect.center().y, g, color);
}

/// Lucide glyph centered at `c`.
pub fn paint_icon(ui: &Ui, c: Pos2, glyph: char, px: f32, color: Color32) {
    ui.painter().text(
        c.round(),
        Align2::CENTER_CENTER,
        glyph,
        theme::icon_font(px),
        color,
    );
}

/// Spinner glyph rotated over `motion.SPINNER_LOOP` (static under reduce motion).
pub fn paint_spinner(ui: &Ui, c: Pos2, px: f32, color: Color32) {
    if reduce_motion(ui.ctx()) {
        paint_icon(ui, c, theme::icon::LOADER_CIRCLE, px, color);
        return;
    }
    let t = ui.input(|i| i.time) as f32;
    let angle = (t / theme::motion::SPINNER_LOOP).fract() * std::f32::consts::TAU;
    let g = ui.painter().layout_no_wrap(
        theme::icon::LOADER_CIRCLE.to_string(),
        theme::icon_font(px),
        color,
    );
    let half = g.size() / 2.0;
    let rot = egui::emath::Rot2::from_angle(angle);
    let pos = c - rot * half;
    let mut shape = egui::epaint::TextShape::new(pos, g, color);
    shape.angle = angle;
    ui.painter().add(shape);
    ui.ctx().request_repaint();
}

/// §2.6 focus ring: 2 px `focus` outside the rect with a 2 px gap (or inset
/// for rows inside scroll areas). Keyboard focus only.
pub fn focus_ring(ui: &Ui, resp: &Response, rect: Rect, radius: u8, inset: bool) {
    if !(resp.has_focus() && keyboard_mode(ui.ctx())) {
        return;
    }
    let p = theme::palette(ui);
    let (r, rr) = if inset {
        (
            rect.shrink(space::XXS),
            radius.saturating_sub(space::XXS as u8),
        )
    } else {
        (
            rect.expand(space::XXS + stroke::FOCUS),
            radius.saturating_add((space::XXS + stroke::FOCUS) as u8),
        )
    };
    ui.painter().rect_stroke(
        r,
        theme::cr(rr),
        egui::Stroke::new(stroke::FOCUS, p.focus),
        StrokeKind::Inside,
    );
}

/// Middle ellipsis so `text` fits `max_w` (§4.3 long names).
pub fn ellipsize_middle(ui: &Ui, text: &str, t: Type, max_w: f32) -> String {
    if text_width(ui, text, t) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    let make = |keep: usize| {
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let mut s: String = chars[..head].iter().collect();
        s.push('…');
        s.extend(&chars[chars.len() - tail..]);
        s
    };
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if text_width(ui, &make(mid), t) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    make(lo)
}

/// End ellipsis for prose (warnings, status copy).
pub fn ellipsize_end(ui: &Ui, text: &str, t: Type, max_w: f32) -> String {
    if text_width(ui, text, t) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    let make = |keep: usize| {
        let mut s: String = chars[..keep].iter().collect();
        s = s.trim_end().to_string();
        s.push('…');
        s
    };
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if text_width(ui, &make(mid), t) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    make(lo)
}

/// Checkbox: 16 px visual centered in a 32 px hit square (§2.2).
pub fn checkbox(ui: &mut Ui, checked: bool, enabled: bool, label: &str) -> Response {
    let s = theme::size::CONTROL;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(theme::v2(s, s), sense);
    paint_checkbox(ui, &resp, rect, checked, enabled);
    resp.widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, enabled, checked, label));
    resp
}

pub fn paint_checkbox(ui: &Ui, resp: &Response, rect: Rect, checked: bool, enabled: bool) {
    let p = theme::palette(ui);
    let bx = Rect::from_center_size(
        rect.center(),
        theme::v2(theme::size::CHECKBOX, theme::size::CHECKBOX),
    );
    let hover = anim(ui, resp.id.with("h"), enabled && resp.hovered());
    let (fill, st) = if !enabled {
        (p.surface_sunken, p.text_disabled)
    } else if checked {
        (
            mix(p.accent, p.accent_hover, hover),
            mix(p.accent, p.accent_hover, hover),
        )
    } else {
        (mix(p.surface, p.surface_hover, hover), p.border_strong)
    };
    ui.painter().rect(
        bx,
        theme::cr(theme::radius::SM),
        fill,
        theme::hairline(st),
        StrokeKind::Inside,
    );
    if checked {
        let c = if enabled {
            p.on_accent
        } else {
            p.text_disabled
        };
        paint_icon(ui, bx.center(), theme::icon::CHECK, theme::size::ICON_SM, c);
    }
    focus_ring(ui, resp, bx, theme::radius::SM, false);
}

/// Section header: caption_strong muted + 1 px rule (§4.4).
pub fn section_header(ui: &mut Ui, title: &str) {
    let p = theme::palette(ui);
    let h = theme::typ::CAPTION_STRONG.line;
    let (rect, _) = ui.allocate_exact_size(theme::v2(ui.available_width(), h), Sense::hover());
    let tr = paint_text(
        ui,
        rect.left(),
        rect.center().y,
        title,
        theme::typ::CAPTION_STRONG,
        p.text_muted,
    );
    let x0 = tr.right() + space::SM;
    if x0 < rect.right() {
        ui.painter().hline(
            x0..=rect.right(),
            rect.center().y.round() + 0.5,
            theme::hairline(p.border),
        );
    }
}

/// Fixed-height single-line label (line height from the type token).
pub fn label(ui: &mut Ui, text: &str, t: Type, color: Color32) -> Response {
    let g = galley(ui, text, t);
    // Width on the 4-pt grid so following widgets stay aligned.
    let w = ceil4(g.size().x);
    let (rect, resp) = ui.allocate_exact_size(theme::v2(w, t.line), Sense::hover());
    paint_galley_v(ui, rect.left(), rect.center().y, g, color);
    resp
}

/// Wrapped paragraph capped at the prose measure (§2.3).
pub fn paragraph(ui: &mut Ui, text: &str, t: Type, color: Color32) -> Response {
    let w = ui.available_width().min(theme::size::PROSE_MAX);
    let g = wrapped(ui, text, t, w);
    let lines = g.rows.len().max(1) as f32;
    let (rect, resp) = ui.allocate_exact_size(theme::v2(w, lines * t.line), Sense::hover());
    // Rows use font metrics; spread them on the token line height.
    for (i, row) in g.rows.iter().enumerate() {
        let cy = rect.top() + t.line * (i as f32 + 0.5);
        let rg = ui
            .painter()
            .layout_no_wrap(row.text(), t.font(), Color32::PLACEHOLDER);
        paint_galley_v(ui, rect.left(), cy, rg, color);
    }
    resp
}

/// Wrapped text laid on the token line height; returns the line count.
pub fn paint_wrapped(ui: &Ui, min: Pos2, width: f32, text: &str, t: Type, color: Color32) -> usize {
    let g = wrapped(ui, text, t, width);
    for (i, row) in g.rows.iter().enumerate() {
        let cy = min.y + t.line * (i as f32 + 0.5);
        paint_text(ui, min.x, cy, &row.text(), t, color);
    }
    g.rows.len().max(1)
}

/// Greedy line breaking at " · " separators (summary lines never split a
/// part like "0 errors" across lines).
pub fn part_lines(ui: &Ui, text: &str, t: Type, width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for part in text.split(" · ") {
        match lines.last_mut() {
            Some(l) if text_width(ui, &format!("{l} · {part}"), t) <= width => {
                l.push_str(" · ");
                l.push_str(part);
            }
            _ => lines.push(part.to_string()),
        }
    }
    lines
}

pub fn wrapped_lines(ui: &Ui, text: &str, t: Type, width: f32) -> usize {
    wrapped(ui, text, t, width).rows.len().max(1)
}

/// Paint a card (radius.lg, elev.1) behind `rect`.
pub fn paint_card(ui: &Ui, rect: Rect, p: &Palette) {
    let fill = p.elevated(1);
    ui.painter()
        .add(p.shadow(1).as_shape(rect, theme::cr(theme::radius::LG)));
    let st = if p.dark {
        p.border
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(
        rect,
        theme::cr(theme::radius::LG),
        fill,
        theme::hairline(st),
        StrokeKind::Inside,
    );
}

/// Run `f` in a child `Ui` confined to `rect`.
pub fn in_rect<R>(ui: &mut Ui, rect: Rect, f: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    // Focus rings sit 4 px outside widgets; keep them unclipped (§2.6).
    let ring = space::XXS + stroke::FOCUS;
    child.set_clip_rect(rect.expand(ring).intersect(ui.clip_rect()));
    f(&mut child)
}

/// Body of a collapsible section, clipped to `openness` (0..1, animated
/// by the caller with `motion.normal`) of its last measured height.
/// Shared by Settings › Advanced and the detail pane sections.
pub fn reveal(ui: &mut Ui, id: egui::Id, openness: f32, body: impl FnOnce(&mut Ui)) {
    if openness <= 0.0 {
        return;
    }
    let key = id.with("reveal_h");
    if openness >= 1.0 {
        let top = ui.cursor().top();
        body(ui);
        let used = ui.cursor().top() - top;
        ui.ctx().data_mut(|d| d.insert_temp(key, used));
        return;
    }
    let w = ui.available_width();
    let full = ui
        .ctx()
        .data_mut(|d| d.get_temp::<f32>(key))
        .unwrap_or(theme::size::ROW);
    let h = (full * openness).round();
    let (cr, _) = ui.allocate_exact_size(theme::v2(w, h), Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(cr.min, theme::v2(w, f32::INFINITY)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(cr.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing = ui.spacing().item_spacing;
    body(&mut child);
    let used = child.min_rect().height();
    ui.ctx().data_mut(|d| d.insert_temp(key, used));
}

/// Round up to the 4-pt grid.
pub fn ceil4(v: f32) -> f32 {
    (v / theme::GRID4).ceil() * theme::GRID4
}

/// Pluralize "1 file" / "2 files".
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
