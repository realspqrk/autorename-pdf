//! §4.4 Detail pane: selected row, padding 16, scrolls independently.
//! Sections are collapsible (32 px header, chevron 12, `motion.normal`).

use egui::{Rect, Sense, Ui, WidgetInfo};

use super::Action;
use crate::gui::components::banner::tone_icon;
use crate::gui::components::{
    BSize, Badge, Button, dur, ellipsize_end, ellipsize_middle, focus_ring, galley, in_rect, label,
    paint_galley_v, paint_icon, paint_text, paint_text_center, wrapped,
};
use crate::gui::model::{
    self, MatchView, Model, Row, provider_label, skip_reason_copy, tier_label,
};
use crate::gui::theme::{self, Tone, motion, size, space, stroke, typ};
use autorename_core::extract::Tier;
use autorename_core::pipeline::{EnrichPlan, LearnAction, LearnStatus, RenameStatus};
use serde_json::Value;

pub fn detail(ui: &mut Ui, rect: Rect, m: &Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(rect, 0, p.surface);
    let Some(i) = m.active.filter(|i| *i < m.rows.len()) else {
        paint_text_center(
            ui,
            rect,
            "Select a file to see details",
            typ::CAPTION,
            p.text_muted,
        );
        return;
    };
    let row = &m.rows[i];
    in_rect(ui, rect, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(theme::margin_all(space::LG))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
                        ui.set_width(ui.available_width());
                        body(ui, m, i, row, acts);
                    });
            });
    });
}

pub fn wrapped_label(ui: &mut Ui, text: &str, t: theme::Type, color: egui::Color32, w: f32) {
    let g = wrapped(ui, text, t, w);
    let rows = g.rows.len().max(1) as f32;
    let (r, _) = ui.allocate_exact_size(theme::v2(w, rows * t.line), Sense::hover());
    for (k, row) in g.rows.iter().enumerate() {
        let cy = r.top() + t.line * (k as f32 + 0.5);
        paint_text(ui, r.left(), cy, &row.text(), t, color);
    }
}

