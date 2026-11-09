//! Orchestration shared by CLI and GUI.
//!
//! - [`plan_files`]: collect PDFs, then per file (bounded `std::thread` pool):
//!   hash → extract → one provider call → harmonize (+ decider) → render
//!   filename → enrichment assessment. Collision suffixes are assigned
//!   afterwards, sequentially in collection order, so a plan is deterministic.
//!   No file is changed. The [`Plan`] is serde (JSON, `schema_version: 4`):
//!   `rename --dry-run`, `rename --plan FILE` and the GUI preview all use it.
//! - [`apply_plan`]: no AI calls. Per item: verify the source hash, rename
//!   (never overwrite), enrich, learn the name-list alias, journal the batch.
//!
//! Providers are blocking (`reqwest::blocking`); workers are plain threads,
//! never a tokio runtime. PDFium access is serialized inside `extract`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::{Decider, Settings};
use crate::extract::{self, ExtractConfig, Tier};
use crate::harmonize::{self, Choice, NameDb};
use crate::journal::{self, Journal, sha256_hex};
use crate::naming::{self, Target};
use crate::providers::{self, DeciderSettings, ExtractRequest, ProviderSettings};
use crate::{Error, ExitCode, Result, dates, enrich, profiles};

/// Version of every machine-readable document (plan, result, error, events).
pub const SCHEMA_VERSION: u32 = 4;
const RENAME_RETRIES: u32 = 3;
const RENAME_DELAY: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Machine-readable error (`{"error": {...}}` and per-file `error`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorInfo {
    /// Stable code: `config_error`, `usage_error`, `no_files`, `auth_error`,
    /// `provider_error`, `io_error`, `conflict`, `no_content`,
    /// `source_changed`, `ocr_models_missing`, `pdfium_missing`, …
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
    /// Retrying the same command may succeed (rate limits, outages).
    pub retryable: bool,
}

impl ErrorInfo {
    pub fn from_error(e: &Error) -> Self {
        ErrorInfo {
            code: e.code().into(),
            message: e.to_string(),
            hint: e.hint().map(Into::into),
            retryable: e.retryable(),
        }
    }

    pub fn new(code: &str, message: impl Into<String>, hint: Option<&str>) -> Self {
        ErrorInfo {
            code: code.into(),
            message: message.into(),
            hint: hint.map(Into::into),
            retryable: false,
        }
    }
}

/// Whole-command failure document (stdout in json mode).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ErrorDocument {
    pub schema_version: u32,
    pub error: ErrorInfo,
}

impl ErrorDocument {
    pub fn new(error: ErrorInfo) -> Self {
        ErrorDocument {
            schema_version: SCHEMA_VERSION,
            error,
        }
    }
}

/// Exit code for a batch: 0 none failed, 5 some failed; when all failed:
/// 11 all auth, 10 all provider/auth, 3 all config, else 1.
pub fn batch_exit_code<'a>(
    total: usize,
    errors: impl IntoIterator<Item = &'a ErrorInfo>,
) -> ExitCode {
    let errors: Vec<&ErrorInfo> = errors.into_iter().collect();
    if errors.is_empty() {
        return ExitCode::Success;
    }
    if errors.len() < total {
        return ExitCode::Partial;
    }
    let all = |codes: &[&str]| errors.iter().all(|e| codes.contains(&e.code.as_str()));
    if all(&["auth_error"]) {
        ExitCode::Auth
    } else if all(&["auth_error", "provider_error"]) {
        ExitCode::Provider
    } else if all(&["config_error"]) {
        ExitCode::Config
    } else {
        ExitCode::General
    }
}

// ---------------------------------------------------------------------------
// Progress events
// ---------------------------------------------------------------------------

/// Progress event (`--output ndjson` prints one per line). `index` is 1-based.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    PlanStarted {
        total: usize,
    },
    FileStarted {
        index: usize,
        total: usize,
        #[serde(with = "crate::fsx::path_serde")]
        #[schemars(with = "crate::fsx::path_serde::Repr")]
        source: PathBuf,
    },
    /// Extraction + AI done; `proposed_name` is before collision suffixes.
    FileAnalyzed {
        index: usize,
        total: usize,
        #[serde(with = "crate::fsx::path_serde")]
        #[schemars(with = "crate::fsx::path_serde::Repr")]
        source: PathBuf,
        proposed_name: Option<String>,
        error: Option<ErrorInfo>,
    },
    ApplyStarted {
        total: usize,
    },
    FileApplied {
        index: usize,
        total: usize,
        #[serde(with = "crate::fsx::path_serde")]
        #[schemars(with = "crate::fsx::path_serde::Repr")]
        source: PathBuf,
        #[serde(default, with = "crate::fsx::path_serde::opt")]
        #[schemars(with = "Option<crate::fsx::path_serde::Repr>")]
        target: Option<PathBuf>,
        rename_status: RenameStatus,
        enrich_status: String,
        learn_status: LearnStatus,
        error: Option<ErrorInfo>,
    },
}

/// Progress sink; called from worker threads.
pub type Progress<'a> = &'a (dyn Fn(&Event) + Sync);

/// A sink that drops every event.
pub fn no_progress(_: &Event) {}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Will be renamed to `target`.
    Rename,
    /// Already carries the rendered name.
    Unchanged,
    /// Analysis failed; see `error`. Apply leaves the file alone.
    Failed,
    /// Not analyzed: planning was cancelled. Apply leaves the file alone.
    Cancelled,
}

/// Enrichment intent. `reason`: `disabled`, `failed`, `unreadable`, or an
/// enrichment skip code (`signed`, `certified`, `encrypted`, `pdf_a`,
/// `e_invoice`, `xref_repaired`, `invalid_xmp`, `read_only`, `unparseable`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EnrichPlan {
    Will,
    Skip { reason: String },
}

/// Name-list change apply will make (undoable), in the list of the plan's
/// profile `harmonize_field`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LearnAction {
    AddAlias { canonical: String, alias: String },
    NewEntry { name: String, review: bool },
}

impl LearnAction {
    fn from_choice(c: Choice) -> Self {
        match c {
            Choice::Alias { canonical, alias } => LearnAction::AddAlias { canonical, alias },
            Choice::New { name, review } => LearnAction::NewEntry { name, review },
        }
    }

