//! eframe app: layout of regions (§3), input/shortcuts (§8), drag & drop,
//! worker messages and actions.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use autorename_core::config;
use autorename_core::journal;
use autorename_core::pipeline::{self, ApplyOptions, Event, PlanOptions};
use egui::{Key, Modifiers, Rect, Sense, Ui};

use super::components::banner::Banner;
use super::components::dialog::{Dialog, DialogResult};
use super::components::drop_zone::DropState;
use super::components::{self as c, Button, galley, paint_galley_v, paint_icon};
use super::model::{Analysis, BannerMsg, Model, Phase, PlanMeta, Row, Tab, Toast, file_name};
use super::screens::{self, Action, detail, empty, header, preview};
use super::theme::{self, Tone, size, space, stroke, typ};
use super::worker::{Msg, NamesDone, NamesJob, Worker, names_mtime};

pub struct App {
    pub m: Model,
    worker: Worker,
    cli_config: Option<PathBuf>,
    offline: bool,
    picker: Option<Receiver<Vec<PathBuf>>>,
    scroll_to_active: bool,
    /// A text field had focus last frame (its Esc only drops focus).
    text_focus_prev: bool,
    /// Frames left to re-request focus on the active row after a jump to
    /// an off-screen row (egui drops focus of widgets clipped out of view).
    refocus_frames: u8,
    stopping: bool,
    /// The open picker chooses an OCR model folder (not PDFs).
    picking_models: bool,
    os_reduce_motion: bool,
    applied_prefs: Option<super::pages::Prefs>,
    /// `gui --rename-now`: rename when the running analysis ends cleanly.
    auto_apply: bool,
    /// Native Wayland file drops (winit 0.30 has none); injected into the
    /// raw input in `raw_input_hook`, so the drop code is shared.
    #[cfg(target_os = "linux")]
    wayland_dnd: Option<super::wayland_dnd::Listener>,
}