fn body(ui: &mut Ui, m: &Model, i: usize, row: &Row, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let w = ui.available_width();
    let name = row
        .final_name()
        .unwrap_or_else(|| row.error_summary().unwrap_or_else(|| "Analyzing…".into()));
    let editable = row.item.is_some() && row.report.is_none() && !row.is_error() && !m.busy();
    // Title: the action column (32 + 8) is reserved in every phase so the
    // title wraps identically whether the pencil is shown or not (§4.4).
    let tw = w - size::CONTROL - space::SM;
    let lines = crate::gui::components::wrapped_lines(ui, &name, typ::MONO_TITLE, tw);
    let th = (lines as f32 * typ::MONO_TITLE.line).max(size::CONTROL);
    let (tr, _) = ui.allocate_exact_size(theme::v2(w, th), Sense::hover());
    crate::gui::components::paint_wrapped(ui, tr.min, tw, &name, typ::MONO_TITLE, p.text);
    if editable {
        let br = Rect::from_min_size(
            egui::pos2(tr.right() - size::CONTROL, tr.top()),
            theme::v2(size::CONTROL, size::CONTROL),
        );
        if Button::icon_only(theme::icon::PENCIL, "Edit proposed name")
            .show_at(ui, br)
            .clicked()
        {
            acts.push(Action::StartEdit(i));
        }
    }
    ui.add_space(space::XS);
    let path = row.source.display().to_string();
    let (pr, _) = ui.allocate_exact_size(theme::v2(w, size::CONTROL_SM), Sense::hover());
    let budget = w - (size::CONTROL_SM + space::XS) * 2.0 - space::SM;
    let from = ellipsize_middle(ui, &format!("from {path}"), typ::CAPTION, budget);
    let fr = paint_text(
        ui,
        pr.left(),
        pr.center().y,
        &from,
        typ::CAPTION,
        p.text_muted,
    );
    let b1 = Rect::from_min_size(
        egui::pos2(
            crate::gui::components::ceil4(fr.right() + space::SM),
            pr.top(),
        ),
        theme::v2(size::CONTROL_SM, size::CONTROL_SM),
    );
    if Button::icon_only(theme::icon::COPY, "Copy original path")
        .size(BSize::Sm)
        .show_at(ui, b1)
        .clicked()
    {
        acts.push(Action::CopyText(path.clone()));
    }
    let b2 = b1.translate(theme::v2(size::CONTROL_SM + space::XS, 0.0));
    if Button::icon_only(theme::icon::EXTERNAL_LINK, "Open file")
        .size(BSize::Sm)
        .show_at(ui, b2)
        .clicked()
    {
        acts.push(Action::OpenPath(row.source.clone()));
    }
    if row.edited.is_some() && row.report.is_none() {
        ui.add_space(space::XS);
        if Button::ghost("Reset to suggested")
            .size(BSize::Sm)
            .show(ui)
            .clicked()
        {
            acts.push(Action::ResetName(i));
        }
    }

    if let Some(e) = row
        .report
        .as_ref()
        .and_then(|r| r.error.as_ref())
        .or_else(|| row.item.as_ref()?.error.as_ref())
    {
        section(ui, "Problem", |ui| {
            let headline = row.error_summary().unwrap_or_else(|| model::error_copy(e));
            icon_line(ui, Tone::Danger, &headline, w);
            ui.add_space(space::XS);
            wrapped_label(ui, &e.message, typ::CAPTION, p.text_muted, w);
            if let Some(h) = &e.hint {
                ui.add_space(space::XS);
                wrapped_label(ui, h, typ::CAPTION, p.text_muted, w);
            }
        });
        return;
    }
    let Some(item) = &row.item else {
        section(ui, "Extraction", |ui| {
            label(
                ui,
                "Waiting for the analysis of this file.",
                typ::BODY,
                p.text_muted,
            );
        });
        return;
    };

    if let Some(rep) = &row.report {
        section(ui, "Result", |ui| {
            let (tone, text) = match rep.rename_status {
                RenameStatus::Renamed => (Tone::Success, "Renamed".to_string()),
                RenameStatus::Unchanged => (Tone::Success, "Name was already correct".into()),
                RenameStatus::Failed => (Tone::Danger, "Not renamed".into()),
                #[allow(unreachable_patterns)]
                _ => (Tone::Neutral, "Not renamed: the run was stopped".into()),
            };
            icon_line(ui, tone, &text, w);
        });
    }

    section(ui, "Fields", |ui| fields(ui, item, w));

    // Name match of the profile's harmonize_field (none: no section).
    if let Some((one, _)) = m.name_label() {
        let nouns = m.name_nouns();
        section(ui, &format!("{one} match"), |ui| {
            company_match(ui, i, row, w, &nouns, acts);
        });
    }

    section(ui, "Metadata", |ui| {
        let (badge, tone, text) = metadata_copy(row, item);
        Badge::new(tone, badge)
            .icon(theme::icon::FILE_PEN_LINE)
            .show(ui);
        ui.add_space(space::SM);
        wrapped_label(ui, &text, typ::BODY, p.text, w);
    });

    section(ui, "Extraction", |ui| {
        let mut parts = Vec::new();
        if let Some(t) = tier_label(item.tier) {
            parts.push(format!("Tier {t}"));
        }
        parts.push(crate::gui::components::plural(
            item.pages.len(),
            "page",
            "pages",
        ));
        if let Some(meta) = &m.meta {
            parts.push(format!(
                "{} {}",
                provider_label(&meta.provider),
                item.model.clone().unwrap_or_else(|| meta.model.clone())
            ));
        }
        wrapped_label(ui, &parts.join(" · "), typ::BODY, p.text, w);
        if let Some(q) = item.quality {
            ui.add_space(space::XS);
            let caption = quality_caption(item.tier, q);
            wrapped_label(ui, &caption, typ::CAPTION, p.text_muted, w);
        }
        ui.add_space(space::SM);
        text_well(ui, &item.pages.join("\n"), w);
    });

    let warnings: Vec<String> = row
        .report
        .as_ref()
        .map_or_else(|| item.warnings.clone(), |r| r.warnings.clone());
    if !warnings.is_empty() {
        section(ui, "Warnings", |ui| {
            for wtxt in &warnings {
                icon_text(ui, Tone::Warning, &sentence(wtxt), w);
                ui.add_space(space::XS);
            }
        });
    }
}

