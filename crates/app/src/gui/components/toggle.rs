//! §5.5 Toggle: 36×20 track, 16 thumb, hit = toggle + label row (32 h).

use egui::{Rect, Response, Sense, StrokeKind, Ui, WidgetInfo};

use super::{dur, focus_ring, galley, paint_galley_v};
use crate::gui::theme::{self, motion, size, space};

pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let p = theme::palette(ui);
    let g = galley(ui, label, theme::typ::BODY);
    let w = size::TOGGLE_W + space::SM + g.size().x.ceil();
    let (rect, mut resp) = ui.allocate_exact_size(theme::v2(w, size::CONTROL), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let track = Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - size::TOGGLE_H / 2.0),
        theme::v2(size::TOGGLE_W, size::TOGGLE_H),
    );
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, *on, dur(ui.ctx(), motion::FAST));
    let (fill, st, thumb) = if *on {
        (p.accent, egui::Color32::TRANSPARENT, p.on_accent)
    } else {
        (p.surface_sunken, p.border_strong, p.border_strong)
    };
    ui.painter().rect(
        track,
        theme::cr(theme::radius::FULL),
        fill,
        theme::hairline(st),
        StrokeKind::Inside,
    );
    let r = size::TOGGLE_THUMB / 2.0;
    let x0 = track.left() + space::XXS + r;
    let x1 = track.right() - space::XXS - r;
    ui.painter()
        .circle_filled(egui::pos2(x0 + (x1 - x0) * t, track.center().y), r, thumb);
    paint_galley_v(ui, track.right() + space::SM, rect.center().y, g, p.text);
    focus_ring(ui, &resp, track, theme::radius::FULL, false);
    let v = *on;
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Switch, true, v, label));
    resp
}
