//! GUI state machine (no egui): rows, phases, filters, plan assembly.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use autorename_core::config::{self, ConfigLocation, Settings};
use autorename_core::extract::Tier;
use autorename_core::journal::UndoReport;
use autorename_core::naming;
use autorename_core::pipeline::{
    ApplyReport, DecidedBy, EnrichPlan, ErrorInfo, FileReport, LearnAction, LearnStatus,
    MatchOutcome, Plan, PlanItem, PlanStatus, PlanSummary, RenameStatus, SCHEMA_VERSION,
};
use serde_json::Value;

use super::components::plan_row::RowStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Rename,
    Companies,
    History,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    /// Review and error rows (§4.3 "Needs a look").
    NeedsLook,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub source: PathBuf,
    pub item: Option<PlanItem>,
    /// Proposed name before collision suffixes (from `FileAnalyzed`).
    pub base_name: Option<String>,
    pub analyzing: bool,
    /// The run ended (Stop or a whole-run error) before this file was
    /// analyzed. The row stays visible with Retry; it is never dropped.
    pub not_analyzed: bool,
    /// Fingerprint of the settings this row's result was analyzed with
    /// (`fingerprint`); a result is stale when it differs from the current.
    pub fp: Option<u64>,
    pub checked: bool,
    /// User-edited final file name (with `.pdf`).
    pub edited: Option<String>,
    pub dont_learn: bool,
    pub report: Option<FileReport>,
}

impl Row {
    pub fn queued(source: PathBuf) -> Self {
        Row {
            source,
            item: None,
            base_name: None,
            analyzing: false,
            not_analyzed: false,
            fp: None,
            checked: false,
            edited: None,
            dont_learn: false,
            report: None,
        }
    }

    pub fn from_item(item: PlanItem) -> Self {
        let mut r = Row::queued(item.source.clone());
        r.checked = item.status != PlanStatus::Failed;
        r.item = Some(item);
        r
    }

    pub fn old_name(&self) -> String {
        file_name(&self.source)
    }

    pub fn status(&self) -> RowStatus {
        if let Some(rep) = &self.report {
            return match rep.rename_status {
                RenameStatus::Renamed => RowStatus::Renamed,
                RenameStatus::Unchanged => RowStatus::Unchanged,
                RenameStatus::Failed => RowStatus::Error,
                RenameStatus::Cancelled => RowStatus::Cancelled,
            };
        }
        let Some(item) = &self.item else {
            if self.not_analyzed && !self.analyzing {
                return RowStatus::Cancelled;
            }
            return if self.analyzing {
                RowStatus::Analyzing
            } else {
                RowStatus::Queued
            };
        };
        if item.status == PlanStatus::Failed || self.edit_error_static().is_some() {
            return RowStatus::Error;
        }
        if self.needs_review() {
            RowStatus::Review
        } else {
            RowStatus::Ready
        }
    }

    fn edit_error_static(&self) -> Option<String> {
        self.edited.as_deref().and_then(validate_name)
    }

    pub fn needs_review(&self) -> bool {
        self.item.as_ref().is_some_and(|i| {
            !i.warnings.is_empty()
                || matches!(i.learn, Some(LearnAction::NewEntry { review: true, .. }))
        })
    }

    /// Where the file is now: the rename target after a successful rename,
    /// unless undo restored that file (`restored` holds restored targets),
    /// else the source.
    pub fn current_path(&self, restored: &HashSet<PathBuf>) -> &std::path::Path {
        match &self.report {
            Some(rep) if rep.rename_status == RenameStatus::Renamed => match &rep.target {
                Some(t) if !restored.contains(t) => t,
                _ => &self.source,
            },
            _ => &self.source,
        }
    }

    pub fn is_error(&self) -> bool {
        self.status() == RowStatus::Error
    }

    pub fn checkable(&self) -> bool {
        self.item.is_some() && !self.is_error() && self.report.is_none()
    }

    pub fn suggested_name(&self) -> Option<&str> {
        self.item.as_ref()?.proposed_name.as_deref()
    }

    pub fn final_name(&self) -> Option<String> {
        if let Some(rep) = &self.report {
            return rep
                .name
                .clone()
                .or_else(|| self.suggested_name().map(str::to_string));
        }
        self.edited
            .clone()
            .or_else(|| self.suggested_name().map(str::to_string))
    }

    /// Short error summary for line 1 (§4.9): the rename failure of a
    /// result row, else the analysis error.
    pub fn error_summary(&self) -> Option<String> {
        if let Some(e) = self.report.as_ref().and_then(|r| r.error.as_ref()) {
            return Some(rename_error_copy(e));
        }
        Some(error_copy(self.item.as_ref()?.error.as_ref()?))
    }

    /// Line 2 problem sentence in plain words (§4.3); `noun` is the name
    /// list's singular ("company"). The raw warning stays in the details.
    /// Result rows show apply-time problems first (metadata, learning,
    /// disk), and nothing under a rename failure (line 1 says it).
    pub fn problem(&self, noun: &str) -> Option<String> {
        let item = self.item.as_ref()?;
        let warnings = match &self.report {
            Some(rep) if rep.error.is_some() => return None,
            Some(rep) => {
                let apply = rep.warnings.iter().find(|w| !item.warnings.contains(w));
                if let Some(w) = apply {
                    return Some(plain_warning(w));
                }
                &rep.warnings
            }
            None => &item.warnings,
        };
        if let Some(w) = warnings.first() {
            // The scan reader found nothing, but AI image reading ran: the
            // name comes from the image, so say that (not "no text").
            if w == OCR_NO_TEXT && item.tier == Some(Tier::VisionNeeded) && item.error.is_none() {
                return Some(OCR_NO_TEXT_VISION.into());
            }
            return Some(plain_warning(w));
        }
        let entry_added = self
            .report
            .as_ref()
            .is_none_or(|r| r.learn_status == LearnStatus::EntryAdded);
        match &item.learn {
            Some(LearnAction::NewEntry { review: true, .. }) if entry_added && !self.dont_learn => {
                Some(format!("New {noun}. Check the spelling."))
            }
            _ => None,
        }
    }
}