fn fields(ui: &mut Ui, item: &autorename_core::pipeline::PlanItem, w: f32) {
    let p = theme::palette(ui);
    let get = |k: &str| item.fields.get(k).and_then(Value::as_str).unwrap_or("");
    let ty = get("document_type");
    let date_raw = get("document_date");
    let date = match (&item.date, date_raw.is_empty()) {
        (Some(d), false) => format!("{d} (from \u{201c}{date_raw}\u{201d})"),
        (Some(d), true) => d.clone(),
        (None, _) => date_raw.to_string(),
    };
    let total_label = if ty == "AP" || ty == "AR" {
        "Total"
    } else {
        "Subject"
    };
    let company = item
        .company
        .clone()
        .unwrap_or_else(|| get("company_name").into());
    let mut pairs: Vec<(String, String)> = vec![
        ("Company".into(), company),
        ("Date".into(), date),
        ("Type".into(), ty.to_string()),
        (total_label.into(), get("description").to_string()),
    ];
    for (k, v) in &item.fields {
        if ![
            "document_date",
            "company_name",
            "document_type",
            "description",
        ]
        .contains(&k.as_str())
        {
            pairs.push((
                sentence(&k.replace('_', " ")),
                v.as_str().unwrap_or_default().into(),
            ));
        }
    }
    // Two columns (§4.4) when every value fits, else one full-width column.
    let col2 = ((w - space::LG) / 2.0).floor();
    let fits = pairs.iter().all(|(_, v)| {
        crate::gui::components::text_width(ui, v, typ::BODY) <= col2 - size::LABEL_COL
    });
    let (per_row, col) = if fits { (2, col2) } else { (1, w) };
    for (k_row, chunk) in pairs.chunks(per_row).enumerate() {
        if k_row > 0 {
            ui.add_space(space::SM);
        }
        let (line, _) = ui.allocate_exact_size(theme::v2(w, typ::BODY.line), Sense::hover());
        for (n, (k, v)) in chunk.iter().enumerate() {
            let r = Rect::from_min_size(
                egui::pos2(line.left() + n as f32 * (col + space::LG), line.top()),
                theme::v2(col, typ::BODY.line),
            );
            paint_text(ui, r.left(), r.center().y, k, typ::CAPTION, p.text_muted);
            let v = if v.is_empty() { "—" } else { v.as_str() };
            let v = ellipsize_end(ui, v, typ::BODY, col - size::LABEL_COL);
            paint_text(
                ui,
                r.left() + size::LABEL_COL,
                r.center().y,
                &v,
                typ::BODY,
                p.text,
            );
        }
    }
}

/// §4.4 company match: decision explained with score and band; candidates
/// for a decider choice; offline review called out. No invented scores.
fn company_match(
    ui: &mut Ui,
    i: usize,
    row: &Row,
    w: f32,
    (one, many): &(String, String),
    acts: &mut Vec<Action>,
) {
    let p = theme::palette(ui);
    let Some(item) = &row.item else { return };
    let Some(mv) = MatchView::from_item(item) else {
        label(
            ui,
            "Nothing to match in this file.",
            typ::BODY,
            p.text_muted,
        );
        return;
    };
    wrapped_label(ui, &mv.headline(one, many), typ::BODY, p.text, w);
    if let Some(ev) = mv.evidence() {
        ui.add_space(space::XS);
        wrapped_label(ui, &ev, typ::CAPTION, p.text_muted, w);
    }
    for c in &mv.candidates {
        ui.add_space(space::XS);
        let t = format!("{} · {:.2}", c.0, c.1);
        let (r, _) = ui.allocate_exact_size(theme::v2(w, typ::CAPTION.line), Sense::hover());
        paint_text(
            ui,
            r.left() + space::LG,
            r.center().y,
            &ellipsize_end(ui, &t, typ::CAPTION, w - space::LG),
            typ::CAPTION,
            p.text_muted,
        );
    }
    let done = row.report.is_some();
    let new_badge = format!("New {one}");
    let learned = row.report.as_ref().map(|r| r.learn_status);
    let (badge, tone, icon, text) = match (&item.learn, learned) {
        (_, _) if row.dont_learn => (
            "Won't learn",
            Tone::Neutral,
            theme::icon::MINUS,
            format!("The {many} list stays unchanged."),
        ),
        (Some(LearnAction::AddAlias { canonical, .. }), Some(LearnStatus::AliasAdded)) => (
            "Alias added",
            Tone::Accent,
            theme::icon::LINK,
            format!("Added to {canonical} in the {many} list."),
        ),
        (Some(LearnAction::AddAlias { .. }), _) if !done => (
            "Alias added",
            Tone::Accent,
            theme::icon::LINK,
            format!("Will be added to the {many} list on rename."),
        ),
        (Some(LearnAction::NewEntry { review: true, .. }), Some(LearnStatus::EntryAdded)) => (
            "Review",
            Tone::Warning,
            theme::icon::FLAG,
            format!("Added to the {many} list and flagged for review."),
        ),
        (Some(LearnAction::NewEntry { review: true, .. }), _) if !done => (
            "Review",
            Tone::Warning,
            theme::icon::FLAG,
            format!("Will be added to the {many} list and flagged for review."),
        ),
        (Some(LearnAction::NewEntry { .. }), Some(LearnStatus::EntryAdded)) => (
            new_badge.as_str(),
            Tone::Accent,
            theme::icon::USER_PLUS,
            format!("Added to the {many} list."),
        ),
        (Some(LearnAction::NewEntry { .. }), _) if !done => (
            new_badge.as_str(),
            Tone::Accent,
            theme::icon::USER_PLUS,
            format!("Will be added to the {many} list on rename."),
        ),
        (Some(_), Some(s)) => (
            "Not learned",
            Tone::Neutral,
            theme::icon::MINUS,
            format!(
                "The {many} list was not changed ({}).",
                format!("{s:?}").to_lowercase()
            ),
        ),
        _ => return,
    };
    ui.add_space(space::SM);
    let (r, _) = ui.allocate_exact_size(theme::v2(w, size::CONTROL_SM), Sense::hover());
    let b = Badge::new(tone, badge).icon(icon);
    let bw = b.width(ui);
    b.paint_at(
        ui,
        Rect::from_min_size(
            egui::pos2(r.left(), r.center().y - size::BADGE / 2.0),
            theme::v2(bw, size::BADGE),
        ),
    );
    if !done && item.learn.is_some() {
        let t = if row.dont_learn {
            "Learn"
        } else {
            "Don't learn"
        };
        let btn = Button::ghost(t).size(BSize::Sm);
        let w2 = btn.width(ui);
        let br = Rect::from_min_size(
            egui::pos2(r.right() - w2, r.top()),
            theme::v2(w2, size::CONTROL_SM),
        );
        if btn.show_at(ui, br).clicked() {
            acts.push(Action::DontLearn(i));
        }
    }
    ui.add_space(space::XS);
    wrapped_label(ui, &text, typ::BODY, p.text_muted, w);
}

