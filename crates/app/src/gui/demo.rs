//! Canned plan for screenshots and tests (`AUTORENAME_GUI_DEMO=1`). No AI
//! calls, no file changes: rename/undo are simulated.
#![cfg_attr(not(test), allow(dead_code))]

use std::path::PathBuf;
use std::time::Instant;

use autorename_core::extract::Tier;
use autorename_core::pipeline::{
    ApplyReport, ApplySummary, CompanyMatch, DecidedBy, EnrichPlan, ErrorInfo, FileReport,
    LearnAction, LearnStatus, MatchCandidate, MatchOutcome, PlanItem, PlanStatus, RenameStatus,
    SCHEMA_VERSION,
};
use serde_json::{Map, Value, json};

use super::model::{Analysis, BannerMsg, ConfigState, Model, Phase, PlanMeta, Row, Tab};
use super::pages::{BatchView, CompaniesState, HistoryState, SettingsForm, page};
use super::theme::Tone;
use autorename_core::harmonize::{Entry, NameDb, Source};

pub const DEMO_DIR: &str = "/home/demo/Scans/Inbox";

struct Spec {
    old: &'static str,
    new: Option<&'static str>,
    tier: Option<Tier>,
    quality: f64,
    enrich: Option<&'static str>,
    learn: Option<LearnAction>,
    warnings: &'static [&'static str],
    error: Option<(&'static str, &'static str)>,
    fields: Value,
}

fn alias(c: &str, a: &str) -> Option<LearnAction> {
    Some(LearnAction::AddAlias {
        canonical: c.into(),
        alias: a.into(),
    })
}

fn new_entry(n: &str, review: bool) -> Option<LearnAction> {
    Some(LearnAction::NewEntry {
        name: n.into(),
        review,
    })
}

fn f(date: &str, company: &str, ty: &str, desc: &str) -> Value {
    json!({"document_date": date, "company_name": company, "document_type": ty, "description": desc})
}

