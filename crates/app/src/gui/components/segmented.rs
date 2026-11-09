//! §5.4 Segmented control: 32 h sunken container, 28 h segments, ←/→/Home/End.

use egui::{Key, Rect, Response, Sense, Ui, WidgetInfo};

use super::{dur, focus_ring, galley, paint_galley_v};
use crate::gui::theme::{self, motion, size, space};

/// Returns the container response; `selected` is updated in place.
pub fn segmented(ui: &mut Ui, id: &str, options: &[String], selected: &mut usize) -> Response {
    let p = theme::palette(ui);
    let widths: Vec<f32> = options
        .iter()
        .map(|o| {
            let w = galley(ui, o, theme::typ::BODY_STRONG).size().x + space::MD * 2.0;
            (w / theme::GRID4).ceil() * theme::GRID4
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + space::XXS * 2.0;
    let (rect, outer) = ui.allocate_exact_size(theme::v2(total, size::CONTROL), Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::cr(theme::radius::MD), p.surface_sunken);
    let base = ui.scope_id().with(id);
    let mut x = rect.left() + space::XXS;
    let mut rects = Vec::with_capacity(options.len());
    for w in &widths {
        rects.push(Rect::from_min_size(
            egui::pos2(x, rect.top() + space::XXS),
            theme::v2(*w, size::SEGMENT),
        ));
        x += w;
    }
    // Sliding indicator.
    let t = dur(ui.ctx(), motion::NORMAL);
    let cur = rects.get(*selected).copied().unwrap_or(rects[0]);
    let ix = ui
        .ctx()
        .animate_value_with_time(base.with("ix"), cur.left(), t);
    let iw = ui
        .ctx()
        .animate_value_with_time(base.with("iw"), cur.width(), t);
    let ind = Rect::from_min_size(egui::pos2(ix, cur.top()), theme::v2(iw, cur.height()));
    let inner_r = theme::cr(theme::radius::MD - space::XXS as u8);
    ui.painter().add(p.shadow(1).as_shape(ind, inner_r));
    ui.painter().rect_filled(ind, inner_r, p.elevated(1));
    let mut new_sel = *selected;
    let mut focus_moved = false;
    for (i, (opt, r)) in options.iter().zip(&rects).enumerate() {
        let resp = ui.interact(*r, base.with(i), Sense::click());
        let is_sel = i == *selected;
        if resp.clicked() {
            new_sel = i;
        }
        if resp.has_focus() {
            let n = options.len();
            let (l, rgt, home, end) = ui.input(|inp| {
                (
                    inp.key_pressed(Key::ArrowLeft),
                    inp.key_pressed(Key::ArrowRight),
                    inp.key_pressed(Key::Home),
                    inp.key_pressed(Key::End),
                )
            });
            let target = if l {
                Some((i + n - 1) % n)
            } else if rgt {
                Some((i + 1) % n)
            } else if home {
                Some(0)
            } else if end {
                Some(n - 1)
            } else {
                None
            };
            if let Some(tg) = target {
                new_sel = tg;
                focus_moved = true;
            }
        }
        let color = if is_sel || resp.hovered() {
            p.text
        } else {
            p.text_muted
        };
        let ty = if is_sel {
            theme::typ::BODY_STRONG
        } else {
            theme::typ::BODY
        };
        let g = galley(ui, opt, ty);
        let gx = r.center().x - g.size().x / 2.0;
        paint_galley_v(ui, gx, r.center().y, g, color);
        focus_ring(ui, &resp, *r, theme::radius::MD - space::XXS as u8, true);
        resp.widget_info(|| WidgetInfo::selected(egui::Role::RadioButton, true, is_sel, opt));
    }
    if focus_moved {
        ui.memory_mut(|m| m.request_focus(base.with(new_sel)));
    }
    *selected = new_sel;
    outer.widget_info(|| WidgetInfo::labeled(egui::Role::RadioGroup, true, id));
    outer
}
