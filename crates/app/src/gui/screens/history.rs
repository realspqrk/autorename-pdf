//! History: undo batches (date, file count, folder) with per-batch Undo;
//! undone batches greyed with an "Undone" badge (§4.6).

use egui::{Rect, Sense, Ui, WidgetInfo};

use super::Action;
use crate::gui::components::{
    Badge, Button, ellipsize_middle, in_rect, paint_text, paint_text_center, plural,
};
use crate::gui::model::Model;
use crate::gui::pages::short_time;
use crate::gui::theme::{self, Tone, size, space, stroke, typ};

pub fn history(ui: &mut Ui, body: Rect, m: &Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(body, 0, p.bg);
    let tr = Rect::from_min_size(body.min, theme::v2(body.width(), size::TOOLBAR));
    ui.painter().hline(
        tr.x_range(),
        tr.bottom() - stroke::HAIRLINE / 2.0,
        theme::hairline(p.border),
    );
    let h = &m.history;
    let active = h.batches.iter().filter(|b| !b.undone).count();
    let summary = format!(
        "{} · {} can be undone",
        plural(h.batches.len(), "batch", "batches"),
        active
    );
    paint_text(
        ui,
        tr.left() + space::LG,
        tr.center().y,
        &summary,
        typ::BODY,
        p.text_muted,
    );
    let list = Rect::from_min_max(egui::pos2(body.left(), tr.bottom()), body.max);
    ui.painter().rect_filled(list, 0, p.surface);
    if let Some(e) = &h.error {
        paint_text_center(ui, list, e, typ::CAPTION, p.danger);
        return;
    }
    if h.batches.is_empty() {
        paint_text_center(
            ui,
            list,
            "No renames yet. Every rename batch appears here with an Undo button.",
            typ::CAPTION,
            p.text_muted,
        );
        return;
    }
    in_rect(ui, list, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("history")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = theme::v2(0.0, 0.0);
                for b in &h.batches {
                    let (r, _) = ui.allocate_exact_size(
                        theme::v2(ui.available_width(), size::ROW),
                        Sense::hover(),
                    );
                    if !ui.is_rect_visible(r) {
                        continue;
                    }
                    ui.painter().hline(
                        (r.left() + space::LG)..=(r.right() - space::LG),
                        r.bottom() - stroke::HAIRLINE / 2.0,
                        theme::hairline(p.border),
                    );
                    let l1 = r.top() + space::SM + typ::BODY.line / 2.0;
                    let l2 = l1 + typ::BODY.line / 2.0 + space::XS + typ::MONO_SM.line / 2.0;
                    let (t1, t2) = if b.undone {
                        (p.text_disabled, p.text_disabled)
                    } else {
                        (p.text, p.text_muted)
                    };
                    let x = r.left() + space::LG;
                    let title = format!(
                        "{} · {}",
                        short_time(&b.timestamp),
                        if b.files > 0 {
                            plural(b.files, "file", "files")
                        } else {
                            plural(b.names, "company name", "company names")
                        }
                    );
                    paint_text(ui, x, l1, &title, typ::BODY_STRONG, t1);
                    let mut right = r.right() - space::LG;
                    let busy = m.undoing.as_deref() == Some(b.id.as_str());
                    if b.undone {
                        let bd = Badge::new(Tone::Neutral, "Undone").icon(theme::icon::UNDO_2);
                        let bw = bd.width(ui);
                        bd.paint_at(
                            ui,
                            Rect::from_min_size(
                                egui::pos2(right - bw, r.center().y - size::BADGE / 2.0),
                                theme::v2(bw, size::BADGE),
                            ),
                        );
                        right -= bw + space::MD;
                    } else {
                        let a11y = format!("Undo batch from {}", short_time(&b.timestamp));
                        let btn = Button::secondary("Undo")
                            .icon(theme::icon::UNDO_2)
                            .a11y(a11y)
                            .loading(busy)
                            .enabled(!m.busy())
                            .tooltip("Restores the original names, removes written metadata and forgets learned company names from this batch");
                        let bw = btn.width(ui);
                        let br = Rect::from_min_size(
                            egui::pos2(right - bw, r.center().y - size::CONTROL / 2.0),
                            theme::v2(bw, size::CONTROL),
                        );
                        if btn.show_at(ui, br).clicked() {
                            acts.push(Action::UndoBatch(b.id.clone()));
                        }
                        right = br.left() - space::MD;
                    }
                    let src = if b.source == "gui" { "App" } else { "Command line" };
                    let l2t = format!("{} · {src}", b.folder);
                    let l2t = ellipsize_middle(ui, &l2t, typ::MONO_SM, right - x);
                    paint_text(ui, x, l2, &l2t, typ::MONO_SM, t2);
                    let label = format!(
                        "{title}, {}, {}",
                        b.folder,
                        if b.undone { "undone" } else { "can be undone" }
                    );
                    let resp = ui.interact(r, ui.scope_id().with(("batch", &b.id)), Sense::hover());
                    resp.widget_info(|| WidgetInfo::labeled(egui::Role::ListItem, true, &label));
                }
            });
    });
}
