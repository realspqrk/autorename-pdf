//! §4.2 Empty / onboarding: drop zone (+ "Set up AI" card on first run).
//! While the card is shown and the full drop zone does not fit, the drop
//! zone turns compact; the page scrolls only when even that does not fit.

use egui::{Rect, Sense, Ui};

use super::Action;
use crate::gui::components::banner::tone_icon;
use crate::gui::components::drop_zone::{DropState, drop_zone};
use crate::gui::components::{
    Badge, Button, Field, Progress, Select, ellipsize_end, galley, in_rect, label, paint_card,
    paint_galley_v, paint_icon, paint_text, paint_wrapped, segmented, wrapped_lines,
};
use crate::gui::model::{Model, PROVIDERS};
use crate::gui::screens::settings::mb;
use crate::gui::theme::{self, Tone, size, space, typ};

/// Card height from the §4.2 rhythm (padding 24, rows 32, gaps 16).
fn card_h(offline: bool, company: bool) -> f32 {
    let head = space::XL + typ::TITLE_SM.line + space::XS + typ::BODY.line + space::LG;
    let seg = size::CONTROL + space::LG;
    // Captions: offline = server helper + model status under the first row
    // (4 + 16 + 16); cloud = model helper under the first row and the key
    // helper with "Get a key" under the key row (2 × (4 + 16)).
    let caption = if offline {
        space::XS + typ::CAPTION.line * 2.0
    } else {
        (space::XS + typ::CAPTION.line) * 2.0
    };
    let body = size::CONTROL + caption + space::LG + size::CONTROL;
    // Company: stacked label, field, helper.
    let company = if company {
        space::LG
            + typ::BODY_STRONG.line
            + space::XS
            + size::CONTROL
            + space::XS
            + typ::CAPTION.line
    } else {
        0.0
    };
    head + seg + body + company + space::XL + size::CONTROL + space::XL
}

/// Configured drop zone (§4.2): width = φ share of the content clamped to
/// [`CARD_W`, `DROP_MAX_W`], height = width × `DROP_ASPECT` (at least
/// `DROP_MIN_H`, at most the free height), on the 8-pt grid.
fn drop_size(rect: Rect, free_h: f32) -> egui::Vec2 {
    let grid8 = |v: f32| (v / theme::GRID8).floor() * theme::GRID8;
    let w = grid8(rect.width() * size::DROP_W_SHARE)
        .clamp(size::CARD_W, size::DROP_MAX_W)
        .min(rect.width() - space::XL * 2.0);
    let h = grid8(w * size::DROP_ASPECT)
        .min(grid8(free_h))
        .max(size::DROP_MIN_H);
    theme::v2(w, h)
}

pub fn empty(ui: &mut Ui, rect: Rect, m: &mut Model, drop: DropState, acts: &mut Vec<Action>) {
    let show_card = !m.is_set_up();
    let hint = ocr_hint_pack(m, show_card);
    if !show_card {
        // Drop zone + hint as one group, centered in the content area.
        let probe_w = drop_size(rect, f32::INFINITY).x;
        let hint_h = if hint.is_some() {
            space::XL + ocr_hint_h(ui, probe_w, &ocr_hint_body(m))
        } else {
            0.0
        };
        let dz = drop_size(rect, rect.height() - space::XL * 2.0 - hint_h);
        let group = dz.y + hint_h;
        let x = theme::snap4(rect.center().x - dz.x / 2.0);
        let top = theme::snap4(rect.top() + ((rect.height() - group) / 2.0).max(space::XL));
        let r = Rect::from_min_size(egui::pos2(x, top), dz);
        let (_, out) = drop_zone(ui, r, drop);
        push_drop(out, acts);
        if let Some(pack) = hint {
            let hr = Rect::from_min_size(
                egui::pos2(x, r.bottom() + space::XL),
                theme::v2(dz.x, hint_h - space::XL),
            );
            ocr_hint(ui, hr, m, pack, acts);
        }
        return;
    }
    let w = size::CARD_W.min(rect.width() - space::XL * 2.0);
    let hint_h = if hint.is_some() {
        space::XL + ocr_hint_h(ui, w, &ocr_hint_body(m))
    } else {
        0.0
    };
    let ch = card_h(m.setup.offline, asks_company(m)) + hint_h;
    // Full drop zone (240–280) while everything fits; otherwise the compact
    // one-row form so the card and its actions stay in view (§4.2).
    let free = rect.height() - space::XL * 3.0 - ch;
    let drop_h = if free >= size::DROP_MIN_H {
        (free.min(size::DROP_H) / theme::GRID4).floor() * theme::GRID4
    } else {
        size::DROP_COMPACT_H
    };
    let content_h = space::XL + drop_h + space::XL + ch + space::XL;
    in_rect(ui, rect, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("onboarding")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (area, _) = ui.allocate_exact_size(
                    theme::v2(ui.available_width(), content_h.max(rect.height())),
                    Sense::hover(),
                );
                let x = theme::snap4(area.center().x - w / 2.0);
                let top = theme::snap4(
                    area.top() + ((area.height() - content_h) / 2.0).max(0.0) + space::XL,
                );
                let dr = Rect::from_min_size(egui::pos2(x, top), theme::v2(w, drop_h));
                let (_, out) = drop_zone(ui, dr, drop);
                push_drop(out, acts);
                let cr = Rect::from_min_size(
                    egui::pos2(x, dr.bottom() + space::XL),
                    theme::v2(w, ch - hint_h),
                );
                setup_card(ui, cr, m, acts);
                if let Some(pack) = hint {
                    let hr = Rect::from_min_size(
                        egui::pos2(x, cr.bottom() + space::XL),
                        theme::v2(w, hint_h - space::XL),
                    );
                    ocr_hint(ui, hr, m, pack, acts);
                }
            });
    });
}

