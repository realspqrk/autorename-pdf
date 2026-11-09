//! §4.3 Preview (toolbar, plan list, action bar), §4.5 progress, §4.6 Done.

use egui::{Rect, Ui};

use super::Action;
use crate::gui::components::plan_row::{PlanRow, RowStatus, RowView, TEXT_X};
use crate::gui::components::{
    Button, Field, Progress, galley, in_rect, paint_card, paint_galley_v, paint_icon, paint_text,
    plural, segmented,
};
use crate::gui::model::{self, Filter, Model, Phase, Row};
use crate::gui::theme::{self, Tone, size, space, stroke, typ};
use autorename_core::pipeline::RenameStatus;

/// "12 files · 2 need a look" (§4.3; the second part only when K > 0).
pub fn summary_text(m: &Model) -> String {
    let c = m.counts();
    let mut s = plural(c.total, "file", "files");
    match c.needs_look() {
        0 => {}
        1 => s.push_str(" · 1 needs a look"),
        k => s.push_str(&format!(" · {k} need a look")),
    }
    s
}

pub fn analysis_text(m: &Model) -> Option<String> {
    let Phase::Analyzing(a) = &m.phase else {
        return None;
    };
    let mut s = format!("Analyzing {} of {}", (a.done + 1).min(a.total), a.total);
    if let Some(eta) = a.eta_secs() {
        s.push_str(" · ");
        s.push_str(&eta_copy(eta));
    }
    Some(s)
}

pub fn eta_copy(secs: f32) -> String {
    if secs < 60.0 {
        format!("about {} s left", (secs / 5.0).ceil() as u32 * 5)
    } else {
        format!("about {} min left", (secs / 60.0).ceil() as u32)
    }
}

/// `list_w`: width of the list pane (progress spans only the list, §4.5).
pub fn toolbar(ui: &mut Ui, rect: Rect, list_w: f32, m: &Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(rect, 0, p.bg);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - stroke::HAIRLINE / 2.0,
        theme::hairline(p.border),
    );
    let cy = rect.center().y;
    let text = analysis_text(m).unwrap_or_else(|| summary_text(m));
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + space::LG, rect.top()),
        egui::pos2(rect.right() - space::LG, rect.bottom()),
    );
    let mut right_edge = inner.right();
    in_rect(ui, inner, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::MD;
            let add = Button::secondary("Add…")
                .icon(theme::icon::PLUS)
                .enabled(!m.busy())
                .a11y("Add files…")
                .show(ui);
            if add.clicked() {
                acts.push(Action::ChooseFiles);
            }
            right_edge = add.rect.left();
            // Filter only while something needs a look (§4.3).
            let c = m.counts();
            let k = c.needs_look();
            if k > 0 {
                let opts = [format!("All {}", c.total), format!("Needs a look {k}")];
                let filters = [Filter::All, Filter::NeedsLook];
                let mut sel = filters.iter().position(|f| *f == m.filter).unwrap_or(0);
                let r = segmented(ui, "Filter", &opts, &mut sel);
                if filters[sel] != m.filter {
                    acts.push(Action::SetFilter(filters[sel]));
                }
                right_edge = r.rect.left();
            }
        });
    });
    let max_w = right_edge - inner.left() - space::MD;
    // One style for the summary and the progress line (no size jump).
    let shown = crate::gui::components::ellipsize_end(ui, &text, typ::BODY, max_w);
    paint_text(ui, inner.left(), cy, &shown, typ::BODY, p.text_muted);
    if let Phase::Analyzing(a) = &m.phase {
        let pr = Rect::from_min_size(
            egui::pos2(rect.left(), rect.bottom() - size::PROGRESS_H),
            theme::v2(list_w, size::PROGRESS_H),
        );
        let label = format!("Analyzing, {} of {}", a.done, a.total);
        Progress {
            fraction: Some(a.done as f32 / a.total.max(1) as f32),
            label: &label,
        }
        .show_at(ui, pr);
    }
}

