//! §5.1 Button: primary / secondary / ghost / danger, sm/md/lg, icon, loading.

use egui::{Color32, Rect, Response, Sense, StrokeKind, Ui, WidgetInfo};

use super::{anim, focus_ring, galley, mix, paint_galley_v, paint_icon, paint_spinner};
use crate::gui::theme::{self, size, space, stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BSize {
    Sm,
    Md,
    Lg,
}

pub struct Button<'a> {
    label: &'a str,
    icon: Option<char>,
    variant: Variant,
    size: BSize,
    enabled: bool,
    loading: bool,
    tooltip: Option<String>,
    a11y: Option<String>,
    muted: bool,
    min_w: Option<f32>,
    salt: Option<egui::Id>,
    /// Toggle-style button that is currently on (e.g. the Settings gear
    /// while Settings is open): tab-active treatment, AccessKit pressed.
    active: bool,
}

impl<'a> Button<'a> {
    pub fn new(variant: Variant, label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            variant,
            size: BSize::Md,
            enabled: true,
            loading: false,
            tooltip: None,
            a11y: None,
            muted: false,
            min_w: None,
            salt: None,
            active: false,
        }
    }

    pub fn primary(label: &'a str) -> Self {
        Self::new(Variant::Primary, label)
    }
    pub fn secondary(label: &'a str) -> Self {
        Self::new(Variant::Secondary, label)
    }
    pub fn ghost(label: &'a str) -> Self {
        Self::new(Variant::Ghost, label)
    }
    pub fn danger(label: &'a str) -> Self {
        Self::new(Variant::Danger, label)
    }

    /// Icon-only button: square, tooltip + AccessKit label required.
    pub fn icon_only(glyph: char, a11y: &str) -> Button<'static> {
        Button {
            icon: Some(glyph),
            tooltip: Some(a11y.to_string()),
            a11y: Some(a11y.to_string()),
            muted: true,
            ..Button::new(Variant::Ghost, "")
        }
    }

    pub fn icon(mut self, glyph: char) -> Self {
        self.icon = Some(glyph);
        self
    }
    pub fn size(mut self, s: BSize) -> Self {
        self.size = s;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn loading(mut self, on: bool) -> Self {
        self.loading = on;
        self
    }
    pub fn tooltip(mut self, t: impl Into<String>) -> Self {
        self.tooltip = Some(t.into());
        self
    }
    pub fn a11y(mut self, t: impl Into<String>) -> Self {
        self.a11y = Some(t.into());
        self
    }
    pub fn active(mut self, on: bool) -> Self {
        self.active = on;
        self
    }

    /// Disambiguates `show_at` ids when the same label repeats in one ui.
    pub fn salt(mut self, s: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.salt = Some(egui::Id::unique(s));
        self
    }

    pub fn min_width(mut self, w: f32) -> Self {
        self.min_w = Some(w);
        self
    }

    pub fn height(&self) -> f32 {
        match self.size {
            BSize::Sm => size::CONTROL_SM,
            BSize::Md => size::CONTROL,
            BSize::Lg => size::CONTROL_LG,
        }
    }

    /// Width on the 4-pt grid.
    pub fn width(&self, ui: &Ui) -> f32 {
        let h = self.height();
        if self.label.is_empty() {
            return h;
        }
        let pad = if self.size == BSize::Lg {
            space::LG
        } else {
            space::MD
        };
        let icon = if self.icon.is_some() || self.loading {
            size::ICON + space::SM
        } else {
            0.0
        };
        let text = galley(ui, self.label, theme::typ::BODY_STRONG).size().x;
        let min = if self.size == BSize::Sm {
            0.0
        } else {
            size::BUTTON_MIN_W
        };
        let w = (pad * 2.0 + icon + text)
            .max(min)
            .max(self.min_w.unwrap_or(0.0));
        (w / theme::GRID4).ceil() * theme::GRID4
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let w = self.width(ui);
        let (rect, resp) = ui.allocate_exact_size(theme::v2(w, self.height()), self.sense());
        self.paint(ui, rect, resp)
    }

    /// Show at a fixed rect (layout computed by the caller).
    pub fn show_at(self, ui: &mut Ui, rect: Rect) -> Response {
        // Does not advance the parent layout (caller owns the geometry).
        let key = self.a11y.clone().unwrap_or_else(|| self.label.to_string());
        let base = self.salt.unwrap_or_else(|| ui.scope_id());
        let resp = ui.interact(rect, base.with(("button", key)), self.sense());
        self.paint(ui, rect, resp)
    }

    fn sense(&self) -> Sense {
        if self.enabled && !self.loading {
            Sense::click()
        } else {
            Sense::hover()
        }
    }

    fn paint(self, ui: &mut Ui, rect: Rect, resp: Response) -> Response {
        let p = theme::palette(ui);
        let hover = anim(ui, resp.id.with("h"), self.enabled && resp.hovered());
        let pressed = self.enabled && resp.is_pointer_button_down_on();
        let (fill, fg, border) = match (self.variant, self.enabled) {
            (_, false) if self.variant == Variant::Ghost => {
                (Color32::TRANSPARENT, p.text_disabled, Color32::TRANSPARENT)
            }
            (_, false) => (p.surface_sunken, p.text_disabled, p.border),
            (Variant::Primary, _) => {
                let f = if pressed {
                    p.accent_pressed
                } else {
                    mix(p.accent, p.accent_hover, hover)
                };
                (f, p.on_accent, Color32::TRANSPARENT)
            }
            (Variant::Danger, _) => {
                let f = if pressed {
                    p.danger_hover
                } else {
                    mix(p.danger, p.danger_hover, hover)
                };
                (f, p.on_accent, Color32::TRANSPARENT)
            }
            (Variant::Secondary, _) => {
                let f = if pressed {
                    p.accent_subtle
                } else {
                    mix(p.surface, p.surface_hover, hover)
                };
                (f, p.text, p.border_strong)
            }
            (Variant::Ghost, _) if self.active => {
                // Same language as the active tab: selection fill, text in
                // `text`, 2 px accent underline (painted below).
                let f = if pressed {
                    p.accent_subtle
                } else {
                    p.selection
                };
                (f, p.text, Color32::TRANSPARENT)
            }
            (Variant::Ghost, _) => {
                let f = if pressed {
                    p.accent_subtle
                } else {
                    mix(Color32::TRANSPARENT, p.surface_hover, hover)
                };
                let fg = if self.muted && !resp.hovered() {
                    p.text_muted
                } else {
                    p.text
                };
                (f, fg, Color32::TRANSPARENT)
            }
        };
        if ui.is_rect_visible(rect) {
            ui.painter().rect(
                rect,
                theme::cr(theme::radius::MD),
                fill,
                theme::hairline(border),
                StrokeKind::Inside,
            );
            let cy = rect.center().y;
            if self.label.is_empty() {
                if let Some(g) = self.icon {
                    if self.loading {
                        paint_spinner(ui, rect.center(), size::ICON, fg);
                    } else {
                        paint_icon(ui, rect.center(), g, size::ICON, fg);
                    }
                }
            } else {
                let g = galley(ui, self.label, theme::typ::BODY_STRONG);
                let has_icon = self.icon.is_some() || self.loading;
                let content = g.size().x
                    + if has_icon {
                        size::ICON + space::SM
                    } else {
                        0.0
                    };
                let mut x = (rect.center().x - content / 2.0).round();
                if has_icon {
                    let c = egui::pos2(x + size::ICON / 2.0, cy);
                    if self.loading {
                        paint_spinner(ui, c, size::ICON, fg);
                    } else if let Some(i) = self.icon {
                        paint_icon(ui, c, i, size::ICON, fg);
                    }
                    x += size::ICON + space::SM;
                }
                paint_galley_v(ui, x, cy, g, fg);
            }
            if self.active {
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        egui::pos2(
                            rect.left() + space::SM,
                            rect.bottom() - stroke::SELECTED_BAR,
                        ),
                        egui::pos2(rect.right() - space::SM, rect.bottom()),
                    ),
                    0,
                    p.accent,
                );
            }
            focus_ring(ui, &resp, rect, theme::radius::MD, false);
        }
        let label = self.a11y.clone().unwrap_or_else(|| self.label.to_string());
        let enabled = self.enabled;
        let active = self.active;
        resp.widget_info(|| {
            if active {
                WidgetInfo::selected(egui::Role::Button, enabled, true, &label)
            } else {
                WidgetInfo::labeled(egui::Role::Button, enabled, &label)
            }
        });
        match self.tooltip {
            Some(t) if enabled => resp.on_hover_text(t),
            Some(t) => resp.on_disabled_hover_text(t),
            None => resp,
        }
    }
}
