//! §5.6 Field: label above, 32 h control, 1 px border_strong, 2 px focus
//! border, error/helper line, suffix adornment, secret show/hide.

use egui::{Rect, Response, Sense, Shape, StrokeKind, TextEdit, Ui};

use super::button::{BSize, Button};
use super::{anim, label, mix, paint_icon, paint_text};
use crate::gui::theme::{self, Type, size, space, stroke};

pub struct Field<'a> {
    text: &'a mut String,
    label: Option<&'a str>,
    a11y: &'a str,
    helper: Option<&'a str>,
    error: Option<&'a str>,
    hint: &'a str,
    suffix: Option<&'a str>,
    secret: Option<&'a mut bool>,
    mono: bool,
    width: Option<f32>,
    enabled: bool,
    id: Option<egui::Id>,
    /// Multi-line: minimum visible text rows (grows with the text).
    rows: Option<usize>,
}

pub struct FieldResponse {
    pub edit: Response,
}

impl<'a> Field<'a> {
    pub fn new(a11y: &'a str, text: &'a mut String) -> Self {
        Self {
            text,
            label: None,
            a11y,
            helper: None,
            error: None,
            hint: "",
            suffix: None,
            secret: None,
            mono: false,
            width: None,
            enabled: true,
            id: None,
            rows: None,
        }
    }
    pub fn label(mut self, l: &'a str) -> Self {
        self.label = Some(l);
        self
    }
    pub fn helper(mut self, h: Option<&'a str>) -> Self {
        self.helper = h;
        self
    }
    pub fn error(mut self, e: Option<&'a str>) -> Self {
        self.error = e;
        self
    }
    pub fn hint(mut self, h: &'a str) -> Self {
        self.hint = h;
        self
    }
    pub fn suffix(mut self, s: &'a str) -> Self {
        self.suffix = Some(s);
        self
    }
    /// Secret value with an eye toggle; `shown` flips on click.
    pub fn secret(mut self, shown: &'a mut bool) -> Self {
        self.secret = Some(shown);
        self
    }
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    /// Multi-line text with at least `rows` visible lines; the control
    /// grows with its content (Enter adds a line).
    pub fn multiline(mut self, rows: usize) -> Self {
        self.rows = Some(rows.max(2));
        self
    }
    /// Control height: 32, or the rows (at least `content_h`) plus
    /// `space.sm` padding, on the 4-pt grid.
    pub fn control_height(rows: Option<usize>, ty: Type, content_h: f32) -> f32 {
        match rows {
            Some(n) => super::ceil4((n as f32 * ty.line).max(content_h) + space::SM * 2.0),
            None => size::CONTROL,
        }
    }
    pub fn id(mut self, id: egui::Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn show(self, ui: &mut Ui) -> FieldResponse {
        let p = theme::palette(ui);
        let w = self.width.unwrap_or_else(|| ui.available_width());
        if let Some(l) = self.label {
            label(ui, l, theme::typ::BODY_STRONG, p.text);
            ui.add_space(space::XS - ui.spacing().item_spacing.y);
        }
        let ty: Type = if self.mono {
            theme::typ::MONO
        } else {
            theme::typ::BODY
        };
        // Multi-line: measure the wrapped text so the frame always holds it.
        let content_h = if self.rows.is_some() {
            super::wrapped(ui, self.text, ty, w - space::MD * 2.0)
                .size()
                .y
        } else {
            0.0
        };
        let h = Self::control_height(self.rows, ty, content_h);
        let (rect, _) = ui.allocate_exact_size(theme::v2(w, h), Sense::hover());
        let bg_idx = ui.painter().add(Shape::Noop);
        let mut right_pad = space::MD;
        let mut suffix_w = 0.0;
        if let Some(s) = self.suffix {
            suffix_w = super::text_width(ui, s, ty).ceil();
            right_pad += suffix_w + space::XS;
        }
        if self.secret.is_some() {
            right_pad += size::CONTROL_SM;
        }
        let pad_y = if self.rows.is_some() { space::SM } else { 0.0 };
        let edit_rect = Rect::from_min_max(
            rect.min + theme::v2(space::MD, pad_y),
            rect.max - theme::v2(right_pad, pad_y),
        );
        let shown = self.secret.as_deref().copied().unwrap_or(true);
        let rows = self.rows;
        let base = match rows {
            Some(n) => TextEdit::multiline(self.text).desired_rows(n),
            None => TextEdit::singleline(self.text),
        };
        let mut te = base
            .frame(egui::Frame::NONE)
            .margin(theme::margin(0.0, 0.0))
            .font(ty.font())
            .text_color(if self.enabled {
                p.text
            } else {
                p.text_disabled
            })
            .desired_width(edit_rect.width())
            .min_size(edit_rect.size())
            .align(if rows.is_some() {
                egui::Align2::LEFT_TOP
            } else {
                egui::Align2::LEFT_CENTER
            })
            .password(!shown)
            .interactive(self.enabled)
            .hint_text(self.hint);
        if let Some(id) = self.id {
            te = te.id(id);
        }
        // Child ui: placing the edit must not move the parent cursor.
        let edit = super::in_rect(ui, edit_rect, |ui| ui.add(te));
        // The visible label is plain paint (no AccessKit node), so the
        // edit carries its own name instead of `labelled_by`.
        let a = self.a11y.to_string();
        let en = self.enabled;
        edit.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, en, &a));
        let edit = edit;
        let hover = anim(ui, edit.id.with("h"), edit.hovered());
        let (fill, border, width) = if !self.enabled {
            (p.surface_sunken, p.border, stroke::HAIRLINE)
        } else if edit.has_focus() {
            (p.surface, p.focus, stroke::FOCUS)
        } else if self.error.is_some() {
            (p.surface, p.danger, stroke::HAIRLINE)
        } else {
            (
                p.surface,
                mix(p.border_strong, p.text_muted, hover),
                stroke::HAIRLINE,
            )
        };
        ui.painter().set(
            bg_idx,
            egui::epaint::RectShape::new(
                rect,
                theme::cr(theme::radius::MD),
                fill,
                egui::Stroke::new(width, border),
                StrokeKind::Inside,
            ),
        );
        let cy = rect.center().y;
        let mut x = rect.right() - space::MD;
        if let Some(shown) = self.secret {
            let r = Rect::from_center_size(
                egui::pos2(rect.right() - space::XS - size::CONTROL_SM / 2.0, cy),
                theme::v2(size::CONTROL_SM, size::CONTROL_SM),
            );
            let (g, a) = if *shown {
                (theme::icon::EYE_OFF, "Hide API key")
            } else {
                (theme::icon::EYE, "Show API key")
            };
            if Button::icon_only(g, a)
                .size(BSize::Sm)
                .show_at(ui, r)
                .clicked()
            {
                *shown = !*shown;
            }
            x = r.left() - space::XS;
        }
        if let Some(s) = self.suffix {
            paint_text(ui, x - suffix_w, cy, s, ty, p.text_muted);
        }
        if let Some(e) = self.error {
            ui.add_space(space::XS - ui.spacing().item_spacing.y);
            let (r, _) =
                ui.allocate_exact_size(theme::v2(w, theme::typ::CAPTION.line), Sense::hover());
            paint_icon(
                ui,
                egui::pos2(r.left() + size::ICON_SM / 2.0, r.center().y),
                theme::icon::CIRCLE_X,
                size::ICON_SM,
                p.danger,
            );
            paint_text(
                ui,
                r.left() + size::ICON_SM + space::XS,
                r.center().y,
                e,
                theme::typ::CAPTION,
                p.danger,
            );
        } else if let Some(h) = self.helper {
            ui.add_space(space::XS - ui.spacing().item_spacing.y);
            label(ui, h, theme::typ::CAPTION, p.text_muted);
        }
        FieldResponse { edit }
    }
}
