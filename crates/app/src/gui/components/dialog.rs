//! §5.11 Dialog: `egui::Modal`, 440/560 wide, padding 24, radius.xl, elev.3,
//! scrim; Esc cancels, Enter = primary; platform button order.

use egui::{Id, Key, Ui};

use super::button::Button;
use super::{label, paragraph};
use crate::gui::theme::{self, size, space};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogResult {
    Open,
    Primary,
    Cancel,
}

pub struct Dialog<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub primary: &'a str,
    pub cancel: &'a str,
    pub destructive: bool,
}

impl Dialog<'_> {
    pub fn show(self, ctx: &egui::Context) -> DialogResult {
        let p = theme::pal(ctx.global_style().visuals.dark_mode);
        let frame = egui::Frame::new()
            .fill(p.elevated(3))
            .stroke(theme::hairline(p.border))
            .corner_radius(theme::cr(theme::radius::XL))
            .inner_margin(theme::margin_all(space::XL))
            .shadow(p.shadow(3));
        let id = Id::unique(self.id);
        let mut result = DialogResult::Open;
        let modal = egui::Modal::new(id)
            .frame(frame)
            .backdrop_color(p.scrim)
            .show(ctx, |ui: &mut Ui| {
                ui.set_width(size::DIALOG_SM - space::XL * 2.0);
                label(ui, self.title, theme::typ::TITLE, p.text);
                ui.add_space(space::SM);
                paragraph(ui, self.body, theme::typ::BODY, p.text_muted);
                ui.add_space(space::XL - space::SM);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let windows = cfg!(windows);
                    // right_to_left: first added is rightmost.
                    let primary = Button::primary(self.primary);
                    let primary = if self.destructive {
                        Button::danger(self.primary)
                    } else {
                        primary
                    };
                    let cancel = Button::secondary(self.cancel);
                    let (pr, cr) = if windows {
                        let c = cancel.show(ui);
                        (primary.show(ui), c)
                    } else {
                        let pr = primary.show(ui);
                        (pr, cancel.show(ui))
                    };
                    // Initial focus: primary, or Cancel for destructive dialogs.
                    let first = if self.destructive { &cr } else { &pr };
                    if ui.memory(|m| m.focused().is_none()) {
                        first.request_focus();
                    }
                    if pr.clicked() {
                        result = DialogResult::Primary;
                    }
                    if cr.clicked() {
                        result = DialogResult::Cancel;
                    }
                });
            });
        let (esc, enter) = ctx.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
        if result == DialogResult::Open {
            if esc || modal.should_close() {
                result = DialogResult::Cancel;
            } else if enter && !self.destructive {
                result = DialogResult::Primary;
            }
        }
        result
    }
}
