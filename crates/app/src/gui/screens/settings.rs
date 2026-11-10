//! §4.7 Settings: left nav 200 (General, Processing, Naming, About), one
//! scroll column ≤ 640 of groups at `space.xl`, sticky footer with
//! [Save changes] when dirty.

use egui::{Rect, Sense, Ui, WidgetInfo};

use super::Action;
use crate::gui::components::banner::{Banner, tone_icon};
use crate::gui::components::{
    Badge, Button, Field, Progress, Select, Stepper, anim, dur, ellipsize_end, ellipsize_middle,
    focus_ring, galley, in_rect, label, mix, paint_galley_v, paint_icon, paint_text,
    paint_text_center, paint_wrapped, segmented, toggle, wrapped_lines,
};
use crate::gui::model::{KeySource, Model, PROVIDERS, noun};
use crate::gui::pages::{
    AMOUNT_PRESETS, DATE_PRESETS, DECIDERS, FormField, KeyState, MAX_PAGES_RANGE, OUTPUT_LANGUAGES,
    RETRIES_RANGE, SCALES, SETTINGS_PAGES, SettingsForm, TEMPERATURE_RANGE, THRESHOLD_RANGE,
    TOGGLES, TextDoc, amount_example, cpu_count, date_example, page,
};
use crate::gui::theme::{self, Tone, motion, size, space, stroke, typ};
use autorename_core::ocr::Pack;
use autorename_core::profiles::Profile;

pub const NAV_W: f32 = size::SETTINGS_NAV;

pub fn settings(ui: &mut Ui, body: Rect, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(body, 0, p.bg);
    let nav = Rect::from_min_size(body.min, theme::v2(NAV_W, body.height()));
    ui.painter().rect_filled(nav, 0, p.surface_sunken);
    ui.painter().vline(
        nav.right() - stroke::HAIRLINE / 2.0,
        nav.y_range(),
        theme::hairline(p.border),
    );
    let mut y = nav.top() + space::LG;
    for (i, name) in SETTINGS_PAGES.iter().enumerate() {
        let r = Rect::from_min_size(
            egui::pos2(nav.left() + space::SM, y),
            theme::v2(NAV_W - space::LG, size::CONTROL),
        );
        let resp = ui.interact(r, ui.scope_id().with(("nav", i)), Sense::click());
        let sel = m.settings.section == i;
        let h = anim(ui, resp.id.with("h"), resp.hovered());
        let fill = if sel {
            p.selection
        } else {
            mix(egui::Color32::TRANSPARENT, p.surface_hover, h)
        };
        ui.painter()
            .rect_filled(r, theme::cr(theme::radius::MD), fill);
        let t = if sel { typ::BODY_STRONG } else { typ::BODY };
        paint_text(ui, r.left() + space::MD, r.center().y, name, t, p.text);
        focus_ring(ui, &resp, r, theme::radius::MD, true);
        resp.widget_info(|| WidgetInfo::selected(egui::Role::Tab, true, sel, *name));
        if resp.clicked() {
            m.settings.section = i;
        }
        y = r.bottom() + space::XS;
    }

    let dirty = m.settings.dirty();
    let footer = dirty.then(|| {
        Rect::from_min_max(
            egui::pos2(nav.right(), body.bottom() - size::ACTION_BAR),
            body.max,
        )
    });
    let content = Rect::from_min_max(
        egui::pos2(nav.right(), body.top()),
        egui::pos2(body.right(), footer.map_or(body.bottom(), |f| f.top())),
    );
    let col_w = (content.width() - space::XL * 2.0).min(size::SETTINGS_COL);
    in_rect(ui, content, |ui| {
        let sa = egui::ScrollArea::vertical()
            .id_salt(("settings", m.settings.section))
            .auto_shrink([false, false]);
        sa.show(ui, |ui| {
            ui.add_space(space::XL);
            ui.horizontal_top(|ui| {
                ui.add_space(space::XL);
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
                    match m.settings.section.min(page::ABOUT) {
                        page::GENERAL => general(ui, m, acts),
                        page::PROCESSING => processing(ui, m, acts),
                        page::NAMING => naming(ui, m, acts),
                        _ => about(ui, acts),
                    }
                    ui.add_space(space::XL);
                });
            });
        });
    });
    if let Some(f) = footer {
        save_footer(ui, f, m, acts);
    }
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

/// Page title (`type.title`) + one muted line.
fn page_title(ui: &mut Ui, t: &str, desc: &str) {
    let p = theme::palette(ui);
    label(ui, t, typ::TITLE, p.text);
    ui.add_space(space::XS);
    para(ui, desc, p.text_muted);
}

/// Group heading (`type.title_sm`) + one-line muted description;
/// `space.xxl` above (§3 rhythm), `space.lg` before the first control.
fn group(ui: &mut Ui, t: &str, desc: &str) {
    let p = theme::palette(ui);
    ui.add_space(space::XXL);
    label(ui, t, typ::TITLE_SM, p.text);
    if !desc.is_empty() {
        ui.add_space(space::XS);
        para(ui, desc, p.text_muted);
    }
    ui.add_space(space::LG);
}

fn para(ui: &mut Ui, text: &str, color: egui::Color32) {
    let w = ui.available_width().min(size::PROSE_MAX);
    let n = wrapped_lines(ui, text, typ::BODY, w);
    let (r, _) = ui.allocate_exact_size(theme::v2(w, typ::BODY.line * n as f32), Sense::hover());
    paint_wrapped(ui, r.min, w, text, typ::BODY, color);
}

fn caption(ui: &mut Ui, text: &str, color: egui::Color32) {
    let w = ui.available_width().min(size::PROSE_MAX);
    let n = wrapped_lines(ui, text, typ::CAPTION, w);
    let (r, _) = ui.allocate_exact_size(theme::v2(w, typ::CAPTION.line * n as f32), Sense::hover());
    paint_wrapped(ui, r.min, w, text, typ::CAPTION, color);
}

/// One helper line under a control (caption, muted).
fn helper(ui: &mut Ui, text: &str) {
    let p = theme::palette(ui);
    ui.add_space(space::XS);
    caption(ui, text, p.text_muted);
}

fn field_label(ui: &mut Ui, t: &str) {
    let p = theme::palette(ui);
    label(ui, t, typ::BODY_STRONG, p.text);
    ui.add_space(space::XS);
}

fn status_line(ui: &mut Ui, tone: Tone, text: &str) -> egui::Response {
    let p = theme::palette(ui);
    let c = if tone == Tone::Neutral {
        p.text_muted
    } else {
        p.tone(tone).1
    };
    ui.add_space(space::XS);
    let w = ui.available_width().min(size::PROSE_MAX);
    let tw = w - size::ICON_SM - space::XS;
    let lines = wrapped_lines(ui, text, typ::CAPTION, tw);
    let (r, resp) = ui.allocate_exact_size(
        theme::v2(w, typ::CAPTION.line * lines as f32),
        Sense::hover(),
    );
    paint_icon(
        ui,
        egui::pos2(
            r.left() + size::ICON_SM / 2.0,
            r.top() + typ::CAPTION.line / 2.0,
        ),
        tone_icon(tone),
        size::ICON_SM,
        c,
    );
    paint_wrapped(
        ui,
        egui::pos2(r.left() + size::ICON_SM + space::XS, r.top()),
        tw,
        text,
        typ::CAPTION,
        c,
    );
    resp
}

fn banner(ui: &mut Ui, tone: Tone, title: &str, body: &str) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(theme::v2(w, Banner::HEIGHT), Sense::hover());
    Banner {
        tone,
        title,
        body,
        action: None,
        dismissible: false,
    }
    .show_at(ui, r);
}

/// Labelled stepper on an `f64` view of the value.
fn stepper(
    ui: &mut Ui,
    title: &str,
    range: (f64, f64, f64),
    decimals: usize,
    value: &mut f64,
) -> bool {
    field_label(ui, title);
    Stepper::new(title, range)
        .decimals(decimals)
        .show(ui, value)
}

fn stepper_u32(ui: &mut Ui, title: &str, range: (f64, f64, f64), value: &mut u32) {
    let mut v = f64::from(*value);
    if stepper(ui, title, range, 0, &mut v) {
        *value = v.round() as u32;
    }
}

/// Collapsible group heading (chevron + `type.title_sm`); returns the
/// openness 0..1 (animated with `motion.normal`, 0 under reduce motion).
fn collapsible(ui: &mut Ui, t: &str, open: &mut bool) -> f32 {
    let p = theme::palette(ui);
    ui.add_space(space::XXL - (size::CONTROL - typ::TITLE_SM.line) / 2.0);
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(theme::v2(w, size::CONTROL), Sense::click());
    if resp.clicked() {
        *open = !*open;
    }
    let h = anim(ui, resp.id.with("h"), resp.hovered());
    ui.painter().rect_filled(
        r,
        theme::cr(theme::radius::MD),
        mix(egui::Color32::TRANSPARENT, p.surface_hover, h),
    );
    let cy = r.center().y;
    paint_icon(
        ui,
        egui::pos2(r.left() + size::ICON / 2.0, cy),
        if *open {
            theme::icon::CHEVRON_DOWN
        } else {
            theme::icon::CHEVRON_RIGHT
        },
        size::ICON,
        p.text_muted,
    );
    paint_galley_v(
        ui,
        r.left() + size::ICON + space::SM,
        cy,
        galley(ui, t, typ::TITLE_SM),
        p.text,
    );
    focus_ring(ui, &resp, r, theme::radius::MD, false);
    let o = *open;
    resp.widget_info(|| {
        WidgetInfo::selected(
            egui::Role::Button,
            true,
            o,
            format!("{t} section, {}", if o { "expanded" } else { "collapsed" }),
        )
    });
    ui.add_space(-(size::CONTROL - typ::TITLE_SM.line) / 2.0);
    ui.ctx()
        .animate_bool_with_time(resp.id.with("open"), o, dur(ui.ctx(), motion::NORMAL))
}

