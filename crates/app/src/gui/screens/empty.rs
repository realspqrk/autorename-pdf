//! §4.2 Empty / onboarding: drop zone (+ "Set up AI" card on first run).
//! The page scrolls when the card does not fit (960 × 640); fields never shrink.

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
fn card_h(offline: bool) -> f32 {
    let head = space::XL + typ::TITLE_SM.line + space::XS + typ::BODY.line + space::LG;
    let seg = size::CONTROL + space::LG;
    // Offline adds the model caption (4 + 16) under the Ollama row.
    let caption = if offline {
        space::XS + typ::CAPTION.line
    } else {
        0.0
    };
    let body = size::CONTROL + caption + space::LG + size::CONTROL;
    head + seg + body + space::XL + size::CONTROL + space::XL
}

pub fn empty(ui: &mut Ui, rect: Rect, m: &mut Model, drop: DropState, acts: &mut Vec<Action>) {
    let show_card = !m.is_set_up();
    let w = size::CARD_W.min(rect.width() - space::XL * 2.0);
    let hint = ocr_hint_pack(m, show_card);
    let hint_h = if hint.is_some() {
        space::XL + ocr_hint_h(ui, w, &ocr_hint_body(m))
    } else {
        0.0
    };
    if !show_card {
        let x = theme::snap4(rect.center().x - w / 2.0);
        let h = (rect.height() - space::XL * 2.0 - hint_h).max(size::DROP_MIN_H);
        let r = Rect::from_min_size(egui::pos2(x, rect.top() + space::XL), theme::v2(w, h));
        let (_, out) = drop_zone(ui, r, drop);
        push_drop(out, acts);
        if let Some(pack) = hint {
            let hr = Rect::from_min_size(
                egui::pos2(x, r.bottom() + space::XL),
                theme::v2(w, hint_h - space::XL),
            );
            ocr_hint(ui, hr, m, pack, acts);
        }
        return;
    }
    let ch = card_h(m.setup.offline) + hint_h;
    let total = space::XL + size::DROP_H + space::XL + ch + space::XL;
    let drop_h = if rect.height() >= total {
        size::DROP_H
    } else {
        // Shrink the drop zone to its minimum first, then scroll.
        (rect.height() - (total - size::DROP_H)).clamp(size::DROP_MIN_H, size::DROP_H)
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

/// Recommendation body for the configured route: where pages that need
/// image reading go today (the provider, or nowhere with vision off);
/// neutral before setup.
pub fn ocr_hint_body(m: &Model) -> String {
    use autorename_core::config::Toggle;
    let Some(s) = m.config.settings.as_ref() else {
        return "Reads scanned text on this computer before using AI image reading.".into();
    };
    if s.pdf.vision == Toggle::Off {
        return "Scans can't be read with your current settings. Download the scan reader or enable AI image reading in Processing settings.".into();
    }
    let provider = PROVIDERS
        .iter()
        .find(|p| p.0 == s.ai.provider.id())
        .map_or("your AI provider", |p| p.1);
    format!(
        "Pages that need image reading are sent to {provider}. Download the scan reader to read scans on this computer first."
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
            theme::v2(size::LABEL_COL, size::CONTROL),
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
            row(ui, "Ollama", |ui| {
                Field::new("Ollama address", &mut m.setup.ollama_url)
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
            // Plain rect (a `horizontal` row would take the 32 px minimum).
            let (cr, _) = ui.allocate_exact_size(
                theme::v2(ui.available_width(), typ::CAPTION.line),
                Sense::hover(),
            );
            let x = cr.left() + size::LABEL_COL + space::SM;
            let t = ellipsize_end(ui, &text, typ::CAPTION, cr.right() - x);
            paint_text(ui, x, cr.center().y, &t, typ::CAPTION, c);
            ui.add_space(space::LG);
            row(ui, "OCR", |ui| ocr_row(ui, m, acts));
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
            ui.add_space(space::LG);
            let var = PROVIDERS[m.setup.provider].2;
            let found = std::env::var(var).is_ok() || m.config.env.contains_key(var);
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
                    (Tone::Danger, theme::icon::CIRCLE_X, "Missing".to_string())
                };
                status(ui, tone, icon, &text);
            });
        }
    });
    // Footer: test feedback left, [Test connection] [Save and continue] right.
    let bar = Rect::from_min_max(
        egui::pos2(inner.left(), inner.bottom() - size::CONTROL),
        inner.max,
    );
    let mut left_edge = bar.right();
    in_rect(ui, bar, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let save = Button::primary("Save and continue").show(ui);
            if save.clicked() {
                acts.push(Action::SaveSetup);
            }
            let test = Button::secondary("Test connection")
                .loading(m.setup_testing)
                .show(ui);
            if test.clicked() {
                acts.push(Action::TestSetup);
            }
            left_edge = test.rect.left();
        });
    });
    let fb = Rect::from_min_max(bar.min, egui::pos2(left_edge - space::MD, bar.bottom()));
    let feedback = if let Some(e) = &m.setup.error {
        Some((Tone::Danger, e.clone()))
    } else if m.setup.offline {
        None
    } else {
        match &m.setup_test {
            Some(Ok(ms)) => Some((Tone::Success, format!("Connected · {ms} ms"))),
            Some(Err(e)) => Some((Tone::Danger, e.clone())),
            None => None,
        }
    };
    if let Some((tone, text)) = feedback {
        in_rect(ui, fb, |ui| {
            status(ui, tone, tone_icon(tone), &text);
        });
    }
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
            "No OCR pack for this language",
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
            label: &format!("Downloading OCR models, {label}"),
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
    let t = format!("Download OCR models ({})", mb(pack.total_size()));
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
