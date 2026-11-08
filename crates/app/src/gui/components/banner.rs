//! §5.7 Banner: semantic fill, 40 % semantic border, icon + title + body, actions right.

use egui::{Rect, StrokeKind, Ui};

use super::button::{BSize, Button};
use super::{galley, paint_galley_v, paint_icon};
use crate::gui::theme::{self, Tone, size, space};

pub struct Banner<'a> {
    pub tone: Tone,
    pub title: &'a str,
    pub body: &'a str,
    pub action: Option<&'a str>,
    pub dismissible: bool,
}

#[derive(Default)]
pub struct BannerResponse {
    pub action: bool,
    pub dismissed: bool,
}

pub fn tone_icon(t: Tone) -> char {
    match t {
        Tone::Success => theme::icon::CIRCLE_CHECK,
        Tone::Warning => theme::icon::TRIANGLE_ALERT,
        Tone::Danger => theme::icon::CIRCLE_X,
        Tone::Accent | Tone::Neutral => theme::icon::INFO,
    }
}

impl Banner<'_> {
    /// Height: 12 padding + 32 control row + 12 padding.
    pub const HEIGHT: f32 = space::MD * 2.0 + size::CONTROL;

    pub fn show_at(self, ui: &mut Ui, rect: Rect) -> BannerResponse {
        let p = theme::palette(ui);
        // Screen readers announce banners (role Alert, title + body).
        let text = format!("{} {}", self.title, self.body).trim().to_string();
        ui.interact(
            rect,
            ui.scope_id().with(("banner", self.title)),
            egui::Sense::hover(),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Alert, true, &text));
        let (fill, fg) = p.tone(self.tone);
        ui.painter().rect(
            rect,
            theme::cr(theme::radius::LG),
            fill,
            theme::hairline(p.tone_border(self.tone)),
            StrokeKind::Inside,
        );
        let cy = rect.center().y;
        let mut x = rect.left() + space::LG;
        paint_icon(
            ui,
            egui::pos2(x + size::ICON / 2.0, cy),
            tone_icon(self.tone),
            size::ICON,
            fg,
        );
        x += size::ICON + space::SM;
        let t = paint_galley_v(
            ui,
            x,
            cy,
            galley(ui, self.title, theme::typ::BODY_STRONG),
            p.text,
        );
        if !self.body.is_empty() {
            paint_galley_v(
                ui,
                t.right() + space::SM,
                cy,
                galley(ui, self.body, theme::typ::BODY),
                p.text,
            );
        }
        let mut out = BannerResponse::default();
        let mut right = rect.right() - space::LG;
        if self.dismissible {
            let r = Rect::from_center_size(
                egui::pos2(right - size::CONTROL_SM / 2.0, cy),
                theme::v2(size::CONTROL_SM, size::CONTROL_SM),
            );
            out.dismissed = Button::icon_only(theme::icon::X, "Dismiss")
                .salt(("banner", self.title))
                .size(BSize::Sm)
                .show_at(ui, r)
                .clicked();
            right = r.left() - space::SM;
        }
        if let Some(a) = self.action {
            let b = Button::secondary(a).salt(("banner_action", self.title));
            let w = b.width(ui);
            let r = Rect::from_min_size(
                egui::pos2(right - w, cy - size::CONTROL / 2.0),
                theme::v2(w, size::CONTROL),
            );
            out.action = b.show_at(ui, r).clicked();
        }
        out
    }
}