/// Path row: label column + mono path (middle ellipsis, full in tooltip).
fn path_row(ui: &mut Ui, k: &str, v: &str) {
    let p = theme::palette(ui);
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(theme::v2(w, size::CONTROL), Sense::hover());
    paint_text(ui, r.left(), r.center().y, k, typ::BODY, p.text);
    let col = size::LABEL_COL * 1.5;
    let t = ellipsize_middle(ui, v, typ::MONO_SM, w - col);
    paint_text(
        ui,
        r.left() + col,
        r.center().y,
        &t,
        typ::MONO_SM,
        p.text_muted,
    );
    if t != v {
        resp.on_hover_text(v);
    }
}

// ---------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------

fn general(ui: &mut Ui, m: &mut Model, acts: &mut Vec<Action>) {
    page_title(
        ui,
        "General",
        "How the app looks and where it keeps its files.",
    );
    group(
        ui,
        "Appearance",
        "Stored on this computer and applied right away.",
    );
    field_label(ui, "Theme");
    let t = ["System".to_string(), "Light".into(), "Dark".into()];
    segmented(ui, "Theme", &t, &mut m.prefs.theme);
    ui.add_space(space::LG);
    field_label(ui, "Fewer animations");
    let r = ["System".to_string(), "On".into(), "Off".into()];
    segmented(ui, "Fewer animations", &r, &mut m.prefs.reduce_motion);
    helper(
        ui,
        "Reduces movement throughout the app. System follows your computer's setting.",
    );
    ui.add_space(space::LG);
    field_label(ui, "App size");
    let s: Vec<String> = SCALES
        .iter()
        .map(|s| format!("{} %", (s * 100.0).round()))
        .collect();
    segmented(ui, "App size", &s, &mut m.prefs.scale);
    helper(ui, "Changes text and control sizes.");

    group(ui, "Files", "Your settings, name lists and undo history.");
    let loc = m.config.location.as_ref().map(|l| l.path.clone());
    let cfg = loc
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "config.yaml (not created yet)".into());
    let lists = loc
        .as_ref()
        .map(|p| autorename_core::config::name_lists_dir(p))
        .or_else(|| {
            m.companies
                .path
                .as_ref()
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        })
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "name-lists (beside config.yaml)".into());
    path_row(ui, "Settings file", &cfg);
    path_row(ui, "Name lists", &lists);
    path_row(ui, "Undo history", &m.journal_display.clone());
    ui.add_space(space::SM);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        if Button::secondary("Open settings file")
            .icon(theme::icon::FILE_TEXT)
            .show(ui)
            .clicked()
        {
            acts.push(Action::OpenConfigFile);
        }
        if Button::secondary("Open settings folder")
            .icon(theme::icon::FOLDER_OPEN)
            .show(ui)
            .clicked()
        {
            acts.push(Action::OpenConfigFolder);
        }
    });
}

// ---------------------------------------------------------------------------
// Processing
// ---------------------------------------------------------------------------

fn processing(ui: &mut Ui, m: &mut Model, acts: &mut Vec<Action>) {
    page_title(
        ui,
        "Processing",
        "What happens to each PDF before it gets its name.",
    );
    privacy(ui, m);
    provider(ui, m, acts);
    reading(ui, m, acts);
    group(ui, "Metadata", "What renamed PDFs carry inside.");
    toggle(
        ui,
        &mut m.settings.form.enrich,
        "Write metadata to renamed PDFs",
    );
    helper(
        ui,
        "Adds title, subject and keywords so other tools can search them. Undo removes them. Signed, encrypted, PDF/A and e-invoice files are left as they are.",
    );
    advanced(ui, m);
}

/// Privacy explanation for the chosen mode; `pending` = not saved yet
/// (the saved mode still applies until Save).
pub fn privacy_text(offline: bool, pending: bool) -> String {
    let (mode, rest) = if offline {
        (
            "Offline",
            "text, page images and name decisions stay on this computer or your local network (Ollama). Requests to the internet are blocked.",
        )
    } else {
        (
            "Cloud",
            "the text of each file (and page images when AI vision is used) is sent to your AI provider. Nothing is renamed until you review the plan.",
        )
    };
    if pending {
        format!("After saving, {rest}")
    } else {
        format!("{mode}: {rest}")
    }
}

fn privacy(ui: &mut Ui, m: &mut Model) {
    let p = theme::palette(ui);
    group(ui, "Privacy", "Where the text of your documents goes.");
    let opts = ["Cloud".to_string(), "Offline".to_string()];
    let saved_offline = m.settings.saved.offline;
    let (f, saved) = (&mut m.settings.form, &m.settings.saved);
    let mut sel = usize::from(f.offline);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::MD;
        segmented(ui, "Privacy mode", &opts, &mut sel);
        if (sel == 1) != saved_offline {
            let now = if saved_offline { "Offline" } else { "Cloud" };
            label(
                ui,
                &format!("Current mode: {now}"),
                typ::CAPTION,
                p.text_muted,
            );
        }
    });
    if (sel == 1) != f.offline {
        f.offline = sel == 1;
        if f.offline && f.provider != 4 {
            f.provider = 4;
            f.provider_changed(saved);
        } else if f.offline {
            f.api_key.clear();
            f.model = autorename_core::config::default_model("ollama")
                .unwrap_or_default()
                .to_string();
        }
    }
    ui.add_space(space::LG);
    let text = privacy_text(f.offline, f.offline != saved_offline);
    para(ui, &text, p.text);
    if let Some(v) = f.offline_violation() {
        ui.add_space(space::LG);
        banner(ui, Tone::Danger, "Can't save:", &v);
    }
}

fn provider(ui: &mut Ui, m: &mut Model, acts: &mut Vec<Action>) {
    group(
        ui,
        "AI provider",
        "The AI that reads each document and suggests its name.",
    );
    let names: Vec<String> = PROVIDERS.iter().map(|p| p.1.to_string()).collect();
    field_label(ui, "Provider");
    let (f, saved) = (&mut m.settings.form, &m.settings.saved);
    let before = f.provider;
    Select {
        a11y: "Provider",
        width: theme::TEXT_EDIT_W,
        options: &names,
        enabled: true,
        mono: false,
    }
    .show(ui, &mut f.provider);
    if before != f.provider {
        f.provider_changed(saved);
        m.settings.test = None;
    }
    ui.add_space(space::LG);
    let default = autorename_core::config::default_model(f.provider_id()).unwrap_or("none");
    let model_help = format!("Leave empty for the provider default ({default}).");
    Field::new("Model", &mut f.model)
        .label("Model")
        .mono()
        .helper(Some(&model_help))
        .width(theme::TEXT_EDIT_W)
        .show(ui);
    let state = f.ai_key_state(&m.settings.saved, &m.config);
    if let Some(state) = state {
        ui.add_space(space::LG);
        let configured = !matches!(state, KeyState::Found(KeySource::Missing(_)));
        let service = PROVIDERS[f.provider.min(4)].1;
        let f = &mut m.settings.form;
        let shown = &mut f.show_key;
        Field::new("API key", &mut f.api_key)
            .label("API key")
            .mono()
            .secret(shown)
            .hint(if configured {
                "Keep the current key"
            } else {
                "Paste your key"
            })
            .width(theme::TEXT_EDIT_W)
            .show(ui);
        key_line(ui, &state, service, service);
    }
    let f = &mut m.settings.form;
    ui.add_space(space::LG);
    let url_err = f.problem(FormField::BaseUrl);
    Field::new("Custom server address (optional)", &mut f.base_url)
        .label("Custom server address (optional)")
        .mono()
        .hint("Provider default")
        .helper(Some(
            "Only for Ollama on another computer or an OpenAI-compatible server.",
        ))
        .error(url_err.as_deref())
        .width(ui.available_width().min(theme::TEXT_EDIT_W * 1.5))
        .show(ui);
    ui.add_space(space::LG);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        let b = Button::secondary("Test connection")
            .loading(m.settings.testing)
            .enabled(!m.settings.testing);
        if b.show(ui).clicked() {
            acts.push(Action::TestSettings);
        }
        match &m.settings.test {
            Some(Ok(ms)) => {
                Badge::new(Tone::Success, format!("Connected · {ms} ms"))
                    .icon(theme::icon::CIRCLE_CHECK)
                    .show(ui);
            }
            Some(Err(_)) => {
                Badge::new(Tone::Danger, "Connection failed")
                    .icon(theme::icon::CIRCLE_X)
                    .show(ui);
            }
            None => {}
        }
    });
    if let Some(Err(e)) = &m.settings.test {
        ui.add_space(space::XS);
        let p = theme::palette(ui);
        caption(ui, e, p.danger);
    }
}