const OCR_HINT_TITLE: &str = "Read scans on this computer (optional)";

/// Recommendation body for the configured route: scans are read here
/// first, but the text still goes to the provider (cloud); with vision off
/// scans can't be read at all. Before setup: the provider picked on the card.
pub fn ocr_hint_body(m: &Model) -> String {
    use autorename_core::config::Toggle;
    let provider = match m.config.settings.as_ref() {
        Some(s) if s.pdf.vision == Toggle::Off => {
            return "Scans can't be read with your current settings. Download the scan reader or enable AI image reading in Processing settings.".into();
        }
        Some(s) => crate::gui::model::provider_label(s.ai.provider.id()),
        None => PROVIDERS[m.setup.provider].1,
    };
    format!(
        "Reads scans on this computer first. The text is still sent to {provider} to suggest the name."
    )
}

/// The OCR pack to recommend (§4.2): not installed, OCR not switched off,
/// downloads allowed, and not already offered by the offline setup card.
fn ocr_hint_pack(m: &Model, card: bool) -> Option<&'static autorename_core::ocr::Pack> {
    if !autorename_core::ocr::ENGINE_AVAILABLE || m.downloads_blocked() || (card && m.setup.offline)
    {
        return None;
    }
    let s = m.config.settings.as_ref();
    if s.is_some_and(|s| s.pdf.ocr == autorename_core::config::Toggle::Off) {
        return None;
    }
    let lang = s.map_or(autorename_core::ocr::AUTO_LANGUAGE, |s| {
        s.ocr.language.as_str()
    });
    let pack = autorename_core::ocr::pack_for_language(lang)?;
    let installed = m
        .settings
        .packs
        .iter()
        .any(|st| st.pack == pack.id && st.installed);
    (!installed).then_some(pack)
}

/// Width left for the hint text beside its button.
fn ocr_hint_text_w(ui: &Ui, w: f32) -> f32 {
    let b = Button::secondary("Download scan reader (00.0 MB)").icon(theme::icon::DOWNLOAD);
    (w - space::LG * 2.0 - size::ICON - space::SM - b.width(ui) - space::LG).max(0.0)
}

/// Recommendation height: padding 12, title, 4, wrapped body, padding 12.
fn ocr_hint_h(ui: &Ui, w: f32, body: &str) -> f32 {
    let lines = wrapped_lines(ui, body, typ::CAPTION, ocr_hint_text_w(ui, w));
    let h = space::MD * 2.0 + typ::BODY_STRONG.line + space::XS + typ::CAPTION.line * lines as f32;
    (h / theme::GRID4).ceil() * theme::GRID4
}