fn specs() -> Vec<Spec> {
    vec![
        Spec {
            old: "scan_0041.pdf",
            new: Some("20251103 Amazon AP 12,13.pdf"),
            tier: Some(Tier::Text),
            quality: 0.94,
            enrich: None,
            learn: None,
            warnings: &[],
            error: None,
            fields: f("03.11.2025", "Amazon", "AP", "12,13"),
        },
        Spec {
            old: "Rechnung (2).pdf",
            new: Some("20251028 Telekom Deutschland AP 54,99.pdf"),
            tier: Some(Tier::Ocr),
            quality: 0.62,
            enrich: None,
            learn: new_entry("Telekom Deutschland", true),
            warnings: &["low confidence on date: two dates found (28.10.25, 01.11.25)"],
            error: None,
            fields: f("28.10.25", "TELEKOM DEUTSCHLAND GMBH", "AP", "54,99"),
        },
        Spec {
            old: "broken.pdf",
            new: None,
            tier: None,
            quality: 0.0,
            enrich: Some("failed"),
            learn: None,
            warnings: &[],
            error: Some((
                "provider_error",
                "request timed out after 60 s (retried 2×)",
            )),
            fields: json!({}),
        },
        Spec {
            old: "IMG_2231.pdf",
            new: Some("20251015 Bauhaus AP 89,90.pdf"),
            tier: Some(Tier::VisionNeeded),
            quality: 0.12,
            enrich: None,
            learn: None,
            warnings: &["low text quality, read from page images"],
            error: None,
            fields: f("15.10.2025", "Bauhaus", "AP", "89,90"),
        },
        Spec {
            old: "Vertrag_signed.pdf",
            new: Some("20250901 Stadtwerke München Contract.pdf"),
            tier: Some(Tier::Text),
            quality: 0.97,
            enrich: Some("signed"),
            learn: None,
            warnings: &[],
            error: None,
            fields: f("01.09.2025", "Stadtwerke München", "Contract", ""),
        },
        Spec {
            old: "invoice-7781.pdf",
            new: Some("20251020 Hetzner Online AP 38,56.pdf"),
            tier: Some(Tier::Text),
            quality: 0.91,
            enrich: None,
            learn: alias("Hetzner Online", "HETZNER ONLINE GMBH"),
            warnings: &[],
            error: None,
            fields: f("20.10.2025", "HETZNER ONLINE GMBH", "AP", "38,56"),
        },
        Spec {
            old: "doc00123.pdf",
            new: Some("20251029 A1 Telekom Austria AP 45,00.pdf"),
            tier: Some(Tier::Ocr),
            quality: 0.71,
            enrich: None,
            learn: None,
            warnings: &[],
            error: None,
            fields: f("29.10.2025", "A1 Telekom Austria", "AP", "45,00"),
        },
        Spec {
            old: "doc00124.pdf",
            new: Some("20251030 Würth Delivery note.pdf"),
            tier: Some(Tier::Text),
            quality: 0.88,
            enrich: None,
            learn: None,
            warnings: &[],
            error: None,
            fields: f("30.10.2025", "Würth", "Delivery note", ""),
        },
        Spec {
            old: "AR-2025-114.pdf",
            new: Some("20251102 Müller Bau AR 1.234,56.pdf"),
            tier: Some(Tier::Text),
            quality: 0.99,
            enrich: None,
            learn: None,
            warnings: &[],
            error: None,
            fields: f("02.11.2025", "Müller Bau", "AR", "1.234,56"),
        },
        Spec {
            old: "statement_oct.pdf",
            new: Some("20251031 Erste Bank Statement.pdf"),
            tier: Some(Tier::Ocr),
            quality: 0.58,
            enrich: None,
            learn: None,
            warnings: &[],
            error: None,
            fields: f("31.10.2025", "Erste Bank", "Statement", ""),
        },
        Spec {
            old: "scan_0042.pdf",
            new: Some("20251104 IKEA AP 249,00.pdf"),
            tier: Some(Tier::Text),
            quality: 0.93,
            enrich: None,
            learn: new_entry("IKEA", false),
            warnings: &[],
            error: None,
            fields: f("04.11.2025", "IKEA", "AP", "249,00"),
        },
        Spec {
            old: "Kontoauszug_11.pdf",
            new: Some("20251105 Raiffeisen Statement.pdf"),
            tier: Some(Tier::Text),
            quality: 0.90,
            enrich: None,
            learn: None,
            warnings: &[],
            error: None,
            fields: f("05.11.2025", "Raiffeisen", "Statement", ""),
        },
    ]
}

const DEMO_TEXT: &str = "Telekom Deutschland GmbH · Landgrabenweg 151 · 53227 Bonn\nRechnung Nr. 4711 0815 2025\nRechnungsdatum 28.10.25    Fällig am 01.11.25\nKundennummer 12 345 678 9\nLeistungszeitraum 01.10.2025 – 31.10.2025\nMagentaZuhause L                         44,95\nMagentaTV Smart                           10,04\nGesamtbetrag inkl. 19 % USt.             54,99 EUR\nZahlbar per Lastschrift. Vielen Dank für Ihren Auftrag.\nSEPA-Mandatsreferenz 98765-43210";

fn cand(name: &str, score: f32) -> MatchCandidate {
    MatchCandidate {
        name: name.into(),
        score,
    }
}

