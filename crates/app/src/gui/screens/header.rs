//! §4.1 Header: mark + title, tabs, Offline badge, details toggle, settings.

use egui::{Rect, Sense, Ui, WidgetInfo};

use super::Action;
use crate::gui::components::{Badge, Button, anim, focus_ring, galley, mix, paint_galley_v};
use crate::gui::model::{Model, Tab};
use crate::gui::theme::{self, Tone, size, space, stroke, typ};

pub fn header(ui: &mut Ui, rect: Rect, m: &Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(rect, 0, p.surface_sunken);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - stroke::HAIRLINE / 2.0,
        theme::hairline(p.border),
    );
    let cy = rect.center().y;
    let x = rect.left() + space::LG;
    theme::paint_mark(ui.painter(), egui::pos2(x, cy - theme::mark::SIZE / 2.0), p);
    let t = paint_galley_v(
        ui,
        x + theme::mark::SIZE + space::SM,
        cy,
        galley(ui, "AutoRename-PDF", typ::TITLE_SM),
        p.text,
    );

    // Tabs.
    let mut tx = theme::snap4(t.right() + space::XL);
    let review = if m.companies.db.is_some() {
        m.companies.review_count()
    } else {
        m.config.review_count
    };
    // Name-matching tab: one stable label for every profile (§4.1), hidden
    // when the profile matches no names.
    let mut tabs: Vec<(Tab, &str, Option<usize>)> = vec![(Tab::Rename, "Rename", None)];
    if m.name_label().is_some() {
        tabs.push((
            Tab::Companies,
            "Name matching",
            (review > 0).then_some(review),
        ));
    }
    tabs.push((Tab::History, "History", None));
    for (tab, label, count) in tabs {
        let g = galley(ui, label, typ::BODY_STRONG);
        let badge = count.map(|n| n.to_string());
        let bw = badge
            .as_ref()
            .map_or(0.0, |b| Badge::new(Tone::Warning, b).width(ui) + space::XS);
        let w = ((g.size().x + space::MD * 2.0 + bw) / theme::GRID4).ceil() * theme::GRID4;
        let r = Rect::from_min_size(
            egui::pos2(tx, cy - size::CONTROL / 2.0),
            theme::v2(w, size::CONTROL),
        );
        let resp = ui.interact(r, ui.scope_id().with(("tab", label)), Sense::click());
        let active = m.tab == tab;
        let h = anim(ui, resp.id.with("h"), resp.hovered());
        ui.painter().rect_filled(
            r,
            theme::cr(theme::radius::MD),
            mix(egui::Color32::TRANSPARENT, p.surface_hover, h),
        );
        let color = if active || resp.hovered() {
            p.text
        } else {
            p.text_muted
        };
        let gr = paint_galley_v(ui, r.left() + space::MD, cy, g, color);
        if let Some(b) = &badge {
            let bw = Badge::new(Tone::Warning, b).width(ui);
            Badge::new(Tone::Warning, b).paint_at(
                ui,
                Rect::from_min_size(
                    egui::pos2(gr.right() + space::XS, cy - size::BADGE / 2.0),
                    theme::v2(bw, size::BADGE),
                ),
            );
        }
        if active {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    egui::pos2(r.left() + space::MD, r.bottom() - stroke::SELECTED_BAR),
                    egui::pos2(r.right() - space::MD, r.bottom()),
                ),
                0,
                p.accent,
            );
        }
        focus_ring(ui, &resp, r, theme::radius::MD, false);
        let a11y = match count {
            Some(n) => format!("{label}, {n} need a look"),
            None => label.to_string(),
        };
        resp.widget_info(|| WidgetInfo::selected(egui::Role::Tab, true, active, &a11y));
        if resp.clicked() {
            acts.push(Action::SwitchTab(tab));
        }
        tx = r.right() + space::XS;
    }

    // Right cluster (right to left): settings, details toggle (Rename with
    // a plan or result), passive Offline badge. Settings live only in
    // Settings (§4.1).
    let gear = Rect::from_min_size(
        egui::pos2(
            rect.right() - space::LG - size::CONTROL,
            cy - size::CONTROL / 2.0,
        ),
        theme::v2(size::CONTROL, size::CONTROL),
    );
    let open = m.tab == Tab::Settings;
    if Button::icon_only(theme::icon::SETTINGS, "Settings")
        .active(open)
        .tooltip(if open { "Close settings" } else { "Settings" })
        .show_at(ui, gear)
        .clicked()
    {
        acts.push(Action::ToggleSettings);
    }
    let mut left = gear.left();
    if m.details_toggle_shown() {
        // Icon + "Details" (§4.1); the id stays fixed while the label flips.
        let open = m.detail_visible;
        let (a11y, tip) = if open {
            ("Hide details", "Hide details (Ctrl+\\)")
        } else {
            ("Show details", "Show details (Ctrl+\\)")
        };
        let b = Button::ghost("Details")
            .icon(theme::icon::PANEL_RIGHT)
            .toggle(open)
            .a11y(a11y)
            .tooltip(tip)
            .id(egui::Id::unique("details_toggle"));
        let w = b.width(ui);
        let r = Rect::from_min_size(
            egui::pos2(gear.left() - space::SM - w, gear.top()),
            theme::v2(w, size::CONTROL),
        );
        if b.show_at(ui, r).clicked() {
            acts.push(Action::ToggleDetails);
        }
        left = r.left();
    }
    let offline = m
        .config
        .settings
        .as_ref()
        .is_some_and(|s| s.privacy == autorename_core::config::Privacy::Offline);
    if offline {
        let b = Badge::new(Tone::Success, "Offline").icon(theme::icon::SHIELD_CHECK);
        let w = b.width(ui);
        let br = Rect::from_min_size(
            egui::pos2(left - space::SM - w, cy - size::BADGE / 2.0),
            theme::v2(w, size::BADGE),
        );
        if br.left() > tx {
            b.paint_at(ui, br);
            ui.interact(br, ui.scope_id().with("offline_badge"), Sense::hover())
                .on_hover_text(OFFLINE_TIP)
                .widget_info(|| WidgetInfo::labeled(egui::Role::Label, true, OFFLINE_A11Y));
        }
    }
}

const OFFLINE_TIP: &str = "Documents stay on this computer. Change this in Settings › Processing.";
const OFFLINE_A11Y: &str = "Offline. Documents stay on this computer";
