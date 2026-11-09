//! §5.9 Progress: 4 h track, accent fill, determinate or indeterminate.

use egui::{Rect, Response, Sense, Ui, WidgetInfo};

use super::{dur, reduce_motion};
use crate::gui::theme::{self, motion};

pub struct Progress<'a> {
    /// `None` = indeterminate.
    pub fraction: Option<f32>,
    pub label: &'a str,
}

impl Progress<'_> {
    pub fn show_at(self, ui: &mut Ui, rect: Rect) -> Response {
        let p = theme::palette(ui);
        let resp = ui.interact(
            rect,
            ui.scope_id().with("progress").with(self.label),
            Sense::hover(),
        );
        let r = theme::cr(theme::radius::FULL);
        ui.painter().rect_filled(rect, r, p.surface_sunken);
        match self.fraction {
            Some(f) => {
                let t = dur(ui.ctx(), motion::NORMAL);
                let f = ui
                    .ctx()
                    .animate_value_with_time(resp.id, f.clamp(0.0, 1.0), t);
                let fill =
                    Rect::from_min_size(rect.min, theme::v2(rect.width() * f, rect.height()));
                ui.painter().rect_filled(fill, r, p.accent);
            }
            None if reduce_motion(ui.ctx()) => {
                let fill = Rect::from_min_size(
                    rect.min,
                    theme::v2(rect.width() * motion::REDUCED_FILL, rect.height()),
                );
                ui.painter().rect_filled(fill, r, p.accent);
            }
            None => {
                let t = (ui.input(|i| i.time) as f32 / motion::INDETERMINATE_LOOP).fract();
                let seg = rect.width() * motion::INDETERMINATE_SEGMENT;
                let x = rect.left() - seg + (rect.width() + seg) * t;
                let fill = Rect::from_min_max(
                    egui::pos2(x.max(rect.left()), rect.top()),
                    egui::pos2((x + seg).min(rect.right()), rect.bottom()),
                );
                ui.painter().rect_filled(fill, r, p.accent);
                ui.ctx().request_repaint();
            }
        }
        let label = self.label.to_string();
        let value = self.fraction.map(f64::from);
        // Polite live region (§4.5); the label only changes per finished file.
        ui.ctx().accesskit_node_builder(resp.id, |n| {
            n.set_live(egui::accesskit::Live::Polite);
        });
        resp.widget_info(|| {
            let mut i = WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &label);
            i.value = value;
            i
        });
        resp
    }
}
