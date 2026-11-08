//! §4.1 Header: mark + title, tabs, privacy chip, profile combo, settings.

use egui::{Rect, Sense, Ui, WidgetInfo};

use super::Action;
use crate::gui::components::{
    Badge, Button, anim, chip, focus_ring, galley, in_rect, mix, paint_galley_v,
};
use crate::gui::model::{Model, Tab, provider_label};
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
    // Name-matching tab: labelled by the active profile's field (§4.1),
    // hidden when the profile matches no names.
    let names = m.name_label().map(|(_, plural)| plural);
    let mut tabs: Vec<(Tab, &str, Option<usize>)> = vec![(Tab::Rename, "Rename", None)];
    if let Some(n) = &names {
        tabs.push((Tab::Companies, n, (review > 0).then_some(review)));
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
            Some(n) => format!("{label}, {n} need review"),
            None => label.to_string(),
        };
        resp.widget_info(|| WidgetInfo::selected(egui::Role::Tab, true, active, &a11y));
        if resp.clicked() {
            acts.push(Action::SwitchTab(tab));
        }
        tx = r.right() + space::XS;
    }

    // Right cluster (right to left): settings, profile combo, privacy chip.
    let right = Rect::from_min_max(
        egui::pos2(tx, rect.top()),
        egui::pos2(rect.right() - space::LG, rect.bottom()),
    );
    in_rect(ui, right, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let open = m.tab == Tab::Settings;
            if Button::icon_only(theme::icon::SETTINGS, "Settings")
                .active(open)
                .tooltip(if open { "Close settings" } else { "Settings" })
                .show(ui)
                .clicked()
            {
                acts.push(Action::ToggleSettings);
            }
            let current = m
                .config
                .settings
                .as_ref()
                .map(|s| s.profile_id.clone())
                .unwrap_or_else(|| "business".into());
            let enabled = m.is_set_up() && !m.busy();
            let ids: Vec<String> = if m.config.profiles.is_empty() {
                vec![current.clone()]
            } else {
                m.config.profiles.clone()
            };
            let names: Vec<String> = ids.iter().map(|i| profile_label(i)).collect();
            let mut sel = ids.iter().position(|i| *i == current).unwrap_or(0);
            let (_, changed) = crate::gui::components::Select {
                a11y: "Profile",
                width: size::COMBO_W,
                options: &names,
                enabled,
                mono: false,
            }
            .show(ui, &mut sel);
            if changed {
                acts.push(Action::SelectProfile(ids[sel].clone()));
            }
            let (tone, icon, text, a11y) = privacy(m);
            if chip(ui, tone, icon, &text, &a11y).clicked() {
                acts.push(Action::OpenSettings(crate::gui::pages::page::PROCESSING));
            }
        });
    });
}

pub fn profile_label(id: &str) -> String {
    match id {
        "business" => "Business".into(),
        "academic" => "Academic".into(),
        other => {
            let mut c = other.replace(['_', '-'], " ");
            if let Some(f) = c.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            c
        }
    }
}

fn privacy(m: &Model) -> (Tone, char, String, String) {
    match &m.config.settings {
        None => (
            Tone::Warning,
            theme::icon::TRIANGLE_ALERT,
            "Not set up".into(),
            "Privacy: AI provider not set up. Open privacy settings".into(),
        ),
        Some(s) => {
            let prov = provider_label(s.ai.provider.id());
            let offline = matches!(s.privacy, autorename_core::config::Privacy::Offline);
            if offline {
                let t = format!("Offline · {prov} {}", s.ai.model);
                (
                    Tone::Success,
                    theme::icon::SHIELD_CHECK,
                    t.clone(),
                    format!(
                        "Privacy: offline, {prov} {}. Open privacy settings",
                        s.ai.model
                    ),
                )
            } else {
                (
                    Tone::Neutral,
                    theme::icon::CLOUD,
                    format!("Cloud · {prov} {}", s.ai.model),
                    format!(
                        "Privacy: cloud, {prov} {}. Open privacy settings",
                        s.ai.model
                    ),
                )
            }
        }
    }
}
