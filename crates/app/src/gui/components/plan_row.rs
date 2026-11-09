//! §4.3 PlanRow: 56 h, padding 8/16, checkbox · status icon · two filename
//! lines · Try again on errors. No badges. One AccessKit ListItem.

use egui::{Rect, Response, Sense, Ui, WidgetInfo};

use super::button::{BSize, Button};
use super::{
    anim, ellipsize_end, ellipsize_middle, focus_ring, mix, paint_checkbox, paint_icon,
    paint_spinner, paint_text,
};
use crate::gui::theme::{self, Tone, size, space, stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStatus {
    Queued,
    Analyzing,
    Ready,
    Review,
    Error,
    Renamed,
    Unchanged,
    /// Not reached: the run was stopped.
    Cancelled,
}

impl RowStatus {
    pub fn icon(self) -> Option<(char, Tone)> {
        match self {
            RowStatus::Ready | RowStatus::Renamed => {
                Some((theme::icon::CIRCLE_CHECK, Tone::Success))
            }
            RowStatus::Unchanged => Some((theme::icon::CIRCLE_CHECK, Tone::Neutral)),
            RowStatus::Cancelled => Some((theme::icon::MINUS, Tone::Neutral)),
            RowStatus::Review => Some((theme::icon::TRIANGLE_ALERT, Tone::Warning)),
            RowStatus::Error => Some((theme::icon::CIRCLE_X, Tone::Danger)),
            RowStatus::Queued | RowStatus::Analyzing => None,
        }
    }
}

pub struct RowView<'a> {
    pub id: egui::Id,
    /// Line 1: proposed name / step text / error summary.
    pub line1: &'a str,
    /// Line 2 after "from ".
    pub old_name: &'a str,
    pub warning: Option<&'a str>,
    pub status: RowStatus,
    pub checked: bool,
    pub checkable: bool,
    pub selected: bool,
    pub edited: bool,
    pub retry: bool,
    pub read_only: bool,
    /// Only the active row is in the Tab order; arrows move between rows.
    pub focusable: bool,
    pub a11y: String,
}

#[derive(Default)]
pub struct RowResponse {
    pub clicked: bool,
    pub double_clicked: bool,
    pub toggled: bool,
    pub retry: bool,
    pub focused: bool,
    pub rect: Option<Rect>,
    pub row: Option<Response>,
}

pub struct PlanRow;

/// Left edge of line 1 relative to the row: padding 8 + checkbox hit 32 +
/// status icon 16 + gap 12.
pub const TEXT_X: f32 = space::SM + size::CONTROL + size::ICON + space::MD;
/// Vertical centres of the two row lines.
pub const ROW_L1: f32 = space::LG + space::XS;
pub const ROW_L2: f32 = ROW_L1 * 2.0;

/// Row background shape: inset `space.xs` from the list edges, radius.md.
pub fn shape_rect(rect: Rect) -> Rect {
    rect.shrink2(theme::v2(space::XS, 0.0))
}