pub fn row_view<'a>(
    m: &'a Model,
    i: usize,
    r: &'a Row,
    line1: &'a str,
    warn: Option<&'a str>,
) -> RowView<'a> {
    // No badges in rows (§4.3): tier, metadata and name matching live in
    // the details panel.
    let done = r.report.is_some();
    RowView {
        id: egui::Id::unique(("row", r.source.as_path())),
        line1,
        old_name: "",
        warning: warn,
        status: r.status(),
        checked: r.checked,
        checkable: r.checkable(),
        selected: m.active == Some(i),
        edited: r.edited.is_some(),
        retry: m.retryable(i) && !m.busy(),
        open: !done && r.status() == RowStatus::Review,
        editable: !done && !m.busy() && matches!(r.status(), RowStatus::Ready | RowStatus::Review),
        read_only: done,
        focusable: m.active == Some(i),
        a11y: model::row_a11y(r),
    }
}

fn line1_for(r: &Row) -> String {
    match r.status() {
        RowStatus::Queued => "Waiting…".into(),
        RowStatus::Analyzing => "Reading text…".into(),
        RowStatus::Cancelled if r.report.is_none() => "Not analyzed · run stopped".into(),
        RowStatus::Error => r
            .error_summary()
            .unwrap_or_else(|| "Couldn't analyze the file".into()),
        _ => r.final_name().unwrap_or_default(),
    }
}

/// Plan list (scrolls). Returns the active row's rect (for scroll-into-view).
pub fn list(
    ui: &mut Ui,
    rect: Rect,
    m: &mut Model,
    scroll_to_active: bool,
    acts: &mut Vec<Action>,
) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(rect, 0, p.surface);
    let mut top = rect.top();
    if m.report.is_some() {
        let cw = rect.width() - space::LG * 2.0;
        let ch = done_card_h(ui, m, cw);
        let cr = Rect::from_min_size(
            egui::pos2(rect.left() + space::LG, rect.top() + space::LG),
            theme::v2(cw, ch),
        );
        done_card(ui, cr, m, acts);
        top = cr.bottom() + space::LG;
        ui.painter().hline(
            rect.x_range(),
            top - stroke::HAIRLINE / 2.0,
            theme::hairline(p.border),
        );
    }
    let lr = Rect::from_min_max(egui::pos2(rect.left(), top), rect.max);
    let visible = m.visible();
    in_rect(ui, lr, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("plan_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = theme::v2(0.0, 0.0);
                if visible.is_empty() {
                    let (r, _) = ui.allocate_exact_size(
                        theme::v2(ui.available_width(), size::ROW),
                        egui::Sense::hover(),
                    );
                    crate::gui::components::paint_text_center(
                        ui,
                        r,
                        "No files match this filter",
                        typ::CAPTION,
                        p.text_muted,
                    );
                }
                for &i in &visible {
                    let editing = m.editing.as_ref().is_some_and(|(e, _)| *e == i);
                    let h = if editing { size::ROW_EDIT } else { size::ROW };
                    let (rr, _) = ui.allocate_exact_size(
                        theme::v2(ui.available_width(), h),
                        egui::Sense::hover(),
                    );
                    if !ui.is_rect_visible(rr) && !(scroll_to_active && m.active == Some(i)) {
                        continue;
                    }
                    if editing {
                        edit_row(ui, rr, m, i, acts);
                        continue;
                    }
                    let r = &m.rows[i];
                    let line1 = line1_for(r);
                    let old = r.old_name();
                    let warn = r.problem(&m.name_nouns().0);
                    let mut v = row_view(m, i, r, &line1, warn.as_deref());
                    v.old_name = &old;
                    let out = PlanRow::show(ui, rr, v);
                    if scroll_to_active && m.active == Some(i) {
                        ui.scroll_to_rect(rr, None);
                    }
                    if out.toggled {
                        acts.push(Action::Toggle(i));
                        acts.push(Action::Activate(i));
                    } else if out.double_clicked && m.report.is_none() {
                        acts.push(Action::StartEdit(i));
                    } else if out.clicked {
                        acts.push(Action::Activate(i));
                    }
                    if out.retry {
                        acts.push(Action::Retry(i));
                    }
                    if out.open {
                        let path = m.rows[i].current_path(&m.restored).to_path_buf();
                        acts.push(Action::OpenPath(path));
                    }
                    if out.edit {
                        acts.push(Action::Activate(i));
                        acts.push(Action::StartEdit(i));
                    }
                }
            });
    });
}