/// Key status copy: (tone, text, storage details for the tooltip).
/// `owner` is whose key it is ("OpenAI"), `service` what needs it ("Jev",
/// "OpenAI Decisions").
pub fn key_status(state: &KeyState, owner: &str, service: &str) -> (Tone, String, String) {
    let file = "the .env file beside the settings file";
    match state {
        KeyState::Typed(v) => (
            Tone::Neutral,
            "The key is stored when you save.".into(),
            format!("Saved in {file} as {v}."),
        ),
        KeyState::Found(KeySource::Process(v)) => (
            Tone::Success,
            "API key found on this computer".into(),
            format!("From the environment variable {v}."),
        ),
        KeyState::Found(KeySource::DotEnv(v)) => (
            Tone::Success,
            "API key saved".into(),
            format!("Stored in {file} as {v}."),
        ),
        KeyState::Found(KeySource::Literal) => (
            Tone::Warning,
            "Paste your key again to store it safely.".into(),
            format!("The key is written in the settings file. Saving it again moves it to {file}."),
        ),
        KeyState::Found(KeySource::Missing(v)) => (
            Tone::Danger,
            format!("Add your {owner} API key to use {service}."),
            format!("Paste it above, or set {v} in {file}."),
        ),
    }
}

fn key_line(ui: &mut Ui, state: &KeyState, owner: &str, service: &str) {
    let (tone, text, details) = key_status(state, owner, service);
    status_line(ui, tone, &text).on_hover_text(details);
}

/// Live status of the OCR step: (tone, text). `models` = pack installed.
pub fn ocr_status(mode: usize, models: bool, vision_on: bool, downloading: bool) -> (Tone, String) {
    if downloading && !models {
        let then = if vision_on {
            "scans go to AI vision until done"
        } else {
            "scans are skipped until done"
        };
        return match mode {
            0 => (Tone::Accent, format!("Auto · downloading models — {then}")),
            1 => (
                Tone::Warning,
                "On · downloading models — scans fail until done".into(),
            ),
            _ => (
                Tone::Neutral,
                "Off · scans aren't read on this computer".into(),
            ),
        };
    }
    match (mode, models) {
        (0, true) => (
            Tone::Success,
            "Auto · reads scans and pictures such as logos".into(),
        ),
        (0, false) if vision_on => (
            Tone::Warning,
            "Auto · models not installed — scans go to AI vision".into(),
        ),
        (0, false) => (
            Tone::Warning,
            "Auto · models not installed — scans are skipped".into(),
        ),
        (1, true) => (
            Tone::Success,
            "On · every page is read on this computer".into(),
        ),
        (1, false) => (
            Tone::Danger,
            "On · models missing — scans will fail until you download them".into(),
        ),
        _ => (
            Tone::Neutral,
            "Off · scans aren't read on this computer".into(),
        ),
    }
}

/// Live status of the AI vision step.
pub fn vision_status(mode: usize, offline: bool, provider: &str, model: &str) -> (Tone, String) {
    let target = if offline {
        format!("uses the local Ollama model {model}")
    } else {
        format!("sends page images to {provider}")
    };
    match mode {
        0 => (
            Tone::Neutral,
            format!("Auto · {target} only when a page can't be read otherwise"),
        ),
        1 => (Tone::Warning, format!("On · {target} for every document")),
        _ => (Tone::Neutral, "Off · no page images are used".into()),
    }
}

/// Neither local OCR nor vision can read a scan.
pub fn scans_unreadable(ocr_mode: usize, models: bool, vision_mode: usize) -> bool {
    let ocr_ok = ocr_mode != 2 && models;
    !ocr_ok && vision_mode == 2
}

fn route_step(ui: &mut Ui, n: usize, title: &str, status: (Tone, String)) {
    let p = theme::palette(ui);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(theme::v2(w, typ::BODY.line), Sense::hover());
    let badge = format!("{n}");
    let br = Rect::from_min_size(
        egui::pos2(r.left(), r.center().y - size::BADGE / 2.0),
        theme::v2(size::BADGE, size::BADGE),
    );
    ui.painter()
        .rect_filled(br, theme::cr(theme::radius::FULL), p.accent_subtle);
    paint_text_center(ui, br, &badge, typ::CAPTION_STRONG, p.accent_text);
    paint_text(
        ui,
        br.right() + space::SM,
        r.center().y,
        title,
        typ::BODY_STRONG,
        p.text,
    );
    step_body(ui, |ui| {
        status_line(ui, status.0, &status.1);
    });
}

/// Indent a route step's body to the step title (badge 20 + gap 8).
fn step_body(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: (size::BADGE + space::SM) as i8,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
            f(ui)
        });
}

/// OCR language name for an `ocr.language` code ("de" → "German").
pub fn ocr_language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "af" => "Afrikaans",
        "az" => "Azerbaijani",
        "bs" => "Bosnian",
        "ca" => "Catalan",
        "cs" => "Czech",
        "cy" => "Welsh",
        "da" => "Danish",
        "de" => "German",
        "es" => "Spanish",
        "et" => "Estonian",
        "eu" => "Basque",
        "fi" => "Finnish",
        "fr" => "French",
        "ga" => "Irish",
        "gl" => "Galician",
        "hr" => "Croatian",
        "hu" => "Hungarian",
        "id" => "Indonesian",
        "is" => "Icelandic",
        "it" => "Italian",
        "ku" => "Kurdish",
        "la" => "Latin",
        "lb" => "Luxembourgish",
        "lt" => "Lithuanian",
        "lv" => "Latvian",
        "mi" => "Maori",
        "ms" => "Malay",
        "mt" => "Maltese",
        "nl" => "Dutch",
        "no" => "Norwegian",
        "oc" => "Occitan",
        "pi" => "Pali",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "qu" => "Quechua",
        "rm" => "Romansh",
        "ro" => "Romanian",
        "rs_latin" => "Serbian (Latin)",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "sq" => "Albanian",
        "sv" => "Swedish",
        "sw" => "Swahili",
        "tl" => "Tagalog",
        "tr" => "Turkish",
        "uz" => "Uzbek",
        "vi" => "Vietnamese",
        other => other,
    }
}

/// The pack serving `lang` (an id or a language code) among `packs`.
fn find_pack<'a>(packs: &'a [Pack], lang: &str) -> Option<&'a Pack> {
    let l = lang.trim().to_ascii_lowercase();
    packs
        .iter()
        .find(|p| p.id == l || p.languages.contains(&l.as_str()))
}

/// Read-only scan-language line (§4.7 Reading) while only one pack exists:
/// "Automatic" and the pack would do the same thing, so there is nothing to
/// choose. `None` (show the select) with two or more packs, or when the
/// config names a language no pack reads (the select repairs it).
pub fn ocr_language_line(current: &str, packs: &[Pack]) -> Option<String> {
    let [pk] = packs else { return None };
    let cur = current.trim();
    let served =
        cur.is_empty() || autorename_core::ocr::is_auto(cur) || find_pack(packs, cur).is_some();
    served.then(|| format!("Reads {}: {}", script_languages(pk), languages_short(pk)))
}

/// Options of the scan-language select (§4.7 Reading, two or more packs):
/// `auto` first, then each installed pack (and the pack of the current
/// value), then the current value when no pack serves it.
/// Returns `(config value, label)` pairs.
pub fn ocr_language_options(
    current: &str,
    packs: &[Pack],
    installed: &[&str],
) -> Vec<(String, String)> {
    let cur = current.trim();
    let cur_pack = (!autorename_core::ocr::is_auto(cur))
        .then(|| find_pack(packs, cur))
        .flatten()
        .map(|pk| pk.id);
    let mut v = vec![(
        autorename_core::ocr::AUTO_LANGUAGE.to_string(),
        "Automatic (all installed languages)".to_string(),
    )];
    v.extend(
        packs
            .iter()
            .filter(|pk| installed.contains(&pk.id) || cur_pack == Some(pk.id))
            .map(|pk| (pk.id.to_string(), pack_option(pk))),
    );
    if !cur.is_empty() && !autorename_core::ocr::is_auto(cur) && cur_pack.is_none() {
        v.push((cur.to_string(), cur.to_string()));
    }
    v
}

/// Index of `current` in its [`ocr_language_options`]: a hand-written code
/// shows as its pack's option.
pub fn ocr_language_index(current: &str, options: &[(String, String)], packs: &[Pack]) -> usize {
    let cur = current.trim();
    if autorename_core::ocr::is_auto(cur) || cur.is_empty() {
        return 0;
    }
    let value = find_pack(packs, cur).map_or(cur, |pk| pk.id);
    options.iter().position(|(v, _)| v == value).unwrap_or(0)
}

/// Languages named first in pack labels (most common documents).
const FEATURED_LANGUAGES: [&str; 5] = ["en", "de", "fr", "es", "it"];

/// Language names of a pack: featured ones first, the rest by name.
pub fn pack_languages(pk: &autorename_core::ocr::Pack) -> Vec<&'static str> {
    let mut rest: Vec<&str> = pk
        .languages
        .iter()
        .filter(|c| !FEATURED_LANGUAGES.contains(c))
        .map(|&c| ocr_language_name(c))
        .collect();
    rest.sort_unstable();
    FEATURED_LANGUAGES
        .iter()
        .filter(|c| pk.languages.contains(c))
        .map(|&c| ocr_language_name(c))
        .chain(rest)
        .collect()
}

fn script_name(pk: &autorename_core::ocr::Pack) -> &'static str {
    match pk.id {
        "latin" => "Latin script",
        id => id,
    }
}

/// "Latin-script languages" (the read-only line's subject).
fn script_languages(pk: &Pack) -> String {
    match pk.id {
        "latin" => "Latin-script languages".into(),
        id => format!("{id} languages"),
    }
}

