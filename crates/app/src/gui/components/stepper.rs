//! Number stepper: [−] value [+] (32 h icon buttons around a 64 px value
//! box, mono). The value box is a SpinButton node with the value.

use egui::{Sense, Ui, WidgetInfo};

use super::{Button, paint_text_center};
use crate::gui::theme::{self, size, space, typ};

pub struct Stepper<'a> {
    label: &'a str,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    enabled: bool,
}

impl<'a> Stepper<'a> {
    /// `label` names the value for AccessKit ("Pages read per PDF").
    pub fn new(label: &'a str, (min, max, step): (f64, f64, f64)) -> Self {
        Self {
            label,
            min,
            max,
            step,
            decimals: 0,
            enabled: true,
        }
    }
    pub fn decimals(mut self, d: usize) -> Self {
        self.decimals = d;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Snap to the step grid inside [min, max].
    pub fn snap(&self, v: f64) -> f64 {
        let n = ((v - self.min) / self.step).round();
        (self.min + n * self.step).clamp(self.min, self.max)
    }

    /// Returns `true` when the value changed.
    pub fn show(self, ui: &mut Ui, value: &mut f64) -> bool {
        let p = theme::palette(ui);
        let eps = self.step / 1000.0;
        let lower = lower_first(self.label);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let down = format!("Decrease {lower}");
            let minus = Button::icon_only(theme::icon::MINUS, &down)
                .enabled(self.enabled && *value > self.min + eps);
            let mr = minus.show(ui);
            let (vr, vresp) = ui
                .allocate_exact_size(theme::v2(size::BUTTON_MIN_W, size::CONTROL), Sense::hover());
            let up = format!("Increase {lower}");
            let plus = Button::icon_only(theme::icon::PLUS, &up)
                .enabled(self.enabled && *value < self.max - eps);
            let pr = plus.show(ui);
            let (dec, inc) = (mr.clicked(), pr.clicked());
            if dec && *value > self.min + eps {
                *value = self.snap(*value - self.step);
                changed = true;
            } else if inc && *value < self.max - eps {
                *value = self.snap(*value + self.step);
                changed = true;
            }
            ui.painter().rect(
                vr,
                theme::cr(theme::radius::MD),
                if self.enabled {
                    p.surface
                } else {
                    p.surface_sunken
                },
                theme::hairline(if self.enabled {
                    p.border_strong
                } else {
                    p.border
                }),
                egui::StrokeKind::Inside,
            );
            let text = format!("{:.*}", self.decimals, *value);
            let fg = if self.enabled {
                p.text
            } else {
                p.text_disabled
            };
            paint_text_center(ui, vr, &text, typ::MONO, fg);
            let v = *value;
            let (label, en) = (self.label.to_string(), self.enabled);
            vresp.widget_info(|| {
                let mut i = WidgetInfo::labeled(egui::Role::SpinButton, en, &label);
                i.value = Some(v);
                i
            });
        });
        changed
    }
}

/// "Pages read per PDF" → "pages read per PDF" (keeps acronyms: "OCR").
pub fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match (c.next(), s.chars().nth(1)) {
        (Some(f), Some(n)) if n.is_lowercase() => f.to_lowercase().chain(c).collect(),
        (Some(f), None) => f.to_lowercase().collect(),
        _ => s.to_string(),
    }
}