/// Plausible match evidence per learn action (mirrors core's bands).
fn evidence(
    fields: &Map<String, Value>,
    company: &str,
    learn: &Option<LearnAction>,
) -> CompanyMatch {
    let raw = fields
        .get("company_name")
        .and_then(Value::as_str)
        .unwrap_or(company)
        .to_string();
    let (outcome, score, band, candidates) = match learn {
        Some(LearnAction::AddAlias { canonical, .. }) => (
            MatchOutcome::Alias,
            0.95,
            Some(">=0.92"),
            vec![cand(canonical, 0.95), cand("Hetzner Cloud", 0.71)],
        ),
        Some(LearnAction::NewEntry { review: true, .. }) => (
            MatchOutcome::Review,
            0.83,
            Some("0.75-0.92"),
            vec![cand("Telekom Austria", 0.83), cand("Deutsche Post", 0.41)],
        ),
        Some(LearnAction::NewEntry { .. }) => (
            MatchOutcome::New,
            0.38,
            Some("<0.75"),
            vec![cand("Bauhaus", 0.38)],
        ),
        None => (MatchOutcome::Known, 1.0, None, vec![cand(company, 1.0)]),
    };
    CompanyMatch {
        extracted: raw.clone(),
        normalized: raw.to_lowercase(),
        outcome,
        score: Some(score),
        band: band.map(str::to_string),
        candidates,
        decided_by: DecidedBy::None,
    }
}

/// Source text matching each row's fields (detail extraction well).
fn page_text(old: &str, fields: &Map<String, Value>, quality: f64) -> Vec<String> {
    if old == "Rechnung (2).pdf" {
        return vec![DEMO_TEXT.to_string(), "Seite 2 von 2".into()];
    }
    let g = |k: &str| fields.get(k).and_then(Value::as_str).unwrap_or("");
    let (company, date, ty, desc) = (
        g("company_name"),
        g("document_date"),
        g("document_type"),
        g("description"),
    );
    let mut lines = vec![company.to_string()];
    match ty {
        "AP" | "AR" => {
            lines.push(format!("Invoice / Rechnung · Date {date}"));
            lines.push(format!("Invoice no. {}", 40_000 + old.len() * 137));
            lines.push(format!("{:<24}{}", "Item", "Amount"));
            lines.push(format!("{:<24}{desc}", "Goods and services"));
            lines.push(format!("{:<24}{desc} EUR", "Total incl. VAT"));
        }
        other => {
            lines.push(format!("{other} · {date}"));
            lines.push("Reference and parties as printed on page 1.".into());
        }
    }
    if quality < 0.3 {
        lines.push("(text read from page images)".into());
    }
    vec![lines.join("\n")]
}

pub fn items() -> Vec<PlanItem> {
    specs()
        .into_iter()
        .map(|s| {
            let source = PathBuf::from(DEMO_DIR).join(s.old);
            let failed = s.error.is_some();
            let fields: Map<String, Value> = s.fields.as_object().cloned().unwrap_or_default();
            let company =
                fields
                    .get("company_name")
                    .and_then(Value::as_str)
                    .map(|c| match &s.learn {
                        Some(LearnAction::AddAlias { canonical, .. }) => canonical.clone(),
                        Some(LearnAction::NewEntry { name, .. }) => name.clone(),
                        None => c.to_string(),
                    });
            let company_match = company.as_ref().map(|c| evidence(&fields, c, &s.learn));
            let pages = if failed {
                Vec::new()
            } else {
                page_text(s.old, &fields, s.quality)
            };
            PlanItem {
                company_match,
                source: source.clone(),
                source_sha256: "0".repeat(64),
                status: if failed {
                    PlanStatus::Failed
                } else {
                    PlanStatus::Rename
                },
                proposed_name: s.new.map(str::to_string),
                target: s.new.map(|n| source.with_file_name(n)),
                fields,
                company,
                date: s
                    .new
                    .map(|n| format!("{}-{}-{}", &n[0..4], &n[4..6], &n[6..8])),
                tier: s.tier,
                quality: (!failed).then_some(s.quality),
                page_quality: Vec::new(),
                model: (!failed).then(|| "gpt-6-luna".to_string()),
                warnings: s.warnings.iter().map(|w| w.to_string()).collect(),
                enrich: match s.enrich {
                    None => EnrichPlan::Will,
                    Some(r) => EnrichPlan::Skip { reason: r.into() },
                },
                learn: s.learn,
                error: s.error.map(|(c, m)| ErrorInfo {
                    code: c.into(),
                    message: m.into(),
                    hint: Some("Retry, or check the provider status.".into()),
                    retryable: true,
                }),
                pages,
                partial: false,
            }
        })
        .collect()
}