/// "Venue" → "venue", "Issuer names" → "issuer names" (keeps "ID").
pub fn noun(label: &str) -> String {
    super::components::stepper::lower_first(label)
}

pub fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn sentence(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Plain cause of a file-system error from its message (§4.9), if known.
/// `rename`: the error came from renaming (else from reading for analysis).
fn io_cause(e: &ErrorInfo, rename: bool) -> Option<&'static str> {
    let m = e.message.to_lowercase();
    let has = |n: &[&str]| n.iter().any(|x| m.contains(x));
    Some(
        if has(&[
            "another program",
            "another process",
            "os error 32",
            "os error 33",
            "resource busy",
        ]) {
            "File is open in another program. Close it and try again"
        } else if has(&[
            "permission denied",
            "access is denied",
            "os error 13",
            "os error 5)",
            "read-only",
        ]) {
            if rename {
                "No permission to change this file"
            } else {
                "Can't open this file (no permission)"
            }
        } else if has(&[
            "no space left",
            "disk full",
            "not enough space",
            "os error 28",
            "os error 112",
        ]) {
            "The disk is full. Free some space and try again"
        } else {
            return None;
        },
    )
}

/// Line 1 of a result row whose rename failed (§4.6, §4.9).
pub fn rename_error_copy(e: &ErrorInfo) -> String {
    match e.code.as_str() {
        "conflict" => "A file with the new name already exists".into(),
        "source_changed" => "File changed since analysis · Re-analyze".into(),
        _ => io_cause(e, true).map_or_else(|| "Couldn't rename the file".into(), str::to_string),
    }
}

/// Plain-words error copy (§4.3, §4.9, §7): no codes in the headline.
pub fn error_copy(e: &ErrorInfo) -> String {
    const READ: &str = "Couldn't read the file";
    match e.code.as_str() {
        "auth_error" => "The AI service rejected the API key".into(),
        "provider_error" if e.message.to_lowercase().contains("timed out") => {
            format!("{READ} · The AI service didn't answer in time")
        }
        "provider_error" => format!("{READ} · The AI service reported a problem"),
        "no_content" => format!("{READ} · No text was found in it"),
        "ocr_failed" => format!("{READ} · The scan couldn't be read"),
        "source_changed" => "File changed since analysis · Re-analyze".into(),
        "conflict" => "A file with the new name already exists".into(),
        "ocr_models_missing" => "This scan needs the scan reader, which isn't installed".into(),
        "io_error" => io_cause(e, false).map_or_else(|| READ.into(), str::to_string),
        _ => {
            let m = e.message.trim_end_matches('.');
            format!("{READ} · {}", sentence(m))
        }
    }
}

/// Plan files a rename did not touch, by reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// Analyzed but unchecked.
    pub not_selected: usize,
    /// The analysis failed.
    pub unread: usize,
    /// Never analyzed (Stop before the file was reached).
    pub not_analyzed: usize,
}

const OCR_NO_TEXT: &str = "OCR returned no text";
const OCR_NO_TEXT_VISION: &str = "The local scan reader couldn't read this page. AI read its image; compare the name with the original.";

/// Core warning strings (`PlanItem::warnings`: extraction, direction,
/// amount, name matching) as one plain sentence for office users (§4.3).
/// Unknown strings fall back to the raw text as a sentence.
pub fn plain_warning(w: &str) -> String {
    let code = w.split_once(':').map_or(w, |(c, _)| c).trim();
    let plain = match code {
        // Own company read as the other party: the company is wrong too.
        "direction_uncertain" if w.contains("(your own company) as the counterparty") => {
            "Your own company was read as the other party. Check the company and type."
        }
        "currency_missing" => "No currency was found. Check the amount.",
        "direction_uncertain" => "It's unclear whether you sent or received this. Check the type.",
        "counterparty_missing" => "No company name was found. Check the company.",
        "visual_scan_incomplete" => "Some pages couldn't be checked fully. Check the name.",
        "ocr_models_missing" => "This scan needs the scan reader, which isn't installed.",
        // Apply-time problems (result rows).
        "Enrichment failed" => "Renamed, but the document details couldn't be saved in it.",
        "durability_warning" => "Renamed, but saving wasn't fully confirmed. Check the file.",
        "Could not open PDF" => "The PDF couldn't be opened. Check the file.",
        "OCR returned no text" => "No text was recognized in the scan. Check the name.",
        c if c.starts_with("Could not extract text from page") => {
            "Some text couldn't be read. Check the name."
        }
        c if c.starts_with("Could not render page") || c.starts_with("OCR failed on page") => {
            "A page couldn't be read. Check the name."
        }
        c if c.starts_with("ocr_") || c == "pdfium_missing" => {
            "Text recognition wasn't available. Check the name."
        }
        c if c.ends_with(" not learned") => {
            let field = noun(c.trim_end_matches(" not learned"));
            return format!("Renamed, but the {field} couldn't be added to your list.");
        }
        c if c.ends_with(" decider failed") => {
            let field = noun(c.trim_end_matches(" decider failed"));
            return format!("The {field} couldn't be checked against your list. Check the name.");
        }
        _ => return sentence(w),
    };
    plain.into()
}