/// Metadata section: plan intent before renaming, actual outcome after.
fn metadata_copy(
    row: &Row,
    item: &autorename_core::pipeline::PlanItem,
) -> (&'static str, Tone, String) {
    if let Some(rep) = &row.report {
        let s = rep.enrich_status.as_str();
        return if s == "enriched" {
            (
                "Metadata",
                Tone::Success,
                "Wrote title, subject, keywords, XMP and a JSON attachment.".into(),
            )
        } else if let Some(reason) = s.strip_prefix("skipped:") {
            (
                "Metadata skipped",
                Tone::Neutral,
                format!(
                    "Skipped: {}. The file was renamed but not changed.",
                    skip_reason_copy(reason)
                ),
            )
        } else if s == "failed" {
            (
                "Metadata failed",
                Tone::Danger,
                "Couldn't write the metadata. The file was renamed; see Warnings.".into(),
            )
        } else {
            (
                "Metadata skipped",
                Tone::Neutral,
                "No metadata was written.".into(),
            )
        };
    }
    match &item.enrich {
        EnrichPlan::Will => (
            "Metadata",
            Tone::Accent,
            "Will write title, subject, keywords, XMP and a JSON attachment into the renamed PDF."
                .into(),
        ),
        EnrichPlan::Skip { reason } => (
            "Metadata skipped",
            Tone::Neutral,
            format!(
                "Skipped: {}. The file is renamed but not changed.",
                skip_reason_copy(reason)
            ),
        ),
    }
}