fn edit_row(ui: &mut Ui, rr: Rect, m: &mut Model, i: usize, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let shape = crate::gui::components::plan_row::shape_rect(rr);
    ui.painter().rect_filled(rr, 0, p.surface);
    ui.painter()
        .rect_filled(shape, theme::cr(theme::radius::MD), p.selection);
    let r = f32::from(theme::radius::MD);
    ui.painter().rect_filled(
        Rect::from_min_size(
            shape.min + theme::v2(0.0, r),
            theme::v2(stroke::SELECTED_BAR, shape.height() - r * 2.0),
        ),
        0,
        p.accent,
    );
    let err = m.editing.as_ref().and_then(|(_, t)| m.edit_error(i, t));
    let fr = Rect::from_min_max(
        egui::pos2(rr.left() + TEXT_X, rr.top() + space::SM),
        egui::pos2(rr.right() - space::LG, rr.bottom()),
    );
    let old = m.rows[i].old_name();
    in_rect(ui, fr, |ui| {
        ui.spacing_mut().item_spacing = theme::v2(0.0, space::XS);
        let Some((_, text)) = m.editing.as_mut() else {
            return;
        };
        let fid = egui::Id::unique("inline_edit");
        let helper = format!("from {old} · Enter to save, Esc to cancel");
        // The error keeps the way out visible (§4.3).
        let shown_err = err.as_ref().map(|e| format!("{e} · Esc to cancel"));
        let resp = Field::new("Edit proposed name", text)
            .mono()
            .suffix(".pdf")
            .error(shown_err.as_deref())
            .helper(Some(&helper))
            .id(fid)
            .show(ui)
            .edit;
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        let (enter, esc) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if esc {
            acts.push(Action::CancelEdit);
        } else if enter && err.is_none() {
            acts.push(Action::CommitEdit);
        } else if enter {
            resp.request_focus();
        }
    });
}

pub(crate) struct DoneSummary {
    icon: char,
    tone: Tone,
    pub title: String,
    /// Plain caption, only non-zero parts (§4.6); may be empty.
    pub line: String,
}

pub(crate) fn done_summary(m: &Model) -> Option<DoneSummary> {
    let rep = m.report.as_ref()?;
    let s = &rep.summary;
    let failed = failed_count(m);
    let (icon, tone) = if m.undone {
        (theme::icon::UNDO_2, Tone::Neutral)
    } else if failed > 0 || s.cancelled > 0 || s.incomplete > 0 {
        (theme::icon::TRIANGLE_ALERT, Tone::Warning)
    } else {
        (theme::icon::CIRCLE_CHECK, Tone::Success)
    };
    // Every file of the plan is accounted for: renamed + couldn't be
    // renamed + already named + not selected + couldn't be read + not
    // analyzed (Stop) + left as it was (rename stopped) = all files.
    let left = m.left_out;
    let total = s.total + left.not_selected + left.unread + left.not_analyzed;
    let title = if m.undone {
        format!("Undid {}", plural(s.renamed, "rename", "renames"))
    } else if s.cancelled > 0 {
        format!("Stopped · renamed {} of {total} files", s.renamed)
    } else if failed > 0 {
        format!("Renamed {} of {total} files", s.renamed)
    } else {
        format!("Renamed {}", plural(s.renamed, "file", "files"))
    };
    let mut parts = Vec::new();
    if !m.undone {
        if failed > 0 {
            parts.push(format!("{failed} couldn't be renamed"));
        }
        if s.unchanged > 0 {
            parts.push(plural(
                s.unchanged,
                "already had its name",
                "already had their names",
            ));
        }
        if left.not_selected > 0 {
            parts.push(format!("{} not selected", left.not_selected));
        }
        if left.unread > 0 {
            parts.push(format!("{} couldn't be read", left.unread));
        }
        if left.not_analyzed > 0 {
            parts.push(format!("{} not analyzed", left.not_analyzed));
        }
        if s.cancelled > 0 {
            parts.push(plural(s.cancelled, "left as it was", "left as they were"));
        }
        if s.incomplete > 0 {
            parts.push(format!("{} renamed with a problem", s.incomplete));
        }
        let review = m.review_entries();
        if review > 0 && m.name_label().is_some() {
            let (one, many) = m.name_nouns();
            parts.push(if review == 1 {
                format!("1 new {one} needs a look")
            } else {
                format!("{review} new {many} need a look")
            });
        }
    }
    Some(DoneSummary {
        icon,
        tone,
        title,
        line: parts.join(" · "),
    })
}