/// Display name of a profile id ("business" → "Business").
pub fn profile_label(id: &str) -> String {
    match id {
        "business" => "Business".into(),
        "academic" => "Academic".into(),
        other => {
            let mut c = other.replace(['_', '-'], " ");
            if let Some(f) = c.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            c
        }
    }
}

/// Core filename rules (§4.3 inline edit). `None` = valid.
pub fn validate_name(name: &str) -> Option<String> {
    let base = name.strip_suffix(".pdf").unwrap_or(name);
    if base.trim().is_empty() {
        return Some("Name can't be empty".into());
    }
    if naming::sanitize_filename(base) != base {
        return Some("Name contains characters that aren't allowed in file names".into());
    }
    if !naming::fits(base, naming::MAX_BASE) {
        return Some("Name is too long".into());
    }
    if naming::is_reserved_device(base) {
        return Some("Name is reserved by Windows".into());
    }
    None
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub total: usize,
    pub done: usize,
    pub started: Instant,
    /// Seconds elapsed when `done` was reached (for the ETA).
    pub done_at: Vec<f32>,
}

impl Analysis {
    /// ETA after 3 files (§4.5).
    pub fn eta_secs(&self) -> Option<f32> {
        if self.done < 3 || self.done >= self.total {
            return None;
        }
        let elapsed = *self.done_at.last()?;
        let per = elapsed / self.done as f32;
        Some(per * (self.total - self.done) as f32)
    }
}

#[derive(Clone, Debug)]
pub enum Phase {
    Idle,
    Analyzing(Analysis),
    Renaming { done: usize, total: usize },
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanMeta {
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub privacy: String,
}

#[derive(Clone, Debug)]
pub struct ConfigState {
    pub location: Option<ConfigLocation>,
    pub tree: Option<Value>,
    pub settings: Option<Settings>,
    pub error: Option<String>,
    pub profiles: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    /// Company entries flagged `review: true` (Companies tab badge).
    pub review_count: usize,
    /// `ai.api_key` as written in config.yaml (before `${VAR}` expansion).
    pub raw_key: Option<String>,
    /// `harmonization.decider_api_key` as written in config.yaml.
    pub raw_decider_key: Option<String>,
}

/// Where the API key comes from (keys belong in `.env` beside the config).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// `${VAR}` resolved from the process environment.
    Process(String),
    /// `${VAR}` resolved from `.env` beside config.yaml.
    DotEnv(String),
    /// Legacy literal key in config.yaml.
    Literal,
    /// `${VAR}` that resolves nowhere, or no key.
    Missing(String),
}

impl ConfigState {
    /// Key source for `var` (the provider's env variable).
    pub fn key_source(&self, var: &str) -> KeySource {
        self.source_of(self.raw_key.as_deref().unwrap_or(""), var)
    }

    /// Decider key status for `decider` (config id) with main `provider`.
    /// `None` = the decider needs no own key (ai, off, or openai-decisions
    /// on the OpenAI provider, which uses the main OpenAI key).
    pub fn decider_key_source(&self, decider: &str, provider: &str) -> Option<KeySource> {
        if !matches!(decider, "openai-decisions" | "jev") {
            return None;
        }
        let var = decider_key_var(decider, provider)?;
        let raw = self.raw_decider_key.as_deref().unwrap_or("");
        Some(if raw.trim().is_empty() {
            // Empty in config: core falls back to `var` from env / .env.
            self.source_of(&format!("${{{var}}}"), var)
        } else {
            self.source_of(raw, var)
        })
    }

    /// Key source of a `${var}` reference (provider or decider switch).
    pub fn env_source(&self, var: &str) -> KeySource {
        self.source_of(&format!("${{{var}}}"), var)
    }

    fn source_of(&self, raw: &str, var: &str) -> KeySource {
        let referenced = raw
            .trim()
            .strip_prefix("${")
            .and_then(|r| r.strip_suffix('}'))
            .map(str::to_string);
        match referenced {
            Some(v) if std::env::var(&v).is_ok_and(|x| !x.is_empty()) => KeySource::Process(v),
            Some(v) if self.env.get(&v).is_some_and(|x| !x.is_empty()) => KeySource::DotEnv(v),
            Some(v) => KeySource::Missing(v),
            None if !raw.trim().is_empty() => KeySource::Literal,
            None => KeySource::Missing(var.to_string()),
        }
    }
}