    fn choice(&self) -> Choice {
        match self {
            LearnAction::AddAlias { canonical, alias } => Choice::Alias {
                canonical: canonical.clone(),
                alias: alias.clone(),
            },
            LearnAction::NewEntry { name, review } => Choice::New {
                name: name.clone(),
                review: *review,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanItem {
    /// Absolute path of the PDF.
    #[serde(with = "crate::fsx::path_serde")]
    #[schemars(with = "crate::fsx::path_serde::Repr")]
    pub source: PathBuf,
    /// SHA-256 of the source when planned; apply refuses if it changed.
    pub source_sha256: String,
    pub status: PlanStatus,
    /// Final file name (including a `_(n)` collision suffix). Edit it in a
    /// saved plan to choose another name; apply renames to the source folder
    /// + this name (validated) and ignores `target`.
    pub proposed_name: Option<String>,
    /// Absolute target path (`= source` when unchanged). Informational:
    /// apply always uses the source folder + `proposed_name`.
    #[serde(default, with = "crate::fsx::path_serde::opt")]
    #[schemars(with = "Option<crate::fsx::path_serde::Repr>")]
    pub target: Option<PathBuf>,
    /// Raw validated model output (before harmonization and sanitizing).
    pub fields: Map<String, Value>,
    /// Harmonized value of the profile's `harmonize_field` (the company for
    /// the business profile; named `company` for every field).
    pub company: Option<String>,
    /// How `company` was matched against the field's name list.
    #[serde(default)]
    pub company_match: Option<CompanyMatch>,
    /// Parsed document date, `YYYY-MM-DD`.
    pub date: Option<String>,
    pub tier: Option<Tier>,
    /// Worst page's score 0..1 after local OCR (per page: `page_quality`).
    pub quality: Option<f64>,
    /// Score per page used (text layer, visual flag, local OCR, image sent).
    #[serde(default)]
    pub page_quality: Vec<PageQuality>,
    /// Model id reported by the provider.
    pub model: Option<String>,
    pub warnings: Vec<String>,
    pub enrich: EnrichPlan,
    pub learn: Option<LearnAction>,
    pub error: Option<ErrorInfo>,
    /// Text sent to the AI, one entry per page used (embedded on enrichment).
    pub pages: Vec<String>,
    /// `pdf.max_pages` cut the document.
    pub partial: bool,
}

/// Score of one page (rebuild spec §Pipeline).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageQuality {
    /// 1-based page number.
    pub page: usize,
    /// Text-layer score 0..1: readability, capped at 0.75 when `visual`.
    pub text: f64,
    /// The page shows a significant picture or drawing.
    #[serde(default)]
    pub visual: bool,
    /// Readability of this page's full local OCR text, when OCR ran on it.
    pub ocr: Option<f64>,
    /// The page image was sent to the model.
    pub image: bool,
}

impl PageQuality {
    fn of(p: &extract::Page) -> Self {
        PageQuality {
            page: p.number,
            text: p.text_score(),
            visual: p.visual,
            ocr: p.ocr_quality,
            image: p.image,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanSummary {
    pub total: usize,
    pub rename: usize,
    pub unchanged: usize,
    pub failed: usize,
    #[serde(default)]
    pub cancelled: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    /// Always 4.
    pub schema_version: u32,
    /// RFC 3339 UTC.
    pub created: String,
    pub profile: String,
    pub provider: String,
    pub model: String,
    /// `cloud` | `offline`
    pub privacy: String,
    /// Collection notices (missing paths, non-PDF files).
    pub warnings: Vec<String>,
    pub summary: PlanSummary,
    pub items: Vec<PlanItem>,
}

impl Plan {
    pub fn exit_code(&self) -> ExitCode {
        batch_exit_code(
            self.items.len(),
            self.items.iter().filter_map(|i| i.error.as_ref()),
        )
    }

    /// Parse a plan file, checking `schema_version`.
    pub fn from_json(text: &str) -> Result<Plan> {
        let v: Value = serde_json::from_str(text).map_err(|e| Error::Usage {
            message: format!("plan is not valid JSON: {e}"),
            hint: Some("Create one with `autorename-pdf rename --plan plan.json <paths>`.".into()),
        })?;
        let version = v.get("schema_version").and_then(Value::as_u64);
        if version != Some(u64::from(SCHEMA_VERSION)) {
            return Err(Error::Usage {
                message: format!(
                    "unsupported plan schema_version {} (expected {SCHEMA_VERSION})",
                    version.map_or("missing".into(), |n| n.to_string())
                ),
                hint: Some("Re-create the plan with this version of autorename-pdf.".into()),
            });
        }
        serde_json::from_value(v).map_err(|e| Error::Usage {
            message: format!("plan does not match the plan schema: {e}"),
            hint: Some("See `autorename-pdf schema plan`.".into()),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlanOptions {
    /// Descend into subfolders of folder arguments.
    pub recursive: bool,
    /// Worker threads (clamped to 1..=files).
    pub jobs: usize,
    /// `name-lists/` folder ([`crate::config::name_lists_dir`]); the
    /// profile's `harmonize_field` picks `<field>.yaml` in it. `None` = no
    /// list (names are kept as extracted).
    pub name_lists_dir: Option<PathBuf>,
    /// OCR models root override.
    pub models_dir: Option<PathBuf>,
    /// Variables from `.env` beside the config (decider keys); the process
    /// environment wins.
    pub env: HashMap<String, String>,
    /// Set to stop scheduling new files; the plan then lists the remaining
    /// files as `cancelled`.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// `min(4, cores)`.
pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(4)
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

fn is_pdf(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn is_hidden(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn walk(dir: &Path, recursive: bool, seen_dirs: &mut HashSet<PathBuf>, out: &mut Collected) {
    let canon = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !seen_dirs.insert(canon) {
        return;
    }
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
        Err(e) => {
            out.warnings
                .push(format!("Cannot read folder {}: {e}", dir.display()));
            return;
        }
    };
    entries.sort();
    for p in entries {
        if is_hidden(&p) {
            continue;
        }
        if p.is_dir() {
            if recursive {
                walk(&p, recursive, seen_dirs, out);
            }
        } else if p.is_file() && is_pdf(&p) {
            out.files.push(p);
        }
    }
}

/// Files in argument order; folders sorted by name (recursive on request).
/// Paths are absolute and de-duplicated (first occurrence wins).
pub fn collect_pdfs(paths: &[PathBuf], recursive: bool) -> Collected {
    let mut raw = Collected::default();
    let mut seen_dirs = HashSet::new();
    for p in paths {
        let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
        if p.is_dir() {
            walk(&p, recursive, &mut seen_dirs, &mut raw);
        } else if p.is_file() {
            if is_pdf(&p) {
                raw.files.push(p);
            } else {
                raw.warnings
                    .push(format!("Not a PDF, skipped: {}", p.display()));
            }
        } else {
            raw.warnings.push(format!("Not found: {}", p.display()));
        }
    }
    let mut seen = HashSet::new();
    raw.files
        .retain(|f| seen.insert(std::fs::canonicalize(f).unwrap_or_else(|_| f.clone())));
    raw
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    settings: &'a Settings,
    extract: ExtractConfig,
    provider: ProviderSettings,
    decider: DeciderSettings,
    /// Name list of the profile's `harmonize_field`.
    db: &'a NameDb,
}

fn tier_code(t: Tier) -> &'static str {
    match t {
        Tier::Text => "text",
        Tier::Ocr => "ocr",
        Tier::VisionNeeded => "vision",
    }
}

fn new_item(source: &Path) -> PlanItem {
    PlanItem {
        source: source.to_path_buf(),
        source_sha256: String::new(),
        status: PlanStatus::Failed,
        proposed_name: None,
        target: None,
        fields: Map::new(),
        company: None,
        company_match: None,
        date: None,
        tier: None,
        quality: None,
        page_quality: Vec::new(),
        model: None,
        warnings: Vec::new(),
        enrich: EnrichPlan::Skip {
            reason: "failed".into(),
        },
        learn: None,
        error: None,
        pages: Vec::new(),
        partial: false,
    }
}

fn fail(mut item: PlanItem, e: ErrorInfo) -> PlanItem {
    item.status = PlanStatus::Failed;
    item.error = Some(e);
    item
}

/// How the company name was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    /// Exact (normalized) hit of a canonical name or alias.
    Known,
    /// High-confidence variant (≥ 0.92) of an existing company.
    Alias,
    /// The decider picked an existing company.
    Decider,
    /// No plausible match (or the decider said "new").
    New,
    /// Ambiguous without a decider verdict: new entry flagged `review: true`.
    Review,
}

/// Who resolved an ambiguous match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DecidedBy {
    None,
    Ai,
    OpenaiDecisions,
    Jev,
    /// The `ai` decider on the local model under `privacy: offline`.
    Offline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MatchCandidate {
    pub name: String,
    pub score: f32,
}

/// Company-match evidence for one file (GUI "why this name").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompanyMatch {
    /// Raw extracted company name.
    pub extracted: String,
    /// Normalized form used for matching.
    pub normalized: String,
    pub outcome: MatchOutcome,
    /// Best candidate score (`1.0` for known).
    pub score: Option<f32>,
    /// `>=0.92` | `0.75-0.92` | `<0.75` (`null` when known or no candidate).
    pub band: Option<String>,
    /// Top 3 candidates.
    pub candidates: Vec<MatchCandidate>,
    pub decided_by: DecidedBy,
}

fn band(score: f64) -> &'static str {
    if score >= harmonize::AUTO_ALIAS_THRESHOLD {
        ">=0.92"
    } else if score >= harmonize::DECIDER_THRESHOLD {
        "0.75-0.92"
    } else {
        "<0.75"
    }
}

struct Harmonized {
    company: String,
    learn: Option<LearnAction>,
    evidence: Option<CompanyMatch>,
    warnings: Vec<String>,
}

/// Harmonized company, learn action, match evidence and warnings for one raw name.
fn harmonize_name(raw: &str, ctx: &Ctx<'_>) -> Harmonized {
    use harmonize::{Decision, Verdict};
    let s = ctx.settings;
    // My own company is never a counterparty to learn.
    if is_my_company(raw, &s.company_name) {
        return Harmonized {
            company: raw.trim().to_string(),
            learn: None,
            evidence: None,
            warnings: Vec::new(),
        };
    }
    let decision = harmonize::decide(ctx.db, raw, None);
    let mut warnings = Vec::new();
    let mut decided_by = DecidedBy::None;
    let verdict = match &decision {
        Decision::AskDecider { name, candidates } if s.auto_learn && s.decider != Decider::Off => {
            match providers::decide_name(candidates, name, &ctx.db.field, &ctx.decider) {
                Ok(v) => {
                    decided_by = match (s.decider, s.privacy) {
                        (Decider::Ai, crate::config::Privacy::Offline) => DecidedBy::Offline,
                        (Decider::Ai, _) => DecidedBy::Ai,
                        (Decider::OpenaiDecisions, _) => DecidedBy::OpenaiDecisions,
                        (Decider::Jev, _) => DecidedBy::Jev,
                        (Decider::Off, _) => DecidedBy::None,
                    };
                    v
                }
                Err(e) => {
                    let (label, _) = harmonize::field_label(&ctx.db.field);
                    warnings.push(format!("{label} decider failed: {e}"));
                    None
                }
            }
        }
        _ => None,
    };
    let r = decision.resolve(raw, verdict.as_ref());
    let outcome = match (&decision, &verdict) {
        (Decision::Skip { .. }, _) => None,
        (Decision::Known { .. }, _) => Some(MatchOutcome::Known),
        (Decision::AddAlias { .. }, _) => Some(MatchOutcome::Alias),
        (Decision::NewEntry { .. }, _) => Some(MatchOutcome::New),
        (Decision::AskDecider { .. }, Some(Verdict::New)) => Some(MatchOutcome::New),
        (Decision::AskDecider { .. }, _) if r.mapped => Some(MatchOutcome::Decider),
        (Decision::AskDecider { .. }, _) => Some(MatchOutcome::Review),
    };
    let evidence = outcome.map(|outcome| {
        let candidates = harmonize::rank(ctx.db, raw, 3);
        let top = match outcome {
            MatchOutcome::Known => Some(1.0),
            _ => candidates.first().map(|c| c.score),
        };
        CompanyMatch {
            extracted: raw.to_string(),
            normalized: harmonize::normalize(ctx.db.kind(), raw),
            outcome,
            score: top.map(|t| t as f32),
            band: (outcome != MatchOutcome::Known)
                .then(|| top.map(|t| band(t).to_string()))
                .flatten(),
            candidates: candidates
                .iter()
                .map(|c| MatchCandidate {
                    name: c.canonical.clone(),
                    score: c.score as f32,
                })
                .collect(),
            decided_by,
        }
    });
    // Learning off: map known/alias hits only, never write.
    let (company, learn) = if s.auto_learn {
        (r.name, r.learn.map(LearnAction::from_choice))
    } else {
        (
            harmonize::lookup(ctx.db, raw).unwrap_or_else(|| raw.trim().to_string()),
            None,
        )
    };
    Harmonized {
        company,
        learn,
        evidence,
        warnings,
    }
}

fn assess_enrich(path: &Path, enabled: bool) -> EnrichPlan {
    if !enabled {
        return EnrichPlan::Skip {
            reason: "disabled".into(),
        };
    }
    match enrich::plan(path) {
        Ok(enrich::Assessment::Eligible) => EnrichPlan::Will,
        Ok(enrich::Assessment::Skip(r)) => EnrichPlan::Skip {
            reason: r.code().into(),
        },
        Err(_) => EnrichPlan::Skip {
            reason: "unreadable".into(),
        },
    }
}

/// Is `name` my company? Harmonized fuzzy match with company normalization
/// (whatever list the profile harmonizes): same normalized form (legal
/// suffixes, case, punctuation ignored) or a non-subset similarity of at
/// least the auto-alias threshold.
fn is_my_company(name: &str, company: &str) -> bool {
    let (a, b) = (
        harmonize::normalize_company(name),
        harmonize::normalize_company(company),
    );
    !a.is_empty()
        && !b.is_empty()
        && (a == b || harmonize::similarity(&a, &b) >= harmonize::AUTO_ALIAS_THRESHOLD)
}

/// Apply a profile [`profiles::Direction`]: when the model returned the
/// incoming or outgoing value, re-derive it from who issued / received the
/// document. Returns a `direction_uncertain` warning when neither or both
/// sides are my company (the model's value is kept).
pub fn derive_direction(
    d: &profiles::Direction,
    fields: &mut Map<String, Value>,
    company: &str,
) -> Option<String> {
    let get = |k: &str| {
        fields
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    if company.trim().is_empty() {
        return None;
    }
    let current = get(&d.field);
    let (issuer, recipient) = (get(&d.issuer), get(&d.recipient));
    let (me_issuer, me_recipient) = (
        is_my_company(&issuer, company),
        is_my_company(&recipient, company),
    );
    // Counterparty: the model's answer unless it is empty or my own company;
    // then the side that is not me, without its legal form (the field asks
    // for none, issuer/recipient are as printed).
    if let Some(cp) = &d.counterparty {
        let answer = get(cp);
        let mine = is_my_company(&answer, company);
        let other = match (me_issuer, me_recipient) {
            (true, false) => Some(&recipient),
            (false, true) => Some(&issuer),
            _ => None,
        };
        if answer.is_empty() || mine {
            match other.filter(|o| !o.is_empty()) {
                Some(o) => {
                    let name = harmonize::strip_legal_form(o);
                    fields.insert(cp.clone(), Value::String(name));
                }
                None if mine => {
                    return Some(format!(
                        "direction_uncertain: the model named {company:?} (your own company) as the counterparty and issuer/recipient do not resolve it (issuer {issuer:?}, recipient {recipient:?}); please review"
                    ));
                }
                None => {}
            }
        }
    }
    if current != d.incoming && current != d.outgoing {
        return None;
    }
    let derived = match (me_issuer, me_recipient) {
        (false, true) => &d.incoming,
        (true, false) => &d.outgoing,
        _ => {
            return Some(format!(
                "direction_uncertain: cannot tell whether {company:?} issued or received this document (issuer {issuer:?}, recipient {recipient:?}); kept {current:?}, please review"
            ));
        }
    };
    fields.insert(d.field.clone(), Value::String(derived.clone()));
    None
}

/// Apply a profile [`profiles::Amount`]: a non-empty amount becomes
/// `format` with `{currency}`/`{amount}` filled in (spaces collapsed), in
/// the target field. Returns `currency_missing` when no currency was found.
pub fn derive_amount(
    a: &profiles::Amount,
    fields: &mut Map<String, Value>,
    format: &str,
) -> Option<String> {
    let get = |k: &str| {
        fields
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let (amount, currency) = (get(&a.amount), get(&a.currency));
    if amount.is_empty() {
        return None;
    }
    let text = format
        .replace("{currency}", &currency)
        .replace("{amount}", &amount);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    fields.insert(a.field.clone(), Value::String(text));
    currency.is_empty().then(|| {
        format!("currency_missing: no currency found for the amount {amount:?}; please review")
    })
}

/// Validated fields + direction warnings of one model answer.
type Interpreted = (Map<String, Value>, Vec<String>);

/// Validate a model answer and derive the direction (warnings separate).
fn interpret(value: &Value, s: &Settings) -> Result<Interpreted> {
    let mut fields = profiles::validate_metadata(&s.profile, value)?;
    let warnings = match &s.profile.direction {
        Some(d) => derive_direction(d, &mut fields, &s.company_name)
            .into_iter()
            .collect(),
        None => Vec::new(),
    };
    Ok((fields, warnings))
}

/// A profile with direction logic found no counterparty (empty field).
fn counterparty_missing(
    d: &profiles::Direction,
    s: &Settings,
    fields: &Map<String, Value>,
) -> bool {
    d.counterparty
        .as_deref()
        .or(s.profile.harmonize_field.as_deref())
        .is_some_and(|f| {
            fields
                .get(f)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .is_empty()
        })
}

/// Everything except the collision-aware target.
fn analyze(path: &Path, ctx: &Ctx<'_>) -> PlanItem {
    let s = ctx.settings;
    let mut item = new_item(path);
    match std::fs::read(path) {
        Ok(bytes) => item.source_sha256 = sha256_hex(&bytes),
        Err(e) => return fail(item, ErrorInfo::from_error(&Error::io(path, e))),
    }
    let ex = match extract::extract_with(path, &ctx.extract) {
        Ok(ex) => ex,
        Err(e) => return fail(item, ErrorInfo::from_error(&e)),
    };
    item.warnings = ex.warnings.clone();
    item.tier = Some(ex.tier);
    item.quality = Some(ex.quality);
    item.page_quality = ex.pages.iter().map(PageQuality::of).collect();
    item.pages = ex.page_texts();
    item.partial = ex.page_count > ex.pages_used;
    let text = ex.combined_text();
    let Some(req) = ExtractRequest::document(
        &s.profile,
        &s.prompt_extension,
        &text,
        ex.page_images.clone(),
    ) else {
        return fail(
            item,
            ErrorInfo::new(
                "no_content",
                "No content extracted",
                Some("The PDF has no usable text layer; try --ocr or --vision."),
            ),
        );
    };
    let resp = match providers::extract(&req, &ctx.provider) {
        Ok(r) => r,
        Err(e) => return fail(item, ErrorInfo::from_error(&e)),
    };
    item.model = Some(resp.model.clone());
    let (mut fields, direction_warnings) = match interpret(&resp.value, s) {
        Ok(r) => r,
        Err(e) => return fail(item, ErrorInfo::from_error(&e)),
    };
    if let Some(d) = &s.profile.direction
        && counterparty_missing(d, s, &fields)
    {
        item.warnings.push(
            "counterparty_missing: no counterparty company found (printed only in a logo or image?); please review"
                .into(),
        );
    }
    item.warnings.extend(direction_warnings);
    if let Some(a) = &s.profile.amount {
        item.warnings
            .extend(derive_amount(a, &mut fields, &s.amount_format));
    }
    let mut named = fields.clone();
    if let Some(hf) = &s.profile.harmonize_field {
        let raw = fields.get(hf).and_then(Value::as_str).unwrap_or("");
        let Harmonized {
            company,
            learn,
            evidence,
            warnings,
        } = harmonize_name(raw, ctx);
        item.warnings.extend(warnings);
        item.learn = learn;
        item.company_match = evidence;
        named.insert(hf.clone(), Value::String(company.clone()));
        item.company = (!company.is_empty()).then_some(company);
    }
    item.date = fields
        .get("document_date")
        .and_then(Value::as_str)
        .and_then(dates::parse_date)
        .map(|d| d.format("%Y-%m-%d").to_string());
    item.fields = fields;
    item.proposed_name = Some(naming::render_filename(
        &s.profile_id,
        &s.profile,
        &named,
        &s.date_format,
    ));
    item.enrich = assess_enrich(path, s.enrich_enabled);
    item.status = PlanStatus::Rename;
    item
}

/// Any directory entry (incl. a dangling symlink) or an unreadable one.
fn occupied(p: &Path) -> bool {
    !matches!(std::fs::symlink_metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Reservation key: canonical directory + file name, case-folded where the
/// filesystem is usually case-insensitive (Windows, macOS).
fn reservation_key(p: &Path) -> String {
    let dir = p.parent().unwrap_or(Path::new(""));
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = if cfg!(any(windows, target_vendor = "apple")) {
        harmonize::casefold(&name)
    } else {
        name
    };
    format!("{}\u{0}{name}", dir.display())
}

/// Collision-aware target (sequential, collection order).
fn finalize(mut item: PlanItem, reserved: &mut HashSet<String>) -> PlanItem {
    if item.error.is_some() || item.status == PlanStatus::Cancelled {
        return item;
    }
    let name = item.proposed_name.clone().unwrap_or_default();
    let taken = |p: &Path| occupied(p) || reserved.contains(&reservation_key(p));
    match naming::plan_target(&item.source, &name, &[], taken) {
        Target::Unchanged => {
            item.status = PlanStatus::Unchanged;
            // The kept name (may carry a `_(n)` suffix from an earlier run).
            item.proposed_name = item
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            item.target = Some(item.source.clone());
        }
        Target::Path(p) => {
            item.status = PlanStatus::Rename;
            item.proposed_name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            reserved.insert(reservation_key(&p));
            item.target = Some(p);
        }
    }
    item
}

fn run_parallel<T: Send>(
    files: &[PathBuf],
    jobs: usize,
    f: impl Fn(usize, &Path) -> T + Sync,
) -> Vec<T> {
    let jobs = jobs.clamp(1, files.len().max(1));
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<T>>> = files.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(path) = files.get(i) else { break };
                    let out = f(i, path);
                    *slots[i].lock().unwrap_or_else(|p| p.into_inner()) = Some(out);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .unwrap_or_else(|p| p.into_inner())
                .expect("every slot is filled")
        })
        .collect()
}

/// Analyze `paths` and build a plan. Changes no files. Errors only for
/// whole-run problems (no PDFs, unreadable name list).
pub fn plan_files(
    paths: &[PathBuf],
    settings: &Settings,
    opts: &PlanOptions,
    progress: Progress<'_>,
) -> Result<Plan> {
    settings.require_provider()?;
    // A typo is not an empty inbox: name the missing path (no_files/4 is only
    // for existing folders without PDFs).
    if let Some(p) = paths.iter().find(|p| std::fs::symlink_metadata(p).is_err()) {
        return Err(Error::Usage {
            message: format!("not found: {}", p.display()),
            hint: Some("Check the path (quote names with spaces).".into()),
        });
    }
    let collected = collect_pdfs(paths, opts.recursive);
    if collected.files.is_empty() {
        return Err(Error::NoFiles {
            message: "No PDF files found in the given paths".into(),
            hint: Some("Pass PDF files or folders; add -r to include subfolders.".into()),
        });
    }
    let field = settings.profile.harmonize_field.as_deref().unwrap_or("");
    let db = match (&opts.name_lists_dir, field) {
        (Some(dir), f) if !f.is_empty() => {
            harmonize::load_db(&crate::config::name_list_in(dir, f), f)?
        }
        _ => NameDb::empty(field),
    };
    let env = |name: &str| {
        std::env::var(name)
            .ok()
            .or_else(|| opts.env.get(name).cloned())
    };
    let mut extract_cfg = ExtractConfig::from_settings(settings);
    extract_cfg.models_dir.clone_from(&opts.models_dir);
    let ctx = Ctx {
        settings,
        extract: extract_cfg,
        provider: ProviderSettings::from_settings(settings),
        decider: DeciderSettings::from_settings(settings, &env),
        db: &db,
    };
    let total = collected.files.len();
    progress(&Event::PlanStarted { total });
    let cancelled = || {
        opts.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::SeqCst))
    };
    let analyzed = run_parallel(&collected.files, opts.jobs, |i, path| {
        if cancelled() {
            let mut item = new_item(path);
            item.status = PlanStatus::Cancelled;
            item.enrich = EnrichPlan::Skip {
                reason: "cancelled".into(),
            };
            return item;
        }
        progress(&Event::FileStarted {
            index: i + 1,
            total,
            source: path.to_path_buf(),
        });
        let item = analyze(path, &ctx);
        progress(&Event::FileAnalyzed {
            index: i + 1,
            total,
            source: path.to_path_buf(),
            proposed_name: item.proposed_name.clone(),
            error: item.error.clone(),
        });
        item
    });
    let mut reserved = HashSet::new();
    let items: Vec<PlanItem> = analyzed
        .into_iter()
        .map(|it| finalize(it, &mut reserved))
        .collect();
    let count = |st: PlanStatus| items.iter().filter(|i| i.status == st).count();
    Ok(Plan {
        schema_version: SCHEMA_VERSION,
        created: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        profile: settings.profile_id.clone(),
        provider: settings.ai.provider.id().into(),
        model: settings.ai.model.clone(),
        privacy: match settings.privacy {
            crate::config::Privacy::Cloud => "cloud".into(),
            crate::config::Privacy::Offline => "offline".into(),
        },
        warnings: collected.warnings,
        summary: PlanSummary {
            total,
            rename: count(PlanStatus::Rename),
            unchanged: count(PlanStatus::Unchanged),
            failed: count(PlanStatus::Failed),
            cancelled: count(PlanStatus::Cancelled),
        },
        items,
    })
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RenameStatus {
    Renamed,
    Unchanged,
    Failed,
    /// Not reached: the run was cancelled before this file.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LearnStatus {
    /// Variant added to an existing entry (`learned`).
    AliasAdded,
    /// New `source: auto` entry.
    EntryAdded,
    /// Already in the name list (e.g. learned earlier in this batch).
    Known,
    /// Nothing to learn for this file.
    None,
    /// `--no-learn` / `harmonization.auto_learn: false`.
    Disabled,
    /// Not reached (the file failed earlier).
    NotRun,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FileReport {
    #[serde(with = "crate::fsx::path_serde")]
    #[schemars(with = "crate::fsx::path_serde::Repr")]
    pub source: PathBuf,
    /// Where the file is now (`= source` when unchanged; `null` on failure).
    #[serde(default, with = "crate::fsx::path_serde::opt")]
    #[schemars(with = "Option<crate::fsx::path_serde::Repr>")]
    pub target: Option<PathBuf>,
    pub name: Option<String>,
    pub rename_status: RenameStatus,
    /// `enriched` | `unchanged` | `skipped:<reason>` | `failed` | `not_run`
    pub enrich_status: String,
    pub learn_status: LearnStatus,
    pub fields: Map<String, Value>,
    pub company: Option<String>,
    pub date: Option<String>,
    pub tier: Option<Tier>,
    pub model: Option<String>,
    pub warnings: Vec<String>,
    /// Hard failure: the file was not renamed (or not processed).
    pub error: Option<ErrorInfo>,
    /// Enrichment was requested and failed (`enrich_status: failed`); the
    /// rename itself succeeded. Counts as a partial failure.
    #[serde(default)]
    pub enrich_error: Option<ErrorInfo>,
    /// Learning the company name failed (`learn_status: failed`). Partial failure.
    #[serde(default)]
    pub learn_error: Option<ErrorInfo>,
}

impl FileReport {
    /// Some requested sub-operation (enrich/learn) failed.
    pub fn incomplete(&self) -> bool {
        self.enrich_error.is_some() || self.learn_error.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplySummary {
    pub total: usize,
    pub renamed: usize,
    pub unchanged: usize,
    /// Failed operations: renames (hard `error`), enrichments
    /// (`enrich_error`) and learned names (`learn_error`).
    pub failed: usize,
    /// Files without `error` whose enrichment or learning failed.
    #[serde(default)]
    pub incomplete: usize,
    /// Files not reached because the run was cancelled.
    #[serde(default)]
    pub cancelled: usize,
    pub enriched: usize,
    pub learned: usize,
}

/// Result of `rename` / `apply`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyReport {
    /// Always 4.
    pub schema_version: u32,
    /// Undo batch (`undo --batch ID`); `null` when nothing changed.
    pub batch_id: Option<String>,
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub warnings: Vec<String>,
    pub summary: ApplySummary,
    pub files: Vec<FileReport>,
}

impl ApplyReport {
    /// [`batch_exit_code`] over hard errors; enrich/learn failures alone
    /// make it partial (5). Cancelled files are not failures.
    pub fn exit_code(&self) -> ExitCode {
        match batch_exit_code(
            self.files.len(),
            self.files.iter().filter_map(|f| f.error.as_ref()),
        ) {
            ExitCode::Success if self.files.iter().any(FileReport::incomplete) => ExitCode::Partial,
            code => code,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    pub journal_path: PathBuf,
    /// `name-lists/` folder to learn into (the list of the profile's
    /// `harmonize_field`, resolved per run); `None` disables learning.
    pub name_lists_dir: Option<PathBuf>,
    /// Journal batch source: `cli` | `gui`.
    pub source: String,
    /// Set to stop before the next file (never mid-file).
    pub cancel: Option<Arc<AtomicBool>>,
}

fn report_for(item: &PlanItem) -> FileReport {
    FileReport {
        source: item.source.clone(),
        target: None,
        name: None,
        rename_status: RenameStatus::Failed,
        enrich_status: "not_run".into(),
        learn_status: LearnStatus::NotRun,
        fields: item.fields.clone(),
        company: item.company.clone(),
        date: item.date.clone(),
        tier: item.tier,
        model: item.model.clone(),
        warnings: item.warnings.clone(),
        error: item.error.clone(),
        enrich_error: None,
        learn_error: None,
    }
}

/// Identity of `path` if its bytes still hash to `sha256`, else `source_changed`.
fn verify_source(path: &Path, sha256: &str) -> Result<crate::fsx::FileId> {
    let (id, sha) = crate::fsx::identify(path).map_err(|e| Error::io(path, e))?;
    if sha != sha256 {
        return Err(Error::Extract {
            code: "source_changed",
            message: format!("{} changed since the plan was made", path.display()),
            hint: Some("Re-run `autorename-pdf rename` to make a fresh plan.".into()),
        });
    }
    Ok(id)
}

fn invalid_plan(i: usize, why: impl std::fmt::Display) -> Error {
    Error::Usage {
        message: format!("plan item {} (items[{i}]): {why}", i + 1),
        hint: Some(format!(
            "Fix items[{i}].proposed_name in the plan file (a single file name ending in .pdf), then run `autorename-pdf apply <plan>` again."
        )),
    }
}

/// Whole-plan check before any mutation: every actionable item has an
/// absolute source and a valid single `.pdf` `proposed_name`; apply renames
/// within the source folder to that name (edit `proposed_name`, then apply).
pub fn validate_plan(plan: &Plan) -> Result<()> {
    if plan.schema_version != SCHEMA_VERSION {
        return Err(Error::Usage {
            message: format!(
                "unsupported plan schema_version {} (expected {SCHEMA_VERSION})",
                plan.schema_version
            ),
            hint: None,
        });
    }
    for (i, item) in plan.items.iter().enumerate() {
        if !matches!(item.status, PlanStatus::Rename | PlanStatus::Unchanged)
            || item.error.is_some()
        {
            continue;
        }
        if !item.source.is_absolute() || item.source.file_name().is_none() {
            return Err(Error::Usage {
                message: format!(
                    "plan item {} (items[{i}]): source must be an absolute file path",
                    i + 1
                ),
                hint: Some(format!(
                    "items[{i}].source is not valid: the plan was edited or files moved. Re-create it with `autorename-pdf rename --plan <plan> <paths>`."
                )),
            });
        }
        let name = item
            .proposed_name
            .as_deref()
            .ok_or_else(|| invalid_plan(i, "missing proposed_name"))?;
        if effective_target(item).is_some() {
            naming::validate_filename(name)
                .map_err(|e| invalid_plan(i, format!("{name:?}: {e}")))?;
        }
    }
    Ok(())
}

/// Where apply puts an actionable item: the source folder + `proposed_name`
/// (`target` in the plan is informational and ignored). `None` = the file
/// already has that name (Unicode-normalized comparison).
fn effective_target(item: &PlanItem) -> Option<PathBuf> {
    let name = item.proposed_name.as_deref()?;
    let target = item.source.parent().unwrap_or(Path::new("/")).join(name);
    let norm = |p: &Path| crate::textutil::nfc(&p.to_string_lossy());
    (norm(&target) != norm(&item.source)).then_some(target)
}

fn enrich_data(
    item: &PlanItem,
    plan: &Plan,
    settings: &Settings,
    now: DateTime<Utc>,
) -> enrich::EnrichData {
    let name = item.proposed_name.as_deref().unwrap_or("");
    let stem = name
        .strip_suffix(".pdf")
        .or_else(|| name.strip_suffix(".PDF"))
        .unwrap_or(name);
    // The title never encodes a collision suffix: duplicates and re-runs
    // embed identical data (enrichment then reports `unchanged`).
    let title = naming::strip_collision_suffix(stem).to_string();
    let company = item.company.clone().unwrap_or_default();
    let field = |k: &str| {
        item.fields
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
    };
    let subject_parts: Vec<&str> = [field("document_type"), company.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    let subject = if subject_parts.is_empty() {
        title.clone()
    } else {
        subject_parts.join(" ")
    };
    let p = &settings.profile;
    let mut keywords: Vec<String> = Vec::new();
    let direction_inputs: Vec<&str> = p
        .direction
        .iter()
        .flat_map(|d| [d.issuer.as_str(), d.recipient.as_str()])
        .chain(
            p.amount
                .iter()
                .flat_map(|a| [a.amount.as_str(), a.currency.as_str()]),
        )
        .collect();
    for k in p.fields.keys() {
        if *k == p.truncate_field || direction_inputs.contains(&k.as_str()) {
            continue;
        }
        let v = if p.harmonize_field.as_deref() == Some(k.as_str()) {
            company.clone()
        } else {
            field(k).to_string()
        };
        if !v.is_empty() && !keywords.contains(&v) {
            keywords.push(v);
        }
    }
    enrich::EnrichData {
        title,
        subject,
        keywords,
        profile: plan.profile.clone(),
        fields: item.fields.clone(),
        tier: item.tier.map_or("text", tier_code).into(),
        provider: plan.provider.clone(),
        model: item.model.clone().unwrap_or_else(|| plan.model.clone()),
        pages: item.pages.clone(),
        partial: item.partial,
        timestamp: now,
    }
}

struct ApplyState<'a> {
    settings: &'a Settings,
    opts: &'a ApplyOptions,
    journal: Journal,
    batch_id: String,
    now: DateTime<Utc>,
}

impl ApplyState<'_> {
    fn batch(&mut self) -> &mut journal::Batch {
        journal::batch_mut(
            &mut self.journal,
            &self.batch_id,
            &self.opts.source,
            self.now,
        )
    }
}

/// Evidence stored with a `review: true` entry (file name only, no path).
fn review_evidence(item: &PlanItem, batch: &str) -> harmonize::ReviewEvidence {
    let m = item.company_match.as_ref();
    harmonize::ReviewEvidence {
        reason: match m.and_then(|m| m.band.as_deref()) {
            Some(band) => format!("ambiguous match ({band}) without a decider verdict"),
            None => "ambiguous match without a decider verdict".into(),
        },
        candidates: m
            .map(|m| {
                m.candidates
                    .iter()
                    .map(|c| harmonize::ReviewCandidate {
                        name: c.name.clone(),
                        score: (f64::from(c.score) * 100.0).round() / 100.0,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        source: item
            .source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        batch: batch.to_string(),
    }
}

fn learn_one(
    action: &LearnAction,
    item: &PlanItem,
    st: &mut ApplyState<'_>,
) -> (LearnStatus, Option<Error>) {
    let (Some(dir), Some(field)) = (
        &st.opts.name_lists_dir,
        st.settings.profile.harmonize_field.clone(),
    ) else {
        return (LearnStatus::Disabled, None);
    };
    let path = crate::config::name_list_in(dir, &field);
    let choice = action.choice();
    let today = st.now.date_naive();
    let evidence = review_evidence(item, &st.batch_id);
    // Locked reload-learn-save per name: concurrent runs never lose names.
    let change = match harmonize::update_db(&path, &field, |db| {
        let change = harmonize::learn_checked(db, &choice, today)?;
        if let (
            Some(harmonize::Change::EntryAdded { canonical }),
            harmonize::Choice::New { review: true, .. },
        ) = (&change, &choice)
            && let Some(i) = db.find(canonical)
        {
            db.entries[i].evidence = Some(evidence);
        }
        Ok(change)
    }) {
        Ok(Some(change)) => change,
        Ok(None) => return (LearnStatus::Known, None),
        Err(e) => return (LearnStatus::Failed, Some(e)),
    };
    let status = match change {
        harmonize::Change::AliasAdded { .. } => LearnStatus::AliasAdded,
        harmonize::Change::EntryAdded { .. } => LearnStatus::EntryAdded,
    };
    st.batch();
    let (id, now) = (st.batch_id.clone(), st.now);
    journal::record_alias(&mut st.journal, &id, &path, &field, change, now);
    (status, None)
}

fn apply_item(item: &PlanItem, plan: &Plan, st: &mut ApplyState<'_>) -> FileReport {
    let mut r = report_for(item);
    if r.error.is_some() || !matches!(item.status, PlanStatus::Rename | PlanStatus::Unchanged) {
        return r;
    }
    let failed = |mut r: FileReport, e: &Error| {
        r.error = Some(ErrorInfo::from_error(e));
        r
    };
    // Bound to the verified object: every rename attempt re-verifies the
    // bytes, and a swapped object is moved back (`rename_file_checked`).
    let check = || verify_source(&item.source, &item.source_sha256);
    let final_path = match effective_target(item) {
        Some(target) => {
            let target = &target;
            let done = match naming::rename_file_checked(
                &item.source,
                target,
                RENAME_RETRIES,
                RENAME_DELAY,
                &check,
            ) {
                Ok(done) => done,
                Err(e) => return failed(r, &e),
            };
            // Committed even if not flushed: journal it and warn.
            r.warnings.extend(done.warnings);
            st.batch();
            let (id, now) = (st.batch_id.clone(), st.now);
            journal::record_rename(&mut st.journal, &id, &item.source, target, now);
            r.rename_status = RenameStatus::Renamed;
            target.clone()
        }
        None => {
            if let Err(e) = check() {
                return failed(r, &e);
            }
            r.rename_status = RenameStatus::Unchanged;
            item.source.clone()
        }
    };
    r.name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    r.target = Some(final_path.clone());

    r.enrich_status = match (&item.enrich, st.settings.enrich_enabled) {
        (_, false) => "skipped:disabled".into(),
        (EnrichPlan::Skip { reason }, _) => format!("skipped:{reason}"),
        (EnrichPlan::Will, true) => {
            let data = enrich_data(item, plan, st.settings, st.now);
            match enrich::apply_expect(&final_path, &data, &item.source_sha256) {
                Ok(status) => {
                    if let enrich::EnrichStatus::Enriched(outcome) = &status {
                        r.warnings.extend(outcome.warnings.iter().cloned());
                        st.batch();
                        let (id, now) = (st.batch_id.clone(), st.now);
                        journal::record_enrich(
                            &mut st.journal,
                            &id,
                            outcome.journal_entry(&final_path),
                            now,
                        );
                    }
                    status.code()
                }
                Err(e) => {
                    r.warnings.push(format!("Enrichment failed: {e}"));
                    r.enrich_error = Some(ErrorInfo::from_error(&e));
                    "failed".into()
                }
            }
        }
    };

    r.learn_status = match (&item.learn, st.settings.auto_learn) {
        (_, false) => LearnStatus::Disabled,
        (None, true) => LearnStatus::None,
        (Some(action), true) => {
            let (status, err) = learn_one(action, item, st);
            if let Some(e) = err {
                let field = st.settings.profile.harmonize_field.as_deref();
                let (_, label) = harmonize::field_label(field.unwrap_or(""));
                r.warnings.push(format!("{label} not learned: {e}"));
                r.learn_error = Some(ErrorInfo::from_error(&e));
            }
            status
        }
    };
    r
}

fn cancelled_report(item: &PlanItem) -> FileReport {
    let mut r = report_for(item);
    r.rename_status = RenameStatus::Cancelled;
    r.enrich_status = "not_run".into();
    r.error = None;
    r
}

fn journal_failed(e: &Error, files: &[FileReport], total: usize) -> Error {
    let changed: Vec<String> = files
        .iter()
        .filter(|f| f.rename_status == RenameStatus::Renamed)
        .filter_map(|f| f.target.as_ref().map(|t| t.display().to_string()))
        .collect();
    Error::General(format!(
        "Undo journal could not be written ({e}); stopped after {} of {total} file(s). \
         Changes not recorded for undo: {}",
        files.len(),
        if changed.is_empty() {
            "none".into()
        } else {
            changed.join(", ")
        }
    ))
}

/// Execute a plan without AI calls.
///
/// The whole plan is validated first ([`validate_plan`], usage error, nothing
/// changed) and the journal is written (`in_progress` batch) before any
/// mutation. If the journal cannot be written mid-run, no further file is
/// touched and the run fails (exit 1) naming the unrecorded changes.
/// Per-file problems are in the report.
pub fn apply_plan(
    plan: &Plan,
    settings: &Settings,
    opts: &ApplyOptions,
    progress: Progress<'_>,
) -> Result<ApplyReport> {
    validate_plan(plan)?;
    let now = Utc::now();
    let mut st = ApplyState {
        settings,
        opts,
        journal: journal::read_journal(&opts.journal_path)?,
        batch_id: journal::new_batch_id(now),
        now,
    };
    // Preflight: the journal must be writable before anything changes.
    let _run = journal::begin_run(&opts.journal_path, &st.batch_id)?;
    st.batch().in_progress = true;
    journal::write_journal(&opts.journal_path, &mut st.journal, Utc::now())?;

    let total = plan.items.len();
    let warnings = plan.warnings.clone();
    progress(&Event::ApplyStarted { total });
    let cancelled = || {
        opts.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::SeqCst))
    };
    let mut files = Vec::with_capacity(total);
    for (i, item) in plan.items.iter().enumerate() {
        let report = if cancelled() {
            cancelled_report(item)
        } else {
            let report = apply_item(item, plan, &mut st);
            if let Err(e) = journal::write_journal(&opts.journal_path, &mut st.journal, Utc::now())
            {
                files.push(report);
                return Err(journal_failed(&e, &files, total));
            }
            report
        };
        progress(&Event::FileApplied {
            index: i + 1,
            total,
            source: report.source.clone(),
            target: report.target.clone(),
            rename_status: report.rename_status,
            enrich_status: report.enrich_status.clone(),
            learn_status: report.learn_status,
            error: report.error.clone(),
        });
        files.push(report);
    }
    // Finish the batch: kept only if it recorded a change (never an empty batch).
    let has_batch = !st.batch().is_empty();
    if has_batch {
        st.batch().in_progress = false;
    } else {
        let id = st.batch_id.clone();
        journal::remove_batch(&mut st.journal, &id);
    }
    journal::write_journal(&opts.journal_path, &mut st.journal, Utc::now())
        .map_err(|e| journal_failed(&e, &files, total))?;
    let count = |f: &dyn Fn(&FileReport) -> bool| files.iter().filter(|r| f(r)).count();
    let summary = ApplySummary {
        total,
        renamed: count(&|r| r.rename_status == RenameStatus::Renamed),
        unchanged: count(&|r| r.rename_status == RenameStatus::Unchanged),
        failed: files
            .iter()
            .map(|r| {
                usize::from(r.error.is_some())
                    + usize::from(r.enrich_error.is_some())
                    + usize::from(r.learn_error.is_some())
            })
            .sum(),
        incomplete: count(&|r| r.error.is_none() && r.incomplete()),
        cancelled: count(&|r| r.rename_status == RenameStatus::Cancelled),
        enriched: count(&|r| r.enrich_status == "enriched"),
        learned: count(&|r| {
            matches!(
                r.learn_status,
                LearnStatus::AliasAdded | LearnStatus::EntryAdded
            )
        }),
    };
    Ok(ApplyReport {
        schema_version: SCHEMA_VERSION,
        batch_id: has_batch.then_some(st.batch_id),
        profile: plan.profile.clone(),
        provider: plan.provider.clone(),
        model: plan.model.clone(),
        warnings,
        summary,
        files,
    })
}

// ---------------------------------------------------------------------------
// Inspect (no AI call, no writes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TextReport {
    pub page_count: usize,
    pub pages_used: usize,
    pub chars: usize,
    /// Worst page's text-layer score 0..1 (readability, 0.75 cap on visual pages).
    pub quality: f64,
    /// Worst page's score after local OCR (per page: the OCR text's
    /// readability where OCR ran, else the text-layer score), when OCR ran here.
    pub ocr_quality: Option<f64>,
    pub threshold: f64,
    /// With the current settings, would OCR / vision run on any page? (OCR
    /// needs its models; vision only for pages still below the threshold.)
    pub would_ocr: bool,
    pub would_vision: bool,
    /// Per page used.
    pub pages: Vec<PageReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PageReport {
    /// 1-based page number.
    pub page: usize,
    /// Non-whitespace chars of the text layer.
    pub chars: usize,
    /// Text-layer score 0..1: readability, capped at 0.75 when `visual`.
    pub quality: f64,
    /// The page shows a significant picture or drawing.
    pub visual: bool,
    /// Readability of this page's full local OCR text, when OCR ran here.
    pub ocr_quality: Option<f64>,
    pub would_ocr: bool,
    pub would_vision: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Inspection {
    #[serde(with = "crate::fsx::path_serde")]
    #[schemars(with = "crate::fsx::path_serde::Repr")]
    pub path: PathBuf,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    /// `eligible` or `skipped:<reason>` (signed, certified, encrypted, pdf_a, …).
    pub enrich: String,
    pub encrypted: bool,
    pub signed: bool,
    pub text: Option<TextReport>,
    /// Embedded AutoRename data (attachment + XMP `arp:` properties), if any.
    pub embedded: Option<Value>,
    /// From the embedded data: were page images sent to the model when this
    /// file was renamed? `null` when not enriched.
    pub images_sent: Option<bool>,
    pub warnings: Vec<String>,
    pub error: Option<ErrorInfo>,
}

/// Output of `inspect`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct InspectReport {
    /// Always 4.
    pub schema_version: u32,
    pub files: Vec<Inspection>,
}

/// Inspect one PDF: embedded data, signature/encryption, text-layer quality.
pub fn inspect(path: &Path, cfg: &ExtractConfig) -> Inspection {
    let mut out = Inspection {
        path: std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
        size: None,
        sha256: None,
        enrich: "not_run".into(),
        encrypted: false,
        signed: false,
        text: None,
        embedded: None,
        images_sent: None,
        warnings: Vec::new(),
        error: None,
    };
    match std::fs::read(path) {
        Ok(b) => {
            out.size = Some(b.len() as u64);
            out.sha256 = Some(sha256_hex(&b));
        }
        Err(e) => {
            out.error = Some(ErrorInfo::from_error(&Error::io(path, e)));
            return out;
        }
    }
    match enrich::plan(path) {
        Ok(enrich::Assessment::Eligible) => out.enrich = "eligible".into(),
        Ok(enrich::Assessment::Skip(r)) => {
            out.enrich = format!("skipped:{}", r.code());
            out.encrypted = r == enrich::SkipReason::Encrypted;
            out.signed = matches!(
                r,
                enrich::SkipReason::Signed | enrich::SkipReason::Certified
            );
        }
        Err(e) => out.warnings.push(e.to_string()),
    }
    match enrich::read(path) {
        Ok(Some(data)) => {
            out.images_sent = Some(data.attachment.content.extraction.tier == "vision");
            out.embedded = serde_json::to_value(data).ok();
        }
        Ok(None) => {}
        Err(e) => out.warnings.push(e.to_string()),
    }
    // Local OCR runs here when it would in a real run (never an error:
    // missing models are a warning); vision never (no AI calls).
    let local_cfg = ExtractConfig {
        ocr: match cfg.ocr {
            crate::config::Toggle::Off => crate::config::Toggle::Off,
            _ => crate::config::Toggle::Auto,
        },
        vision: crate::config::Toggle::Off,
        ..cfg.clone()
    };
    match extract::extract_with(path, &local_cfg) {
        Ok(ex) => {
            out.warnings.extend(ex.warnings.iter().cloned());
            let thr = cfg.text_quality_threshold;
            let pages: Vec<PageReport> = ex
                .pages
                .iter()
                .map(|p| PageReport {
                    page: p.number,
                    chars: p.text.chars().filter(|c| !c.is_whitespace()).count(),
                    quality: p.text_score(),
                    visual: p.visual,
                    ocr_quality: p.ocr_quality,
                    would_ocr: p.ocr_quality.is_some() || cfg.ocr == crate::config::Toggle::On,
                    would_vision: extract::should_run(cfg.vision, p.quality, thr),
                    errors: p.errors.clone(),
                })
                .collect();
            let ocr_ran = ex.pages.iter().any(|p| p.ocr_quality.is_some());
            out.text = Some(TextReport {
                page_count: ex.page_count,
                pages_used: ex.pages_used,
                chars: ex.text.chars().count(),
                quality: extract::quality::document_quality(pages.iter().map(|p| p.quality)),
                ocr_quality: ocr_ran.then_some(ex.quality),
                threshold: thr,
                would_ocr: pages.iter().any(|p| p.would_ocr),
                would_vision: pages.iter().any(|p| p.would_vision),
                pages,
            });
        }
        Err(e) => out.error = Some(ErrorInfo::from_error(&e)),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_rules() {
        let e = |c: &str| ErrorInfo::new(c, "m", None);
        let auth = e("auth_error");
        let prov = e("provider_error");
        let io = e("io_error");
        let cfg = e("config_error");
        assert_eq!(batch_exit_code(2, []), ExitCode::Success);
        assert_eq!(batch_exit_code(2, [&auth]), ExitCode::Partial);
        assert_eq!(batch_exit_code(2, [&auth, &auth]), ExitCode::Auth);
        assert_eq!(batch_exit_code(2, [&auth, &prov]), ExitCode::Provider);
        assert_eq!(batch_exit_code(2, [&prov, &io]), ExitCode::General);
        assert_eq!(batch_exit_code(1, [&cfg]), ExitCode::Config);
    }

    fn report(error: bool, enrich_failed: bool) -> FileReport {
        let mut item = new_item(Path::new("/x/a.pdf"));
        item.error = None;
        let mut r = report_for(&item);
        if error {
            r.error = Some(ErrorInfo::new("io_error", "m", None));
        }
        if enrich_failed {
            r.enrich_error = Some(ErrorInfo::new("io_error", "m", None));
        }
        r
    }

    #[test]
    fn direction_is_derived_from_issuer_and_recipient() {
        let d = profiles::Direction {
            field: "document_type".into(),
            issuer: "issuer".into(),
            recipient: "recipient".into(),
            incoming: "AP".into(),
            outgoing: "AR".into(),
            counterparty: None,
        };
        let run = |ty: &str, issuer: &str, recipient: &str| {
            let mut f =
                serde_json::json!({"document_type": ty, "issuer": issuer, "recipient": recipient})
                    .as_object()
                    .unwrap()
                    .clone();
            let w = derive_direction(&d, &mut f, "Spqrk");
            (f["document_type"].as_str().unwrap().to_string(), w)
        };
        // The model said AR, but the bill is addressed to us → AP.
        assert_eq!(
            run("AR", "Springfield Power Co.", "Spqrk GmbH"),
            ("AP".into(), None)
        );
        assert_eq!(
            run("AP", "SPQRK GmbH", "Wayne Enterprises Inc."),
            ("AR".into(), None)
        );
        // Both/neither: keep the guess, warn.
        let (ty, w) = run("AP", "ACME", "Globex");
        assert_eq!(ty, "AP");
        assert!(w.unwrap().starts_with("direction_uncertain"));
        assert!(run("AR", "Spqrk", "Spqrk GmbH").1.is_some());
        // Not an invoice: untouched, no warning.
        assert_eq!(run("Letter", "ACME", "Spqrk"), ("Letter".into(), None));
        // A similar but different company is not mine.
        assert!(run("AP", "Spqrk Holding International", "ACME").1.is_some());
    }

    #[test]
    fn counterparty_is_never_my_company() {
        let d = profiles::Direction {
            field: "document_type".into(),
            issuer: "issuer".into(),
            recipient: "recipient".into(),
            incoming: "AP".into(),
            outgoing: "AR".into(),
            counterparty: Some("company_name".into()),
        };
        let run = |cp: &str, ty: &str, issuer: &str, recipient: &str| {
            let mut f = serde_json::json!({"company_name": cp, "document_type": ty, "issuer": issuer, "recipient": recipient})
                .as_object().unwrap().clone();
            let w = derive_direction(&d, &mut f, "Spqrk");
            (
                f["company_name"].as_str().unwrap().to_string(),
                f["document_type"].as_str().unwrap().to_string(),
                w,
            )
        };
        // The model's counterparty is kept (legal form stripped as asked).
        assert_eq!(
            run(
                "ACME Corporation",
                "AP",
                "ACME Corporation GmbH",
                "Spqrk GmbH"
            ),
            ("ACME Corporation".into(), "AP".into(), None)
        );
        // The model put my company as counterparty: the other side, without
        // its printed legal form.
        assert_eq!(
            run("Spqrk GmbH", "AR", "ACME Corp", "Spqrk GmbH"),
            ("ACME".into(), "AP".into(), None)
        );
        assert_eq!(
            run("Spqrk", "AP", "Spqrk", "Wayne Enterprises Inc."),
            ("Wayne Enterprises".into(), "AR".into(), None)
        );
        // No counterparty: the other side, stripped.
        assert_eq!(
            run("", "AP", "Mustermann Consulting GmbH", "Spqrk GmbH"),
            ("Mustermann Consulting".into(), "AP".into(), None)
        );
        assert_eq!(
            run("", "AP", "ACME,Inc.", "Spqrk GmbH"),
            ("ACME".into(), "AP".into(), None)
        );
        assert_eq!(
            run("", "AP", "Initech Solutions (AG)", "Spqrk GmbH"),
            ("Initech Solutions".into(), "AP".into(), None)
        );
        // Unresolvable and the guess is me → kept + warning.
        let (cp, _, w) = run("Spqrk", "Letter", "", "");
        assert_eq!(cp, "Spqrk");
        assert!(w.unwrap().starts_with("direction_uncertain"));
    }

    #[test]
    fn amount_is_formatted_deterministically() {
        let a = profiles::Amount {
            field: "description".into(),
            amount: "total_amount".into(),
            currency: "currency".into(),
        };
        let run = |amount: &str, currency: &str, desc: &str, fmt: &str| {
            let mut f = serde_json::json!({"total_amount": amount, "currency": currency, "description": desc})
                .as_object()
                .unwrap()
                .clone();
            let w = derive_amount(&a, &mut f, fmt);
            (f["description"].as_str().unwrap().to_string(), w)
        };
        let def = "{currency} {amount}";
        // The model's description text never decides the format.
        assert_eq!(
            run("4,258.80", "EUR", "4,258.80", def),
            ("EUR 4,258.80".into(), None)
        );
        assert_eq!(
            run("1.320,00", "EUR", "", "{amount} {currency}"),
            ("1.320,00 EUR".into(), None)
        );
        let (d, w) = run("4,258.80", "", "", def);
        assert_eq!(d, "4,258.80");
        assert!(w.unwrap().starts_with("currency_missing"));
        // No amount (letters, contracts): description untouched.
        assert_eq!(
            run("", "", "Contract renewal", def),
            ("Contract renewal".into(), None)
        );
    }

    #[test]
    fn enrich_or_learn_failures_are_partial() {
        let doc = |files| ApplyReport {
            schema_version: SCHEMA_VERSION,
            batch_id: None,
            profile: String::new(),
            provider: String::new(),
            model: String::new(),
            warnings: vec![],
            summary: ApplySummary::default(),
            files,
        };
        assert_eq!(
            doc(vec![report(false, false)]).exit_code(),
            ExitCode::Success
        );
        assert_eq!(
            doc(vec![report(false, true)]).exit_code(),
            ExitCode::Partial
        );
        assert_eq!(
            doc(vec![report(true, false)]).exit_code(),
            ExitCode::General
        );
    }

    fn unchanged_plan(dir: &Path) -> Plan {
        let src = dir.join("a.pdf");
        std::fs::write(&src, b"%PDF").unwrap();
        let mut item = new_item(&src);
        item.status = PlanStatus::Unchanged;
        item.source_sha256 = sha256_hex(b"%PDF");
        item.proposed_name = Some("a.pdf".into());
        item.target = Some(src);
        Plan {
            schema_version: SCHEMA_VERSION,
            created: String::new(),
            profile: "business".into(),
            provider: "ollama".into(),
            model: "m".into(),
            privacy: "cloud".into(),
            warnings: vec![],
            summary: PlanSummary::default(),
            items: vec![item],
        }
    }

    fn test_settings() -> Settings {
        let tree = crate::config::from_raw(
            serde_json::json!({"ai": {"provider": "ollama"}, "enrich": {"enabled": false}}),
            &|_| None,
        )
        .unwrap()
        .0;
        crate::config::settings(&tree, None).unwrap()
    }

    #[test]
    fn cancelled_apply_touches_nothing_and_is_not_a_failure() {
        let d = tempfile::tempdir().unwrap();
        let plan = unchanged_plan(d.path());
        let journal_path = d.path().join("journal.json");
        let opts = ApplyOptions {
            journal_path: journal_path.clone(),
            cancel: Some(Arc::new(AtomicBool::new(true))),
            ..Default::default()
        };
        let r = apply_plan(&plan, &test_settings(), &opts, &no_progress).unwrap();
        assert_eq!(r.summary.cancelled, 1);
        assert_eq!(r.files[0].rename_status, RenameStatus::Cancelled);
        assert_eq!(r.exit_code(), ExitCode::Success);
        assert!(r.batch_id.is_none());
        assert!(
            journal::read_journal(&journal_path)
                .unwrap()
                .batches
                .is_empty()
        );
        // Not cancelled: the batch is finished (not in progress).
        let opts = ApplyOptions {
            journal_path: journal_path.clone(),
            ..Default::default()
        };
        let r = apply_plan(&plan, &test_settings(), &opts, &no_progress).unwrap();
        // Nothing changed → no batch at all (no empty batches).
        assert!(r.batch_id.is_none());
        assert!(
            journal::read_journal(&journal_path)
                .unwrap()
                .batches
                .is_empty()
        );
    }

    #[test]
    fn summary_failed_counts_failed_sub_operations() {
        let d = tempfile::tempdir().unwrap();
        let mut plan = unchanged_plan(d.path());
        plan.items[0].learn = Some(LearnAction::NewEntry {
            name: "ACME".into(),
            review: false,
        });
        let lists = d.path().join("name-lists");
        let names = crate::config::name_list_in(&lists, "company_name");
        std::fs::create_dir_all(&lists).unwrap();
        std::fs::write(&names, "{ not: [valid").unwrap();
        let opts = ApplyOptions {
            journal_path: d.path().join("journal.json"),
            name_lists_dir: Some(lists),
            ..Default::default()
        };
        let r = apply_plan(&plan, &test_settings(), &opts, &no_progress).unwrap();
        assert!(r.files[0].learn_error.is_some());
        assert_eq!(r.summary.failed, 1);
        assert_eq!(r.exit_code(), ExitCode::Partial);
    }

    #[test]
    fn learning_is_per_field_and_undo_reverts_the_recorded_list() {
        let d = tempfile::tempdir().unwrap();
        let lists = d.path().join("name-lists");
        let journal_path = d.path().join("journal.json");
        let opts = ApplyOptions {
            journal_path: journal_path.clone(),
            name_lists_dir: Some(lists.clone()),
            ..Default::default()
        };
        let venues = {
            let tree = crate::config::from_raw(
                serde_json::json!({"ai": {"provider": "ollama"}, "enrich": {"enabled": false},
                    "profile": "academic", "profiles": {"academic": {"harmonize_field": "journal_name"}}}),
                &|_| None,
            )
            .unwrap()
            .0;
            crate::config::settings(&tree, None).unwrap()
        };
        let mut plan = unchanged_plan(d.path());
        plan.items[0].learn = Some(LearnAction::NewEntry {
            name: "Nature Ltd".into(),
            review: false,
        });
        let learned =
            |s: &Settings| apply_plan(&plan, s, &opts, &no_progress).unwrap().files[0].learn_status;
        // Business run: a company. Venue run: the same text is new in its
        // own list (never "known" from the company list).
        assert_eq!(learned(&test_settings()), LearnStatus::EntryAdded);
        assert_eq!(learned(&venues), LearnStatus::EntryAdded);
        let (companies, journals) = (
            crate::config::name_list_in(&lists, "company_name"),
            crate::config::name_list_in(&lists, "journal_name"),
        );
        let canon = |p: &Path, f: &str| -> Vec<String> {
            harmonize::load_db(p, f)
                .unwrap()
                .entries
                .into_iter()
                .map(|e| e.canonical)
                .collect()
        };
        assert_eq!(canon(&companies, "company_name"), ["Nature Ltd"]);
        assert_eq!(canon(&journals, "journal_name"), ["Nature Ltd"]);
        // Undo needs no profile: each batch reverts the list it recorded.
        let j = journal::read_journal(&journal_path).unwrap();
        let fields: Vec<&str> = j
            .batches
            .iter()
            .map(|b| b.aliases[0].field.as_str())
            .collect();
        assert_eq!(fields, ["company_name", "journal_name"]);
        let r = journal::undo_at(&journal_path, &journal::UndoTarget::Last, Utc::now()).unwrap();
        assert_eq!(r.aliases[0].file, journals);
        assert!(canon(&journals, "journal_name").is_empty());
        assert_eq!(canon(&companies, "company_name"), ["Nature Ltd"]);
        journal::undo_at(&journal_path, &journal::UndoTarget::Last, Utc::now()).unwrap();
        assert!(canon(&companies, "company_name").is_empty());
    }

    #[test]
    fn plan_validation_rejects_escapes() {
        let d = tempfile::tempdir().unwrap();
        let mut plan = unchanged_plan(d.path());
        assert!(validate_plan(&plan).is_ok());
        // Editing only proposed_name is the contract; target is ignored.
        plan.items[0].proposed_name = Some("b.pdf".into());
        plan.items[0].target = Some(std::env::temp_dir().join("elsewhere.conf"));
        assert!(validate_plan(&plan).is_ok());
        assert_eq!(
            effective_target(&plan.items[0]),
            Some(d.path().join("b.pdf"))
        );
        for name in ["../b.pdf", "b.conf", "sub/b.pdf", "CON.pdf", ""] {
            plan.items[0].proposed_name = Some(name.into());
            assert_eq!(
                validate_plan(&plan).unwrap_err().exit_code(),
                ExitCode::Usage,
                "{name}"
            );
        }
    }

    #[test]
    fn edited_proposed_name_is_applied_without_touching_target() {
        let d = tempfile::tempdir().unwrap();
        let mut plan = unchanged_plan(d.path());
        plan.items[0].proposed_name = Some("Edited name.pdf".into());
        let opts = ApplyOptions {
            journal_path: d.path().join("journal.json"),
            ..Default::default()
        };
        let r = apply_plan(&plan, &test_settings(), &opts, &no_progress).unwrap();
        assert_eq!(r.files[0].rename_status, RenameStatus::Renamed);
        assert!(d.path().join("Edited name.pdf").exists());
    }

    #[test]
    fn collect_sorts_dedups_and_filters() {
        let d = std::env::temp_dir().join(format!("arp-collect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        for n in ["b.pdf", "a.PDF", "c.txt", ".hidden.pdf", "sub/z.pdf"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        let c = collect_pdfs(&[d.clone(), d.join("b.pdf"), d.join("nope.pdf")], false);
        let names: Vec<_> = c
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.PDF", "b.pdf"]);
        assert_eq!(c.warnings.len(), 1);
        let c = collect_pdfs(std::slice::from_ref(&d), true);
        assert_eq!(c.files.len(), 3);
        let c = collect_pdfs(&[d.join("c.txt")], false);
        assert!(c.files.is_empty() && c.warnings[0].starts_with("Not a PDF"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