fn meta() -> PlanMeta {
    PlanMeta {
        profile: "business".into(),
        provider: "openai".into(),
        model: "gpt-6-luna".into(),
        privacy: "cloud".into(),
    }
}

/// Demo config state: a valid cloud setup without touching disk.
pub fn config() -> ConfigState {
    let tree = autorename_core::config::defaults();
    let mut tree = tree;
    if let Some(ai) = tree.get_mut("ai").and_then(Value::as_object_mut) {
        ai.insert("api_key".into(), json!("sk-demo"));
        ai.insert("model".into(), json!("gpt-6-luna"));
    }
    let settings = autorename_core::config::settings(&tree, None).ok();
    ConfigState {
        location: None,
        profiles: vec!["business".into(), "academic".into()],
        settings,
        error: None,
        tree: Some(tree),
        // Key referenced as ${OPENAI_API_KEY} and stored in .env (policy).
        env: [("OPENAI_API_KEY".to_string(), "sk-demo".to_string())].into(),
        review_count: 3,
        raw_key: Some("${OPENAI_API_KEY}".into()),
        raw_decider_key: None,
    }
}

pub fn unconfigured() -> ConfigState {
    ConfigState {
        location: None,
        profiles: vec!["business".into(), "academic".into()],
        settings: None,
        error: Some("No config file yet".into()),
        tree: None,
        env: Default::default(),
        review_count: 0,
        raw_key: None,
        raw_decider_key: None,
    }
}

/// First run: nothing set up, nothing dropped.
/// Hermetic paths for screenshots (no XDG lookups).
fn demo_paths(m: &mut Model) {
    m.journal_display = "/home/demo/.local/share/autorename-pdf/journal.json".into();
}

pub fn empty_model() -> Model {
    let mut m = Model::new(unconfigured());
    m.demo = true;
    fill_pages(&mut m);
    m
}

/// Offline branch of the setup card with the OCR download action.
pub fn offline_setup_model() -> Model {
    let mut m = empty_model();
    m.setup.offline = true;
    m.setup_test = Some(Ok(38));
    m
}

/// Set up, nothing dropped yet: the drop zone fills the content area.
pub fn configured_empty_model() -> Model {
    let mut m = Model::new(config());
    m.demo = true;
    fill_pages(&mut m);
    m
}

fn entry(
    canonical: &str,
    aliases: &[&str],
    learned: &[&str],
    auto: bool,
    added: Option<&str>,
    review: bool,
) -> Entry {
    Entry {
        canonical: canonical.into(),
        aliases: aliases.iter().map(|a| a.to_string()).collect(),
        learned: learned.iter().map(|a| a.to_string()).collect(),
        source: if auto { Source::Auto } else { Source::User },
        added: added.map(str::to_string),
        review,
        evidence: None,
    }
}

pub fn companies_db() -> NameDb {
    NameDb {
        field: autorename_core::harmonize::COMPANY_FIELD.into(),
        entries: vec![
            entry(
                "Amazon",
                &["AMAZON EU S.A R.L.", "Amazon.de"],
                &[],
                false,
                None,
                false,
            ),
            entry(
                "Hetzner Online",
                &[],
                &["HETZNER ONLINE GMBH"],
                true,
                Some("2025-10-20"),
                false,
            ),
            entry(
                "Telekom Deutschland",
                &[],
                &["TELEKOM DEUTSCHLAND GMBH"],
                true,
                Some("2025-10-28"),
                true,
            ),
            entry(
                "Telekom Austria",
                &["A1 Telekom Austria"],
                &[],
                false,
                None,
                false,
            ),
            entry(
                "Müller Bau",
                &[],
                &["Mueller Bau GmbH", "MÜLLER BAU"],
                true,
                Some("2025-11-02"),
                true,
            ),
            entry(
                "Würth",
                &[],
                &["Adolf Würth GmbH & Co. KG"],
                true,
                Some("2025-10-30"),
                false,
            ),
            entry("Stadtwerke München", &["SWM"], &[], false, None, false),
            entry("Erste Bank", &[], &[], true, Some("2025-10-31"), true),
        ],
    }
}

