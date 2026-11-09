//! Select: custom 32 h secondary trigger (radius.md, 1 px border_strong,
//! padding 12, Lucide chevron-down 16) + themed egui popup list (§2.7).

use egui::{Popup, Rect, Response, Sense, StrokeKind, Ui, WidgetInfo};

use super::{anim, ellipsize_end, focus_ring, mix, paint_icon, paint_text};
use crate::gui::theme::{self, size, space, typ};

pub struct Select<'a> {
    pub a11y: &'a str,
    pub width: f32,
    pub options: &'a [String],
    pub enabled: bool,
    pub mono: bool,
}

impl Select<'_> {
    /// Returns the trigger response and `true` when the selection changed.
    pub fn show(self, ui: &mut Ui, selected: &mut usize) -> (Response, bool) {
        let p = theme::palette(ui);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(theme::v2(self.width, size::CONTROL), sense);
        let h = anim(ui, resp.id.with("h"), self.enabled && resp.hovered());
        let (fill, border, fg) = if self.enabled {
            (mix(p.surface, p.surface_hover, h), p.border_strong, p.text)
        } else {
            (p.surface_sunken, p.border, p.text_disabled)
        };
        ui.painter().rect(
            rect,
            theme::cr(theme::radius::MD),
            fill,
            theme::hairline(border),
            StrokeKind::Inside,
        );
        let text = self.options.get(*selected).cloned().unwrap_or_default();
        let t = if self.mono { typ::MONO } else { typ::BODY };
        let max = rect.width() - space::MD * 2.0 - size::ICON - space::SM;
        let shown = ellipsize_end(ui, &text, t, max);
        paint_text(ui, rect.left() + space::MD, rect.center().y, &shown, t, fg);
        paint_icon(
            ui,
            egui::pos2(rect.right() - space::MD - size::ICON / 2.0, rect.center().y),
            theme::icon::CHEVRON_DOWN,
            size::ICON,
            if self.enabled {
                p.text_muted
            } else {
                p.text_disabled
            },
        );
        focus_ring(ui, &resp, rect, theme::radius::MD, false);
        let label = format!("{}: {text}", self.a11y);
        let en = self.enabled;
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::ComboBox, en, &label));
        let mut changed = false;
        if self.enabled {
            Popup::menu(&resp).width(self.width).show(|ui| {
                ui.set_min_width(self.width - space::SM * 2.0);
                // Long lists scroll at `combo_height` (§2.7).
                egui::ScrollArea::vertical()
                    .max_height(ui.spacing().combo_height)
                    .show(ui, |ui| {
                        for (i, o) in self.options.iter().enumerate() {
                            let r = ui.add_sized(
                                theme::v2(ui.available_width(), size::CONTROL),
                                // Trailing grow atom: options left-aligned like the trigger.
                                egui::Button::selectable(
                                    i == *selected,
                                    (o.as_str(), egui::Atom::grow()),
                                ),
                            );
                            if r.clicked() {
                                *selected = i;
                                changed = true;
                                ui.close();
                            }
                        }
                    });
            });
        }
        (resp, changed)
    }
}

/// Paint-only rect variant used by `show_at` callers (fixed layout).
pub fn trigger_rect(min: egui::Pos2, width: f32) -> Rect {
    Rect::from_min_size(min, theme::v2(width, size::CONTROL))
}