fn failed_count(m: &Model) -> usize {
    m.report.as_ref().map_or(0, |rep| {
        rep.files
            .iter()
            .filter(|f| f.rename_status == RenameStatus::Failed)
            .count()
    })
}

/// Retry label from its actual targets (§4.6): failed only → "Try again
/// (2)"; failed + not reached → "Try remaining 3 files".
fn retry_label(m: &Model) -> Option<String> {
    let n = m.retry_targets().len();
    match n {
        0 => None,
        n if n == failed_count(m) => Some(format!("Try again ({n})")),
        n => Some(format!("Try remaining {}", plural(n, "file", "files"))),
    }
}

/// Labels of the Done card's secondary actions, in order.
fn done_actions(m: &Model) -> Vec<String> {
    let mut v = vec!["Undo rename".to_string(), "Open folder".into()];
    if let Some(l) = retry_label(m) {
        v.push(l);
    }
    if m.review_entries() > 0 && m.name_label().is_some() {
        v.push(format!("Review {}", m.name_nouns().1));
    }
    v
}

/// Rows the action buttons need inside `inner_w` (they wrap, never overflow).
fn action_rows(ui: &Ui, m: &Model, inner_w: f32) -> usize {
    let mut rows = 1;
    let mut x = 0.0;
    for l in done_actions(m) {
        let w = Button::secondary(&l).icon(theme::icon::CHECK).width(ui);
        if x > 0.0 && x + space::SM + w > inner_w {
            rows += 1;
            x = w;
        } else {
            x += if x > 0.0 { space::SM } else { 0.0 } + w;
        }
    }
    rows
}

/// Card height from the wrapped summary (§4.6; padding 24, gaps 8/16).
pub fn done_card_h(ui: &Ui, m: &Model, width: f32) -> f32 {
    let inner_w = width - space::XL * 2.0;
    let lines = done_summary(m).map_or(0, |d| caption_lines(ui, &d.line, inner_w).len());
    let rows = action_rows(ui, m, inner_w) as f32;
    let caption = if lines > 0 {
        space::SM + typ::BODY.line * lines as f32
    } else {
        0.0
    };
    space::XL
        + typ::HEADLINE.line
        + caption
        + space::LG
        + size::CONTROL * rows
        + space::SM * (rows - 1.0)
        + space::XL
}

fn caption_lines(ui: &Ui, line: &str, width: f32) -> Vec<String> {
    if line.is_empty() {
        return Vec::new();
    }
    crate::gui::components::part_lines(ui, line, typ::BODY, width)
}

fn done_card(ui: &mut Ui, rect: Rect, m: &Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    paint_card(ui, rect, p);
    let Some(rep) = &m.report else { return };
    let Some(d) = done_summary(m) else { return };
    let inner = rect.shrink(space::XL);
    let cy = inner.top() + typ::HEADLINE.line / 2.0;
    let c = if d.tone == Tone::Neutral {
        p.text_muted
    } else {
        p.tone(d.tone).1
    };
    paint_icon(
        ui,
        egui::pos2(inner.left() + size::ICON_LG / 2.0, cy),
        d.icon,
        size::ICON_LG,
        c,
    );
    let tx = inner.left() + size::ICON_LG + space::SM;
    let title =
        crate::gui::components::ellipsize_end(ui, &d.title, typ::HEADLINE, inner.right() - tx);
    paint_galley_v(ui, tx, cy, galley(ui, &title, typ::HEADLINE), p.text);
    let top = inner.top() + typ::HEADLINE.line + space::SM;
    for (k, l) in caption_lines(ui, &d.line, inner.width()).iter().enumerate() {
        let cy = top + typ::BODY.line * (k as f32 + 0.5);
        paint_text(ui, inner.left(), cy, l, typ::BODY, p.text_muted);
    }
    let rows = action_rows(ui, m, inner.width()) as f32;
    let br = Rect::from_min_max(
        egui::pos2(
            inner.left(),
            inner.bottom() - size::CONTROL * rows - space::SM * (rows - 1.0),
        ),
        inner.max,
    );
    in_rect(ui, br, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = theme::v2(space::SM, space::SM);
            let can_undo = rep.batch_id.is_some() && !m.undone && !m.busy();
            if Button::secondary("Undo rename")
                .icon(theme::icon::UNDO_2)
                .enabled(can_undo)
                .tooltip("Restores the original names, removes the written metadata and forgets names learned in this run")
                .show(ui)
                .clicked()
            {
                acts.push(Action::Undo);
            }
            if Button::secondary("Open folder")
                .icon(theme::icon::FOLDER_OPEN)
                .show(ui)
                .clicked()
            {
                acts.push(Action::OpenFolder);
            }
            let labels = done_actions(m);
            let n = m.retry_targets().len();
            if n > 0
                && Button::secondary(&labels[2])
                    .icon(theme::icon::ROTATE_CCW)
                    .enabled(!m.busy())
                    .tooltip("Analyzes the failed and not-reached files again in a new plan; renamed files stay renamed")
                    .show(ui)
                    .clicked()
            {
                acts.push(Action::RetryFailed);
            }
            if let Some(l) = labels.get(if n > 0 { 3 } else { 2 })
                && Button::secondary(l)
                    .icon(theme::icon::BUILDING_2)
                    .show(ui)
                    .clicked()
            {
                acts.push(Action::SwitchTab(model::Tab::Companies));
            }
        });
    });
}