pub fn history() -> HistoryState {
    let b =
        |id: &str, ts: &str, files: usize, folder: &str, source: &str, undone: bool| BatchView {
            id: id.into(),
            timestamp: ts.into(),
            files,
            names: 0,
            folder: folder.into(),
            source: source.into(),
            undone,
        };
    HistoryState {
        batches: vec![
            b(
                "20251107T093000Z-demo",
                "2025-11-07T09:30:00Z",
                10,
                DEMO_DIR,
                "gui",
                false,
            ),
            b(
                "20251104T171205Z-a1",
                "2025-11-04T17:12:05Z",
                3,
                "/home/demo/Scans/Receipts",
                "gui",
                false,
            ),
            b(
                "20251031T080144Z-b2",
                "2025-10-31T08:01:44Z",
                24,
                "/home/demo/Documents/Invoices 2025",
                "cli",
                true,
            ),
            b(
                "20251020T121530Z-c3",
                "2025-10-20T12:15:30Z",
                1,
                "/home/demo/Downloads",
                "gui",
                false,
            ),
        ],
        loaded: true,
        error: None,
    }
}

/// Settings/Companies/History data without disk access.
fn fill_pages(m: &mut Model) {
    demo_paths(m);
    m.companies = CompaniesState {
        db: Some(companies_db()),
        path: Some(PathBuf::from(
            "/home/demo/.config/autorename-pdf/name-lists/company_name.yaml",
        )),
        ..Default::default()
    };
    m.history = history();
    m.settings.packs =
        autorename_core::ocr::models::list(std::path::Path::new("/nonexistent/autorename-demo"));
    if let Some(s) = &m.config.settings {
        let f = SettingsForm::from_settings(s);
        m.settings.form = f.clone();
        m.settings.saved = f;
        m.settings.loaded = true;
    }
}

/// Settings on a page (`pages::page::*`). Processing shows a passed
/// connection test and a model download in progress.
pub fn settings_model(section: usize) -> Model {
    let mut m = configured_empty_model();
    m.tab = Tab::Settings;
    m.settings.section = section;
    if section == page::PROCESSING {
        m.settings.test = Some(Ok(412));
        if let Some(p) = m.settings.packs.first() {
            m.settings.model_busy = Some(p.pack.to_string());
            m.settings.model_progress = (12_400_000, p.total_size);
        }
    }
    m
}

/// Processing with no models and AI vision off: "Scans can't be read".
pub fn settings_ocr_unreadable_model() -> Model {
    let mut m = settings_model(page::PROCESSING);
    m.settings.model_busy = None;
    m.settings.form.vision = 2;
    m
}

/// Processing with the Advanced group expanded.
pub fn settings_advanced_model() -> Model {
    let mut m = settings_model(page::PROCESSING);
    m.settings.advanced = true;
    m
}

/// Naming with Jev chosen and no key yet (key row + status).
pub fn settings_decider_jev_model() -> Model {
    let mut m = settings_model(page::NAMING);
    m.settings.form.decider = 2;
    m
}

/// Naming in offline mode: only AI / Off are offered.
pub fn settings_decider_offline_model() -> Model {
    let mut m = settings_model(page::NAMING);
    m.settings.form.offline = true;
    m.settings.form.provider = 4;
    m.settings.saved = m.settings.form.clone();
    m
}

/// Naming with the academic profile as default: no name matching.
pub fn settings_naming_academic_model() -> Model {
    let mut m = settings_model(page::NAMING);
    m.settings.form.profile = "academic".into();
    m.settings.saved = m.settings.form.clone();
    m
}