const STORE_DETAIL_W: &str = "detail_w";
const STORE_PREFS: &str = "prefs";
/// Core's message for a cancelled model download.
/// Banner action that re-analyzes a plan whose settings changed.
pub const REANALYZE: &str = "Re-analyze";
pub const CANCELLED: &str = "Download cancelled";
/// Offline mode refuses model downloads (CLI parity).
pub const DOWNLOAD_BLOCKED: &str = "Offline mode doesn't download models. Use Import from folder… with files copied from another computer.";
const STORE_DETAIL_VISIBLE: &str = "detail_visible";

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Vec<PathBuf>,
        cli_config: Option<PathBuf>,
        offline: bool,
        launch: super::Launch,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        let demo = std::env::var_os("AUTORENAME_GUI_DEMO").is_some_and(|v| v != "0");
        let m = if demo {
            super::demo::preview_model()
        } else {
            Model::new(super::model::ConfigState::load(
                cli_config.as_deref(),
                offline,
            ))
        };
        let mut app = App::with_model(m, Some(cc.egui_ctx.clone()));
        app.cli_config = cli_config;
        app.offline = offline;
        app.m.force_offline = offline;
        if offline {
            app.m.setup.offline = true;
        }
        if let Some(st) = cc.storage {
            app.m.detail_w = eframe::get_value(st, STORE_DETAIL_W).unwrap_or(None);
            app.m.detail_visible = eframe::get_value(st, STORE_DETAIL_VISIBLE).unwrap_or(true);
            app.m.prefs = eframe::get_value(st, STORE_PREFS).unwrap_or_default();
        }
        app.os_reduce_motion = super::motion::os_reduce_motion();
        if !app.m.demo {
            app.m.settings.packs = autorename_core::ocr::models::default_root()
                .map(|r| autorename_core::ocr::models::list(&r))
                .unwrap_or_default();
        }
        app.apply_prefs(&cc.egui_ctx);
        if !paths.is_empty() {
            app.add_paths(&paths);
        }
        // Only for the files handed over at launch, and only if they are
        // actually being analyzed (no setup needed first).
        app.auto_apply = launch == super::Launch::RenameNow && app.m.analyzing();
        #[cfg(target_os = "linux")]
        {
            app.wayland_dnd = super::wayland_dnd::Listener::start(cc);
        }
        app
    }

    /// Test/demo constructor: no config discovery, no worker repaint hook.
    pub fn with_model(m: Model, ctx: Option<egui::Context>) -> Self {
        App {
            m,
            worker: Worker::new(ctx),
            cli_config: None,
            offline: false,
            picker: None,
            scroll_to_active: false,
            text_focus_prev: false,
            refocus_frames: 0,
            stopping: false,
            picking_models: false,
            os_reduce_motion: false,
            applied_prefs: None,
            auto_apply: false,
            #[cfg(target_os = "linux")]
            wayland_dnd: None,
        }
    }

    /// Theme, zoom and reduce motion from prefs; env var overrides (§2.6).
    pub fn apply_prefs(&mut self, ctx: &egui::Context) {
        let pr = self.m.prefs;
        if self.applied_prefs == Some(pr) {
            return;
        }
        self.applied_prefs = Some(pr);
        ctx.set_theme(match pr.theme {
            1 => egui::ThemePreference::Light,
            2 => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        });
        let env = std::env::var("AUTORENAME_REDUCE_MOTION").ok();
        let reduce = match (env.as_deref(), pr.reduce_motion) {
            (Some(v), _) => v != "0",
            (None, 1) => true,
            (None, 2) => false,
            (None, _) => self.os_reduce_motion,
        };
        c::set_reduce_motion(ctx, reduce);
        let z = super::pages::SCALES[pr.scale.min(super::pages::SCALES.len() - 1)];
        if (ctx.zoom_factor() - z).abs() > f32::EPSILON {
            ctx.set_zoom_factor(z);
        }
    }

    /// Lazy loads when a tab opens (no disk access in demo mode).
    fn load_tab(&mut self) {
        match self.m.tab {
            Tab::Settings if !self.m.settings.loaded => {
                if let Some(s) = &self.m.config.settings {
                    let f = super::pages::SettingsForm::from_settings(s);
                    self.m.settings.form = f.clone();
                    self.m.settings.saved = f;
                    self.m.settings.loaded = true;
                }
            }
            Tab::Companies => self.check_names_changed(),
            Tab::History if !self.m.history.loaded && !self.m.demo => {
                self.m.history = super::pages::HistoryState::load();
            }
            _ => {}
        }
    }

    /// Follow the active profile's name list (a profile switch, a saved
    /// config): a new list is loaded on the worker; a profile without
    /// harmonize_field has no list and no tab. Demo data never touches disk.
    fn sync_name_list(&mut self) {
        if self.m.name_label().is_none() && self.m.tab == Tab::Companies {
            self.m.tab = Tab::Rename;
        }
        if self.m.demo {
            return;
        }
        let wanted = self.m.config.name_list();
        if wanted.as_ref().map(|(p, _)| p) != self.m.companies.path.as_ref() {
            // Keep `seq` growing so results for the old list are dropped.
            let seq = self.m.companies.seq;
            self.m.companies = Default::default();
            self.m.companies.seq = seq;
            if let Some((p, f)) = wanted {
                self.m.companies.path = Some(p);
                self.m.companies.field = f;
                self.reload_names();
            }
        }
    }

    /// Reload the shown name list on the worker (after apply/undo, on tab
    /// entry, after an external edit). Waits for a running edit.
    fn reload_names(&mut self) {
        let c = &mut self.m.companies;
        if self.m.demo {
            return;
        }
        let Some(path) = c.path.clone() else {
            return;
        };
        if c.pending {
            c.reload_after = true;
            return;
        }
        c.seq += 1;
        c.loading = true;
        c.checked = Some(Instant::now());
        self.worker.names(NamesJob {
            seq: c.seq,
            path,
            field: c.field.clone(),
            edit: None,
        });
    }

    /// Companies tab open: reload when the file's mtime changed (another
    /// run, an editor). One `stat` per second at most.
    fn check_names_changed(&mut self) {
        let c = &self.m.companies;
        if self.m.demo
            || c.busy()
            || c.checked
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        let Some(path) = c.path.clone() else {
            return;
        };
        self.m.companies.checked = Some(Instant::now());
        if names_mtime(&path) != self.m.companies.mtime {
            self.reload_names();
        }
    }

    fn on_names(&mut self, ctx: &egui::Context, d: NamesDone) {
        let c = &mut self.m.companies;
        if c.path.as_ref() != Some(&d.path) {
            return; // a list the profile no longer uses
        }
        if d.edit {
            c.pending = false;
        } else if d.seq != c.seq {
            return; // superseded by a newer load
        } else {
            c.loading = false;
        }
        match d.result {
            Ok((db, mtime)) => {
                c.set_db(db);
                c.mtime = mtime;
                c.error = None;
                self.m.config.review_count = c.review_count();
                if let Some(t) = d.done {
                    self.toast(ctx, t);
                }
            }
            Err(e) => {
                if c.db.is_none() {
                    c.db = Some(autorename_core::harmonize::NameDb::empty(&c.field));
                }
                c.error = Some(e);
            }
        }
        if !self.m.companies.pending && std::mem::take(&mut self.m.companies.reload_after) {
            self.reload_names();
        }
    }

    // -----------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------

    pub fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut acts = Vec::new();
        self.apply_prefs(&ctx);
        self.sync_name_list();
        self.load_tab();
        self.poll(&ctx);
        self.sync_stale_banner();
        self.input(&ctx, &mut acts);

        let full = ui.max_rect();
        let p = theme::palette(ui);
        ui.painter().rect_filled(full, 0, p.bg);
        let header_r = Rect::from_min_size(full.min, theme::v2(full.width(), size::HEADER));
        header::header(ui, header_r, &self.m, &mut acts);
        let body = Rect::from_min_max(egui::pos2(full.left(), header_r.bottom()), full.max);
        let hovered = self.hover_state(&ctx);
        let on_empty =
            self.m.tab == Tab::Rename && self.m.rows.is_empty() && self.m.report.is_none();
        match self.m.tab {
            Tab::Rename if on_empty => {
                let st = hovered.unwrap_or(if self.m.busy() {
                    DropState::Disabled
                } else {
                    DropState::Rest
                });
                empty::empty(ui, body, &mut self.m, st, &mut acts);
            }
            Tab::Rename => self.rename_screen(ui, body, &mut acts),
            Tab::Companies => screens::companies::companies(ui, body, &mut self.m, &mut acts),
            Tab::History => screens::history::history(ui, body, &self.m, &mut acts),
            Tab::Settings => screens::settings::settings(ui, body, &mut self.m, &mut acts),
        }
        if let Some(st) = hovered
            && !on_empty
        {
            self.drop_overlay(ui, full, st);
        }
        self.toasts(ui, full);
        if self.m.license_open {
            screens::settings::license_dialog(&ctx, &mut self.m.license_open);
        }
        if self.m.confirm_stop {
            let r = Dialog {
                id: "confirm_stop",
                title: "Stop the analysis?",
                body: "Files analyzed so far stay in the list. The others are removed and can be added again.",
                primary: "Stop analysis",
                cancel: "Keep analyzing",
                destructive: false,
            }
            .show(&ctx);
            match r {
                DialogResult::Primary => {
                    self.m.confirm_stop = false;
                    acts.push(Action::Stop);
                }
                DialogResult::Cancel => self.m.confirm_stop = false,
                DialogResult::Open => {}
            }
        }
        self.scroll_to_active = false;
        self.text_focus_prev = ctx.text_edit_focused();
        for a in acts {
            self.act(&ctx, a);
        }
    }

    fn rename_screen(&mut self, ui: &mut Ui, body: Rect, acts: &mut Vec<Action>) {
        let p = theme::palette(ui);
        let done = self.m.report.is_some();
        let mut top = body.top();
        if !done {
            let tr = Rect::from_min_size(body.min, theme::v2(body.width(), size::TOOLBAR));
            let list_w = body.width() - self.detail_w(ui, body.width());
            preview::toolbar(ui, tr, list_w, &mut self.m, acts);
            top = tr.bottom();
        }
        // Banners (max 2, newest first) under the toolbar, inset 16.
        let mut dismissed = None;
        let mut action = None;
        for (k, b) in self.m.banners.iter().rev().take(2).enumerate() {
            let idx = self.m.banners.len() - 1 - k;
            let r = Rect::from_min_size(
                egui::pos2(body.left() + space::LG, top + space::LG),
                theme::v2(body.width() - space::LG * 2.0, Banner::HEIGHT),
            );
            let out = Banner {
                tone: b.tone,
                title: &b.title,
                body: &b.body,
                action: b.action.as_deref(),
                dismissible: true,
            }
            .show_at(ui, r);
            if out.dismissed {
                dismissed = Some(idx);
            }
            if out.action {
                action = Some(idx);
            }
            top = r.bottom();
        }
        if !self.m.banners.is_empty() {
            top += space::LG;
            ui.painter().hline(
                body.x_range(),
                top - stroke::HAIRLINE / 2.0,
                theme::hairline(p.border),
            );
        }
        if let Some(i) = dismissed {
            acts.push(Action::DismissBanner(i));
        }
        if let Some(i) = action {
            acts.push(Action::BannerPrimary(i));
        }
        let ar = Rect::from_min_max(
            egui::pos2(body.left(), body.bottom() - size::ACTION_BAR),
            body.max,
        );
        let content = Rect::from_min_max(
            egui::pos2(body.left(), top),
            egui::pos2(body.right(), ar.top()),
        );
        let (list_r, detail_r) = self.split(ui, content);
        preview::list(ui, list_r, &mut self.m, self.scroll_to_active, acts);
        if let Some(dr) = detail_r {
            detail::detail(ui, dr, &self.m, acts);
        }
        preview::action_bar(ui, ar, &self.m, acts);
    }

    /// φ split with a resizable divider (8 px hit, 1 px visual, double-click resets).
    /// Detail width + divider (0 when hidden), φ by default (§3).
    fn detail_w(&self, ui: &Ui, content_w: f32) -> f32 {
        if !self.m.detail_visible {
            return 0.0;
        }
        let window_w = ui.ctx().content_rect().width();
        let max = (content_w - size::DETAIL_MIN - stroke::HAIRLINE).min(size::DETAIL_MAX);
        self.m
            .detail_w
            .unwrap_or_else(|| theme::detail_width(window_w))
            .clamp(size::DETAIL_MIN.min(max), max.max(size::DETAIL_MIN))
            + stroke::HAIRLINE
    }

    fn split(&mut self, ui: &mut Ui, content: Rect) -> (Rect, Option<Rect>) {
        if !self.m.detail_visible {
            return (content, None);
        }
        let p = theme::palette(ui);
        let max = (content.width() - size::DETAIL_MIN - stroke::HAIRLINE).min(size::DETAIL_MAX);
        let dw = self.detail_w(ui, content.width()) - stroke::HAIRLINE;
        let list_w = content.width() - dw - stroke::HAIRLINE;
        let list_r = Rect::from_min_size(content.min, theme::v2(list_w, content.height()));
        let div_x = list_r.right();
        let div = Rect::from_min_max(
            egui::pos2(div_x, content.top()),
            egui::pos2(div_x + stroke::HAIRLINE, content.bottom()),
        );
        let hit = div.expand2(theme::v2((size::DIVIDER_HIT - stroke::HAIRLINE) / 2.0, 0.0));
        let resp = ui.interact(hit, ui.scope_id().with("divider"), Sense::click_and_drag());
        let active = resp.hovered() || resp.dragged();
        ui.painter()
            .rect_filled(div, 0, if active { p.focus } else { p.border });
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resp.dragged() {
            let nw = (dw - resp.drag_delta().x).clamp(size::DETAIL_MIN, max.max(size::DETAIL_MIN));
            self.m.detail_w = Some(nw);
        }
        if resp.double_clicked() {
            self.m.detail_w = None;
        }
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Splitter, true, "Resize detail pane")
        });
        let detail_r = Rect::from_min_max(egui::pos2(div.right(), content.top()), content.max);
        (list_r, Some(detail_r))
    }

    fn hover_state(&self, ctx: &egui::Context) -> Option<DropState> {
        let files = ctx.input(|i| i.raw.hovered_files.clone());
        if files.is_empty() {
            return None;
        }
        if self.m.busy() {
            return Some(DropState::Disabled);
        }
        let ok = files
            .iter()
            .filter(|f| match &f.path {
                Some(p) => p.is_dir() || is_pdf(p),
                None => true,
            })
            .count();
        Some(if ok == 0 {
            DropState::Invalid
        } else {
            DropState::Valid(ok)
        })
    }

    fn drop_overlay(&self, ui: &mut Ui, full: Rect, st: DropState) {
        let p = theme::palette(ui);
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop_overlay"));
        let painter = ui.ctx().layer_painter(layer);
        painter.rect_filled(full, 0, p.drop_scrim);
        let text = match st {
            DropState::Valid(n) => format!("Drop to add {}", c::plural(n, "item", "items")),
            DropState::Invalid => "Only PDFs and folders".into(),
            DropState::Disabled => "Wait until the current run finishes".into(),
            DropState::Rest => String::new(),
        };
        let g = painter.layout_no_wrap(text, typ::TITLE_SM.font(), p.text);
        let w = theme::snap4(g.size().x + space::XL * 2.0);
        let r = Rect::from_center_size(full.center(), theme::v2(w, size::CONTROL_LG + space::XL));
        let border = if st == DropState::Invalid {
            p.danger
        } else {
            p.accent
        };
        painter.add(p.shadow(2).as_shape(r, theme::cr(theme::radius::XL)));
        painter.rect(
            r,
            theme::cr(theme::radius::XL),
            p.elevated(2),
            egui::Stroke::new(stroke::FOCUS, border),
            egui::StrokeKind::Inside,
        );
        let pos = r.center() - g.size() / 2.0;
        painter.galley(pos.round(), g, p.text);
    }

    fn toasts(&mut self, ui: &mut Ui, full: Rect) {
        let now = ui.input(|i| i.time);
        for t in &mut self.m.toasts {
            if t.until < 0.0 {
                t.until = now + f64::from(theme::motion::TOAST_SECS);
            }
        }
        self.m.toasts.retain(|t| t.until > now);
        if self.m.toasts.is_empty() {
            return;
        }
        let p = theme::palette(ui);
        let mut bottom = full.bottom() - size::ACTION_BAR - space::LG;
        let mut close = None;
        let mut hold = Vec::new();
        let shown: Vec<String> = self
            .m
            .toasts
            .iter()
            .rev()
            .take(3)
            .map(|t| t.text.clone())
            .collect();
        for (k, text) in shown.iter().enumerate() {
            let t = Toast {
                text: text.clone(),
                until: 0.0,
            };
            let g = galley(ui, &t.text, typ::BODY);
            let w =
                (g.size().x + size::ICON + size::CONTROL_SM + space::LG * 2.0 + space::SM * 2.0)
                    .clamp(size::TOAST_MIN, size::TOAST_MAX);
            let h = size::CONTROL + space::MD * 2.0 - space::SM;
            let r = Rect::from_min_size(
                egui::pos2(theme::snap4(full.center().x - w / 2.0), bottom - h),
                theme::v2(theme::snap4(w), h),
            );
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(("toast", k)));
            let painter = ui.ctx().layer_painter(layer);
            painter.add(p.shadow(2).as_shape(r, theme::cr(theme::radius::XL)));
            let st = if p.dark {
                p.border
            } else {
                egui::Color32::TRANSPARENT
            };
            painter.rect(
                r,
                theme::cr(theme::radius::XL),
                p.elevated(2),
                theme::hairline(st),
                egui::StrokeKind::Inside,
            );
            let mut child = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(r));
            let cy = r.center().y;
            paint_icon(
                &child,
                egui::pos2(r.left() + space::LG + size::ICON / 2.0, cy),
                theme::icon::CIRCLE_CHECK,
                size::ICON,
                p.success,
            );
            paint_galley_v(
                &child,
                r.left() + space::LG + size::ICON + space::SM,
                cy,
                g,
                p.text,
            );
            let xr = Rect::from_center_size(
                egui::pos2(r.right() - space::SM - size::CONTROL_SM / 2.0, cy),
                theme::v2(size::CONTROL_SM, size::CONTROL_SM),
            );
            if Button::icon_only(theme::icon::X, "Dismiss notification")
                .size(c::BSize::Sm)
                .show_at(&mut child, xr)
                .clicked()
            {
                close = Some(self.m.toasts.len() - 1 - k);
            }
            if child.rect_contains_pointer(r) {
                // Paused on hover (§5.8).
                hold.push(self.m.toasts.len() - 1 - k);
            }
            bottom = r.top() - space::SM;
        }
        let until = now + f64::from(theme::motion::TOAST_SECS);
        for i in hold {
            self.m.toasts[i].until = self.m.toasts[i].until.max(until);
        }
        if let Some(i) = close {
            self.m.toasts.remove(i);
        }
        if let Some(next) = self.m.toasts.iter().map(|t| t.until).reduce(f64::min) {
            ui.ctx()
                .request_repaint_after_secs((next - now).max(0.0) as f32);
        }
    }

    fn toast(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        let now = ctx.input(|i| i.time);
        self.m.toasts.push(Toast {
            text: text.into(),
            until: now + f64::from(theme::motion::TOAST_SECS),
        });
    }

    // -----------------------------------------------------------------------
    // Input
    // -----------------------------------------------------------------------

    fn input(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        if self.refocus_frames > 0 {
            self.refocus_frames -= 1;
            if let Some(r) = self.m.active.and_then(|a| self.m.rows.get(a)) {
                let id = egui::Id::unique(("row", r.source.as_path()));
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
        let (key_event, pointer) = ctx.input(|i| {
            let k = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
            (k, i.pointer.any_pressed())
        });
        if key_event {
            c::set_keyboard_mode(ctx, true);
        } else if pointer {
            c::set_keyboard_mode(ctx, false);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            acts.push(Action::AddPaths(dropped));
        }
        if let Some(rx) = &self.picker
            && let Ok(paths) = rx.try_recv()
        {
            self.picker = None;
            if std::mem::take(&mut self.picking_models) {
                if let Some(dir) = paths.into_iter().next() {
                    let id = self.ocr_pack_id();
                    self.model_op(&id, Some(dir), false);
                }
            } else if !paths.is_empty() {
                acts.push(Action::AddPaths(paths));
            }
        }
        let cmd = Modifiers::COMMAND;
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        let take = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if take(shift_cmd, Key::O) {
            acts.push(Action::ChooseFolder);
        }
        if take(cmd, Key::O) {
            acts.push(Action::ChooseFiles);
        }
        for (k, t) in [
            (Key::Num1, Tab::Rename),
            (Key::Num2, Tab::Companies),
            (Key::Num3, Tab::History),
        ] {
            // Ctrl+2 does nothing while the profile matches no names.
            if take(cmd, k) && (t != Tab::Companies || self.m.name_label().is_some()) {
                acts.push(Action::SwitchTab(t));
            }
        }
        if take(cmd, Key::Comma) {
            acts.push(Action::SwitchTab(Tab::Settings));
        }
        if take(cmd, Key::Backslash) {
            self.m.detail_visible = !self.m.detail_visible;
        }
        if take(cmd, Key::F) {
            self.m.tab = Tab::Rename;
            ctx.memory_mut(|m| m.request_focus(egui::Id::unique("search")));
        }
        if take(cmd, Key::Enter) {
            // Ctrl+Enter = the visible primary of the current tab, honoring
            // its enabled state (never a hidden Rename behind Settings).
            match self.m.tab {
                Tab::Settings
                    if self.m.settings.dirty()
                        && !self.m.busy()
                        && self.m.settings.blocker(&self.m.config).is_none() =>
                {
                    acts.push(Action::SaveSettings);
                }
                Tab::Rename if self.m.report.is_some() && !self.m.busy() => {
                    acts.push(Action::RenameMore);
                }
                Tab::Rename if self.m.can_rename() => acts.push(Action::Rename),
                _ => {}
            }
        }
        if take(cmd, Key::Z) && self.m.report.is_some() {
            acts.push(Action::Undo);
        }
        if take(cmd, Key::R) {
            acts.push(Action::RetryFailed);
        }
        // Esc leaves Settings for the previous tab — unless it is closing a
        // popup or leaving a text field (that Esc only drops focus).
        if self.m.tab == Tab::Settings
            && !self.text_focus_prev
            && !egui::Popup::is_any_open(ctx)
            && !self.m.confirm_stop
            && !self.m.license_open
            && self.m.companies.merge.is_none()
            && take(Modifiers::NONE, Key::Escape)
        {
            acts.push(Action::CloseSettings);
            return;
        }
        if self.m.tab != Tab::Rename || self.m.rows.is_empty() || self.m.editing.is_some() {
            if self.m.editing.is_none() && self.m.analyzing() && take(Modifiers::NONE, Key::Escape)
            {
                self.m.confirm_stop = true;
            }
            return;
        }
        if take(cmd, Key::A) && !ctx.text_edit_focused() {
            for r in &mut self.m.rows {
                if r.checkable() {
                    r.checked = true;
                }
            }
        }
        if ctx.text_edit_focused() || self.m.confirm_stop {
            return;
        }
        // Row navigation applies when a row (or nothing) has focus.
        let row_ids: Vec<egui::Id> = self
            .m
            .rows
            .iter()
            .map(|r| egui::Id::unique(("row", r.source.as_path())))
            .collect();
        let focused = ctx.memory(|m| m.focused());
        let row_focus = focused.is_none_or(|f| row_ids.contains(&f));
        if !row_focus {
            return;
        }
        let visible = self.m.visible();
        if visible.is_empty() {
            return;
        }
        let pos = self
            .m
            .active
            .and_then(|a| visible.iter().position(|v| *v == a));
        let page = 8;
        let mut target = None;
        let last = visible.len() - 1;
        if take(Modifiers::NONE, Key::ArrowDown) {
            target = Some(pos.map_or(0, |p| (p + 1).min(last)));
        }
        if take(Modifiers::NONE, Key::ArrowUp) {
            target = Some(pos.map_or(0, |p| p.saturating_sub(1)));
        }
        if take(Modifiers::NONE, Key::Home) {
            target = Some(0);
        }
        if take(Modifiers::NONE, Key::End) {
            target = Some(last);
        }
        if take(Modifiers::NONE, Key::PageDown) {
            target = Some(pos.map_or(0, |p| (p + page).min(last)));
        }
        if take(Modifiers::NONE, Key::PageUp) {
            target = Some(pos.map_or(0, |p| p.saturating_sub(page)));
        }
        if let Some(t) = target {
            let idx = visible[t];
            self.m.active = Some(idx);
            self.scroll_to_active = true;
            if focused.is_some() {
                ctx.memory_mut(|m| m.request_focus(row_ids[idx]));
                self.refocus_frames = 2;
            }
        }
        if let Some(a) = self.m.active {
            if take(Modifiers::NONE, Key::Space) {
                acts.push(Action::Toggle(a));
            }
            if take(Modifiers::NONE, Key::F2)
                || (focused == Some(row_ids[a]) && take(Modifiers::NONE, Key::Enter))
            {
                acts.push(Action::StartEdit(a));
            }
            if take(Modifiers::NONE, Key::Delete) && !self.m.busy() && self.m.report.is_none() {
                self.m.rows.remove(a);
                self.m.active = if self.m.rows.is_empty() {
                    None
                } else {
                    Some(a.min(self.m.rows.len() - 1))
                };
            }
        }
        if self.m.analyzing() && take(Modifiers::NONE, Key::Escape) {
            self.m.confirm_stop = true;
        }
    }

    // -----------------------------------------------------------------------
    // Worker messages
    // -----------------------------------------------------------------------

    #[cfg(test)]
    pub(crate) fn worker_send_done(&self, generation: u64) {
        let _ = self.worker.tx.send(Msg::AnalysisDone(generation));
    }

    #[cfg(test)]
    pub(crate) fn worker_send(&self, msg: Msg) {
        let _ = self.worker.tx.send(msg);
    }

    /// Poll worker messages once without cancelling (tests drive real runs).
    #[cfg(test)]
    pub(crate) fn poll_once(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
        self.sync_stale_banner();
    }

    #[cfg(test)]
    pub(crate) fn frame_poll(&mut self, ctx: &egui::Context) {
        self.worker.cancel();
        self.poll(ctx);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.worker.rx.try_recv() {
            match msg {
                Msg::Event(g, e) => self.on_event(g, e),
                Msg::Items(g, items, warnings) if g == self.m.generation => {
                    self.m.merge_items(items);
                    for w in warnings {
                        self.toast(ctx, w);
                    }
                }
                Msg::AnalysisFailed(g, e) if g == self.m.generation => {
                    self.auto_apply = false;
                    self.banner(
                        Tone::Danger,
                        &super::model::error_copy(&e),
                        e.hint.as_deref().unwrap_or(""),
                    );
                }
                Msg::AnalysisDone(g) if g == self.m.generation => {
                    let stopped = self.stopping;
                    // Rows without a result stay visible ("not analyzed",
                    // with Retry); nothing is dropped silently.
                    let mut removed = 0;
                    for r in &mut self.m.rows {
                        if r.item.is_none() && r.report.is_none() {
                            r.analyzing = false;
                            r.not_analyzed = true;
                            removed += 1;
                        }
                    }
                    if self.stopping && removed > 0 {
                        self.toast(
                            ctx,
                            format!(
                                "Stopped · {} not analyzed",
                                c::plural(removed, "file", "files")
                            ),
                        );
                    }
                    self.stopping = false;
                    if self.m.active.is_some_and(|a| a >= self.m.rows.len()) {
                        self.m.active = (!self.m.rows.is_empty()).then_some(0);
                    }
                    self.m.phase = Phase::Idle;
                    if std::mem::take(&mut self.auto_apply) && !stopped {
                        self.rename_now(ctx);
                    }
                }
                Msg::Applied(Ok(rep)) => {
                    self.m.apply_report(rep);
                    self.m.history.loaded = false;
                    // Learned names and review entries show up at once.
                    self.reload_names();
                }
                Msg::Applied(Err(e)) => {
                    self.m.phase = Phase::Idle;
                    self.banner(Tone::Danger, "Renaming failed.", &e.message);
                }
                // Undo results are routed by batch id: only the batch that
                // was undone changes state, never a newer result.
                Msg::Undone(batch, res) => {
                    if self.m.undoing.as_deref() == Some(batch.as_str()) {
                        self.m.undoing = None;
                    }
                    let is_current = self.m.report.as_ref().and_then(|r| r.batch_id.as_deref())
                        == Some(batch.as_str());
                    match res {
                        Ok(rep) => {
                            let restored = rep
                                .files
                                .iter()
                                .filter(|f| f.status == journal::UndoStatus::Restored)
                                .count();
                            let failed = rep.files.len() - restored;
                            self.toast(
                                ctx,
                                format!("Undid {}", c::plural(restored, "rename", "renames")),
                            );
                            if failed > 0 {
                                let t = format!(
                                    "{} changed after renaming and were not restored.",
                                    c::plural(failed, "file", "files")
                                );
                                self.banner(Tone::Warning, &t, "");
                            }
                            if is_current {
                                self.m.undone = true;
                                self.m.undo = Some(rep);
                            }
                        }
                        Err(e) => self.banner(Tone::Danger, "Undo failed.", &e.message),
                    }
                    self.m.history.loaded = false;
                    self.reload_names();
                }
                Msg::Tested(r) => {
                    if self.m.setup_testing {
                        self.m.setup_testing = false;
                        self.m.setup_test = Some(r);
                    } else {
                        self.m.settings.testing = false;
                        self.m.settings.test = Some(r);
                    }
                }
                Msg::ModelProgress { downloaded, total } => {
                    self.m.settings.model_progress = (downloaded, total);
                }
                Msg::Names(d) => self.on_names(ctx, d),
                Msg::ModelDone(r) => {
                    self.m.settings.model_busy = None;
                    self.m.settings.model_msg = Some(r);
                    self.m.settings.packs = autorename_core::ocr::models::default_root()
                        .map(|r| autorename_core::ocr::models::list(&r))
                        .unwrap_or_default();
                }
                _ => {}
            }
        }
    }

    fn on_event(&mut self, g: u64, e: Event) {
        match e {
            Event::FileStarted { source, .. } if g == self.m.generation => {
                if let Some(r) = self.m.rows.iter_mut().find(|r| r.source == source) {
                    r.analyzing = true;
                }
            }
            Event::FileAnalyzed {
                source,
                proposed_name,
                ..
            } if g == self.m.generation => {
                if let Some(r) = self.m.rows.iter_mut().find(|r| r.source == source) {
                    r.base_name = proposed_name;
                }
                if let Phase::Analyzing(a) = &mut self.m.phase {
                    a.done += 1;
                    a.done_at.push(a.started.elapsed().as_secs_f32());
                }
            }
            Event::FileApplied { .. } => {
                if let Phase::Renaming { done, .. } = &mut self.m.phase {
                    *done += 1;
                }
            }
            _ => {}
        }
    }

    fn banner(&mut self, tone: Tone, title: &str, body: &str) {
        self.m.banners.push(BannerMsg {
            tone,
            title: title.into(),
            body: body.into(),
            action: None,
        });
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Profile change: the shown plan's results become stale (fingerprint)
    /// and are re-analyzed with the new profile.
    fn select_profile(&mut self, ctx: &egui::Context, id: &str) {
        if self.m.busy() {
            return;
        }
        self.m.config.select_profile(id);
        // The name list (and the tab label/count) follow the profile at
        // once, whichever tab is open.
        self.sync_name_list();
        if self.m.rows.is_empty() || self.m.report.is_some() {
            return;
        }
        let n = self.schedule(&[], &[]);
        if n > 0 {
            let label = super::screens::header::profile_label(id);
            self.toast(
                ctx,
                format!(
                    "Analyzing {} again with the {label} profile",
                    c::plural(n, "file", "files")
                ),
            );
        }
    }

    pub(crate) fn act(&mut self, ctx: &egui::Context, a: Action) {
        self.act_inner(ctx, a);
        self.sync_stale_banner();
    }

    fn act_inner(&mut self, ctx: &egui::Context, a: Action) {
        match a {
            Action::ChooseFiles | Action::ChooseFolder => {
                if self.m.busy() || self.picker.is_some() {
                    return;
                }
                if self.m.demo {
                    // Demo: the "picked" files are the canned plan.
                    let mut m = super::demo::preview_model();
                    m.prefs = self.m.prefs;
                    m.tab = Tab::Rename;
                    self.m = m;
                    self.m.editing = None;
                    self.m.rows[8].edited = None;
                    self.m.rows[9].checked = true;
                    self.m.active = Some(0);
                    return;
                }
                self.picker = Some(pick(a == Action::ChooseFolder, ctx.clone()));
            }
            Action::AddPaths(p) => {
                self.m.tab = Tab::Rename;
                self.add_paths(&p);
            }
            Action::Rename => self.rename(ctx),
            Action::Stop => {
                self.worker.cancel();
                self.stopping = true;
            }
            Action::ClearList => self.m.clear(),
            Action::RenameMore => self.m.clear(),
            Action::Undo => self.undo(ctx),
            Action::Retry(i) => self.retry(&[i]),
            Action::RetryFailed => {
                let idx = self.m.retry_targets();
                self.retry(&idx);
            }
            Action::OpenFolder => {
                if let Some(r) = self.m.rows.first() {
                    let dir = r.source.parent().map(Path::to_path_buf).unwrap_or_default();
                    open_path(&dir);
                }
            }
            Action::OpenPath(p) => open_path(&p),
            Action::ShowLicense => self.m.license_open = true,
            Action::OpenUrl(url) => open_path(Path::new(&url)),
            Action::ToggleSettings => {
                if self.m.tab == Tab::Settings {
                    self.act_inner(ctx, Action::CloseSettings);
                } else {
                    self.act_inner(ctx, Action::SwitchTab(Tab::Settings));
                }
            }
            Action::CloseSettings => {
                if self.m.tab != Tab::Settings {
                    return;
                }
                self.m.tab = if self.m.prev_tab == Tab::Settings {
                    Tab::Rename
                } else {
                    self.m.prev_tab
                };
                if self.m.tab == Tab::History && !self.m.demo {
                    self.m.history.loaded = false;
                }
                if self.m.settings.dirty() {
                    // Edits stay in the form (footer returns with Settings).
                    self.toast(
                        ctx,
                        "Unsaved settings kept · open Settings to save or discard",
                    );
                }
            }
            Action::OpenSettings(page) => {
                self.act_inner(ctx, Action::SwitchTab(Tab::Settings));
                self.m.settings.section = page.min(super::pages::page::ABOUT);
                self.m.settings.scroll_top = true;
            }
            Action::SwitchTab(Tab::Companies) if self.m.name_label().is_none() => {}
            Action::SwitchTab(t) => {
                if t == Tab::Settings && self.m.tab != Tab::Settings {
                    self.m.prev_tab = self.m.tab;
                }
                if t == Tab::History && !self.m.demo {
                    // Always show the journal as it is now.
                    self.m.history.loaded = false;
                }
                self.m.tab = t;
                if t == Tab::Companies {
                    // Always show the list as it is on disk now.
                    self.reload_names();
                }
            }
            Action::SaveSetup => self.save_setup(),
            Action::SelectProfile(id) => self.select_profile(ctx, &id),
            Action::SetFilter(f) => self.m.filter = f,
            Action::Activate(i) => self.m.active = Some(i),
            Action::Toggle(i) => self.m.toggle(i),
            Action::StartEdit(i) => {
                if self.m.busy() || self.m.report.is_some() {
                    return;
                }
                if let Some(r) = self.m.rows.get(i)
                    && r.checkable()
                {
                    let name = r.final_name().unwrap_or_default();
                    let base = name.strip_suffix(".pdf").unwrap_or(&name).to_string();
                    self.m.active = Some(i);
                    self.m.editing = Some((i, base));
                }
            }
            Action::CommitEdit => {
                if let Some((i, t)) = self.m.editing.clone()
                    && self.m.commit_edit(i, &t)
                {
                    self.m.editing = None;
                    ctx.memory_mut(|m| {
                        m.request_focus(egui::Id::unique(("row", self.m.rows[i].source.as_path())))
                    });
                }
            }
            Action::CancelEdit => {
                if let Some((i, _)) = self.m.editing.take() {
                    ctx.memory_mut(|m| {
                        m.request_focus(egui::Id::unique(("row", self.m.rows[i].source.as_path())))
                    });
                }
            }
            Action::ResetName(i) => {
                if let Some(r) = self.m.rows.get_mut(i) {
                    r.edited = None;
                }
            }
            Action::DontLearn(i) => {
                if let Some(r) = self.m.rows.get_mut(i) {
                    r.dont_learn = !r.dont_learn;
                }
            }
            Action::CopyText(t) => {
                ctx.copy_text(t);
                self.toast(ctx, "Copied the original path");
            }
            Action::TestSetup => match self.setup_settings() {
                Ok(st) if !self.m.demo => {
                    self.m.setup_testing = true;
                    self.m.setup_test = None;
                    self.worker.test_connection(st);
                }
                Ok(_) => self.m.setup_test = Some(Ok(412)),
                Err(e) => self.m.setup_test = Some(Err(e)),
            },
            Action::TestSettings => match self.form_settings() {
                Ok(st) if !self.m.demo => {
                    self.m.settings.testing = true;
                    self.m.settings.test = None;
                    self.worker.test_connection(st);
                }
                Ok(_) => self.m.settings.test = Some(Ok(412)),
                Err(e) => self.m.settings.test = Some(Err(e)),
            },
            Action::SaveSettings => {
                if self.m.busy() {
                    // Privacy/provider must not change under a running job.
                    self.m.settings.error =
                        Some("Wait for the current run to finish or stop it, then save.".into());
                } else {
                    self.save_settings(ctx);
                }
            }
            Action::DiscardSettings => {
                self.m.settings.form = self.m.settings.saved.clone();
                self.m.settings.error = None;
                self.m.settings.date_custom = false;
                self.m.settings.language_other = false;
            }
            Action::ModelFetch(id) => self.model_op(&id, None, false),
            Action::ModelRemove(id) => self.model_op(&id, None, true),
            Action::ModelCancel => {
                if self.m.demo {
                    self.m.settings.model_busy = None;
                    self.m.settings.model_msg = Some(Err(CANCELLED.into()));
                } else {
                    self.worker.cancel_models();
                }
            }
            Action::ModelImport => {
                if self.picker.is_none() && !self.m.demo {
                    self.picking_models = true;
                    self.picker = Some(pick(true, ctx.clone()));
                }
            }
            Action::OpenConfigFile => {
                if let Some(l) = &self.m.config.location {
                    open_path(&l.path);
                }
            }
            Action::OpenConfigFolder => {
                if let Some(d) = self
                    .m
                    .config
                    .location
                    .as_ref()
                    .and_then(|l| l.path.parent())
                {
                    open_path(d);
                }
            }
            Action::OpenCompaniesFile => {
                if let Some(p) = self
                    .m
                    .companies
                    .path
                    .clone()
                    .or_else(|| self.m.config.name_list().map(|(p, _)| p))
                {
                    open_path(&p);
                }
            }
            Action::KeepCompany(name) => {
                let msg = format!("Kept as a new {}", screens::companies::nouns(&self.m).0);
                self.company_op(ctx, &msg, move |db| {
                    autorename_core::harmonize::set_review(db, &name, false)
                });
            }
            Action::MergeCompany(from, into) => {
                let msg = format!("Merged into {into}");
                self.company_op(ctx, &msg, move |db| {
                    autorename_core::harmonize::merge(db, &from, &into)
                });
            }
            Action::RenameCompany(old, new) => {
                let msg = format!("Renamed to {new}");
                self.company_op(ctx, &msg, move |db| rename_entry(db, &old, &new));
            }
            Action::RemoveAlias(canonical, alias) => {
                self.company_op(ctx, "Alias removed", move |db| {
                    if let Some(i) = db.find(&canonical) {
                        db.entries[i].learned.retain(|a| *a != alias);
                    }
                    Ok(())
                });
            }
            Action::UndoBatch(id) => {
                if self.m.busy() {
                    return;
                }
                if self.m.demo {
                    if let Some(b) = self.m.history.batches.iter_mut().find(|b| b.id == id) {
                        b.undone = true;
                        let n = b.files;
                        self.toast(ctx, format!("Undid {}", c::plural(n, "rename", "renames")));
                    }
                    return;
                }
                let Some(jp) = journal::default_journal_path() else {
                    return;
                };
                self.m.undoing = Some(id.clone());
                self.worker.undo(jp, id);
            }
            Action::DismissBanner(i) | Action::BannerPrimary(i) => {
                if i < self.m.banners.len() {
                    let b = self.m.banners.remove(i);
                    if matches!(a, Action::BannerPrimary(_))
                        && b.action.as_deref() == Some(REANALYZE)
                    {
                        self.schedule(&[], &[]);
                    }
                }
            }
        }
    }

    fn ocr_pack_id(&self) -> String {
        let lang = self
            .m
            .config
            .settings
            .as_ref()
            .map_or(autorename_core::ocr::AUTO_LANGUAGE.to_string(), |s| {
                s.ocr.language.clone()
            });
        autorename_core::ocr::pack_for_language(&lang)
            .map_or("latin".to_string(), |p| p.id.to_string())
    }

    /// Download / import (`dir`) / remove an OCR pack on the worker.
    fn model_op(&mut self, id: &str, dir: Option<PathBuf>, remove: bool) {
        if self.m.settings.model_busy.is_some() {
            return;
        }
        let Some(pack) = autorename_core::ocr::pack(id) else {
            return;
        };
        self.m.settings.model_msg = None;
        if dir.is_none() && !remove && self.m.downloads_blocked() {
            // Same policy as `models fetch` under privacy: offline.
            self.m.settings.model_msg = Some(Err(DOWNLOAD_BLOCKED.into()));
            return;
        }
        if self.m.demo {
            for st in &mut self.m.settings.packs {
                if st.pack == id {
                    st.installed = !remove;
                }
            }
            return;
        }
        self.m.settings.model_busy = Some(id.to_string());
        self.m.settings.model_progress = (0, pack.total_size());
        self.worker.models(match (dir, remove) {
            (_, true) => super::worker::ModelOp::Remove(pack),
            (Some(d), _) => super::worker::ModelOp::Import(pack, d),
            (None, _) => super::worker::ModelOp::Fetch(pack),
        });
    }

    /// Name list edit: in memory (demo) or `update_db` on the worker (it
    /// waits for the list's lock; the UI never does). One edit at a time.
    fn company_op(
        &mut self,
        ctx: &egui::Context,
        done: &str,
        f: impl FnOnce(&mut autorename_core::harmonize::NameDb) -> autorename_core::Result<()>
        + Send
        + 'static,
    ) {
        if self.m.demo {
            let mut db = self.m.companies.db.clone().unwrap_or_else(|| {
                autorename_core::harmonize::NameDb::empty(autorename_core::harmonize::COMPANY_FIELD)
            });
            match f(&mut db) {
                Ok(()) => {
                    self.m.companies.set_db(db);
                    self.m.companies.error = None;
                    self.toast(ctx, done.to_string());
                }
                Err(e) => self.m.companies.error = Some(e.to_string()),
            }
            return;
        }
        let c = &mut self.m.companies;
        if c.busy() {
            return;
        }
        let Some(path) = c.path.clone() else {
            c.error = Some("No name list".into());
            return;
        };
        c.pending = true;
        c.error = None;
        self.worker.names(NamesJob {
            seq: c.seq,
            path,
            field: c.field.clone(),
            edit: Some((Box::new(f), done.to_string())),
        });
    }

    fn env_lookup(&self) -> impl Fn(&str) -> Option<String> + '_ {
        |name: &str| {
            std::env::var(name)
                .ok()
                .or_else(|| self.m.config.env.get(name).cloned())
        }
    }

    /// A connection test must not reach the cloud when `--offline` is set,
    /// or (Settings form) while the saved config is offline: save the
    /// privacy change first.
    fn guard_offline_test(&self, st: &config::Settings, form: bool) -> Result<(), String> {
        let saved_offline = form
            && self
                .m
                .config
                .settings
                .as_ref()
                .is_some_and(|s| s.privacy == config::Privacy::Offline);
        let cloud = st.privacy != config::Privacy::Offline
            || st.ai.provider.id() != "ollama"
            || (!st.ai.base_url.trim().is_empty() && !config::is_local_url(&st.ai.base_url));
        if self.m.force_offline && cloud {
            return Err("Offline mode (--offline) blocks cloud providers. Choose Ollama on this computer or your network.".into());
        }
        if saved_offline && cloud {
            return Err("Privacy is set to Offline. Save the switch to Cloud before testing a cloud provider.".into());
        }
        Ok(())
    }

    /// Settings for "Test connection" in the setup card (nothing saved).
    fn setup_settings(&self) -> Result<config::Settings, String> {
        let s = &self.m.setup;
        let (id, _, var) = if s.offline {
            super::model::PROVIDERS[4]
        } else {
            super::model::PROVIDERS[s.provider.min(3)]
        };
        let key = if !s.api_key.trim().is_empty() {
            s.api_key.trim().to_string()
        } else if var.is_empty() {
            String::new()
        } else {
            format!("${{{var}}}")
        };
        let raw = serde_json::json!({
            "config_version": 4,
            "privacy": if s.offline { "offline" } else { "cloud" },
            "ai": {
                "provider": id,
                "model": if s.offline || s.model.trim().is_empty() { serde_json::Value::Null } else { s.model.trim().into() },
                "api_key": key,
                "base_url": if s.offline { s.ollama_url.trim() } else { "" },
            }
        });
        let env = self.env_lookup();
        let (tree, _) = config::from_raw(raw, &env).map_err(|e| e.to_string())?;
        let st = config::settings(&tree, None).map_err(|e| e.to_string())?;
        self.guard_offline_test(&st, false)?;
        Ok(st)
    }

    /// Settings from the (unsaved) Settings form, for "Test connection".
    pub(crate) fn form_settings(&self) -> Result<config::Settings, String> {
        let f = &self.m.settings.form;
        let mut tree = self.m.config.tree.clone().unwrap_or_else(config::defaults);
        let set = |tree: &mut serde_json::Value, sec: &str, k: &str, v: serde_json::Value| {
            if let Some(o) = tree.get_mut(sec).and_then(|x| x.as_object_mut()) {
                o.insert(k.into(), v);
            }
        };
        set(&mut tree, "ai", "provider", f.provider_id().into());
        let model = match f.model.trim() {
            "" => config::default_model(f.provider_id()).unwrap_or_default(),
            m => m,
        };
        set(&mut tree, "ai", "model", model.into());
        set(&mut tree, "ai", "base_url", f.base_url.trim().into());
        // Same key resolution as the status line and Save: a provider
        // switch never tests with the previous provider's key.
        use super::pages::Credential;
        let lookup = self.env_lookup();
        let key = match f.ai_credential(&self.m.settings.saved) {
            Credential::Typed(_) => Some(f.api_key.trim().to_string()),
            Credential::Env(var) => Some(lookup(var).unwrap_or_default()),
            Credential::Clear => Some(String::new()),
            Credential::Saved | Credential::None => None,
        };
        if let Some(k) = key {
            set(&mut tree, "ai", "api_key", k.into());
        }
        if let Some(o) = tree.as_object_mut() {
            o.insert(
                "privacy".into(),
                if f.offline { "offline" } else { "cloud" }.into(),
            );
        }
        let st = config::settings(&tree, None).map_err(|e| e.to_string())?;
        self.guard_offline_test(&st, true)?;
        Ok(st)
    }

    /// Write the Settings form into config.yaml (comments kept) + key into `.env`.
    fn save_settings(&mut self, ctx: &egui::Context) {
        let f = self.m.settings.form.clone();
        let saved = self.m.settings.saved.clone();
        if let Some(v) = self.m.settings.blocker(&self.m.config) {
            self.m.settings.error = Some(v);
            return;
        }
        if self.m.demo {
            self.m.settings.saved = f;
            self.toast(ctx, "Settings saved");
            return;
        }
        let path = self
            .m
            .config
            .location
            .as_ref()
            .map(|l| l.path.clone())
            .unwrap_or_else(|| config::discover_config(self.cli_config.as_deref()).path);
        let mut text = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| crate::cmd::STARTER_CONFIG.to_string());
        // Only changed keys: untouched lines (comments, `null`) stay as written.
        for (sec, key, val) in f.changed_edits(&saved) {
            text = autorename_core::config_edit::set(&text, sec, key, &val);
        }
        // Validate before writing (problems → footer message, nothing saved).
        // Typed keys count as present (they are written to .env on save).
        let writes = f.env_writes(&saved);
        let lookup = self.env_lookup();
        let env = |n: &str| {
            writes
                .iter()
                .find(|(k, _)| *k == n)
                .map(|(_, v)| v.clone())
                .or_else(|| lookup(n))
        };
        let check = config::parse_yaml(&text)
            .and_then(|raw| config::from_raw(raw, &env).map_err(|e| e.to_string()))
            .map(|(tree, _)| config::validate(&tree, None));
        drop(lookup);
        match check {
            Ok(v) => {
                let errs: Vec<_> = v
                    .issues
                    .iter()
                    .filter(|i| i.level == config::Level::Error)
                    .collect();
                if let Some(first) = errs.first() {
                    self.m.settings.error = Some(format!(
                        "config.yaml would have {}: {} ({})",
                        c::plural(errs.len(), "problem", "problems"),
                        first.message,
                        first.field
                    ));
                    return;
                }
            }
            Err(e) => {
                self.m.settings.error = Some(e);
                return;
            }
        }
        let env_path = config::env_path(&path);
        let env = match (!writes.is_empty())
            .then(|| env_with(&env_path, &writes))
            .transpose()
        {
            Ok(env) => env,
            Err(e) => {
                self.m.settings.error = Some(e);
                return;
            }
        };
        let res = (|| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            autorename_core::fsx::write_atomic(&path, text.as_bytes())?;
            if let Some(env) = &env {
                autorename_core::config_edit::write_env(&env_path, env)?;
            }
            Ok(())
        })();
        match res {
            Ok(()) => {
                self.m.config = super::model::ConfigState::load(Some(&path), self.offline);
                self.m.settings.loaded = false;
                self.m.settings.error = self.m.config.error.clone();
                self.m.settings.test = None;
                self.load_tab();
                self.sync_name_list();
                self.toast(ctx, "Settings saved");
            }
            Err(e) => {
                self.m.settings.error = Some(format!("Couldn't save {}: {e}", path.display()))
            }
        }
    }

    pub fn add_paths(&mut self, paths: &[PathBuf]) {
        if self.m.busy() {
            return;
        }
        if self.m.config.settings.is_none() {
            self.m.setup.error = Some("Set up AI first, then add your files again.".into());
            return;
        }
        if self.m.report.is_some() {
            self.m.clear();
        }
        let collected = pipeline::collect_pdfs(paths, false);
        if collected.files.is_empty() {
            // Timer starts on the next frame (no ctx here).
            self.m.toasts.push(Toast {
                text: "No PDFs in the dropped files or folders.".into(),
                until: -1.0,
            });
            return;
        }
        let added = self.m.enqueue(&collected.files);
        self.schedule(&added, &[]);
    }

    /// Start one analysis with the CURRENT settings for every row that needs
    /// it (stale fingerprint, waiting, not analyzed), plus `extra` new files
    /// and `explicit` rows (errors the user retries). Returns the file count.
    /// Rows with a current result are never re-analyzed.
    fn schedule(&mut self, extra: &[PathBuf], explicit: &[usize]) -> usize {
        if self.m.busy() || self.m.report.is_some() {
            return 0;
        }
        let Some(settings) = self.m.config.settings.clone() else {
            return 0;
        };
        let mut files: Vec<PathBuf> = Vec::new();
        for (i, r) in self.m.rows.iter().enumerate() {
            if (self.m.needs_analysis(r) || explicit.contains(&i)) && !files.contains(&r.source) {
                files.push(r.source.clone());
            }
        }
        for f in extra {
            if !files.contains(f) {
                files.push(f.clone());
            }
        }
        if files.is_empty() {
            return 0;
        }
        self.m.editing = None;
        for r in &mut self.m.rows {
            if files.contains(&r.source) {
                *r = Row::queued(r.source.clone());
            }
        }
        if self.m.demo {
            // Demo: instant "analysis" from the canned items.
            let fp = self.m.current_fp();
            let items = super::demo::items();
            for r in &mut self.m.rows {
                if let Some(it) = items.iter().find(|it| it.source == r.source) {
                    *r = Row::from_item(it.clone());
                    r.fp = fp;
                }
            }
            self.m.meta = super::demo::preview_model().meta;
            if self.m.active.is_none() {
                self.m.active = (!self.m.rows.is_empty()).then_some(0);
            }
            return files.len();
        }
        let n = files.len();
        self.start_analysis(files, settings);
        n
    }

    /// The Re-analyze banner exists iff some row's result is stale.
    pub(crate) fn sync_stale_banner(&mut self) {
        let stale = self.m.stale_count() > 0 && !self.m.busy();
        let has = self
            .m
            .banners
            .iter()
            .any(|b| b.action.as_deref() == Some(REANALYZE));
        if stale && !has {
            self.m.banners.push(BannerMsg {
                tone: Tone::Warning,
                title: "Settings changed — re-analyze.".into(),
                body: "Some results were made with other privacy, provider or profile settings."
                    .into(),
                action: Some(REANALYZE.into()),
            });
        } else if !stale && has {
            self.m
                .banners
                .retain(|b| b.action.as_deref() != Some(REANALYZE));
        }
    }

    fn start_analysis(&mut self, files: Vec<PathBuf>, settings: config::Settings) {
        self.m.generation += 1;
        for r in &mut self.m.rows {
            if files.contains(&r.source) {
                r.not_analyzed = false;
            }
        }
        let loc = self.m.config.location.clone();
        self.m.run_fp = Some(super::model::fingerprint(&settings));
        {
            self.m.meta = Some(PlanMeta {
                profile: settings.profile_id.clone(),
                provider: settings.ai.provider.id().into(),
                model: settings.ai.model.clone(),
                privacy: match settings.privacy {
                    config::Privacy::Cloud => "cloud".into(),
                    config::Privacy::Offline => "offline".into(),
                },
            });
        }
        self.m.phase = Phase::Analyzing(Analysis {
            total: files.len(),
            done: 0,
            started: Instant::now(),
            done_at: Vec::new(),
        });
        let opts = PlanOptions {
            recursive: false,
            jobs: pipeline::default_jobs(),
            name_lists_dir: loc.as_ref().map(|l| config::name_lists_dir(&l.path)),
            models_dir: None,
            env: self.m.config.env.clone(),
            cancel: None,
        };
        self.worker
            .analyze(self.m.generation, files, settings, opts);
    }

    fn retry(&mut self, idx: &[usize]) {
        if self.m.busy() {
            return;
        }
        if self.m.report.is_none() {
            // Retried rows plus every stale or waiting row, one run.
            self.schedule(&[], idx);
            return;
        }
        if idx.is_empty() {
            return;
        }
        let Some(settings) = self.m.config.settings.clone() else {
            return;
        };
        {
            // After renaming: a fresh plan with only the failed / not-reached
            // files. Renamed files stay renamed; their batch keeps its Undo
            // in History.
            let files: Vec<PathBuf> = idx
                .iter()
                .filter_map(|&i| self.m.rows.get(i).map(|r| r.source.clone()))
                .collect();
            let prefs = self.m.prefs;
            self.m.clear();
            self.m.prefs = prefs;
            if self.m.demo {
                self.m.meta = super::demo::preview_model().meta;
                self.m.rows = super::demo::items()
                    .into_iter()
                    .filter(|it| files.contains(&it.source))
                    .map(|mut it| {
                        it.error = None;
                        it.status = autorename_core::pipeline::PlanStatus::Rename;
                        Row::from_item(it)
                    })
                    .collect();
                self.m.stamp_current();
                self.m.active = (!self.m.rows.is_empty()).then_some(0);
            } else {
                let added = self.m.enqueue(&files);
                self.start_analysis(added, settings);
            }
        }
    }

    fn rename(&mut self, ctx: &egui::Context) {
        if !self.m.can_rename() {
            return;
        }
        // Every included row has a current-fingerprint result (can_rename),
        // so the current settings are the plan's settings.
        let Some(plan) = self.m.build_plan() else {
            return;
        };
        if self.m.demo {
            let rep = super::demo::simulate_apply(&self.m);
            self.m.apply_report(rep);
            return;
        }
        let Some(settings) = self.m.config.settings.clone() else {
            return;
        };
        let Some(jp) = journal::default_journal_path() else {
            self.banner(Tone::Danger, "No data folder for the undo journal.", "");
            return;
        };
        let name_lists = self
            .m
            .config
            .location
            .as_ref()
            .map(|l| config::name_lists_dir(&l.path));
        self.m.phase = Phase::Renaming {
            done: 0,
            total: plan.items.len(),
        };
        self.worker.apply(
            plan,
            settings,
            ApplyOptions {
                journal_path: jp,
                name_lists_dir: name_lists,
                source: "gui".into(),
                cancel: None,
            },
        );
        ctx.request_repaint();
    }

    /// `--rename-now` after the analysis: rename when every file is ready;
    /// otherwise stay in the preview and say why.
    pub fn rename_now(&mut self, ctx: &egui::Context) {
        let c = self.m.counts();
        if c.review == 0 && c.errors == 0 && c.pending == 0 && c.checked > 0 {
            self.rename(ctx);
        } else if c.review + c.errors > 0 {
            self.banner(
                Tone::Warning,
                "Nothing renamed yet.",
                "Some files need a look. Check the flagged rows, then rename.",
            );
        }
    }

    fn undo(&mut self, ctx: &egui::Context) {
        if self.m.undone || self.m.busy() {
            return;
        }
        let Some(batch) = self.m.report.as_ref().and_then(|r| r.batch_id.clone()) else {
            return;
        };
        if self.m.demo {
            self.m.undone = true;
            let n = self.m.report.as_ref().map_or(0, |r| r.summary.renamed);
            self.toast(ctx, format!("Undid {}", c::plural(n, "rename", "renames")));
            return;
        }
        let Some(jp) = journal::default_journal_path() else {
            return;
        };
        self.m.undoing = Some(batch.clone());
        self.worker.undo(jp, batch);
    }

    /// Write the starter config with the chosen provider (+ key into `.env`).
    fn save_setup(&mut self) {
        if self.m.demo {
            self.m.config = super::demo::config();
            return;
        }
        let s = self.m.setup.clone();
        let loc = self
            .m
            .config
            .location
            .clone()
            .unwrap_or_else(|| config::discover_config(self.cli_config.as_deref()));
        let path = loc.path.clone();
        if path.exists() {
            self.m.setup.error = Some(format!(
                "{} has problems: {}",
                file_name(&path),
                self.m.config.error.clone().unwrap_or_default()
            ));
            return;
        }
        let (id, _, var) = if s.offline {
            super::model::PROVIDERS[4]
        } else {
            super::model::PROVIDERS[s.provider.min(3)]
        };
        let text: String = crate::cmd::STARTER_CONFIG
            .lines()
            .map(|l| {
                let t = l.trim_start();
                let indent = &l[..l.len() - t.len()];
                if t.starts_with("privacy:") {
                    format!("privacy: {}", if s.offline { "offline" } else { "cloud" })
                } else if t.starts_with("provider:") && indent == "  " {
                    format!("{indent}provider: {id}")
                } else if t.starts_with("model:")
                    && indent == "  "
                    && !s.model.trim().is_empty()
                    && !s.offline
                {
                    let q = autorename_core::config_edit::quoted(s.model.trim());
                    format!("{indent}model: {q}")
                } else if t.starts_with("api_key:") {
                    if var.is_empty() {
                        format!("{indent}api_key: \"\"")
                    } else {
                        format!("{indent}api_key: \"${{{var}}}\"")
                    }
                } else if t.starts_with("base_url:") && s.offline {
                    let q = autorename_core::config_edit::quoted(s.ollama_url.trim());
                    format!("{indent}base_url: {q}")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let env_path = config::env_path(&path);
        let env = if !s.api_key.trim().is_empty() && !var.is_empty() {
            match env_with(&env_path, &[(var, s.api_key.trim().to_string())]) {
                Ok(env) => Some(env),
                Err(e) => {
                    self.m.setup.error = Some(e);
                    return;
                }
            }
        } else {
            None
        };
        let res = (|| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            autorename_core::fsx::write_atomic(&path, format!("{text}\n").as_bytes())?;
            if let Some(env) = &env {
                autorename_core::config_edit::write_env(&env_path, env)?;
            }
            Ok(())
        })();
        match res {
            Ok(()) => {
                self.m.config = super::model::ConfigState::load(Some(&path), self.offline);
                self.m.setup.error = self.m.config.error.clone();
            }
            Err(e) => self.m.setup.error = Some(format!("Couldn't save {}: {e}", path.display())),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    #[cfg_attr(not(target_os = "linux"), expect(unused_variables))]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        #[cfg(target_os = "linux")]
        if let Some(dnd) = &self.wayland_dnd {
            dnd.inject(raw);
        }
    }

    fn on_exit(&mut self) {
        // Stop the Wayland listener while winit's display is still alive.
        #[cfg(target_os = "linux")]
        {
            self.wayland_dnd = None;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORE_DETAIL_W, &self.m.detail_w);
        eframe::set_value(storage, STORE_DETAIL_VISIBLE, &self.m.detail_visible);
        eframe::set_value(storage, STORE_PREFS, &self.m.prefs);
    }
}

fn is_pdf(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// Native picker on a helper thread (the future is created on the UI thread,
/// as rfd requires on macOS) so the UI keeps painting.
fn pick(folder: bool, ctx: egui::Context) -> Receiver<Vec<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let dlg = rfd::AsyncFileDialog::new().set_title(if folder {
        "Choose a folder"
    } else {
        "Choose PDF files"
    });
    let fut: std::pin::Pin<Box<dyn std::future::Future<Output = Vec<PathBuf>> + Send>> = if folder {
        Box::pin(async move {
            dlg.pick_folder()
                .await
                .map(|h| vec![h.path().to_path_buf()])
                .unwrap_or_default()
        })
    } else {
        let dlg = dlg.add_filter("PDF", &["pdf", "PDF"]);
        Box::pin(async move {
            dlg.pick_files()
                .await
                .map(|v| v.iter().map(|h| h.path().to_path_buf()).collect())
                .unwrap_or_default()
        })
    };
    std::thread::spawn(move || {
        let _ = tx.send(block_on(fut));
        ctx.request_repaint();
    });
    rx
}

fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

fn open_path(p: &Path) {
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(p).spawn();
}

/// GUI Rename: a spelling change of one entry (`harmonize::rename`; the old
/// spelling stays an alias). A name that already belongs to another entry
/// is refused with a pointer to Merge, never merged silently.
pub(crate) fn rename_entry(
    db: &mut autorename_core::harmonize::NameDb,
    old: &str,
    new: &str,
) -> autorename_core::Result<()> {
    use autorename_core::harmonize;
    let owner = harmonize::index(db)
        .get(&harmonize::normalize(db.kind(), new))
        .map(|&j| db.entries[j].canonical.clone());
    if let Some(owner) = owner.filter(|o| o != old) {
        return Err(autorename_core::Error::General(format!(
            "\u{201c}{new}\u{201d} already belongs to {owner}. Use Merge into\u{2026} to combine them."
        )));
    }
    harmonize::rename(db, old, new)
}

/// The `.env` at `path` with `writes` applied, checked as dotenvy will read
/// it: the whole file parses and every written key reads back exactly.
/// Save refuses (writes nothing) otherwise.
fn env_with(path: &std::path::Path, writes: &[(&str, String)]) -> Result<String, String> {
    use autorename_core::config_edit::{check_env, get_env, set_env};
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let env = writes
        .iter()
        .fold(old, |env, (var, value)| set_env(&env, var, value));
    let fix = format!("Fix {} first, then save again.", path.display());
    check_env(&env).map_err(|e| format!("{e}. {fix}"))?;
    match writes
        .iter()
        .find(|(k, v)| get_env(&env, k).as_ref() != Some(v))
    {
        Some((k, _)) => Err(format!(".env would not keep {k}. {fix}")),
        None => Ok(env),
    }
}