impl ConfigState {
    pub fn load(cli: Option<&Path>, offline: bool) -> Self {
        let loc = config::discover_config(cli);
        let env = config::read_dotenv(&loc.path);
        let mut st = ConfigState {
            location: Some(loc.clone()),
            tree: None,
            settings: None,
            error: None,
            profiles: Vec::new(),
            env,
            review_count: 0,
            raw_key: None,
            raw_decider_key: None,
        };
        if !loc.path.exists() {
            st.error = Some("No config file yet".into());
            return st;
        }
        let raw = std::fs::read_to_string(&loc.path)
            .ok()
            .and_then(|t| config::parse_yaml(&t).ok());
        let at = |sec: &str, key: &str| {
            raw.as_ref()
                .and_then(|v| v.get(sec)?.get(key)?.as_str().map(str::to_string))
        };
        st.raw_key = at("ai", "api_key");
        st.raw_decider_key = at("harmonization", "decider_api_key");
        match config::load(&loc.path) {
            Ok(loaded) => {
                let mut tree = loaded.tree;
                config::apply_overrides(
                    &mut tree,
                    &config::Overrides {
                        offline,
                        ..Default::default()
                    },
                );
                st.profiles = autorename_core::profiles::resolve_profiles(&tree)
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                match config::settings(&tree, None) {
                    Ok(s) => st.settings = Some(s),
                    Err(e) => st.error = Some(e.to_string()),
                }
                st.tree = Some(tree);
            }
            Err(e) => st.error = Some(e.to_string()),
        }
        st.review_count = st.count_reviews();
        st
    }

    /// Switch the active profile in memory (tests; the GUI switches
    /// profiles only by saving Settings).
    #[cfg(test)]
    pub fn select_profile(&mut self, id: &str) {
        if let Some(tree) = &self.tree {
            match config::settings(tree, Some(id)) {
                Ok(s) => {
                    self.settings = Some(s);
                    self.error = None;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        self.review_count = self.count_reviews();
    }

    /// Name list of the active profile's `harmonize_field`: `(path, field)`.
    /// `None` when the profile matches no names (or there is no config).
    pub fn name_list(&self) -> Option<(PathBuf, String)> {
        let field = self.settings.as_ref()?.profile.harmonize_field.clone()?;
        let loc = self.location.as_ref()?;
        Some((config::name_list_path(&loc.path, &field), field))
    }

    /// The active profile's `harmonize_field` (`None` = no name matching).
    /// Without a valid config the built-in business profile applies.
    pub fn name_field(&self) -> Option<String> {
        match &self.settings {
            Some(s) => s.profile.harmonize_field.clone(),
            None => Some(autorename_core::harmonize::COMPANY_FIELD.to_string()),
        }
    }

    fn count_reviews(&self) -> usize {
        self.name_list()
            .and_then(|(p, f)| autorename_core::harmonize::load_db(&p, &f).ok())
            .map_or(0, |db| db.entries.iter().filter(|e| e.review).count())
    }
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub until: f64,
}

#[derive(Clone, Debug)]
pub struct BannerMsg {
    pub tone: super::theme::Tone,
    pub title: String,
    pub body: String,
    pub action: Option<String>,
}

pub struct Model {
    pub tab: Tab,
    /// Tab to return to when Settings closes (gear / Esc).
    pub prev_tab: Tab,
    /// Settings › About document dialog (license, third-party notices).
    pub doc_open: Option<super::pages::TextDoc>,
    pub rows: Vec<Row>,
    pub active: Option<usize>,
    pub filter: Filter,
    pub phase: Phase,
    pub meta: Option<PlanMeta>,
    pub report: Option<ApplyReport>,
    /// Files of the plan the last rename did not touch. They leave the
    /// result list; the Done card counts them (§4.6).
    pub left_out: LeftOut,
    pub undo: Option<UndoReport>,
    pub undone: bool,
    /// Rename targets the undo of the current batch put back (per file:
    /// a failed restore keeps its target).
    pub restored: HashSet<PathBuf>,
    pub config: ConfigState,
    pub detail_visible: bool,
    /// User-resized detail width (`None` = φ formula).
    pub detail_w: Option<f32>,
    pub editing: Option<(usize, String)>,
    pub banners: Vec<BannerMsg>,
    pub toasts: Vec<Toast>,
    pub confirm_stop: bool,
    pub demo: bool,
    pub generation: u64,
    /// Onboarding card state.
    pub setup: SetupForm,
    /// Setup-card "Test connection" result.
    pub setup_test: Option<Result<u64, String>>,
    pub setup_testing: bool,
    /// Batch id of an undo still running (blocks conflicting actions).
    pub undoing: Option<String>,
    /// `--offline`: cloud requests and model downloads are refused.
    pub force_offline: bool,
    /// Fingerprint of the settings the running/last analysis used; stamped
    /// on every row its results land in.
    pub run_fp: Option<u64>,
    /// Undo journal path shown in Settings › General (injectable for tests).
    pub journal_display: String,
    pub settings: super::pages::SettingsState,
    pub prefs: super::pages::Prefs,
    pub companies: super::pages::CompaniesState,
    pub history: super::pages::HistoryState,
}

#[derive(Clone, Debug, Default)]
pub struct SetupForm {
    pub offline: bool,
    pub provider: usize,
    pub model: String,
    pub api_key: String,
    pub show_key: bool,
    pub ollama_url: String,
    /// `company.name` (optional; asked only for profiles with `direction`).
    pub company: String,
    pub error: Option<String>,
}

/// Provider id and display name; the API-key variable comes from core
/// (`config::default_key_var`, see [`key_var`]).
pub const PROVIDERS: [(&str, &str); 5] = [
    ("openai", "OpenAI"),
    ("anthropic", "Anthropic"),
    ("gemini", "Gemini"),
    ("xai", "xAI"),
    ("ollama", "Ollama"),
];

/// Default API-key variable of a provider ("" for Ollama).
pub fn key_var(id: &str) -> &'static str {
    config::default_key_var(id).unwrap_or("")
}

/// The provider's own page for creating an API key (setup card "Get a
/// key"); `None` for Ollama. Checked 2026-11-09 against the providers' docs.
pub fn key_url(id: &str) -> Option<&'static str> {
    match id {
        "openai" => Some("https://platform.openai.com/api-keys"),
        "anthropic" => Some("https://platform.claude.com/settings/keys"),
        "gemini" => Some("https://aistudio.google.com/apikey"),
        "xai" => Some("https://console.x.ai/team/default/api-keys"),
        _ => None,
    }
}

pub fn provider_label(id: &str) -> &str {
    PROVIDERS.iter().find(|p| p.0 == id).map_or(id, |p| p.1)
}

impl Model {
    pub fn new(config: ConfigState) -> Self {
        // Setup card pre-fill from an existing config.
        let company = config
            .tree
            .as_ref()
            .and_then(|t| t.pointer("/company/name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Model {
            tab: Tab::Rename,
            prev_tab: Tab::Rename,
            doc_open: None,
            rows: Vec::new(),
            active: None,
            filter: Filter::All,
            phase: Phase::Idle,
            meta: None,
            report: None,
            undo: None,
            undone: false,
            restored: HashSet::new(),
            left_out: LeftOut::default(),
            config,
            // Expert view, opt-in (§3).
            detail_visible: false,
            detail_w: None,
            editing: None,
            banners: Vec::new(),
            toasts: Vec::new(),
            confirm_stop: false,
            demo: false,
            generation: 0,
            setup: SetupForm {
                model: config::default_model("openai")
                    .unwrap_or_default()
                    .to_string(),
                ollama_url: "http://localhost:11434".into(),
                company,
                ..Default::default()
            },
            setup_test: None,
            setup_testing: false,
            undoing: None,
            force_offline: false,
            run_fp: None,
            journal_display: autorename_core::journal::default_journal_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            settings: Default::default(),
            prefs: Default::default(),
            companies: Default::default(),
            history: Default::default(),
        }
    }

    /// `(singular, plural)` of the active name list (core `field_label`,
    /// e.g. Company / Companies); `None` when the profile matches no names.
    pub fn name_label(&self) -> Option<(String, String)> {
        self.config
            .name_field()
            .map(|f| autorename_core::harmonize::field_label(&f))
    }

    /// Lower-case nouns for running copy ("company", "companies").
    pub fn name_nouns(&self) -> (String, String) {
        let (s, p) = self
            .name_label()
            .unwrap_or_else(|| autorename_core::harmonize::field_label("company_name"));
        (noun(&s), noun(&p))
    }

    pub fn is_set_up(&self) -> bool {
        self.config.settings.is_some()
    }

    pub fn analyzing(&self) -> bool {
        matches!(self.phase, Phase::Analyzing(_))
    }

    pub fn renaming(&self) -> bool {
        matches!(self.phase, Phase::Renaming { .. })
    }

    /// Model downloads are refused under `--offline` or privacy: offline.
    pub fn downloads_blocked(&self) -> bool {
        self.force_offline
            || self
                .config
                .settings
                .as_ref()
                .is_some_and(|s| s.privacy == config::Privacy::Offline)
    }

    /// Whether row `i` offers Retry: analysis errors before renaming; after
    /// renaming, files that failed or were not reached (never renamed ones).
    pub fn retryable(&self, i: usize) -> bool {
        let Some(r) = self.rows.get(i) else {
            return false;
        };
        match &r.report {
            Some(rep) => matches!(
                rep.rename_status,
                RenameStatus::Failed | RenameStatus::Cancelled
            ),
            None => r.is_error() || r.status() == RowStatus::Cancelled,
        }
    }

    pub fn retry_targets(&self) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&i| self.retryable(i))
            .collect()
    }