impl PlanRow {
    pub fn show(ui: &mut Ui, rect: Rect, v: RowView<'_>) -> RowResponse {
        let p = theme::palette(ui);
        let sense = if v.focusable {
            Sense::click()
        } else {
            Sense::CLICK
        };
        let row = ui.interact(rect, v.id, sense);
        if v.focusable {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    v.id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
        }
        let mut out = RowResponse {
            clicked: row.clicked(),
            double_clicked: row.double_clicked(),
            focused: row.has_focus(),
            rect: Some(rect),
            ..Default::default()
        };
        let hover = anim(ui, v.id.with("h"), row.hovered());
        let shape = shape_rect(rect);
        let painter = ui.painter();
        painter.rect_filled(rect, 0, p.surface);
        if !v.selected {
            painter.hline(
                (rect.left() + space::LG)..=(rect.right() - space::LG),
                rect.bottom() - stroke::HAIRLINE / 2.0,
                theme::hairline(p.border),
            );
        }
        let fill = if v.selected {
            p.selection
        } else {
            mix(p.surface, p.surface_hover, hover)
        };
        painter.rect_filled(shape, theme::cr(theme::radius::MD), fill);
        if v.selected {
            let bar = Rect::from_min_size(
                shape.min + theme::v2(0.0, f32::from(theme::radius::MD)),
                theme::v2(
                    stroke::SELECTED_BAR,
                    shape.height() - f32::from(theme::radius::MD) * 2.0,
                ),
            );
            painter.rect_filled(bar, 0, p.accent);
        }
        // Line 1 centred at +20 so the 32 px checkbox hit area starts on the
        // 4-pt grid (+4); line 2 centred at +40 (block 10…48 in 56).
        let l1y = rect.top() + ROW_L1;
        let l2y = rect.top() + ROW_L2;

        // Checkbox (32 hit, centered on line 1).
        if !v.read_only {
            let cb = Rect::from_center_size(
                egui::pos2(rect.left() + space::SM + size::CONTROL / 2.0, l1y),
                theme::v2(size::CONTROL, size::CONTROL),
            );
            let sense = if v.checkable {
                Sense::CLICK
            } else {
                Sense::hover()
            };
            let cresp = ui.interact(cb, v.id.with("check"), sense);
            paint_checkbox(ui, &cresp, cb, v.checked, v.checkable);
            let lab = if v.checked { "Include" } else { "Exclude" };
            let label = format!("{lab} {}", v.old_name);
            let (checked, en) = (v.checked, v.checkable);
            cresp.widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, en, checked, &label));
            if cresp.clicked() {
                out.toggled = true;
            }
        }
        // Status icon.
        let ic = egui::pos2(
            rect.left() + space::SM + size::CONTROL + size::ICON / 2.0,
            l1y,
        );
        match v.status.icon() {
            Some((g, tone)) => {
                let c = if tone == Tone::Neutral {
                    p.text_muted
                } else {
                    p.tone(tone).1
                };
                paint_icon(ui, ic, g, size::ICON, c);
            }
            None if v.status == RowStatus::Analyzing => {
                paint_spinner(ui, ic, size::ICON, p.accent_text);
            }
            None => paint_icon(
                ui,
                ic,
                theme::icon::LOADER_CIRCLE,
                size::ICON,
                p.text_disabled,
            ),
        }

        // Right edge (right − 16): Try again on retryable errors.
        let mut right = rect.right() - space::LG;
        if v.retry {
            let b = Button::ghost("Try again")
                .icon(theme::icon::ROTATE_CCW)
                .size(BSize::Sm);
            let w = b.width(ui);
            let r = Rect::from_min_size(
                egui::pos2(
                    ((right - w) / theme::GRID4).floor() * theme::GRID4,
                    l1y - size::CONTROL_SM / 2.0,
                ),
                theme::v2(w, size::CONTROL_SM),
            );
            let a = format!("Try again {}", v.old_name);
            out.retry = b.a11y(a).show_at(ui, r).clicked();
            right = r.left() - space::MD;
        }

        // Line 1: up to Try again; filenames use a middle ellipsis.
        let x = rect.left() + TEXT_X;
        let mut max_w = (right - x).max(0.0);
        if v.edited {
            max_w -= size::ICON + space::SM;
        }
        let (t1, c1) = match v.status {
            RowStatus::Queued | RowStatus::Analyzing => (theme::typ::BODY, p.text_muted),
            RowStatus::Error => (theme::typ::BODY_STRONG, p.danger),
            _ if !v.checked && !v.read_only => (theme::typ::MONO, p.text_muted),
            _ => (theme::typ::MONO, p.text),
        };
        let is_name = !matches!(
            v.status,
            RowStatus::Queued | RowStatus::Analyzing | RowStatus::Error
        );
        let shown = if is_name {
            ellipsize_middle(ui, v.line1, t1, max_w)
        } else {
            ellipsize_end(ui, v.line1, t1, max_w)
        };
        let r1 = paint_text(ui, x, l1y, &shown, t1, c1);
        if v.edited {
            paint_icon(
                ui,
                egui::pos2(r1.right() + space::SM + size::ICON / 2.0, l1y),
                theme::icon::PENCIL,
                size::ICON,
                p.text_muted,
            );
        }
        // Line 2 has its own budget: x=68 … right − 16 (§4.3).
        let line2_right = rect.right() - space::LG;
        let from = format!("from {}", v.old_name);
        let budget2 = (line2_right - x).max(0.0);
        // With a warning the original name gets at most half of line 2.
        let from_budget = if v.warning.is_some() {
            budget2 / 2.0
        } else {
            budget2
        };
        let from_shown = ellipsize_middle(ui, &from, theme::typ::MONO_SM, from_budget);
        let r2 = paint_text(ui, x, l2y, &from_shown, theme::typ::MONO_SM, p.text_muted);
        let mut warn_cut = false;
        if let Some(w) = v.warning {
            let ix = r2.right() + space::SM;
            paint_icon(
                ui,
                egui::pos2(ix + size::ICON_SM / 2.0, l2y),
                theme::icon::TRIANGLE_ALERT,
                size::ICON_SM,
                p.warning,
            );
            let wx = ix + size::ICON_SM + space::XS;
            let rest = (line2_right - wx).max(0.0);
            let shown_w = ellipsize_end(ui, w, theme::typ::CAPTION, rest);
            warn_cut = shown_w != w;
            paint_text(ui, wx, l2y, &shown_w, theme::typ::CAPTION, p.warning);
        }
        let mut tip = Vec::new();
        if shown != v.line1 {
            tip.push(v.line1.to_string());
        }
        if warn_cut && let Some(w) = v.warning {
            tip.push(w.to_string());
        }
        if !tip.is_empty() {
            out.row = Some(row.clone().on_hover_text(tip.join("\n")));
        }
        focus_ring(ui, &row, shape, theme::radius::MD, true);
        let selected = v.selected;
        let a11y = v.a11y;
        row.widget_info(|| WidgetInfo::selected(egui::Role::ListItem, true, selected, &a11y));
        out.row.get_or_insert(row);
        out
    }
}