/// One optional recommendation: download the scan reader (never automatic).
fn ocr_hint(
    ui: &mut Ui,
    rect: Rect,
    m: &Model,
    pack: &'static autorename_core::ocr::Pack,
    acts: &mut Vec<Action>,
) {
    let p = theme::palette(ui);
    let route = ocr_hint_body(m);
    ui.interact(rect, ui.scope_id().with("ocr_hint"), Sense::hover())
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Note, true, format!("{OCR_HINT_TITLE}. {route}"))
        });
    ui.painter().rect(
        rect,
        theme::cr(theme::radius::LG),
        p.surface,
        theme::hairline(p.border),
        egui::StrokeKind::Inside,
    );
    let x = rect.left() + space::LG;
    let top = rect.top() + space::MD;
    paint_icon(
        ui,
        egui::pos2(x + size::ICON / 2.0, top + typ::BODY_STRONG.line / 2.0),
        theme::icon::INFO,
        size::ICON,
        p.text_muted,
    );
    let tx = x + size::ICON + space::SM;
    paint_text(
        ui,
        tx,
        top + typ::BODY_STRONG.line / 2.0,
        OCR_HINT_TITLE,
        typ::BODY_STRONG,
        p.text,
    );
    let busy = m.settings.model_busy.as_deref() == Some(pack.id);
    let body = if busy {
        let (done, total) = m.settings.model_progress;
        format!(
            "Downloading the scan reader, {} of {}.",
            mb(done),
            mb(total.max(1))
        )
    } else {
        route
    };
    paint_wrapped(
        ui,
        egui::pos2(tx, top + typ::BODY_STRONG.line + space::XS),
        ocr_hint_text_w(ui, rect.width()),
        &body,
        typ::CAPTION,
        p.text_muted,
    );
    let download = format!("Download scan reader ({})", mb(pack.total_size()));
    let b = if busy {
        Button::secondary("Cancel download")
    } else {
        Button::secondary(&download)
            .icon(theme::icon::DOWNLOAD)
            .enabled(m.settings.model_busy.is_none())
            .tooltip("Downloads the hash-pinned model files once; nothing is sent.")
    };
    let bw = b.width(ui);
    let br = Rect::from_min_size(
        egui::pos2(
            rect.right() - space::LG - bw,
            rect.center().y - size::CONTROL / 2.0,
        ),
        theme::v2(bw, size::CONTROL),
    );
    if b.salt("ocr_hint").show_at(ui, br).clicked() {
        acts.push(if busy {
            Action::ModelCancel
        } else {
            Action::ModelFetch(pack.id.to_string())
        });
    }
}

fn push_drop(out: crate::gui::components::drop_zone::DropResponse, acts: &mut Vec<Action>) {
    if out.choose_files {
        acts.push(Action::ChooseFiles);
    }
    if out.choose_folder {
        acts.push(Action::ChooseFolder);
    }
}

fn row<R>(ui: &mut Ui, text: &str, f: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.set_height(size::CONTROL);
        let (r, _) = ui.allocate_exact_size(
            theme::v2(size::SETUP_LABEL_COL, size::CONTROL),
            egui::Sense::hover(),
        );
        paint_galley_v(
            ui,
            r.left(),
            r.center().y,
            galley(ui, text, typ::BODY_STRONG),
            p.text,
        );
        f(ui)
    })
    .inner
}

fn status(ui: &mut Ui, tone: Tone, icon: char, text: &str) {
    let p = theme::palette(ui);
    let c = if tone == Tone::Neutral {
        p.text_muted
    } else {
        p.tone(tone).1
    };
    let (r, _) = ui.allocate_exact_size(
        theme::v2(ui.available_width(), size::CONTROL),
        egui::Sense::hover(),
    );
    paint_icon(
        ui,
        egui::pos2(r.left() + size::ICON_SM / 2.0, r.center().y),
        icon,
        size::ICON_SM,
        c,
    );
    let t = ellipsize_end(
        ui,
        text,
        typ::CAPTION,
        r.width() - size::ICON_SM - space::XS,
    );
    paint_text(
        ui,
        r.left() + size::ICON_SM + space::XS,
        r.center().y,
        &t,
        typ::CAPTION,
        c,
    );
}