pub fn action_bar(ui: &mut Ui, rect: Rect, m: &Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(rect, 0, p.surface);
    ui.painter().hline(
        rect.x_range(),
        rect.top() + stroke::HAIRLINE / 2.0,
        theme::hairline(p.border),
    );
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + space::LG, rect.top()),
        egui::pos2(rect.right() - space::LG, rect.bottom()),
    );
    let cy = rect.center().y;
    let c = m.counts();
    if let Phase::Renaming { done, total } = m.phase {
        let pr = Rect::from_min_size(
            egui::pos2(inner.left(), cy - size::PROGRESS_H / 2.0),
            theme::v2(size::PROGRESS_RENAME_W, size::PROGRESS_H),
        );
        let label = format!("Renaming {} of {total}", (done + 1).min(total));
        Progress {
            fraction: Some(done as f32 / total.max(1) as f32),
            label: &label,
        }
        .show_at(ui, pr);
        paint_text(
            ui,
            pr.right() + space::MD,
            cy,
            &label,
            typ::BODY,
            p.text_muted,
        );
    } else if m.report.is_some() {
        paint_text(
            ui,
            inner.left(),
            cy,
            "Undo is also available in History.",
            typ::BODY,
            p.text_muted,
        );
    } else {
        let mut t = format!("{} of {} selected", c.checked, c.total);
        if c.edited > 0 {
            t.push_str(&format!(
                " · {}",
                plural(c.edited, "name edited", "names edited")
            ));
        }
        paint_text(ui, inner.left(), cy, &t, typ::BODY, p.text_muted);
    }
    in_rect(ui, inner, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            if m.report.is_some() {
                if Button::primary("Rename more files")
                    .enabled(!m.busy())
                    .show(ui)
                    .clicked()
                {
                    acts.push(Action::RenameMore);
                }
                return;
            }
            let label = format!("Rename {}", plural(c.checked, "file", "files"));
            let can = m.can_rename();
            let mut b = Button::primary(&label)
                .icon(theme::icon::CHECK)
                .enabled(can)
                .loading(m.renaming());
            if m.editing.is_some() {
                b = b.tooltip("Finish or cancel the filename edit first");
            } else if c.checked == 0 {
                b = b.tooltip("Select at least one file");
            } else if m.analyzing() {
                b = b.tooltip("Wait for the analysis to finish, or stop it");
            }
            if b.show(ui).clicked() {
                acts.push(Action::Rename);
            }
            if m.analyzing() || m.renaming() {
                let tip = if m.renaming() {
                    "Stops before the next file; files already renamed stay renamed"
                } else {
                    "Files analyzed so far stay in the list"
                };
                if Button::secondary("Stop").tooltip(tip).show(ui).clicked() {
                    acts.push(Action::Stop);
                }
            } else if Button::secondary("Clear list")
                .enabled(!m.busy())
                .show(ui)
                .clicked()
            {
                acts.push(Action::ClearList);
            }
        });
    });
}