    /// The visible Rename primary is enabled: something checked, nothing
    /// running, no inline edit pending, no result shown.
    pub fn can_rename(&self) -> bool {
        let current = self.current_fp();
        self.report.is_none()
            && !self.busy()
            && self.editing.is_none()
            && self.counts().checked > 0
            && self
                .rows
                .iter()
                .filter(|r| r.checked && r.checkable())
                .all(|r| r.fp.is_some() && r.fp == current)
    }

    /// Fingerprint of the current effective settings (config + `--offline`,
    /// which `ConfigState::load` already applied).
    pub fn current_fp(&self) -> Option<u64> {
        self.config.settings.as_ref().map(fingerprint)
    }

    /// A row's result was made with other settings than the current ones.
    pub fn is_stale(&self, r: &Row) -> bool {
        r.item.is_some() && r.report.is_none() && r.fp != self.current_fp()
    }

    pub fn stale_count(&self) -> usize {
        self.rows.iter().filter(|r| self.is_stale(r)).count()
    }

    /// Mark every analyzed row as made with the current settings (demo and
    /// fixtures, where no worker run stamps them).
    pub fn stamp_current(&mut self) {
        let fp = self.current_fp();
        for r in &mut self.rows {
            if r.item.is_some() {
                r.fp = fp;
            }
        }
    }

    /// Rows that need an analysis with the current settings: stale results
    /// and rows without a result (waiting / not analyzed).
    pub fn needs_analysis(&self, r: &Row) -> bool {
        r.report.is_none() && (r.item.is_none() || self.is_stale(r))
    }

    /// §4.1 details toggle: only on Rename while a plan or result exists.
    pub fn details_toggle_shown(&self) -> bool {
        self.tab == Tab::Rename && (!self.rows.is_empty() || self.report.is_some())
    }