fn setup_card(ui: &mut Ui, rect: Rect, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    paint_card(ui, rect, p);
    let inner = rect.shrink(space::XL);
    in_rect(ui, inner, |ui| {
        ui.spacing_mut().item_spacing = theme::v2(space::SM, 0.0);
        label(ui, "Set up AI", typ::TITLE_SM, p.text);
        ui.add_space(space::XS);
        label(
            ui,
            "Where should document text be processed?",
            typ::BODY,
            p.text_muted,
        );
        ui.add_space(space::LG);
        let opts = [
            "Cloud provider".to_string(),
            "Offline on this computer".to_string(),
        ];
        let mut sel = usize::from(m.setup.offline);
        segmented(ui, "Processing location", &opts, &mut sel);
        if (sel == 1) != m.setup.offline {
            m.setup.offline = sel == 1;
            m.setup_test = None;
        }
        ui.add_space(space::LG);
        if m.setup.offline {
            row(ui, "Ollama server", |ui| {
                Field::new("Ollama server", &mut m.setup.ollama_url)
                    .mono()
                    .width(theme::TEXT_EDIT_W)
                    .show(ui);
                ui.add_space(space::SM);
                let (tone, text) = match &m.setup_test {
                    Some(Ok(ms)) => (Tone::Success, format!("Connected · {ms} ms")),
                    Some(Err(_)) => (Tone::Danger, "Not reachable".into()),
                    None => (Tone::Neutral, "Not tested yet".into()),
                };
                status(ui, tone, tone_icon(tone), &text);
            });
            // Model status: the model the test actually asked (no invented
            // availability list; Ollama's model list is not queried).
            ui.add_space(space::XS);
            let model = autorename_core::config::default_model("ollama").unwrap_or("gemma4");
            let (c, text) = match &m.setup_test {
                Some(Ok(ms)) => (
                    p.success,
                    format!("Model {model} answered the test request in {ms} ms."),
                ),
                Some(Err(e)) => (p.danger, format!("Model {model}: {e}")),
                None => (
                    p.text_muted,
                    format!("Uses model {model}. Test connection sends it a one-line request."),
                ),
            };
            caption_line(ui, OLLAMA_HELPER, p.text_muted);
            caption_line(ui, &text, c);
            ui.add_space(space::LG);
            row(ui, "Scan reader", |ui| ocr_row(ui, m, acts));
        } else {
            row(ui, "Provider", |ui| {
                let names: Vec<String> = PROVIDERS[..4].iter().map(|p| p.1.to_string()).collect();
                let before = m.setup.provider;
                Select {
                    a11y: "Provider",
                    width: size::COMBO_W + space::XL,
                    options: &names,
                    enabled: true,
                    mono: false,
                }
                .show(ui, &mut m.setup.provider);
                if before != m.setup.provider {
                    m.setup_test = None;
                    m.setup.model =
                        autorename_core::config::default_model(PROVIDERS[m.setup.provider].0)
                            .unwrap_or_default()
                            .to_string();
                }
                ui.add_space(space::LG);
                let mw = (galley(ui, "Model", typ::BODY_STRONG).size().x / theme::GRID4).ceil()
                    * theme::GRID4;
                let (lr, _) =
                    ui.allocate_exact_size(theme::v2(mw, size::CONTROL), egui::Sense::hover());
                paint_galley_v(
                    ui,
                    lr.left(),
                    lr.center().y,
                    galley(ui, "Model", typ::BODY_STRONG),
                    p.text,
                );
                Field::new("Model", &mut m.setup.model)
                    .mono()
                    .width(ui.available_width())
                    .show(ui);
            });
            ui.add_space(space::XS);
            caption_line(ui, MODEL_HELPER, p.text_muted);
            ui.add_space(space::LG);
            let (pid, pname) = PROVIDERS[m.setup.provider];
            let var = crate::gui::model::key_var(pid);
            let found = m.setup_blocker().is_none();
            row(ui, "API key", |ui| {
                let shown = &mut m.setup.show_key;
                Field::new("API key", &mut m.setup.api_key)
                    .mono()
                    .secret(shown)
                    .hint(if found {
                        "Using environment variable"
                    } else {
                        "Paste your key"
                    })
                    .width(theme::TEXT_EDIT_W)
                    .show(ui);
                ui.add_space(space::SM);
                let (tone, icon, text) = if !m.setup.api_key.is_empty() {
                    (
                        Tone::Neutral,
                        theme::icon::KEY_ROUND,
                        "Will be saved to .env".to_string(),
                    )
                } else if found {
                    (
                        Tone::Success,
                        theme::icon::CIRCLE_CHECK,
                        format!("Found in {var}"),
                    )
                } else {
                    (
                        Tone::Danger,
                        theme::icon::CIRCLE_X,
                        format!("Add your {pname} API key"),
                    )
                };
                status(ui, tone, icon, &text);
            });
            ui.add_space(space::XS);
            key_helper(ui, pname, crate::gui::model::key_url(pid), acts);
        }
        // Your company (§4.2): optional, only for profiles with `direction`;
        // same copy as Settings › Naming.
        if asks_company(m) {
            ui.add_space(space::LG);
            // Stacked (the label is longer than the label column); neutral
            // helper, no warning here (Settings keeps its status).
            Field::new("Your company (optional)", &mut m.setup.company)
                .label("Your company (optional)")
                .hint("e.g. Northwind Traders Ltd")
                .helper(Some(COMPANY_HELPER))
                .width(theme::TEXT_EDIT_W)
                .show(ui);
        }
    });
    // Footer: test feedback left, [Test connection] [Save and continue] right.
    let bar = Rect::from_min_max(
        egui::pos2(inner.left(), inner.bottom() - size::CONTROL),
        inner.max,
    );
    let mut left_edge = bar.right();
    // Same key rule as Settings' save blocker; no network test required.
    let blocker = m.setup_blocker();
    in_rect(ui, bar, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let mut save = Button::primary("Save and continue").enabled(blocker.is_none());
            let mut test = Button::secondary("Test connection")
                .enabled(blocker.is_none())
                .loading(m.setup_testing);
            if let Some(b) = &blocker {
                save = save.tooltip(b.clone());
                test = test.tooltip(b.clone());
            }
            if save.show(ui).clicked() {
                acts.push(Action::SaveSetup);
            }
            let test = test.show(ui);
            if test.clicked() {
                acts.push(Action::TestSetup);
            }
            left_edge = test.rect.left();
        });
    });
    let fb = Rect::from_min_max(bar.min, egui::pos2(left_edge - space::MD, bar.bottom()));
    let feedback = if let Some(e) = &m.setup.error {
        Some((Tone::Danger, tone_icon(Tone::Danger), e.clone()))
    } else if let Some(b) = blocker {
        // Why the primary is disabled (the key row says what is missing).
        Some((Tone::Neutral, theme::icon::INFO, b))
    } else if m.setup.offline {
        None
    } else {
        match &m.setup_test {
            Some(Ok(ms)) => Some((
                Tone::Success,
                tone_icon(Tone::Success),
                format!("Connected · {ms} ms"),
            )),
            Some(Err(e)) => Some((Tone::Danger, tone_icon(Tone::Danger), e.clone())),
            // The order of the two steps, until the first test.
            None => Some((
                Tone::Neutral,
                theme::icon::INFO,
                "Paste your key, then click Test connection.".to_string(),
            )),
        }
    };
    if let Some((tone, icon, text)) = feedback {
        in_rect(ui, fb, |ui| {
            status(ui, tone, icon, &text);
        });
    }
}