fn sentence(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// Collapsible section: 32 px header (chevron 12 + caption_strong + rule),
/// `space.xl` above, `space.sm` below; body animates with `motion.normal`.
/// What the score measured: the extracted text (text layer + local OCR),
/// or, when page images went to the AI, the text before that reading — a
/// low score there says nothing about the AI's result.
pub fn quality_caption(tier: Option<Tier>, q: f64) -> String {
    let word = if q >= theme::QUALITY_HIGH {
        "good"
    } else if q >= theme::QUALITY_MEDIUM {
        "fair"
    } else {
        "poor"
    };
    match tier {
        Some(Tier::VisionNeeded) => {
            format!("Text readability before AI image reading: {word} ({q:.2})")
        }
        _ => format!("Extracted text readability: {word} ({q:.2})"),
    }
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    let p = theme::palette(ui);
    let id = ui.scope_id().with(("section", title));
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let gap = (size::CONTROL - typ::CAPTION_STRONG.line) / 2.0;
    ui.add_space(space::XL - gap);
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(theme::v2(w, size::CONTROL), Sense::click());
    let open = state.is_open();
    if resp.clicked() {
        state.set_open(!open);
    }
    let cy = r.center().y;
    let hover = resp.hovered();
    let c = if hover { p.text } else { p.text_muted };
    paint_icon(
        ui,
        egui::pos2(r.left() + size::ICON_SM / 2.0, cy),
        if open {
            theme::icon::CHEVRON_DOWN
        } else {
            theme::icon::CHEVRON_RIGHT
        },
        size::ICON_SM,
        c,
    );
    let tr = paint_galley_v(
        ui,
        r.left() + size::ICON_SM + space::XS,
        cy,
        galley(ui, title, typ::CAPTION_STRONG),
        c,
    );
    if tr.right() + space::SM < r.right() {
        ui.painter().hline(
            (tr.right() + space::SM)..=r.right(),
            cy.round() + stroke::HAIRLINE / 2.0,
            theme::hairline(p.border),
        );
    }
    focus_ring(ui, &resp, r, theme::radius::SM, true);
    let o = state.is_open();
    resp.widget_info(|| {
        WidgetInfo::selected(
            egui::Role::Button,
            true,
            o,
            format!(
                "{title} section, {}",
                if o { "expanded" } else { "collapsed" }
            ),
        )
    });
    // Openness animates with motion.normal (0 under reduce motion).
    let t = dur(ui.ctx(), motion::NORMAL);
    let openness = ui.ctx().animate_bool_with_time(id.with("open"), o, t);
    state.store(ui.ctx());
    ui.add_space(space::SM - gap);
    crate::gui::components::reveal(ui, id, openness, body);
}

fn icon_line(ui: &mut Ui, tone: Tone, text: &str, w: f32) {
    let p = theme::palette(ui);
    let (r, _) = ui.allocate_exact_size(theme::v2(w, typ::BODY.line), Sense::hover());
    let c = if tone == Tone::Neutral {
        p.text_muted
    } else {
        p.tone(tone).1
    };
    paint_icon(
        ui,
        egui::pos2(r.left() + size::ICON / 2.0, r.center().y),
        tone_icon(tone),
        size::ICON,
        c,
    );
    let tw = w - size::ICON - space::SM;
    let t = ellipsize_end(ui, text, typ::BODY, tw);
    paint_text(
        ui,
        r.left() + size::ICON + space::SM,
        r.center().y,
        &t,
        typ::BODY,
        p.text,
    );
}

/// Icon + wrapped text (warnings are prose: never truncated in the detail).
fn icon_text(ui: &mut Ui, tone: Tone, text: &str, w: f32) {
    let p = theme::palette(ui);
    let tw = w - size::ICON - space::SM;
    let lines = crate::gui::components::wrapped_lines(ui, text, typ::BODY, tw);
    let (r, _) =
        ui.allocate_exact_size(theme::v2(w, typ::BODY.line * lines as f32), Sense::hover());
    paint_icon(
        ui,
        egui::pos2(r.left() + size::ICON / 2.0, r.top() + typ::BODY.line / 2.0),
        tone_icon(tone),
        size::ICON,
        p.tone(tone).1,
    );
    crate::gui::components::paint_wrapped(
        ui,
        egui::pos2(r.left() + size::ICON + space::SM, r.top()),
        tw,
        text,
        typ::BODY,
        p.text,
    );
}

fn text_well(ui: &mut Ui, text: &str, w: f32) {
    let p = theme::palette(ui);
    let id = ui.scope_id().with("show_all");
    let mut all = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let lines: Vec<&str> = text.lines().collect();
    let n = if all {
        lines.len()
    } else {
        lines.len().min(size::WELL_LINES)
    };
    let h = n.max(1) as f32 * typ::MONO_SM.line + space::SM * 2.0;
    let (r, _) = ui.allocate_exact_size(theme::v2(w, h), Sense::hover());
    ui.painter()
        .rect_filled(r, theme::cr(theme::radius::MD), p.surface_sunken);
    let inner = r.shrink(space::SM);
    for (k, l) in lines.iter().take(n).enumerate() {
        let cy = inner.top() + typ::MONO_SM.line * (k as f32 + 0.5);
        let l = ellipsize_end(ui, l, typ::MONO_SM, inner.width());
        paint_galley_v(ui, inner.left(), cy, galley(ui, &l, typ::MONO_SM), p.text);
    }
    if lines.len() > size::WELL_LINES {
        ui.add_space(space::XS);
        let t = if all { "Show less" } else { "Show all" };
        if Button::ghost(t).size(BSize::Sm).show(ui).clicked() {
            all = !all;
            ui.data_mut(|d| d.insert_temp(id, all));
        }
    }
}