    pub fn busy(&self) -> bool {
        self.analyzing() || self.renaming() || self.undoing.is_some()
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for r in &self.rows {
            c.total += 1;
            match r.status() {
                RowStatus::Ready
                | RowStatus::Renamed
                | RowStatus::Unchanged
                | RowStatus::Cancelled => {}
                RowStatus::Review => c.review += 1,
                RowStatus::Error => c.errors += 1,
                RowStatus::Queued | RowStatus::Analyzing => c.pending += 1,
            }
            if r.checked && r.checkable() {
                c.checked += 1;
            }
            if r.edited.is_some() {
                c.edited += 1;
            }
        }
        c
    }

    pub fn visible(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| match self.filter {
                Filter::All => true,
                Filter::NeedsLook => matches!(r.status(), RowStatus::Review | RowStatus::Error),
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Paths queued for analysis: new rows appended (dedup by path).
    pub fn enqueue(&mut self, files: &[PathBuf]) -> Vec<PathBuf> {
        let mut added = Vec::new();
        for f in files {
            if self.rows.iter().any(|r| &r.source == f) {
                continue;
            }
            self.rows.push(Row::queued(f.clone()));
            added.push(f.clone());
        }
        if self.active.is_none() && !self.rows.is_empty() {
            self.active = Some(0);
        }
        added
    }

    /// Merge analyzed items; re-suffix names that collide with rows from an
    /// earlier run ("Add…" while a plan is shown). Cancelled items (Stop)
    /// leave the list.
    pub fn merge_items(&mut self, items: Vec<PlanItem>) {
        for mut item in items {
            if item.status == PlanStatus::Cancelled {
                if let Some(r) = self.rows.iter_mut().find(|r| r.source == item.source) {
                    r.analyzing = false;
                    r.not_analyzed = true;
                }
                continue;
            }
            let Some(idx) = self.rows.iter().position(|r| r.source == item.source) else {
                continue;
            };
            if item.status == PlanStatus::Rename
                && let Some(target) = item.target.clone()
            {
                let taken: Vec<PathBuf> = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != idx)
                    .filter_map(|(_, r)| target_of(r))
                    .collect();
                if taken.contains(&target) {
                    let base = self.rows[idx]
                        .base_name
                        .clone()
                        .or_else(|| item.proposed_name.clone())
                        .unwrap_or_default();
                    if let naming::Target::Path(p) =
                        naming::plan_target(&item.source, &base, &taken, |p| p.exists())
                    {
                        item.proposed_name = Some(file_name(&p));
                        item.target = Some(p);
                    }
                }
            }
            let fp = self.run_fp;
            let row = &mut self.rows[idx];
            row.fp = fp;
            row.analyzing = false;
            row.checked = item.status != PlanStatus::Failed;
            row.item = Some(item);
        }
    }

    pub fn toggle(&mut self, idx: usize) {
        if let Some(r) = self.rows.get_mut(idx)
            && r.checkable()
        {
            r.checked = !r.checked;
        }
    }

    /// Plan with the checked, edited rows (apply input, no new AI calls).
    pub fn build_plan(&self) -> Option<Plan> {
        let meta = self.meta.clone()?;
        let items: Vec<PlanItem> = self
            .rows
            .iter()
            .filter(|r| r.checked && r.checkable())
            .filter_map(|r| {
                let mut it = r.item.clone()?;
                if let Some(name) = &r.edited {
                    let target = it.source.with_file_name(name);
                    it.status = if target == it.source {
                        PlanStatus::Unchanged
                    } else {
                        PlanStatus::Rename
                    };
                    it.proposed_name = Some(name.clone());
                    it.target = Some(target);
                }
                if r.dont_learn {
                    it.learn = None;
                }
                Some(it)
            })
            .collect();
        if items.is_empty() {
            return None;
        }
        let count = |s: PlanStatus| items.iter().filter(|i| i.status == s).count();
        Some(Plan {
            schema_version: SCHEMA_VERSION,
            created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            profile: meta.profile,
            provider: meta.provider,
            model: meta.model,
            privacy: meta.privacy,
            warnings: Vec::new(),
            summary: PlanSummary {
                total: items.len(),
                rename: count(PlanStatus::Rename),
                unchanged: count(PlanStatus::Unchanged),
                failed: count(PlanStatus::Failed),
                cancelled: 0,
            },
            items,
        })
    }

    /// Collision check for an inline edit (§4.3): other rows + existing files.
    pub fn edit_error(&self, idx: usize, name: &str) -> Option<String> {
        let full = if name.ends_with(".pdf") {
            name.to_string()
        } else {
            format!("{name}.pdf")
        };
        if let Some(e) = validate_name(&full) {
            return Some(e);
        }
        let row = self.rows.get(idx)?;
        let target = row.source.with_file_name(&full);
        if target != row.source {
            if self
                .rows
                .iter()
                .enumerate()
                .any(|(i, r)| i != idx && target_of(r).as_ref() == Some(&target))
            {
                return Some("Another file in this list already uses this name".into());
            }
            if !self.demo && target.exists() {
                return Some("A file with this name already exists in the folder".into());
            }
        }
        None
    }

    pub fn commit_edit(&mut self, idx: usize, name: &str) -> bool {
        if self.edit_error(idx, name).is_some() {
            return false;
        }
        let full = if name.ends_with(".pdf") {
            name.to_string()
        } else {
            format!("{name}.pdf")
        };
        if let Some(r) = self.rows.get_mut(idx) {
            r.edited = if r.suggested_name() == Some(full.as_str()) {
                None
            } else {
                Some(full)
            };
        }
        true
    }

    pub fn apply_report(&mut self, report: ApplyReport) {
        self.restored.clear();
        for f in &report.files {
            if let Some(r) = self.rows.iter_mut().find(|r| r.source == f.source) {
                r.report = Some(f.clone());
            }
        }
        // Excluded rows leave the result list; every result stays visible.
        let mut left = LeftOut::default();
        for r in self.rows.iter().filter(|r| r.report.is_none()) {
            match &r.item {
                None => left.not_analyzed += 1,
                Some(_) if r.is_error() => left.unread += 1,
                Some(_) => left.not_selected += 1,
            }
        }
        self.left_out = left;
        self.rows.retain(|r| r.report.is_some());
        self.filter = Filter::All;
        self.active = (!self.rows.is_empty()).then_some(0);
        self.report = Some(report);
        self.phase = Phase::Done;
    }

    /// Keep the active row and the inline edit inside the visible rows
    /// (after a filter change): else the first visible row, or none.
    pub fn sync_active(&mut self) {
        let visible = self.visible();
        if self.active.is_some_and(|a| !visible.contains(&a)) {
            self.active = visible.first().copied();
        }
        if self
            .editing
            .as_ref()
            .is_some_and(|(e, _)| !visible.contains(e))
        {
            self.editing = None;
        }
    }

    /// New name-list entries flagged for review that this batch actually
    /// created (report `EntryAdded`, renamed or unchanged), deduplicated.
    pub fn review_entries(&self) -> usize {
        if self.undone {
            return 0;
        }
        let mut names: Vec<String> = self
            .rows
            .iter()
            .filter(|r| {
                r.report
                    .as_ref()
                    .is_some_and(|f| f.learn_status == LearnStatus::EntryAdded)
            })
            .filter_map(|r| match &r.item.as_ref()?.learn {
                Some(LearnAction::NewEntry { name, review: true }) => Some(name.to_lowercase()),
                _ => None,
            })
            .collect();
        names.sort();
        names.dedup();
        names.len()
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.active = None;
        self.report = None;
        self.left_out = LeftOut::default();
        self.undo = None;
        self.undone = false;
        self.restored.clear();
        self.meta = None;
        self.run_fp = None;
        self.editing = None;
        self.phase = Phase::Idle;
        self.filter = Filter::All;
    }
}

fn target_of(r: &Row) -> Option<PathBuf> {
    if let Some(n) = &r.edited {
        return Some(r.source.with_file_name(n));
    }
    let it = r.item.as_ref()?;
    (it.status == PlanStatus::Rename)
        .then(|| it.target.clone())
        .flatten()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub total: usize,
    pub review: usize,
    pub errors: usize,
    pub pending: usize,
    pub checked: usize,
    pub edited: usize,
}

impl Counts {
    /// Rows that need a look (§4.3): review + error.
    pub fn needs_look(&self) -> usize {
        self.review + self.errors
    }
}

pub fn tier_label(t: Option<Tier>) -> Option<&'static str> {
    Some(match t? {
        Tier::Text => "Text",
        Tier::Ocr => "OCR",
        Tier::VisionNeeded => "Vision",
    })
}