/// "English, German, French + 45 more".
fn languages_short(pk: &Pack) -> String {
    let langs = pack_languages(pk);
    let shown = &langs[..langs.len().min(3)];
    let more = langs.len() - shown.len();
    let mut out = shown.join(", ");
    if more > 0 {
        out.push_str(&format!(" + {more} more"));
    }
    out
}

/// Select label of a pack: "Latin script: English, German, French + 45 more".
pub fn pack_option(pk: &autorename_core::ocr::Pack) -> String {
    format!("{}: {}", script_name(pk), languages_short(pk))
}

/// Tooltip of the scan-language select: every language that is read.
pub fn ocr_language_details(current: &str) -> String {
    let list = |pk: &autorename_core::ocr::Pack| {
        format!("{}: {}.", script_name(pk), pack_languages(pk).join(", "))
    };
    if autorename_core::ocr::is_auto(current) || current.trim().is_empty() {
        let packs: Vec<String> = autorename_core::ocr::PACKS.iter().map(list).collect();
        return format!("Uses every installed scan reader. {}", packs.join(" "));
    }
    match autorename_core::ocr::pack_for_language(current) {
        Some(pk) => list(pk),
        None => format!("No scan reader for \"{}\".", current.trim()),
    }
}

fn reading(ui: &mut Ui, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    group(
        ui,
        "Reading",
        "How text is taken from each page. Free steps on this computer come first.",
    );
    stepper_u32(
        ui,
        "Pages read per PDF",
        MAX_PAGES_RANGE,
        &mut m.settings.form.max_pages,
    );
    helper(
        ui,
        "Only the first pages are read and sent. Names are almost always on page 1.",
    );
    ui.add_space(space::XL);
    para(
        ui,
        "Auto uses a step only for pages the step before couldn't fully read: scans, and pages with pictures such as logos. On uses it for every page.",
        p.text_muted,
    );
    ui.add_space(space::LG);
    let f = m.settings.form.clone();
    let pack_id = autorename_core::ocr::pack_for_language(&f.ocr_language).map(|pk| pk.id);
    let models = m
        .settings
        .packs
        .iter()
        .any(|st| Some(st.pack) == pack_id && st.installed);
    route_step(
        ui,
        1,
        "Embedded text",
        (
            Tone::Neutral,
            "Always read first; pages with good text and no pictures stop here".into(),
        ),
    );
    ui.add_space(space::LG);
    let opts: Vec<String> = TOGGLES.iter().map(|t| t.to_string()).collect();
    let downloading = pack_id.is_some_and(|id| m.settings.model_busy.as_deref() == Some(id));
    route_step(
        ui,
        2,
        "Read scans on this computer",
        ocr_status(f.ocr, models, f.vision != 2, downloading),
    );
    // Local reading does not make a cloud setup private: say where the
    // text goes next.
    let ocr_help = if f.offline {
        "Text recognition (OCR) reads scanned pages on this computer.".to_string()
    } else {
        format!(
            "Text recognition (OCR) reads scanned pages on this computer. The text is still sent to {} to suggest the name.",
            PROVIDERS[f.provider.min(4)].1
        )
    };
    step_body(ui, |ui| {
        helper(ui, &ocr_help);
        ui.add_space(space::SM);
        segmented(
            ui,
            "Read scans on this computer",
            &opts,
            &mut m.settings.form.ocr,
        );
    });
    ui.add_space(space::LG);
    let provider = PROVIDERS[f.provider.min(4)].1;
    let model = if f.model.trim().is_empty() {
        autorename_core::config::default_model(f.provider_id())
            .unwrap_or("default")
            .to_string()
    } else {
        f.model.trim().to_string()
    };
    route_step(
        ui,
        3,
        "Read page images with AI",
        vision_status(f.vision, f.offline, provider, &model),
    );
    ui.add_space(space::SM);
    step_body(ui, |ui| {
        segmented(
            ui,
            "Read page images with AI",
            &opts,
            &mut m.settings.form.vision,
        );
    });
    let f = m.settings.form.clone();
    if scans_unreadable(f.ocr, models, f.vision) {
        ui.add_space(space::LG);
        banner(
            ui,
            Tone::Danger,
            "Scans can't be read.",
            "Download the scan reader below or turn on step 3.",
        );
    }
    ui.add_space(space::XL);
    field_label(ui, "Scan languages");
    let packs = autorename_core::ocr::PACKS;
    if let Some(line) = ocr_language_line(&f.ocr_language, packs) {
        // One pack: a plain line, every language in the tooltip.
        let w = ui.available_width().min(size::PROSE_MAX);
        let (r, resp) = ui.allocate_exact_size(theme::v2(w, typ::BODY.line), Sense::hover());
        let shown = ellipsize_end(ui, &line, typ::BODY, w);
        paint_text(ui, r.left(), r.center().y, &shown, typ::BODY, p.text);
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::Label, true, &line));
        resp.on_hover_text(ocr_language_details(&f.ocr_language));
    } else {
        let installed: Vec<&str> = m
            .settings
            .packs
            .iter()
            .filter(|st| st.installed)
            .map(|st| st.pack)
            .collect();
        let options = ocr_language_options(&f.ocr_language, packs, &installed);
        let names: Vec<String> = options.iter().map(|(_, l)| l.clone()).collect();
        let mut sel = ocr_language_index(&f.ocr_language, &options, packs);
        let before = sel;
        let (resp, changed) = Select {
            a11y: "Scan languages",
            width: ui.available_width(),
            options: &names,
            enabled: true,
            mono: false,
        }
        .show(ui, &mut sel);
        resp.on_hover_text(ocr_language_details(&f.ocr_language));
        if changed
            && sel != before
            && let Some((v, _)) = options.get(sel)
        {
            m.settings.form.ocr_language = v.clone();
        }
    }
    if pack_id.is_none() {
        status_line(ui, Tone::Warning, "No OCR model reads this language yet.");
    } else {
        helper(
            ui,
            "A scan reader reads all languages of its script at once, so mixed English and German pages work.",
        );
    }
    ui.add_space(space::XL);
    models_table(ui, m, acts);
}

/// Plain name of an OCR pack for the models list ("Scan reader · …").
pub fn pack_title(id: &str, description: &str) -> String {
    match id {
        "latin" => "Scan reader · English and other Latin-script languages".into(),
        _ => format!("Scan reader · {description}"),
    }
}

/// Tooltip of an OCR pack row: engine/version and every language.
pub fn pack_details(id: &str, description: &str) -> String {
    let langs = autorename_core::ocr::pack(id)
        .map(pack_languages)
        .unwrap_or_default();
    format!("{description}. Languages: {}.", langs.join(", "))
}

