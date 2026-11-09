//! §4.8 Companies: toolbar (filter + search), φ split list | detail, review
//! actions, merge dialog (560) and rename dialog.

use egui::{Id, Key, Rect, Sense, Ui, WidgetInfo};

use super::Action;
use crate::gui::components::{
    Badge, Button, Field, anim, ellipsize_end, focus_ring, in_rect, label, mix, paint_text,
    paint_text_center, paint_wrapped, segmented, wrapped_lines,
};
use crate::gui::model::{Model, noun};
use crate::gui::pages::{CompanyFilter, entry_caption, spellings_caption};
use crate::gui::theme::{self, Tone, size, space, stroke, typ};
use autorename_core::harmonize::{self, Source};

/// Lower-case nouns of the shown list ("company", "companies"; "venue",
/// "venues"): the loaded list's field, else the active profile's.
pub fn nouns(m: &Model) -> (String, String) {
    match &m.companies.db {
        Some(db) => {
            let (one, many) = harmonize::field_label(&db.field);
            (noun(&one), noun(&many))
        }
        None => m.name_nouns(),
    }
}

pub fn companies(ui: &mut Ui, body: Rect, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(body, 0, p.bg);
    let tr = Rect::from_min_size(body.min, theme::v2(body.width(), size::TOOLBAR));
    toolbar(ui, tr, m);
    let content = Rect::from_min_max(egui::pos2(body.left(), tr.bottom()), body.max);
    // The detail pane holds the review actions: always visible (§4.8).
    let dw = theme::detail_width(ui.ctx().content_rect().width());
    let list = Rect::from_min_size(
        content.min,
        theme::v2(content.width() - dw - stroke::HAIRLINE, content.height()),
    );
    let detail_r = Rect::from_min_max(
        egui::pos2(list.right() + stroke::HAIRLINE, content.top()),
        content.max,
    );
    ui.painter().vline(
        list.right() + stroke::HAIRLINE / 2.0,
        content.y_range(),
        theme::hairline(p.border),
    );
    list_pane(ui, list, m);
    detail(ui, detail_r, m, acts);
    if m.companies.merge.is_some() {
        merge_dialog(ui.ctx(), m, acts);
    }
    if m.companies.rename.is_some() {
        rename_dialog(ui.ctx(), m, acts);
    }
}

fn toolbar(ui: &mut Ui, rect: Rect, m: &mut Model) {
    let p = theme::palette(ui);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - stroke::HAIRLINE / 2.0,
        theme::hairline(p.border),
    );
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + space::LG, rect.top()),
        egui::pos2(rect.right() - space::LG, rect.bottom()),
    );
    let (_, many) = nouns(m);
    let search_label = format!("Search {many}");
    let c = &mut m.companies;
    let n = c.entries().len();
    let review = c.review_count();
    in_rect(ui, inner, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::MD;
            let opts = [
                format!("All {n}"),
                format!("Needs a look {review}"),
                "Added automatically".to_string(),
                "Added by you".to_string(),
            ];
            let filters = [
                CompanyFilter::All,
                CompanyFilter::Review,
                CompanyFilter::Learned,
                CompanyFilter::Manual,
            ];
            let mut sel = filters.iter().position(|f| *f == c.filter).unwrap_or(0);
            segmented(ui, "List filter", &opts, &mut sel);
            c.filter = filters[sel];
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                Field::new(&search_label, &mut c.search)
                    .hint("Search")
                    .width(size::SEARCH_W)
                    .show(ui);
            });
        });
    });
}

