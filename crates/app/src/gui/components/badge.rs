//! §5.2 Badge (20 h, static) and interactive chip (24 h).

use egui::{Rect, Response, Sense, StrokeKind, Ui, WidgetInfo};

use super::{anim, focus_ring, galley, mix, paint_galley_v, paint_icon};
use crate::gui::theme::{self, Tone, size, space};

pub struct Badge<'a> {
    pub text: std::borrow::Cow<'a, str>,
    pub icon: Option<char>,
    pub tone: Tone,
    /// Dropped first when a row runs out of room (default-state info).
    pub optional: bool,
}

impl<'a> Badge<'a> {
    pub fn new(tone: Tone, text: impl Into<std::borrow::Cow<'a, str>>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            tone,
            optional: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn icon(mut self, g: char) -> Self {
        self.icon = Some(g);
        self
    }

    pub fn width(&self, ui: &Ui) -> f32 {
        let t = galley(ui, &self.text, theme::typ::CAPTION_STRONG).size().x;
        let i = if self.icon.is_some() {
            size::ICON_SM + space::XS
        } else {
            0.0
        };
        ((space::SM * 2.0 + i + t) / theme::GRID4).ceil() * theme::GRID4
    }

    pub fn paint_at(&self, ui: &Ui, rect: Rect) {
        let p = theme::palette(ui);
        let (fill, fg) = p.tone(self.tone);
        ui.painter()
            .rect_filled(rect, theme::cr(theme::radius::FULL), fill);
        let mut x = rect.left() + space::SM;
        let cy = rect.center().y;
        if let Some(g) = self.icon {
            paint_icon(
                ui,
                egui::pos2(x + size::ICON_SM / 2.0, cy),
                g,
                size::ICON_SM,
                fg,
            );
            x += size::ICON_SM + space::XS;
        }
        paint_galley_v(
            ui,
            x,
            cy,
            galley(ui, &self.text, theme::typ::CAPTION_STRONG),
            fg,
        );
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let w = self.width(ui);
        let (rect, resp) = ui.allocate_exact_size(theme::v2(w, size::BADGE), Sense::hover());
        self.paint_at(ui, rect);
        resp
    }
}

/// Interactive chip (privacy chip): 24 h, hover surface_hover, focus ring.
pub fn chip(ui: &mut Ui, tone: Tone, icon: char, text: &str, a11y: &str) -> Response {
    let b = Badge::new(tone, text).icon(icon);
    let w = b.width(ui) + size::ICON_SM + space::XS;
    let (rect, resp) = ui.allocate_exact_size(theme::v2(w, size::CONTROL_SM), Sense::click());
    let p = theme::palette(ui);
    let (fill, fg) = p.tone(tone);
    let h = anim(ui, resp.id.with("h"), resp.hovered());
    let fill = if tone == Tone::Neutral {
        mix(fill, p.surface_hover, h)
    } else {
        fill
    };
    ui.painter().rect(
        rect,
        theme::cr(theme::radius::FULL),
        fill,
        theme::hairline(mix(egui::Color32::TRANSPARENT, p.border_strong, h)),
        StrokeKind::Inside,
    );
    let cy = rect.center().y;
    let mut x = rect.left() + space::SM;
    paint_icon(
        ui,
        egui::pos2(x + size::ICON_SM / 2.0, cy),
        icon,
        size::ICON_SM,
        fg,
    );
    x += size::ICON_SM + space::XS;
    let tr = paint_galley_v(ui, x, cy, galley(ui, text, theme::typ::CAPTION_STRONG), fg);
    paint_icon(
        ui,
        egui::pos2(tr.right() + space::XS + size::ICON_SM / 2.0, cy),
        theme::icon::CHEVRON_DOWN,
        size::ICON_SM,
        fg,
    );
    focus_ring(ui, &resp, rect, theme::radius::FULL, false);
    resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, a11y));
    resp
}