pub fn skip_reason_copy(reason: &str) -> String {
    match reason {
        "signed" => "signed PDF".into(),
        "certified" => "certified PDF".into(),
        "encrypted" => "encrypted PDF".into(),
        "pdf_a" => "PDF/A archive file".into(),
        "e_invoice" => "e-invoice (ZUGFeRD/Factur-X)".into(),
        "disabled" => "metadata writing is off".into(),
        other => other.replace('_', " "),
    }
}

/// Company-match evidence for the detail pane (§4.4), from core's
/// `PlanItem.company_match`. Scores are shown only when core supplies them.
pub struct MatchView {
    pub extracted: String,
    pub canonical: String,
    pub outcome: MatchOutcome,
    pub score: Option<f32>,
    pub band: Option<String>,
    /// Candidates listed for decider choices and review cases.
    pub candidates: Vec<(String, f32)>,
    pub decided_by: DecidedBy,
}

impl MatchView {
    pub fn from_item(item: &PlanItem) -> Option<Self> {
        let cm = item.company_match.as_ref()?;
        let show_candidates = matches!(cm.outcome, MatchOutcome::Decider | MatchOutcome::Review);
        Some(MatchView {
            extracted: cm.extracted.clone(),
            canonical: item.company.clone().unwrap_or_else(|| cm.extracted.clone()),
            outcome: cm.outcome,
            score: cm.score,
            band: cm.band.clone(),
            candidates: if show_candidates {
                cm.candidates
                    .iter()
                    .map(|c| (c.name.clone(), c.score))
                    .collect()
            } else {
                Vec::new()
            },
            decided_by: cm.decided_by,
        })
    }

    /// `one`/`many`: lower-case nouns of the list ("company", "companies").
    pub fn headline(&self, one: &str, many: &str) -> String {
        let (raw, can) = (&self.extracted, &self.canonical);
        match self.outcome {
            MatchOutcome::Known => format!("\u{201c}{raw}\u{201d} → {can} · known {one}"),
            MatchOutcome::Alias => {
                format!("\u{201c}{raw}\u{201d} → {can} · spelling variant of a known {one}")
            }
            MatchOutcome::Decider => {
                format!(
                    "\u{201c}{raw}\u{201d} → {can} · chosen by {}",
                    decider_copy(self.decided_by)
                )
            }
            MatchOutcome::New => {
                format!(
                    "\u{201c}{raw}\u{201d} matches no known {one} · new entry \u{201c}{can}\u{201d}"
                )
            }
            MatchOutcome::Review => format!(
                "\u{201c}{raw}\u{201d} is close to known {many} · new entry \u{201c}{can}\u{201d} needs review"
            ),
        }
    }