fn models_table(ui: &mut Ui, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let w = ui.available_width();
    // Leave room for the status badge and the row action (never overlap).
    let reserve = Badge::new(Tone::Accent, "Downloading…")
        .icon(theme::icon::LOADER_CIRCLE)
        .width(ui)
        + Button::secondary("Cancel download").width(ui)
        + space::SM * 3.0;
    let tw = (w - reserve).max(0.0);
    for st in m.settings.packs.clone() {
        let title = pack_title(st.pack, st.description);
        // The title may take two lines rather than lose its meaning.
        let lines = wrapped_lines(ui, &title, typ::BODY_STRONG, tw).min(2);
        let h = space::SM
            + typ::BODY_STRONG.line * lines as f32
            + space::XS
            + typ::CAPTION.line
            + space::SM;
        let (r, resp) = ui.allocate_exact_size(theme::v2(w, h.max(size::ROW)), Sense::hover());
        resp.on_hover_text(pack_details(st.pack, st.description));
        ui.painter().hline(
            r.x_range(),
            r.bottom() - stroke::HAIRLINE / 2.0,
            theme::hairline(p.border),
        );
        let top = r.center().y - (h - space::SM * 2.0) / 2.0;
        paint_wrapped(
            ui,
            egui::pos2(r.left(), top),
            tw,
            &title,
            typ::BODY_STRONG,
            p.text,
        );
        let l2 = top + typ::BODY_STRONG.line * lines as f32 + space::XS + typ::CAPTION.line / 2.0;
        let cap = format!("{} download", mb(st.total_size));
        paint_text(ui, r.left(), l2, &cap, typ::CAPTION, p.text_muted);
        let busy = m.settings.model_busy.as_deref() == Some(st.pack);
        let mut right = r.right();
        let btn = |ui: &mut Ui, b: Button<'_>, right: &mut f32| {
            let bw = b.width(ui);
            let br = Rect::from_min_size(
                egui::pos2(*right - bw, r.center().y - size::CONTROL / 2.0),
                theme::v2(bw, size::CONTROL),
            );
            *right = br.left() - space::SM;
            b.show_at(ui, br).clicked()
        };
        let any_busy = m.settings.model_busy.is_some();
        if busy {
            if btn(
                ui,
                Button::secondary("Cancel download")
                    .a11y(format!("Cancel download of {}", st.pack)),
                &mut right,
            ) {
                acts.push(Action::ModelCancel);
            }
        } else if st.installed {
            let a = format!("Remove {}", st.pack);
            if btn(
                ui,
                Button::danger("Remove")
                    .icon(theme::icon::TRASH_2)
                    .a11y(a)
                    .enabled(!any_busy),
                &mut right,
            ) {
                acts.push(Action::ModelRemove(st.pack.to_string()));
            }
        } else {
            let a = format!("Download {} ({})", st.pack, mb(st.total_size));
            if btn(
                ui,
                Button::secondary("Download")
                    .icon(theme::icon::DOWNLOAD)
                    .a11y(a)
                    .loading(busy)
                    .enabled((busy || !any_busy) && !m.downloads_blocked())
                    .tooltip(if m.downloads_blocked() {
                        crate::gui::app::DOWNLOAD_BLOCKED
                    } else {
                        "Downloads the hash-pinned model files"
                    }),
                &mut right,
            ) {
                acts.push(Action::ModelFetch(st.pack.to_string()));
            }
        }
        let (tone, t, icon) = if st.installed {
            (Tone::Success, "Installed", theme::icon::CIRCLE_CHECK)
        } else if busy {
            (Tone::Accent, "Downloading…", theme::icon::LOADER_CIRCLE)
        } else {
            (Tone::Neutral, "Not installed", theme::icon::DOWNLOAD)
        };
        let b = Badge::new(tone, t).icon(icon);
        let bw = b.width(ui);
        b.paint_at(
            ui,
            Rect::from_min_size(
                egui::pos2(right - bw - space::SM, r.center().y - size::BADGE / 2.0),
                theme::v2(bw, size::BADGE),
            ),
        );
        if busy {
            let (done, total) = m.settings.model_progress;
            ui.add_space(space::SM);
            let (pr, _) = ui.allocate_exact_size(theme::v2(w, typ::CAPTION.line), Sense::hover());
            let label = format!("Downloading {} of {}", mb(done), mb(total.max(1)));
            let bar = Rect::from_min_size(
                egui::pos2(pr.left(), pr.center().y - size::PROGRESS_H / 2.0),
                theme::v2(size::PROGRESS_RENAME_W, size::PROGRESS_H),
            );
            Progress {
                fraction: Some(done as f32 / total.max(1) as f32),
                label: &label,
            }
            .show_at(ui, bar);
            paint_text(
                ui,
                bar.right() + space::MD,
                pr.center().y,
                &label,
                typ::CAPTION,
                p.text_muted,
            );
            ui.add_space(space::SM);
        }
    }
    if let Some(msg) = &m.settings.model_msg {
        let (tone, t) = match msg {
            Ok(t) => (Tone::Success, t.as_str()),
            Err(t) if t == crate::gui::app::CANCELLED => (Tone::Neutral, t.as_str()),
            Err(t) => (Tone::Danger, t.as_str()),
        };
        status_line(ui, tone, t);
    }
    ui.add_space(space::LG);
    if Button::secondary("Import from folder…")
        .icon(theme::icon::FOLDER_OPEN)
        .enabled(m.settings.model_busy.is_none())
        .show(ui)
        .clicked()
    {
        acts.push(Action::ModelImport);
    }
    helper(
        ui,
        "For computers without internet: copy the models folder from another computer. Every file is checked.",
    );
}

pub fn mb(bytes: u64) -> String {
    let m = bytes as f64 / 1_000_000.0;
    if m >= 100.0 {
        format!("{m:.0} MB")
    } else {
        format!("{m:.1} MB")
    }
}

fn advanced(ui: &mut Ui, m: &mut Model) {
    let openness = collapsible(ui, "Advanced", &mut m.settings.advanced);
    let p = theme::palette(ui);
    ui.add_space(space::XS);
    para(
        ui,
        "Rarely needed. The defaults suit most documents.",
        p.text_muted,
    );
    let id = ui.scope_id().with("advanced");
    crate::gui::components::reveal(ui, id, openness, |ui| {
        ui.add_space(space::LG);
        let f = &mut m.settings.form;
        stepper(
            ui,
            "Minimum text readability",
            THRESHOLD_RANGE,
            2,
            &mut f.threshold,
        );
        helper(
            ui,
            "Pages scoring below this are read again. Pages with pictures or drawings start at 0.75, so a limit above that reads them with OCR. 1.00 means clean text with nothing in pictures.",
        );
        ui.add_space(space::LG);
        stepper(
            ui,
            "AI temperature",
            TEMPERATURE_RANGE,
            1,
            &mut f.temperature,
        );
        helper(ui, "Lower values make answers more consistent.");
        ui.add_space(space::LG);
        stepper_u32(
            ui,
            "Retries after provider errors",
            RETRIES_RANGE,
            &mut f.max_retries,
        );
        helper(ui, "For timeouts and busy servers.");
        ui.add_space(space::LG);
        stepper_u32(
            ui,
            "OCR threads",
            (1.0, cpu_count(), 1.0),
            &mut f.ocr_threads,
        );
        helper(
            ui,
            "More threads read scans faster and use more of the CPU.",
        );
    });
}

// ---------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------

fn naming(ui: &mut Ui, m: &mut Model, acts: &mut Vec<Action>) {
    page_title(ui, "Naming", "How the new file name is built.");
    let profiles = m
        .config
        .tree
        .as_ref()
        .and_then(|t| autorename_core::profiles::resolve_profiles(t).ok())
        .map(|ps| ps.into_iter().collect::<Vec<_>>())
        .unwrap_or_default();
    profiles_group(ui, m, &profiles, acts);
    let selected = profiles
        .iter()
        .find(|(id, _)| *id == m.settings.form.profile)
        .map(|(_, pr)| pr.clone());
    let uses = m
        .config
        .tree
        .as_ref()
        .and_then(|t| Uses::of(t, &m.settings.form.profile))
        .unwrap_or(Uses::ALL);
    if uses.direction {
        company_group(ui, m);
    }
    format_group(ui, m, uses);
    matching_group(ui, m, selected.as_ref(), acts);
    group(
        ui,
        "Extra instructions",
        "Added to the AI instructions for every file.",
    );
    let example = instructions_example(selected.as_ref());
    Field::new("Extra instructions", &mut m.settings.form.prompt_extension)
        .hint("Optional")
        .multiline(4)
        .helper(Some(example))
        .show(ui);
}

/// Which naming settings a profile actually uses (the Naming page shows
/// only those): invoice codes and your company (`direction`), the amount
/// format (`amount`), the document-type language (`{language}` in its
/// instructions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Uses {
    pub direction: bool,
    pub amount: bool,
    pub language: bool,
}

impl Uses {
    pub const ALL: Uses = Uses {
        direction: true,
        amount: true,
        language: true,
    };

    /// Uses of profile `id` in config `tree` (`None` = unknown profile).
    /// Profile text is interpolated at resolve time, so the language use is
    /// found by resolving with a marker as `output.language`.
    pub fn of(tree: &serde_json::Value, id: &str) -> Option<Uses> {
        const MARK: &str = "\u{1}language\u{1}";
        let mut t = tree.clone();
        if let Some(o) = t.get_mut("output").and_then(|o| o.as_object_mut()) {
            o.insert("language".into(), MARK.into());
        }
        let ps = autorename_core::profiles::resolve_profiles(&t).ok()?;
        let prof = ps.get(id)?;
        Some(Uses {
            direction: prof.direction.is_some(),
            amount: prof.amount.is_some(),
            language: prof.intro.contains(MARK)
                || prof.fields.values().any(|f| f.description.contains(MARK)),
        })
    }
}

/// Example for the extra-instructions helper, fitting the profile.
pub fn instructions_example(prof: Option<&Profile>) -> &'static str {
    match prof {
        Some(p) if p.direction.is_some() => "For example: Use the brand name, not the legal name.",
        Some(p) if p.fields.contains_key("title") => {
            "For example: Keep paper titles in their original language."
        }
        _ => "For example: Keep names short.",
    }
}

/// Humanized template fields: "Date, company, document type, description".
pub fn template_caption(prof: &Profile) -> String {
    let tokens = autorename_core::profiles::parse_placeholders(&prof.template).unwrap_or_default();
    let names: Vec<String> = tokens
        .iter()
        .filter_map(|t| match t {
            autorename_core::profiles::Token::Field(f) if f == "document_date" => {
                Some("date".to_string())
            }
            autorename_core::profiles::Token::Field(f) => {
                Some(noun(&crate::gui::model::field_title(f)))
            }
            autorename_core::profiles::Token::Literal(_) => None,
        })
        .collect();
    let mut s = names.join(", ");
    if let Some(c) = s.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    s
}