fn list_pane(ui: &mut Ui, rect: Rect, m: &mut Model) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(rect, 0, p.surface);
    let visible = m.companies.visible();
    if visible.is_empty() {
        let (_, many) = nouns(m);
        let t = if m.companies.db.is_none() && m.companies.loading {
            format!("Loading {many}…")
        } else if m.companies.entries().is_empty() {
            format!("No {many} yet. They are learned when you rename files.")
        } else {
            format!("No {many} match this filter")
        };
        paint_text_center(ui, rect, &t, typ::CAPTION, p.text_muted);
        return;
    }
    if m.companies.active.is_none_or(|a| !visible.contains(&a)) {
        m.companies.active = visible.first().copied();
    }
    in_rect(ui, rect, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("companies_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = theme::v2(0.0, 0.0);
                for &i in &visible {
                    let (rr, _) = ui.allocate_exact_size(
                        theme::v2(ui.available_width(), size::ROW),
                        Sense::hover(),
                    );
                    if !ui.is_rect_visible(rr) {
                        continue;
                    }
                    let e = m.companies.entries()[i].clone();
                    let sel = m.companies.active == Some(i);
                    let id = Id::unique(("company", &e.canonical));
                    let resp = ui.interact(rr, id, if sel { Sense::click() } else { Sense::CLICK });
                    let shape = crate::gui::components::plan_row::shape_rect(rr);
                    let h = anim(ui, id.with("h"), resp.hovered());
                    if !sel {
                        ui.painter().hline(
                            (rr.left() + space::LG)..=(rr.right() - space::LG),
                            rr.bottom() - stroke::HAIRLINE / 2.0,
                            theme::hairline(p.border),
                        );
                    }
                    let fill = if sel {
                        p.selection
                    } else {
                        mix(p.surface, p.surface_hover, h)
                    };
                    ui.painter()
                        .rect_filled(shape, theme::cr(theme::radius::MD), fill);
                    if sel {
                        // Shared row treatment (§5.3): 2 px accent bar inside
                        // the radius.md corners, like PlanRow.
                        let r6 = f32::from(theme::radius::MD);
                        ui.painter().rect_filled(
                            Rect::from_min_size(
                                shape.min + theme::v2(0.0, r6),
                                theme::v2(stroke::SELECTED_BAR, shape.height() - r6 * 2.0),
                            ),
                            0,
                            p.accent,
                        );
                    }
                    let l1 = rr.top() + space::SM + typ::BODY.line / 2.0;
                    let l2 = l1 + typ::BODY.line / 2.0 + space::XS + typ::CAPTION.line / 2.0;
                    let mut right = rr.right() - space::LG;
                    // Provenance is in the caption; only the review state
                    // gets a badge.
                    let mut badges = Vec::new();
                    if e.review {
                        badges.push(
                            Badge::new(Tone::Warning, "Needs a look").icon(theme::icon::FLAG),
                        );
                    }
                    for b in badges.iter().rev() {
                        let bw = b.width(ui);
                        let r = Rect::from_min_size(
                            egui::pos2(right - bw, l1 - size::BADGE / 2.0),
                            theme::v2(bw, size::BADGE),
                        );
                        b.paint_at(ui, r);
                        right = r.left() - space::XS;
                    }
                    let x = rr.left() + space::LG;
                    let name =
                        ellipsize_end(ui, &e.canonical, typ::BODY_STRONG, right - x - space::MD);
                    paint_text(ui, x, l1, &name, typ::BODY_STRONG, p.text);
                    paint_text(ui, x, l2, &entry_caption(&e), typ::CAPTION, p.text_muted);
                    focus_ring(ui, &resp, shape, theme::radius::MD, true);
                    let a11y = format!(
                        "{}, {}{}",
                        e.canonical,
                        entry_caption(&e),
                        if e.review { ", needs a look" } else { "" }
                    );
                    resp.widget_info(|| {
                        WidgetInfo::selected(egui::Role::ListItem, true, sel, &a11y)
                    });
                    if resp.clicked() {
                        m.companies.active = Some(i);
                    }
                    if sel && resp.has_focus() {
                        let pos = visible.iter().position(|v| *v == i).unwrap_or(0);
                        let (up, down) = ui.input(|inp| {
                            (
                                inp.key_pressed(Key::ArrowUp),
                                inp.key_pressed(Key::ArrowDown),
                            )
                        });
                        let next = if down {
                            visible.get(pos + 1)
                        } else if up && pos > 0 {
                            visible.get(pos - 1)
                        } else {
                            None
                        };
                        if let Some(&n) = next {
                            m.companies.active = Some(n);
                            let nid = Id::unique(("company", &m.companies.entries()[n].canonical));
                            ui.memory_mut(|mem| mem.request_focus(nid));
                        }
                    }
                    if sel {
                        ui.memory_mut(|mem| {
                            mem.set_focus_lock_filter(
                                id,
                                egui::EventFilter {
                                    vertical_arrows: true,
                                    ..Default::default()
                                },
                            )
                        });
                    }
                }
            });
    });
}

