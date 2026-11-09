//! §5.2 Badge (20 h, static).

use egui::{Rect, Response, Sense, StrokeKind, Ui, WidgetInfo};

use super::{anim, focus_ring, galley, mix, paint_galley_v, paint_icon};
use crate::gui::theme::{self, Tone, size, space};

pub struct Badge<'a> {
    pub text: std::borrow::Cow<'a, str>,
    pub icon: Option<char>,
    pub tone: Tone,
}

impl<'a> Badge<'a> {
    pub fn new(tone: Tone, text: impl Into<std::borrow::Cow<'a, str>>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            tone,
        }
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