fn profiles_group(
    ui: &mut Ui,
    m: &mut Model,
    profiles: &[(String, Profile)],
    acts: &mut Vec<Action>,
) {
    let p = theme::palette(ui);
    group(
        ui,
        "Profile",
        "The kind of documents you rename. Each has its own name pattern.",
    );
    let w = ui.available_width();
    let form = m.settings.form.clone();
    for (id, prof) in profiles {
        let (r, _) = ui.allocate_exact_size(theme::v2(w, size::ROW_EDIT), Sense::hover());
        let rid = ui.scope_id().with(("profile_radio", id));
        let resp = ui.interact(r, rid, Sense::click());
        let selected = form.profile == *id;
        let h = anim(ui, rid.with("h"), resp.hovered());
        ui.painter().rect_filled(
            r,
            theme::cr(theme::radius::MD),
            mix(egui::Color32::TRANSPARENT, p.surface_hover, h),
        );
        ui.painter().hline(
            r.x_range(),
            r.bottom() - stroke::HAIRLINE / 2.0,
            theme::hairline(p.border),
        );
        // Line 1 centred at +16 so the 16 px radio sits on the grid.
        let l1 = r.top() + space::LG;
        let name = crate::gui::model::profile_label(id);
        let c = egui::pos2(r.left() + space::MD + size::CHECKBOX / 2.0, l1);
        let ring = if selected { p.accent } else { p.border_strong };
        ui.painter().circle(
            c,
            size::CHECKBOX / 2.0 - stroke::HAIRLINE / 2.0,
            p.surface,
            egui::Stroke::new(
                if selected {
                    stroke::FOCUS
                } else {
                    stroke::HAIRLINE
                },
                ring,
            ),
        );
        if selected {
            ui.painter().circle_filled(c, size::DOT / 2.0, p.accent);
        }
        focus_ring(ui, &resp, r, theme::radius::MD, true);
        let sample = sample_name(id, prof, &form);
        let a11y = format!("{name} profile, for example {sample}");
        resp.widget_info(|| WidgetInfo::selected(egui::Role::RadioButton, true, selected, &a11y));
        if resp.clicked() {
            m.settings.form.profile = id.clone();
        }
        let x = r.left() + space::MD + size::CHECKBOX + space::SM;
        let tr = paint_text(ui, x, l1, &name, typ::BODY_STRONG, p.text);
        if *id == m.settings.saved.profile {
            let b = Badge::new(Tone::Accent, "Default").icon(theme::icon::CHECK);
            let bw = b.width(ui);
            b.paint_at(
                ui,
                Rect::from_min_size(
                    egui::pos2(tr.right() + space::SM, l1 - size::BADGE / 2.0),
                    theme::v2(bw, size::BADGE),
                ),
            );
        }
        let tw = r.right() - space::MD - x;
        let l2 = l1 + typ::BODY.line / 2.0 + space::XS + typ::CAPTION.line / 2.0;
        let cap = ellipsize_end(ui, &template_caption(prof), typ::CAPTION, tw);
        paint_text(ui, x, l2, &cap, typ::CAPTION, p.text_muted);
        // Example name rendered by core's formatter with the form's formats.
        let l3 = l2 + typ::CAPTION.line / 2.0 + space::XS + typ::MONO_SM.line / 2.0;
        let t = ellipsize_middle(ui, &sample, typ::MONO_SM, tw);
        paint_text(ui, x, l3, &t, typ::MONO_SM, p.text);
        if t != sample {
            resp.on_hover_text(sample);
        }
    }
    if profiles.is_empty() {
        para(
            ui,
            "Profiles appear once config.yaml is valid.",
            p.text_muted,
        );
    }
    ui.add_space(space::LG);
    if Button::secondary("Open settings file")
        .icon(theme::icon::EXTERNAL_LINK)
        .salt("profiles")
        .show(ui)
        .clicked()
    {
        acts.push(Action::OpenConfigFile);
    }
    helper(
        ui,
        "Name patterns and fields are edited in the settings file.",
    );
}

/// Example filename for a profile: sample values (with the form's codes,
/// amount and date format) through the real naming code.
pub fn sample_name(_id: &str, prof: &Profile, f: &SettingsForm) -> String {
    let sample = |k: &str| -> String {
        match k {
            "document_date" => "03.11.2025".into(),
            "company_name" | "issuer" => "Northwind".into(),
            "recipient" => "Your company".into(),
            "document_type" => f.incoming.trim().to_string(),
            "description" => "Notes".into(),
            "total_amount" => "12,13".into(),
            "currency" => "EUR".into(),
            "first_author_surname" => "Vaswani".into(),
            "journal_name" => "NeurIPS".into(),
            "title" => "Attention Is All You Need".into(),
            other => crate::gui::model::field_title(other),
        }
    };
    let mut fields: serde_json::Map<String, serde_json::Value> = prof
        .fields
        .keys()
        .map(|k| (k.clone(), serde_json::Value::String(sample(k))))
        .collect();
    if let Some(d) = &prof.direction {
        fields.insert(d.field.clone(), f.incoming.trim().into());
    }
    if let Some(a) = &prof.amount {
        fields.insert(a.amount.clone(), "12,13".into());
        fields.insert(a.currency.clone(), "EUR".into());
        autorename_core::pipeline::derive_amount(
            a,
            &mut fields,
            &f.amount_format,
            &f.decimal_separator,
        );
    }
    autorename_core::naming::render_filename(prof, &fields, &f.date_format)
}

fn company_group(ui: &mut Ui, m: &mut Model) {
    let f = &mut m.settings.form;
    group(
        ui,
        "Your company",
        "Tells bills you receive from invoices you send.",
    );
    Field::new("Your company names", &mut f.company)
        .label("Your company names")
        .hint("e.g. ACME GmbH")
        .helper(Some(
            "One per line, as printed on invoices: company, trading or old names, your own name. Never used as the other party in a name.",
        ))
        .multiline(3)
        .width(theme::TEXT_EDIT_W)
        .show(ui);
    if f.company.trim().is_empty() {
        status_line(
            ui,
            Tone::Warning,
            "Add your company so the AI can tell bills you receive from invoices you send.",
        );
    }
}

fn format_group(ui: &mut Ui, m: &mut Model, uses: Uses) {
    let p = theme::palette(ui);
    let desc = match (uses.direction, uses.amount) {
        (true, true) => "Codes, dates and amounts in the name.",
        (true, false) => "Codes and dates in the name.",
        (false, true) => "Dates and amounts in the name.",
        (false, false) => "How dates appear in the name.",
    };
    group(ui, "Name format", desc);
    if uses.direction {
        codes(ui, m);
        ui.add_space(space::LG);
    }
    date_format(ui, m);
    if uses.amount {
        ui.add_space(space::LG);
        amount_format(ui, m);
    }
    if uses.language {
        ui.add_space(space::LG);
        type_language(ui, m, p);
    }
}

/// Codes for bills you receive / invoices you send, side by side.
fn codes(ui: &mut Ui, m: &mut Model) {
    // Codes side by side (two columns, gap 16).
    let col = ((ui.available_width() - space::LG) / 2.0)
        .min(theme::TEXT_EDIT_W)
        .floor();
    let col = (col / theme::GRID4).floor() * theme::GRID4;
    let f = &mut m.settings.form;
    let (e_in, e_out) = (
        f.problem(FormField::Incoming),
        f.problem(FormField::Outgoing),
    );
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = theme::v2(space::LG, 0.0);
        ui.vertical(|ui| {
            ui.set_width(col);
            ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
            Field::new("Code for bills you receive", &mut f.incoming)
                .label("Code for bills you receive")
                .mono()
                .error(e_in.as_deref())
                .helper(Some("e.g. AP, or ER in German"))
                .width(col)
                .show(ui);
        });
        ui.vertical(|ui| {
            ui.set_width(col);
            ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
            Field::new("Code for invoices you send", &mut f.outgoing)
                .label("Code for invoices you send")
                .mono()
                .error(e_out.as_deref())
                .helper(Some("e.g. AR"))
                .width(col)
                .show(ui);
        });
    });
}

/// Date format: presets shown as examples, plus Custom.
fn date_format(ui: &mut Ui, m: &mut Model) {
    let f = &mut m.settings.form;
    let preset = DATE_PRESETS.iter().position(|d| *d == f.date_format);
    let custom = m.settings.date_custom || preset.is_none();
    let mut opts: Vec<String> = DATE_PRESETS.iter().map(|d| date_example(d)).collect();
    opts.push("Custom…".into());
    let mut sel = if custom {
        DATE_PRESETS.len()
    } else {
        preset.unwrap_or(0)
    };
    field_label(ui, "Date");
    let (_, changed) = Select {
        a11y: "Date format",
        width: theme::TEXT_EDIT_W,
        options: &opts,
        enabled: true,
        mono: true,
    }
    .show(ui, &mut sel);
    if changed {
        m.settings.date_custom = sel == DATE_PRESETS.len();
        if let Some(d) = DATE_PRESETS.get(sel) {
            m.settings.form.date_format = (*d).to_string();
        }
    }
    let f = &mut m.settings.form;
    if m.settings.date_custom || DATE_PRESETS.iter().all(|d| *d != f.date_format) {
        ui.add_space(space::SM);
        let err = f.problem(FormField::DateFormat);
        let example = format!(
            "%Y year · %m month · %d day. Example: {}",
            date_example(&f.date_format)
        );
        Field::new("Custom date format", &mut f.date_format)
            .label("Custom date format")
            .mono()
            .hint("%Y%m%d")
            .error(err.as_deref())
            .helper(Some(&example))
            .width(theme::TEXT_EDIT_W)
            .show(ui);
        if err.is_some() {
            // The legend and live example stay under the error.
            helper(ui, &example);
        }
    }
}