/// Naming with a custom date format and invalid codes (field helpers,
/// Save disabled).
pub fn settings_naming_invalid_model() -> Model {
    let mut m = settings_model(page::NAMING);
    m.settings.form.date_format = "%d/%m/%Y".into();
    m.settings.form.outgoing = "AP".into();
    m
}

/// Settings with unsaved changes (sticky footer) and an offline violation.
pub fn settings_dirty_model() -> Model {
    let mut m = settings_model(page::PROCESSING);
    m.settings.test = None;
    m.settings.model_busy = None;
    m.settings.form.offline = true;
    m
}

pub fn companies_model() -> Model {
    let mut m = configured_empty_model();
    m.tab = Tab::Companies;
    m.companies.active = Some(2);
    m
}

pub fn merge_model() -> Model {
    let mut m = companies_model();
    m.companies.merge = Some(crate::gui::pages::MergeDialog {
        from: "Telekom Deutschland".into(),
        search: String::new(),
        target: Some("Telekom Austria".into()),
    });
    m
}

pub fn history_model() -> Model {
    let mut m = configured_empty_model();
    m.tab = Tab::History;
    m
}

/// Renaming in progress: action bar progress + Stop.
pub fn renaming_model() -> Model {
    let mut m = preview_model();
    m.phase = Phase::Renaming { done: 6, total: 10 };
    m
}

/// Partial result: 2 failed, 1 not reached after Stop.
pub fn partial_model() -> Model {
    let mut m = preview_model();
    let mut rep = simulate_apply(&m);
    for (k, f) in rep.files.iter_mut().enumerate() {
        if k == 2 || k == 5 {
            f.rename_status = RenameStatus::Failed;
            f.error = Some(ErrorInfo {
                code: "io_error".into(),
                message: "The file is open in another program.".into(),
                hint: Some("Close it and retry.".into()),
                retryable: true,
            });
        }
        if k == 9 {
            f.rename_status = RenameStatus::Cancelled;
            f.enrich_status = "not_run".into();
        }
    }
    rep.summary.renamed = rep
        .files
        .iter()
        .filter(|f| f.rename_status == RenameStatus::Renamed)
        .count();
    for f in &mut rep.files {
        if f.rename_status != RenameStatus::Renamed {
            f.enrich_status = "not_run".into();
            f.learn_status = LearnStatus::NotRun;
        }
    }
    rep.summary.enriched = rep
        .files
        .iter()
        .filter(|f| f.enrich_status == "enriched")
        .count();
    rep.summary.failed = 2;
    rep.summary.cancelled = 1;
    m.apply_report(rep);
    m
}

/// Each banner tone stacked under the toolbar (max 2 shown, newest first).
pub fn banners_model(tones: [Tone; 2]) -> Model {
    let mut m = preview_model();
    let copy = |t: Tone| match t {
        Tone::Danger => (
            "OpenAI rejected the API key.",
            "Check it in Settings › Processing.",
            "Open provider settings",
        ),
        Tone::Warning => (
            "This scan needs OCR models, which aren't installed.",
            "",
            "Download OCR models",
        ),
        Tone::Success => ("Undid 10 renames.", "", ""),
        _ => (
            "Analysis stopped.",
            "Completed files are ready to review.",
            "",
        ),
    };
    for t in tones {
        let (title, body, action) = copy(t);
        m.banners.push(BannerMsg {
            tone: t,
            title: title.into(),
            body: body.into(),
            action: (!action.is_empty()).then(|| action.to_string()),
        });
    }
    m
}

/// Inline edit with a valid name (helper shows the original name).
pub fn valid_edit_model() -> Model {
    let mut m = preview_model();
    m.editing = Some((1, "20251028 Telekom Deutschland AP 54,99 Oktober".into()));
    m
}