fn detail(ui: &mut Ui, rect: Rect, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let (one, _) = nouns(m);
    ui.painter().rect_filled(rect, 0, p.surface);
    let Some(e) = m
        .companies
        .active
        .and_then(|a| m.companies.entries().get(a))
        .cloned()
    else {
        paint_text_center(
            ui,
            rect,
            "Select an entry to see details",
            typ::CAPTION,
            p.text_muted,
        );
        return;
    };
    // One list edit at a time; none while the list is (re)loading.
    let idle = !m.companies.busy();
    in_rect(ui, rect, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("company_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(theme::margin_all(space::LG))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
                        let w = ui.available_width();
                        ui.set_width(w);
                        label(ui, &e.canonical, typ::TITLE_SM, p.text);
                        ui.add_space(space::XS);
                        // Provenance lives in the Source section below.
                        label(ui, &spellings_caption(&e), typ::CAPTION, p.text_muted);
                        if let Some(err) = &m.companies.error {
                            ui.add_space(space::SM);
                            let n = wrapped_lines(ui, err, typ::CAPTION, w);
                            let (r, _) = ui.allocate_exact_size(
                                theme::v2(w, typ::CAPTION.line * n as f32),
                                Sense::hover(),
                            );
                            paint_wrapped(ui, r.min, w, err, typ::CAPTION, p.danger);
                        }
                        ui.add_space(space::XL);
                        crate::gui::components::section_header(ui, "Other spellings");
                        ui.add_space(space::SM);
                        if e.aliases.is_empty() && e.learned.is_empty() {
                            label(ui, "No other spellings yet.", typ::BODY, p.text_muted);
                        }
                        for (a, learned) in e
                            .aliases
                            .iter()
                            .map(|a| (a, false))
                            .chain(e.learned.iter().map(|a| (a, true)))
                        {
                            let (r, _) =
                                ui.allocate_exact_size(theme::v2(w, size::CONTROL), Sense::hover());
                            // Right to left: remove button, provenance badge
                            // (only where Source doesn't already say it: your
                            // entry, learned spelling); the spelling gets the
                            // rest.
                            let mut right = r.right();
                            if learned {
                                right -= size::CONTROL_SM + space::SM;
                                if e.source == Source::User {
                                    let b = Badge::new(Tone::Neutral, "Added automatically");
                                    let bw = b.width(ui);
                                    b.paint_at(
                                        ui,
                                        Rect::from_min_size(
                                            egui::pos2(
                                                right - bw,
                                                r.center().y - size::BADGE / 2.0,
                                            ),
                                            theme::v2(bw, size::BADGE),
                                        ),
                                    );
                                    right -= bw + space::SM;
                                }
                            }
                            let t = ellipsize_end(ui, a, typ::MONO_SM, (right - r.left()).max(0.0));
                            if t != *a {
                                let tr = Rect::from_min_max(r.min, egui::pos2(right, r.bottom()));
                                ui.interact(
                                    tr,
                                    ui.scope_id().with(("spelling", a)),
                                    Sense::hover(),
                                )
                                .on_hover_text(a.as_str());
                            }
                            paint_text(ui, r.left(), r.center().y, &t, typ::MONO_SM, p.text);
                            if learned {
                                let xr = Rect::from_center_size(
                                    egui::pos2(r.right() - size::CONTROL_SM / 2.0, r.center().y),
                                    theme::v2(size::CONTROL_SM, size::CONTROL_SM),
                                );
                                let a11y = format!("Remove spelling {a}");
                                if Button::icon_only(theme::icon::X, &a11y)
                                    .size(crate::gui::components::BSize::Sm)
                                    .enabled(idle)
                                    .show_at(ui, xr)
                                    .clicked()
                                {
                                    acts.push(Action::RemoveAlias(e.canonical.clone(), a.clone()));
                                }
                            }
                        }
                        ui.add_space(space::XL);
                        crate::gui::components::section_header(ui, "Source");
                        ui.add_space(space::SM);
                        let src = match e.source {
                            Source::Auto => "Added automatically while renaming",
                            Source::User => "Added or confirmed by you",
                        };
                        label(ui, src, typ::BODY, p.text);
                        if let Some(d) = &e.added {
                            ui.add_space(space::XS);
                            label(ui, &format!("Added {d}"), typ::CAPTION, p.text_muted);
                        }
                        ui.add_space(space::XL);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = theme::v2(space::SM, space::SM);
                            let keep = format!("Keep as new {one}");
                            if e.review
                                && Button::primary(&keep)
                                    .icon(theme::icon::CHECK)
                                    .enabled(idle)
                                    .show(ui)
                                    .clicked()
                            {
                                acts.push(Action::KeepCompany(e.canonical.clone()));
                            }
                            if Button::secondary("Merge into…")
                                .icon(theme::icon::GIT_MERGE)
                                .enabled(idle)
                                .show(ui)
                                .clicked()
                            {
                                m.companies.merge = Some(crate::gui::pages::MergeDialog {
                                    from: e.canonical.clone(),
                                    ..Default::default()
                                });
                            }
                            if Button::secondary("Rename…")
                                .icon(theme::icon::PENCIL)
                                .enabled(idle)
                                .show(ui)
                                .clicked()
                            {
                                m.companies.rename =
                                    Some((e.canonical.clone(), e.canonical.clone()));
                            }
                            // Your own entries: merge and rename keep every
                            // hand-written spelling (`harmonize::merge` /
                            // `rename`); the file stays one click away.
                            if e.source == Source::User
                                && Button::secondary("Open file")
                                    .icon(theme::icon::EXTERNAL_LINK)
                                    .show(ui)
                                    .clicked()
                            {
                                acts.push(Action::OpenCompaniesFile);
                            }
                        });
                    });
            });
    });
}

