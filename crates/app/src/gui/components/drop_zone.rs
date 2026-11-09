//! §5.10 Drop zone: radius.xl, dashed border_strong, states, Enter = choose files.

use egui::{Key, Pos2, Rect, Response, Sense, Shape, Ui, WidgetInfo};

use super::button::Button;
use super::{ellipsize_end, focus_ring, galley, paint_galley_v, paint_icon};
use crate::gui::theme::{self, size, space, stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropState {
    Rest,
    /// Drag over with this many PDFs/folders.
    Valid(usize),
    Invalid,
    Disabled,
}

#[derive(Default)]
pub struct DropResponse {
    pub choose_files: bool,
    pub choose_folder: bool,
}

fn dashed_rounded(rect: Rect, r: f32) -> Vec<Pos2> {
    // Rounded rectangle path approximated with quarter-circle segments.
    let mut pts = Vec::new();
    let steps = 8;
    let corners = [
        (rect.right() - r, rect.top() + r, -90.0f32),
        (rect.right() - r, rect.bottom() - r, 0.0),
        (rect.left() + r, rect.bottom() - r, 90.0),
        (rect.left() + r, rect.top() + r, 180.0),
    ];
    for (cx, cy, start) in corners {
        for i in 0..=steps {
            let a = (start + 90.0 * i as f32 / steps as f32).to_radians();
            pts.push(Pos2::new(cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    pts.push(pts[0]);
    pts
}

/// Full form (≥ `DROP_MIN_H`), centered stack: icon 48 · 16 · title 24 ·
/// 4 · caption 16 · 24 · buttons 32. Returns the two button origins.
fn stacked_layout(
    ui: &Ui,
    rect: Rect,
    title: &str,
    sub: &str,
    state: DropState,
    (w1, w2): (f32, f32),
) -> (Pos2, Pos2) {
    let buttons_w = w1 + space::SM + w2;
    let p = theme::palette(ui);
    let block = size::ICON_XL
        + space::LG
        + theme::typ::TITLE_SM.line
        + space::XS
        + theme::typ::CAPTION.line
        + space::XL
        + size::CONTROL;
    let mut y = theme::snap4(rect.center().y - block / 2.0);
    let cx = rect.center().x;
    let icon_c = if state == DropState::Invalid {
        p.danger
    } else {
        p.text_muted
    };
    paint_icon(
        ui,
        egui::pos2(cx, y + size::ICON_XL / 2.0),
        theme::icon::FILE_UP,
        size::ICON_XL,
        icon_c,
    );
    y += size::ICON_XL + space::LG;
    let g = galley(ui, title, theme::typ::TITLE_SM);
    paint_galley_v(
        ui,
        cx - g.size().x / 2.0,
        y + theme::typ::TITLE_SM.line / 2.0,
        g,
        p.text,
    );
    y += theme::typ::TITLE_SM.line + space::XS;
    let g = galley(ui, sub, theme::typ::CAPTION);
    paint_galley_v(
        ui,
        cx - g.size().x / 2.0,
        y + theme::typ::CAPTION.line / 2.0,
        g,
        p.text_muted,
    );
    y += theme::typ::CAPTION.line + space::XL;
    let x0 = theme::snap4(cx - buttons_w / 2.0);
    (egui::pos2(x0, y), egui::pos2(x0 + w1 + space::SM, y))
}

/// Compact form (`DROP_COMPACT_H`, below `DROP_MIN_H`): one row, padding
/// 24, title + caption left (ellipsized), buttons right, all centered
/// vertically. Used while the setup card needs the height.
fn compact_layout(
    ui: &Ui,
    rect: Rect,
    title: &str,
    sub: &str,
    (w1, w2): (f32, f32),
) -> (Pos2, Pos2) {
    let buttons_w = w1 + space::SM + w2;
    let p = theme::palette(ui);
    let inner = rect.shrink2(theme::v2(space::XL, 0.0));
    let text_w = (inner.width() - buttons_w - space::LG).max(0.0);
    let block = theme::typ::TITLE_SM.line + space::XS + theme::typ::CAPTION.line;
    let top = rect.center().y - block / 2.0;
    let t = ellipsize_end(ui, title, theme::typ::TITLE_SM, text_w);
    paint_galley_v(
        ui,
        inner.left(),
        top + theme::typ::TITLE_SM.line / 2.0,
        galley(ui, &t, theme::typ::TITLE_SM),
        p.text,
    );
    let s = ellipsize_end(ui, sub, theme::typ::CAPTION, text_w);
    paint_galley_v(
        ui,
        inner.left(),
        top + theme::typ::TITLE_SM.line + space::XS + theme::typ::CAPTION.line / 2.0,
        galley(ui, &s, theme::typ::CAPTION),
        p.text_muted,
    );
    let y = theme::snap4(rect.center().y - size::CONTROL / 2.0);
    let x0 = inner.right() - buttons_w;
    (egui::pos2(x0, y), egui::pos2(x0 + w1 + space::SM, y))
}

pub fn drop_zone(ui: &mut Ui, rect: Rect, state: DropState) -> (Response, DropResponse) {
    let p = theme::palette(ui);
    let enabled = state != DropState::Disabled;
    let resp = ui.interact(
        rect,
        ui.scope_id().with("drop_zone"),
        if enabled {
            Sense::focusable_noninteractive()
        } else {
            Sense::hover()
        },
    );
    let rr = f32::from(theme::radius::XL);
    let (fill, border, solid) = match state {
        DropState::Valid(_) => (p.accent_subtle, p.accent, true),
        DropState::Invalid => (p.danger_subtle, p.danger, true),
        DropState::Rest | DropState::Disabled => (p.surface, p.border_strong, false),
    };
    ui.painter()
        .rect_filled(rect, theme::cr(theme::radius::XL), fill);
    let inner = rect.shrink(stroke::HAIRLINE / 2.0);
    if solid {
        ui.painter().rect_stroke(
            rect,
            theme::cr(theme::radius::XL),
            egui::Stroke::new(stroke::FOCUS, border),
            egui::StrokeKind::Inside,
        );
    } else {
        ui.painter().extend(Shape::dashed_line(
            &dashed_rounded(inner, rr),
            theme::hairline(border),
            stroke::DASH,
            stroke::DASH_GAP,
        ));
    }
    focus_ring(ui, &resp, rect, theme::radius::XL, true);

    let (title, sub) = match state {
        DropState::Valid(n) => (
            format!("Drop to add {}", super::plural(n, "item", "items")),
            "Nothing is renamed until you review the plan.".to_string(),
        ),
        DropState::Invalid => (
            "Only PDFs and folders".to_string(),
            "Other file types are ignored.".to_string(),
        ),
        DropState::Disabled => (
            "Analysis in progress".to_string(),
            "You can add more files when it finishes.".to_string(),
        ),
        DropState::Rest => (
            "Drop PDFs or folders here".to_string(),
            "Nothing is renamed until you review the plan.".to_string(),
        ),
    };
    let b1 = Button::secondary("Choose files…")
        .icon(theme::icon::FILE_TEXT)
        .enabled(enabled)
        .a11y("Choose files…");
    let b2 = Button::secondary("Choose folder…")
        .icon(theme::icon::FOLDER_OPEN)
        .enabled(enabled);
    let (w1, w2) = (b1.width(ui), b2.width(ui));
    let (r1, r2) = if rect.height() < size::DROP_MIN_H {
        compact_layout(ui, rect, &title, &sub, (w1, w2))
    } else {
        stacked_layout(ui, rect, &title, &sub, state, (w1, w2))
    };
    let r1 = Rect::from_min_size(r1, theme::v2(w1, size::CONTROL));
    let r2 = Rect::from_min_size(r2, theme::v2(w2, size::CONTROL));
    let mut out = DropResponse {
        choose_files: b1.show_at(ui, r1).clicked(),
        choose_folder: b2.show_at(ui, r2).clicked(),
    };
    if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        out.choose_files = true;
    }
    let label = format!("{title}. {sub} Press Enter to choose files.");
    resp.widget_info(|| WidgetInfo::labeled(egui::Role::Group, enabled, &label));
    (resp, out)
}