/// Key helper under the API key field (aligned with the fields): who pays,
/// whose account, and the provider's key page.
fn key_helper(ui: &mut Ui, provider: &str, url: Option<&str>, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let (cr, _) = ui.allocate_exact_size(
        theme::v2(ui.available_width(), typ::CAPTION.line),
        Sense::hover(),
    );
    let x = cr.left() + size::SETUP_LABEL_COL + space::SM;
    let text = format!("Use a key from your company's account. {provider} charges per use.");
    let link_w = url.map_or(0.0, |_| {
        crate::gui::components::text_width(ui, KEY_LINK, typ::CAPTION) + space::SM
    });
    let t = ellipsize_end(ui, &text, typ::CAPTION, cr.right() - x - link_w);
    let tr = paint_text(ui, x, cr.center().y, &t, typ::CAPTION, p.text_muted);
    if let Some(url) = url {
        let lx = theme::snap4(tr.right() + space::SM);
        if crate::gui::components::link_at(ui, lx, cr.center().y, KEY_LINK, typ::CAPTION, url)
            .clicked()
        {
            acts.push(Action::OpenUrl(url.to_string()));
        }
    }
}

const KEY_LINK: &str = "Get a key";

const COMPANY_HELPER: &str =
    "Helps tell bills you receive from invoices you send. You can add it later in Settings.";