fn modal_frame(p: &theme::Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.elevated(3))
        .stroke(theme::hairline(p.border))
        .corner_radius(theme::cr(theme::radius::XL))
        .inner_margin(theme::margin_all(space::XL))
        .shadow(p.shadow(3))
}

/// Dialog footer in platform order, right-aligned (§5.11).
fn footer(ui: &mut Ui, primary: Button<'_>, cancel: &str) -> (bool, bool) {
    let mut out = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        if cfg!(windows) {
            out.1 = Button::secondary(cancel).show(ui).clicked();
            out.0 = primary.show(ui).clicked();
        } else {
            out.0 = primary.show(ui).clicked();
            out.1 = Button::secondary(cancel).show(ui).clicked();
        }
    });
    out
}

fn merge_dialog(ctx: &egui::Context, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::pal(ctx.global_style().visuals.dark_mode);
    let Some(md) = m.companies.merge.clone() else {
        return;
    };
    let Some(db) = m.companies.db.clone() else {
        return;
    };
    let (one, many) = nouns(m);
    let search_label = format!("Search {many}");
    let mut close = false;
    let mut search = md.search.clone();
    let mut target = md.target.clone();
    let modal = egui::Modal::new(Id::unique("merge_dialog"))
        .frame(modal_frame(p))
        .backdrop_color(p.scrim)
        .show(ctx, |ui| {
            let w = size::DIALOG_MD - space::XL * 2.0;
            ui.set_width(w);
            ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
            label(ui, &format!("Merge {} into…", md.from), typ::TITLE, p.text);
            ui.add_space(space::LG);
            Field::new(&search_label, &mut search)
                .hint("Search")
                .width(w)
                .show(ui);
            ui.add_space(space::SM);
            let q = search.to_lowercase();
            let mut cands: Vec<(String, f64)> = db
                .entries
                .iter()
                .filter(|e| e.canonical != md.from)
                .filter(|e| q.is_empty() || e.canonical.to_lowercase().contains(&q))
                .map(|e| {
                    let s = harmonize::similarity(
                        &harmonize::normalize(db.kind(), &md.from),
                        &harmonize::normalize(db.kind(), &e.canonical),
                    );
                    (e.canonical.clone(), s)
                })
                .collect();
            cands.sort_by(|a, b| b.1.total_cmp(&a.1));
            cands.truncate(6);
            for (name, score) in &cands {
                let (r, _) = ui.allocate_exact_size(theme::v2(w, size::CONTROL), Sense::hover());
                let id = Id::unique(("merge_target", name));
                let resp = ui.interact(r, id, Sense::click());
                let sel = target.as_deref() == Some(name.as_str());
                let h = anim(ui, id.with("h"), resp.hovered());
                let fill = if sel {
                    p.selection
                } else {
                    mix(egui::Color32::TRANSPARENT, p.surface_hover, h)
                };
                ui.painter()
                    .rect_filled(r, theme::cr(theme::radius::MD), fill);
                paint_text(
                    ui,
                    r.left() + space::MD,
                    r.center().y,
                    name,
                    typ::BODY,
                    p.text,
                );
                // The score is expert detail: tooltip only.
                let sc = format!("Similarity {score:.2}");
                focus_ring(ui, &resp, r, theme::radius::MD, true);
                let resp = resp.on_hover_text(&sc);
                resp.widget_info(|| {
                    WidgetInfo::selected(egui::Role::RadioButton, true, sel, name.as_str())
                });
                if resp.clicked() {
                    target = Some(name.clone());
                }
            }
            if cands.is_empty() {
                label(
                    ui,
                    &format!("No other {many} match."),
                    typ::BODY,
                    p.text_muted,
                );
            }
            ui.add_space(space::LG);
            // What the merge changes, and what it doesn't (existing files).
            let preview = match &target {
                Some(t) => {
                    format!("Future documents use {t}. Existing file names stay as they are.")
                }
                None => format!("Choose the {one} to keep."),
            };
            crate::gui::components::paragraph(ui, &preview, typ::BODY, p.text_muted);
            ui.add_space(space::XL);
            let merge_label = target
                .as_ref()
                .map_or_else(|| "Merge".to_string(), |t| format!("Merge into {t}"));
            let (ok, cancel) = footer(
                ui,
                // A long name ends in "…" (full text in tooltip/AccessKit);
                // Cancel always stays in the dialog.
                Button::primary(&merge_label)
                    .max_width(w - Button::secondary("Cancel").width(ui) - space::SM)
                    .enabled(target.is_some() && !m.companies.busy()),
                "Cancel",
            );
            if ok && let Some(t) = &target {
                acts.push(Action::MergeCompany(md.from.clone(), t.clone()));
                close = true;
            }
            if cancel {
                close = true;
            }
        });
    if modal.should_close() || ctx.input(|i| i.key_pressed(Key::Escape)) {
        close = true;
    }
    if close {
        m.companies.merge = None;
    } else if let Some(md) = m.companies.merge.as_mut() {
        md.search = search;
        md.target = target;
    }
}