/// Amount format presets (a hand-written format stays selectable).
fn amount_format(ui: &mut Ui, m: &mut Model) {
    let f = &mut m.settings.form;
    let mut fmts: Vec<String> = AMOUNT_PRESETS.iter().map(|s| s.to_string()).collect();
    if !fmts.contains(&f.amount_format) {
        fmts.push(f.amount_format.clone());
    }
    let opts: Vec<String> = fmts
        .iter()
        .map(|a| amount_example(a, &f.decimal_separator))
        .collect();
    let mut sel = fmts.iter().position(|a| *a == f.amount_format).unwrap_or(0);
    field_label(ui, "Amount");
    let (_, changed) = Select {
        a11y: "Amount format",
        width: theme::TEXT_EDIT_W,
        options: &opts,
        enabled: true,
        mono: true,
    }
    .show(ui, &mut sel);
    if changed && let Some(a) = fmts.get(sel) {
        f.amount_format = a.clone();
    }
    if let Some(e) = f.problem(FormField::AmountFormat) {
        status_line(ui, Tone::Danger, &e);
    }
    ui.add_space(space::LG);
    let seps = autorename_core::config::DECIMAL_SEPARATORS;
    let opts: Vec<String> = seps
        .iter()
        .map(|s| match *s {
            "," => "Comma: 1234,56".to_string(),
            _ => "Point: 1234.56".to_string(),
        })
        .collect();
    let mut sel = seps
        .iter()
        .position(|s| *s == f.decimal_separator)
        .unwrap_or(0);
    field_label(ui, "Decimal mark");
    let (_, changed) = Select {
        a11y: "Decimal mark",
        width: theme::TEXT_EDIT_W,
        options: &opts,
        enabled: true,
        mono: false,
    }
    .show(ui, &mut sel);
    if changed {
        f.decimal_separator = seps[sel].to_string();
    }
}

/// Language of document types (`output.language`).
fn type_language(ui: &mut Ui, m: &mut Model, p: &theme::Palette) {
    let f = &mut m.settings.form;
    let known = OUTPUT_LANGUAGES
        .iter()
        .position(|l| l.eq_ignore_ascii_case(f.out_language.trim()));
    let other = m.settings.language_other || known.is_none();
    let mut opts: Vec<String> = OUTPUT_LANGUAGES.iter().map(|s| s.to_string()).collect();
    opts.push("Other…".into());
    let mut sel = if other {
        OUTPUT_LANGUAGES.len()
    } else {
        known.unwrap_or(0)
    };
    field_label(ui, "Language for document type names");
    let (_, changed) = Select {
        a11y: "Language for document type names",
        width: theme::TEXT_EDIT_W,
        options: &opts,
        enabled: true,
        mono: false,
    }
    .show(ui, &mut sel);
    if changed {
        m.settings.language_other = sel == OUTPUT_LANGUAGES.len();
        if let Some(l) = OUTPUT_LANGUAGES.get(sel) {
            m.settings.form.out_language = (*l).to_string();
        }
    }
    let f = &mut m.settings.form;
    if m.settings.language_other || known.is_none() {
        ui.add_space(space::SM);
        let err = f.problem(FormField::OutLanguage);
        Field::new("Other language", &mut f.out_language)
            .hint("e.g. Greek")
            .error(err.as_deref())
            .width(theme::TEXT_EDIT_W)
            .show(ui);
    }
    ui.add_space(space::XS);
    caption(
        ui,
        "For documents that aren't invoices, e.g. Letter or Contract.",
        p.text_muted,
    );
}

/// Deciders offered: offline mode allows only local AI and Off (§4.7).
pub fn decider_choices(offline: bool) -> Vec<usize> {
    if offline {
        vec![0, 3]
    } else {
        vec![0, 1, 2, 3]
    }
}

/// One-line explanation of a decider (config id).
pub fn decider_explanation(id: &str, offline: bool) -> &'static str {
    match id {
        "ai" if offline => "The local model picks the right entry among close matches.",
        "ai" => "Your AI provider picks the right entry among close matches.",
        "openai-decisions" => "OpenAI Decisions picks; only the name and candidates are sent.",
        "jev" => "Jev picks; only the name and candidates are sent.",
        _ => "Close matches become new entries marked for review.",
    }
}

/// Status copy for the decider key (tone, text, details); `None` when no
/// key row (the decider needs no key).
pub fn decider_key_status(m: &Model) -> Option<(Tone, String, String)> {
    let f = &m.settings.form;
    let id = f.decider_id();
    if !matches!(id, "openai-decisions" | "jev") {
        return None;
    }
    let Some(state) = f.decider_key_state(&m.settings.saved, &m.config) else {
        return Some((
            Tone::Success,
            "Uses your OpenAI key".into(),
            "OpenAI Decisions uses the API key of your OpenAI provider.".into(),
        ));
    };
    let service = crate::gui::pages::decider_service(id);
    let owner = if id == "jev" { "Jev" } else { "OpenAI" };
    Some(key_status(&state, owner, service))
}

/// Path of the name list a field uses (demo: the loaded list's folder).
fn list_path(m: &Model, field: &str) -> std::path::PathBuf {
    if let Some(l) = &m.config.location {
        return autorename_core::config::name_list_path(&l.path, field);
    }
    let dir = m
        .companies
        .path
        .as_ref()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| autorename_core::config::NAME_LISTS_DIR_NAME.into());
    autorename_core::config::name_list_in(&dir, field)
}

fn matching_group(ui: &mut Ui, m: &mut Model, prof: Option<&Profile>, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let pname = crate::gui::model::profile_label(&m.settings.form.profile);
    let Some(field) = prof.and_then(|pr| pr.harmonize_field.clone()) else {
        group(
            ui,
            "Name matching",
            &format!("The {pname} profile doesn't match names."),
        );
        return;
    };
    let (one, many) = autorename_core::harmonize::field_label(&field);
    let (one_l, many_l) = (noun(&one), noun(&many));
    group(
        ui,
        &many,
        &format!("Spelling variants of the same {one_l} get one name."),
    );
    toggle(
        ui,
        &mut m.settings.form.learn,
        &format!("Learn new {many_l} automatically"),
    );
    helper(
        ui,
        &format!("New {many_l} and spellings are added to the list when you rename."),
    );
    ui.add_space(space::LG);
    decider(ui, m, &one_l);
    ui.add_space(space::LG);
    let path = list_path(m, &field);
    let shown = path.display().to_string();
    let title = format!("{many} list");
    field_label(ui, &title);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        let w = ui.available_width() - size::CONTROL - space::SM;
        let t = ellipsize_middle(ui, &shown, typ::MONO_SM, w);
        label(ui, &t, typ::MONO_SM, p.text_muted);
        let a11y = format!("Open {} list", many_l);
        if Button::icon_only(theme::icon::EXTERNAL_LINK, &a11y)
            .tooltip(shown.clone())
            .show(ui)
            .clicked()
        {
            // A list that doesn't exist yet opens its folder.
            let target = if path.exists() {
                path.clone()
            } else {
                path.parent()
                    .map_or(path.clone(), std::path::Path::to_path_buf)
            };
            acts.push(Action::OpenPath(target));
        }
    });
}

fn decider(ui: &mut Ui, m: &mut Model, one: &str) {
    let title = format!("Who checks similar {one} names?");
    field_label(ui, &title);
    let offline = m.settings.form.offline;
    let choices = decider_choices(offline);
    let names: Vec<String> = choices
        .iter()
        .map(|&i| {
            if offline && i == 0 {
                "AI on this computer".to_string()
            } else {
                DECIDERS[i].1.to_string()
            }
        })
        .collect();
    let mut sel = choices
        .iter()
        .position(|&i| i == m.settings.form.decider)
        .unwrap_or(0);
    let (_, changed) = Select {
        a11y: &title,
        width: theme::TEXT_EDIT_W,
        options: &names,
        enabled: m.settings.form.learn,
        mono: false,
    }
    .show(ui, &mut sel);
    if changed && m.settings.form.decider != choices[sel] {
        m.settings.form.decider = choices[sel];
        // A key typed for another service is never saved for this one.
        m.settings.form.decider_key.clear();
        m.settings.form.show_decider_key = false;
    }
    if offline && matches!(m.settings.form.decider, 1 | 2) {
        status_line(
            ui,
            Tone::Danger,
            "Not available offline: it is a cloud service.",
        );
        return;
    }
    helper(
        ui,
        decider_explanation(m.settings.form.decider_id(), offline),
    );
    let Some((tone, text, details)) = decider_key_status(m) else {
        return;
    };
    if m.settings.form.decider_key_var().is_some() {
        ui.add_space(space::LG);
        let label = format!(
            "{} key",
            crate::gui::pages::decider_service(m.settings.form.decider_id())
        );
        let f = &mut m.settings.form;
        let shown = &mut f.show_decider_key;
        Field::new(&label, &mut f.decider_key)
            .label(&label)
            .mono()
            .secret(shown)
            .hint("Paste your key")
            .width(theme::TEXT_EDIT_W)
            .show(ui);
    }
    status_line(ui, tone, &text).on_hover_text(details);
}

// ---------------------------------------------------------------------------
// About
// ---------------------------------------------------------------------------

pub const SUPPORT_LINKS: [(&str, &str); 4] = [
    ("GitHub Sponsors", "https://github.com/sponsors/realspqrk"),
    ("Ko-fi", "https://ko-fi.com/spqrk"),
    ("PayPal", "https://paypal.me/realSPQRK"),
    ("PhraseVault", "https://phrasevault.app"),
];

pub const LICENSE_LINE: &str = "MIT — provided as is, without warranty of any kind. No liability for renamed or modified files; keep backups.";
pub const SUPPORT_COPY: &str = "Enjoying AutoRename-PDF? It may save you hours. Even in the age of AI, building and maintaining it takes a lot of time and resources. Please consider a donation — or try PhraseVault, our text expander. You may enjoy it too.";
pub const THIRD_PARTY: &str = "Open-source components used in this app: PDFium (BSD-3-Clause) · ONNX Runtime (MIT) · PaddleOCR models (Apache-2.0) · Inter and JetBrains Mono (OFL 1.1) · Lucide icons (ISC) · egui (MIT or Apache-2.0)";
pub const IMPRINT: &str = "SPQRK IT-Services · Gerhard Petermeir, BA MSc · Haag 8, 8344 Bad Gleichenberg, Austria · office@spqrk.net · VAT ATU69739438";
pub const LICENSE_TEXT: &str = include_str!("../../../../../LICENSE");
/// Generated by `cargo xtask notices` (repo root).
pub const NOTICES_TEXT: &str = include_str!("../../../../../THIRD_PARTY_NOTICES.md");