/// The setup card asks for your company when the active profile (the
/// starter's `business` before a config exists) uses `direction`, the same
/// rule as Settings › Naming.
fn asks_company(m: &Model) -> bool {
    use crate::gui::screens::settings::Uses;
    static STARTER: std::sync::OnceLock<Option<serde_json::Value>> = std::sync::OnceLock::new();
    let tree = match &m.config.tree {
        Some(t) => Some(t),
        None => STARTER
            .get_or_init(|| autorename_core::config::parse_yaml(crate::cmd::STARTER_CONFIG).ok())
            .as_ref(),
    };
    let Some(tree) = tree else { return true };
    let id = m
        .config
        .settings
        .as_ref()
        .map(|s| s.profile_id.clone())
        .or_else(|| {
            tree.get("profile")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "business".into());
    Uses::of(tree, &id).unwrap_or(Uses::ALL).direction
}

const OLLAMA_HELPER: &str =
    "Where Ollama runs. Usually this computer; change it only if Ollama runs elsewhere.";
const MODEL_HELPER: &str =
    "The recommended model is preselected. Change it only if you need another.";

/// One caption line aligned with the setup fields (plain rect: a
/// `horizontal` row would take the 32 px minimum).
fn caption_line(ui: &mut Ui, text: &str, color: egui::Color32) {
    let (cr, _) = ui.allocate_exact_size(
        theme::v2(ui.available_width(), typ::CAPTION.line),
        Sense::hover(),
    );
    let x = cr.left() + size::SETUP_LABEL_COL + space::SM;
    let t = ellipsize_end(ui, text, typ::CAPTION, cr.right() - x);
    paint_text(ui, x, cr.center().y, &t, typ::CAPTION, color);
}

fn ocr_row(ui: &mut Ui, m: &mut Model, acts: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let lang = m
        .config
        .settings
        .as_ref()
        .map_or(autorename_core::ocr::AUTO_LANGUAGE.to_string(), |s| {
            s.ocr.language.clone()
        });
    let Some(pack) = autorename_core::ocr::pack_for_language(&lang) else {
        label(
            ui,
            "No scan reader for this language",
            typ::CAPTION,
            p.text_muted,
        );
        return;
    };
    let installed = m
        .settings
        .packs
        .iter()
        .any(|s| s.pack == pack.id && s.installed);
    if installed {
        Badge::new(Tone::Success, "Installed")
            .icon(theme::icon::CIRCLE_CHECK)
            .show(ui);
        return;
    }
    let busy = m.settings.model_busy.as_deref() == Some(pack.id);
    if busy {
        let (done, total) = m.settings.model_progress;
        let (r, _) = ui.allocate_exact_size(
            theme::v2(ui.available_width(), size::CONTROL),
            Sense::hover(),
        );
        let bar = Rect::from_min_size(
            egui::pos2(r.left(), r.center().y - size::PROGRESS_H / 2.0),
            theme::v2(size::PROGRESS_RENAME_W, size::PROGRESS_H),
        );
        let label = format!("{} of {}", mb(done), mb(total.max(1)));
        Progress {
            fraction: Some(done as f32 / total.max(1) as f32),
            label: &format!("Downloading the scan reader, {label}"),
        }
        .show_at(ui, bar);
        paint_text(
            ui,
            bar.right() + space::MD,
            r.center().y,
            &label,
            typ::CAPTION,
            p.text_muted,
        );
        let b = Button::secondary("Cancel download");
        let bw = b.width(ui);
        let br = Rect::from_min_size(
            egui::pos2(r.right() - bw, r.top()),
            theme::v2(bw, size::CONTROL),
        );
        if b.show_at(ui, br).clicked() {
            acts.push(Action::ModelCancel);
        }
        return;
    }
    Badge::new(Tone::Neutral, "Not installed").show(ui);
    let t = format!("Download scan reader ({})", mb(pack.total_size()));
    if Button::secondary(&t)
        .icon(theme::icon::DOWNLOAD)
        .enabled(m.settings.model_busy.is_none() && !m.downloads_blocked())
        .tooltip(if m.downloads_blocked() {
            crate::gui::app::DOWNLOAD_BLOCKED
        } else {
            "Downloads the hash-pinned model files"
        })
        .show(ui)
        .clicked()
    {
        acts.push(Action::ModelFetch(pack.id.to_string()));
    }
}