fn rename_dialog(ctx: &egui::Context, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::pal(ctx.global_style().visuals.dark_mode);
    let Some((old, mut new)) = m.companies.rename.clone() else {
        return;
    };
    let mut close = false;
    let title = format!("Rename {}", nouns(m).0);
    let modal = egui::Modal::new(Id::unique("rename_company"))
        .frame(modal_frame(p))
        .backdrop_color(p.scrim)
        .show(ctx, |ui| {
            let w = size::DIALOG_SM - space::XL * 2.0;
            ui.set_width(w);
            ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
            label(ui, &title, typ::TITLE, p.text);
            ui.add_space(space::LG);
            let helper = format!("\u{201c}{old}\u{201d} is kept as another spelling.");
            let empty = new.trim().is_empty();
            Field::new("New name", &mut new)
                .label("New name")
                .helper(Some(&helper))
                .error(empty.then_some("Name can't be empty"))
                .width(w)
                .show(ui);
            ui.add_space(space::XL);
            let (ok, cancel) = footer(
                ui,
                Button::primary("Rename")
                    .enabled(!empty && new.trim() != old && !m.companies.busy()),
                "Cancel",
            );
            if ok {
                acts.push(Action::RenameCompany(old.clone(), new.trim().to_string()));
                close = true;
            }
            close |= cancel;
        });
    if modal.should_close() || ctx.input(|i| i.key_pressed(Key::Escape)) {
        close = true;
    }
    m.companies.rename = if close { None } else { Some((old, new)) };
}