    /// "Similarity 0.95 ≥ 0.92: added as alias" etc. `None` without a score.
    pub fn evidence(&self) -> Option<String> {
        let sc = self.score?;
        if self.outcome == MatchOutcome::Known {
            return Some("Exact match after normalizing (similarity 1.00)".into());
        }
        let band = self.band.as_deref().unwrap_or("");
        let why = match (band, self.outcome) {
            (">=0.92", _) => "≥ 0.92: added as an alias automatically".to_string(),
            (_, MatchOutcome::Review) if self.decided_by == DecidedBy::None => {
                "below 0.92 with the decider off: new entry flagged for review".into()
            }
            (_, MatchOutcome::Review) => {
                "the decider gave no verdict: new entry flagged for review".into()
            }
            ("0.75-0.92", MatchOutcome::New) => format!(
                "between 0.75 and 0.92: {} chose a new entry",
                decider_copy(self.decided_by)
            ),
            ("0.75-0.92", _) => format!(
                "between 0.75 and 0.92: {} chose among the candidates",
                decider_copy(self.decided_by)
            ),
            ("<0.75", _) => "< 0.75: new entry".into(),
            _ => String::new(),
        };
        let mut s = format!("Best similarity {sc:.2}");
        if !why.is_empty() {
            s.push_str(&format!(" · {why}"));
        }
        if self.decided_by == DecidedBy::Offline {
            s.push_str(" · decided offline by the local model");
        }
        Some(s)
    }
}

pub fn decider_copy(d: DecidedBy) -> &'static str {
    match d {
        DecidedBy::None => "no decider",
        DecidedBy::Ai => "the AI decider",
        DecidedBy::OpenaiDecisions => "OpenAI Decisions",
        DecidedBy::Jev => "Jev",
        DecidedBy::Offline => "the local model",
    }
}

/// Env variable a remote decider's key lives in (`None` = it uses the main
/// OpenAI key: openai-decisions on the OpenAI provider). Mirrors core's
/// `config::decider_key_fallback`.
pub fn decider_key_var(decider: &str, provider: &str) -> Option<&'static str> {
    config::decider_key_fallback(decider, provider)
}

/// Settings fingerprint over everything that can change a plan row
pub fn fingerprint(s: &Settings) -> u64 {
    use std::hash::{Hash, Hasher};
    // Every setting that can change a proposed name, tier, badge or warning;
    // only pure performance knobs (OCR threads, retries) are left out.
    let mut v = serde_json::to_value(s).unwrap_or_default();
    v["ocr"]["threads"] = serde_json::Value::Null;
    v["ai"]["max_retries"] = serde_json::Value::Null;
    v["provider_issues"] = serde_json::Value::Null;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    h.finish()
}

/// Words for AccessKit row labels (§8).
pub fn row_a11y(r: &Row) -> String {
    let old = r.old_name();
    let mut parts = vec![old];
    match r.status() {
        RowStatus::Queued => parts.push("waiting for analysis".into()),
        RowStatus::Analyzing => parts.push("analyzing".into()),
        RowStatus::Error => parts.push(format!("error: {}", r.error_summary().unwrap_or_default())),
        RowStatus::Renamed => {
            parts.push(format!("renamed to {}", r.final_name().unwrap_or_default()))
        }
        RowStatus::Unchanged => parts.push("name unchanged".into()),
        RowStatus::Cancelled => parts.push("not renamed, the run was stopped".into()),
        s => {
            parts.push(format!(
                "will be renamed to {}",
                r.final_name().unwrap_or_default()
            ));
            if let Some(it) = &r.item {
                if let Some(t) = tier_label(it.tier) {
                    parts.push(format!("{} extraction", t.to_lowercase()));
                }
                parts.push(match &it.enrich {
                    EnrichPlan::Will => "metadata will be written".into(),
                    EnrichPlan::Skip { reason } => {
                        format!("metadata skipped, {}", skip_reason_copy(reason))
                    }
                });
            }
            if s == RowStatus::Review {
                parts.push("needs review".into());
            }
            if r.edited.is_some() {
                parts.push("name edited".into());
            }
            parts.push(if r.checked {
                "included".into()
            } else {
                "excluded".into()
            });
        }
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_source_follows_env_policy() {
        let mut c = super::super::demo::config();
        assert_eq!(
            c.key_source("OPENAI_API_KEY"),
            KeySource::DotEnv("OPENAI_API_KEY".into())
        );
        c.raw_key = Some("sk-literal".into());
        assert_eq!(c.key_source("OPENAI_API_KEY"), KeySource::Literal);
        c.raw_key = Some("${AUTORENAME_TEST_UNSET_KEY}".into());
        assert_eq!(
            c.key_source("OPENAI_API_KEY"),
            KeySource::Missing("AUTORENAME_TEST_UNSET_KEY".into())
        );
    }

    #[test]
    fn validate_name_rules() {
        assert!(validate_name("20251103 Amazon AP 12,13.pdf").is_none());
        assert!(validate_name("a/b.pdf").is_some());
        assert!(validate_name(".pdf").is_some());
        assert!(validate_name("CON.pdf").is_some());
    }
}