/// Bauhaus (vision, warning) selected: detail shows lower sections.
pub fn warning_detail_model() -> Model {
    let mut m = preview_model();
    m.active = Some(3);
    m
}

/// Inline edit with a collision helper (row grows to 72).
pub fn invalid_edit_model() -> Model {
    let mut m = preview_model();
    m.editing = Some((1, "20251103 Amazon AP 12,13".into()));
    m
}

/// Plan list with mixed states (ready, review, error, edited, excluded).
pub fn preview_model() -> Model {
    let mut m = Model::new(config());
    m.demo = true;
    m.meta = Some(meta());
    m.rows = items().into_iter().map(Row::from_item).collect();
    // AR-2025-114: user-edited name; statement_oct: excluded.
    m.rows[8].edited = Some("20251102 Müller Bau AR 1.234,56 Anzahlung.pdf".into());
    m.rows[9].checked = false;
    m.active = Some(1);
    m.stamp_current();
    fill_pages(&mut m);
    m
}

/// Analysis in progress: 8 of 12 done, 2 analyzing, 2 queued.
pub fn progress_model() -> Model {
    let mut m = preview_model();
    m.rows[8].edited = None;
    m.rows[9].checked = true;
    for (i, r) in m.rows.iter_mut().enumerate().skip(8) {
        r.item = None;
        r.checked = false;
        r.analyzing = i < 10;
    }
    m.generation = 1;
    m.phase = Phase::Analyzing(Analysis {
        total: 12,
        done: 8,
        started: Instant::now(),
        done_at: vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0],
    });
    m.active = Some(1);
    m
}

/// Simulated apply of the checked rows of `m` (no file changes).
pub fn simulate_apply(m: &Model) -> ApplyReport {
    let files: Vec<FileReport> = m
        .rows
        .iter()
        .filter(|r| r.checked && r.checkable())
        .filter_map(|r| {
            let it = r.item.as_ref()?;
            let name = r.final_name();
            let enrich_status = match &it.enrich {
                EnrichPlan::Will => "enriched".to_string(),
                EnrichPlan::Skip { reason } => format!("skipped:{reason}"),
            };
            let learn_status = match (&it.learn, r.dont_learn) {
                (Some(LearnAction::AddAlias { .. }), false) => LearnStatus::AliasAdded,
                (Some(LearnAction::NewEntry { .. }), false) => LearnStatus::EntryAdded,
                _ => LearnStatus::None,
            };
            Some(FileReport {
                source: it.source.clone(),
                target: name.as_ref().map(|n| it.source.with_file_name(n)),
                name,
                rename_status: RenameStatus::Renamed,
                enrich_status,
                learn_status,
                fields: it.fields.clone(),
                company: it.company.clone(),
                date: it.date.clone(),
                tier: it.tier,
                model: it.model.clone(),
                warnings: it.warnings.clone(),
                error: None,
                enrich_error: None,
                learn_error: None,
            })
        })
        .collect();
    let count = |f: &dyn Fn(&FileReport) -> bool| files.iter().filter(|x| f(x)).count();
    ApplyReport {
        schema_version: SCHEMA_VERSION,
        batch_id: Some("20251107T093000Z-demo".into()),
        profile: "business".into(),
        provider: "openai".into(),
        model: "gpt-6-luna".into(),
        warnings: Vec::new(),
        summary: ApplySummary {
            total: files.len(),
            renamed: count(&|x| x.rename_status == RenameStatus::Renamed),
            unchanged: 0,
            failed: 0,
            incomplete: 0,
            cancelled: 0,
            enriched: count(&|x| x.enrich_status == "enriched"),
            learned: count(&|x| {
                matches!(
                    x.learn_status,
                    LearnStatus::AliasAdded | LearnStatus::EntryAdded
                )
            }),
        },
        files,
    }
}

/// Done summary after renaming the preview's checked rows.
pub fn done_model() -> Model {
    let mut m = preview_model();
    let report = simulate_apply(&m);
    m.apply_report(report);
    m
}