fn about(ui: &mut Ui, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    page_title(
        ui,
        "About",
        &format!("AutoRename-PDF {}", env!("CARGO_PKG_VERSION")),
    );
    ui.add_space(space::XL);
    para(ui, LICENSE_LINE, p.text);
    ui.add_space(space::LG);
    if Button::secondary("View license")
        .icon(theme::icon::FILE_TEXT)
        .show(ui)
        .clicked()
    {
        acts.push(Action::ShowDoc(TextDoc::License));
    }
    ui.add_space(space::XL);
    // Support card (elev.1, padding 24).
    let w = ui.available_width();
    let inner_w = w - space::XL * 2.0;
    let lines = wrapped_lines(ui, SUPPORT_COPY, typ::BODY, inner_w.min(size::PROSE_MAX));
    let h = space::XL
        + typ::TITLE_SM.line
        + space::SM
        + typ::BODY.line * lines as f32
        + space::LG
        + size::CONTROL
        + space::XL;
    let (card, _) = ui.allocate_exact_size(theme::v2(w, h), Sense::hover());
    crate::gui::components::paint_card(ui, card, p);
    let inner = card.shrink(space::XL);
    in_rect(ui, inner, |ui| {
        ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
        label(ui, "Support AutoRename-PDF", typ::TITLE_SM, p.text);
        ui.add_space(space::SM);
        let tw = inner.width().min(size::PROSE_MAX);
        let (r, _) =
            ui.allocate_exact_size(theme::v2(tw, typ::BODY.line * lines as f32), Sense::hover());
        paint_wrapped(ui, r.min, tw, SUPPORT_COPY, typ::BODY, p.text_muted);
        ui.add_space(space::LG);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = theme::v2(space::SM, space::SM);
            for (i, (name, url)) in SUPPORT_LINKS.iter().enumerate() {
                let b = if i == 0 {
                    Button::primary(name)
                } else {
                    Button::secondary(name)
                }
                .icon(theme::icon::EXTERNAL_LINK)
                .tooltip(*url);
                if b.show(ui).clicked() {
                    acts.push(Action::OpenUrl(url.to_string()));
                }
            }
        });
    });
    ui.add_space(space::XL);
    caption(ui, THIRD_PARTY, p.text_muted);
    ui.add_space(space::LG);
    if Button::secondary("Third-party notices")
        .icon(theme::icon::FILE_TEXT)
        .show(ui)
        .clicked()
    {
        acts.push(Action::ShowDoc(TextDoc::ThirdPartyNotices));
    }
    ui.add_space(space::XL);
    caption(ui, IMPRINT, p.text_muted);
}

/// License or third-party notices in a 560 dialog, Esc / Close dismisses.
/// The text is wrapped once per width (cached) and drawn with `show_rows`,
/// so the 10k-line notices stay cheap.
pub fn doc_dialog(ctx: &egui::Context, doc: TextDoc, open: &mut bool) {
    let (title, text) = match doc {
        TextDoc::License => ("License", LICENSE_TEXT),
        TextDoc::ThirdPartyNotices => ("Third-party notices", NOTICES_TEXT),
    };
    let p = theme::pal(ctx.global_style().visuals.dark_mode);
    let frame = egui::Frame::new()
        .fill(p.elevated(3))
        .stroke(theme::hairline(p.border))
        .corner_radius(theme::cr(theme::radius::XL))
        .inner_margin(theme::margin_all(space::XL))
        .shadow(p.shadow(3));
    // Text area: the window height minus margins and the dialog chrome.
    let chrome = space::XL * 2.0 + typ::TITLE.line + space::LG + space::XL + size::CONTROL;
    let max_h = (ctx.content_rect().height() - space::XXL * 2.0 - chrome).max(size::DROP_H);
    let modal = egui::Modal::new(egui::Id::unique(("doc_dialog", doc)))
        .frame(frame)
        .backdrop_color(p.scrim)
        .show(ctx, |ui| {
            let w = size::DIALOG_MD - space::XL * 2.0;
            ui.set_width(w);
            ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
            label(ui, title, typ::TITLE, p.text);
            ui.add_space(space::LG);
            // Wrap clear of the scroll bar.
            let lines = doc_lines(ui, doc, text, w - space::LG);
            let row_h = typ::MONO_SM.line;
            let (r, resp) = ui.allocate_exact_size(
                theme::v2(w, max_h.min(row_h * lines.len() as f32)),
                Sense::hover(),
            );
            resp.widget_info(|| WidgetInfo::labeled(egui::Role::Document, true, title));
            in_rect(ui, r, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("doc_scroll", doc))
                    .auto_shrink([false, false])
                    .show_rows(ui, row_h, lines.len(), |ui, range| {
                        for (l, heading) in &lines[range] {
                            let (lr, _) =
                                ui.allocate_exact_size(theme::v2(w, row_h), Sense::hover());
                            let t = if *heading {
                                typ::CAPTION_STRONG
                            } else {
                                typ::MONO_SM
                            };
                            paint_text(ui, lr.left(), lr.center().y, l, t, p.text);
                        }
                    });
            });
            ui.add_space(space::XL);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if Button::secondary("Close").show(ui).clicked() {
                    *open = false;
                }
            });
        });
    if modal.should_close() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        *open = false;
    }
}

/// Display rows `(text, heading)` of `text` wrapped to `width` in mono_sm,
/// cached in egui memory per document and column count.
type DocRows = std::sync::Arc<Vec<(String, bool)>>;

fn doc_lines(ui: &Ui, doc: TextDoc, text: &str, width: f32) -> DocRows {
    let adv = crate::gui::components::text_width(ui, "M", typ::MONO_SM).max(1.0);
    let cols = ((width / adv).floor() as usize).max(1);
    let id = egui::Id::unique(("doc_lines", doc, cols));
    if let Some(v) = ui.data(|d| d.get_temp::<DocRows>(id)) {
        return v;
    }
    let v = std::sync::Arc::new(doc_rows(text, cols));
    ui.data_mut(|d| d.insert_temp(id, v.clone()));
    v
}

/// Display rows of a Markdown-ish document: the generator note and code
/// fences are dropped, `#` headings lose their hashes and are marked
/// (drawn strong); everything else is soft-wrapped at `cols`.
pub fn doc_rows(text: &str, cols: usize) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("Generated by `cargo xtask notices`") {
            continue;
        }
        let heading = t.starts_with('#');
        let body = if heading {
            t.trim_start_matches('#').trim_start()
        } else {
            line
        };
        if body.trim().is_empty() {
            out.push((String::new(), false));
            continue;
        }
        out.extend(wrap_cols(body, cols).into_iter().map(|l| (l, heading)));
    }
    out
}

/// Soft-wrap every line at `cols` characters.
pub fn wrap_cols(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest: Vec<char> = line.trim_end().chars().collect();
        while rest.len() > cols {
            let cut = rest[..=cols]
                .iter()
                .rposition(|c| *c == ' ')
                .filter(|&i| i > 0)
                .unwrap_or(cols);
            out.push(
                rest[..cut]
                    .iter()
                    .collect::<String>()
                    .trim_end()
                    .to_string(),
            );
            let skip = if rest.get(cut) == Some(&' ') {
                cut + 1
            } else {
                cut
            };
            rest.drain(..skip);
        }
        out.push(rest.into_iter().collect());
    }
    out
}

fn save_footer(ui: &mut Ui, rect: Rect, m: &Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    ui.painter().rect_filled(rect, 0, p.surface);
    ui.painter().hline(
        rect.x_range(),
        rect.top() + stroke::HAIRLINE / 2.0,
        theme::hairline(p.border),
    );
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + space::XL, rect.top()),
        egui::pos2(rect.right() - space::LG, rect.bottom()),
    );
    let blocker = m.settings.blocker(&m.config);
    let (msg, color) = match (&m.settings.error, &blocker) {
        (Some(e), _) => (e.clone(), p.danger),
        (None, Some(b)) => (b.clone(), p.danger),
        (None, None) => ("Unsaved changes".into(), p.text_muted),
    };
    // The message yields to the buttons (never under them).
    let btns = Button::primary("Save changes").width(ui) + Button::secondary("Discard").width(ui);
    let mw = inner.width() - btns - space::SM - space::LG;
    let t = ellipsize_end(ui, &msg, typ::BODY, mw.max(0.0));
    let r = paint_text(ui, inner.left(), inner.center().y, &t, typ::BODY, color);
    if t != msg {
        ui.interact(r, ui.scope_id().with("footer_msg"), Sense::hover())
            .on_hover_text(msg);
    }
    in_rect(ui, inner, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let mut b = Button::primary("Save changes").enabled(blocker.is_none() && !m.busy());
            if let Some(v) = &blocker {
                b = b.tooltip(v.clone());
            } else if m.busy() {
                b = b.tooltip("Wait for the current run to finish or stop it, then save");
            }
            if b.show(ui).clicked() {
                acts.push(Action::SaveSettings);
            }
            if Button::secondary("Discard").show(ui).clicked() {
                acts.push(Action::DiscardSettings);
            }
        });
    });
}
