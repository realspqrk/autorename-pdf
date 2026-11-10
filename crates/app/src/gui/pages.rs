//! State of the Settings, Companies and History screens (no egui).

use std::path::PathBuf;

use autorename_core::config::{Decider, Privacy, Settings, Toggle};
use autorename_core::harmonize::{Entry, NameDb, Source};
use autorename_core::journal::{self, Journal};
use autorename_core::ocr::PackStatus;

use super::model::{ConfigState, KeySource, PROVIDERS};

/// Long text shown from About in the document dialog (§4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextDoc {
    License,
    ThirdPartyNotices,
}

/// Settings pages (left nav, §4.7).
pub const SETTINGS_PAGES: [&str; 4] = ["General", "Processing", "Naming", "About"];
pub mod page {
    pub const GENERAL: usize = 0;
    pub const PROCESSING: usize = 1;
    pub const NAMING: usize = 2;
    pub const ABOUT: usize = 3;
}

pub const TOGGLES: [&str; 3] = ["Auto", "On", "Off"];
pub const DECIDERS: [(&str, &str); 4] = [
    ("ai", "Your selected AI provider"),
    ("openai-decisions", "OpenAI Decisions"),
    ("jev", "Jev"),
    ("off", "Off"),
];
pub const SCALES: [f32; 4] = [0.9, 1.0, 1.1, 1.25];

/// `output.date_format` presets (strftime); anything else is Custom.
pub const DATE_PRESETS: [&str; 4] = ["%Y%m%d", "%Y-%m-%d", "%d.%m.%Y", "%Y_%m_%d"];
/// `output.amount_format` presets.
pub const AMOUNT_PRESETS: [&str; 3] = ["{currency} {amount}", "{amount} {currency}", "{amount}"];
/// `output.language` choices; anything else is typed under Other.
pub const OUTPUT_LANGUAGES: [&str; 15] = [
    "English",
    "German",
    "French",
    "Spanish",
    "Italian",
    "Portuguese",
    "Dutch",
    "Polish",
    "Swedish",
    "Danish",
    "Norwegian",
    "Finnish",
    "Czech",
    "Hungarian",
    "Turkish",
];

/// Stepper ranges (min, max, step).
pub const MAX_PAGES_RANGE: (f64, f64, f64) = (1.0, 10.0, 1.0);
pub const THRESHOLD_RANGE: (f64, f64, f64) = (0.5, 1.0, 0.05);
pub const TEMPERATURE_RANGE: (f64, f64, f64) = (0.0, 1.0, 0.1);
pub const RETRIES_RANGE: (f64, f64, f64) = (0.0, 5.0, 1.0);

/// Logical CPUs (upper bound of the OCR threads stepper).
pub fn cpu_count() -> f64 {
    std::thread::available_parallelism().map_or(4, |n| n.get()) as f64
}

/// Characters a file name can't contain (core `naming::sanitize_filename`).
pub const FORBIDDEN_HINT: &str = "Can't contain / \\ : * ? \" < > |";
/// Date format that renders a character a file name can't contain.
pub const DATE_FORBIDDEN_HINT: &str = "Use dots or dashes instead of / or :, e.g. %d.%m.%Y.";

/// Editable copy of every global config key (§4.7). Profile templates and
/// fields stay in the YAML file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsForm {
    pub provider: usize,
    pub model: String,
    /// New key typed by the user (saved into `.env`); empty = keep.
    pub api_key: String,
    pub show_key: bool,
    pub base_url: String,
    pub offline: bool,
    /// `ai.temperature` (0.0–1.0).
    pub temperature: f64,
    pub max_retries: u32,
    pub max_pages: u32,
    pub ocr: usize,
    pub vision: usize,
    /// `pdf.text_quality_threshold` (0.50–1.00, step 0.05).
    pub threshold: f64,
    /// `pdf.incoming_invoice` / `pdf.outgoing_invoice` (AP / AR).
    pub incoming: String,
    pub outgoing: String,
    pub ocr_language: String,
    pub ocr_threads: u32,
    /// `company.names`, one per line (commas occur in legal names).
    pub company: String,
    /// `output.language`, `output.date_format`, `output.amount_format`,
    /// `output.decimal_separator`.
    pub out_language: String,
    pub date_format: String,
    pub amount_format: String,
    pub decimal_separator: String,
    pub enrich: bool,
    pub learn: bool,
    pub decider: usize,
    /// New decider key typed by the user (saved into `.env`); empty = keep.
    pub decider_key: String,
    pub show_decider_key: bool,
    pub prompt_extension: String,
    /// Default profile (`profile:` key).
    pub profile: String,
}

fn toggle_idx(t: Toggle) -> usize {
    match t {
        Toggle::Auto => 0,
        Toggle::On => 1,
        Toggle::Off => 2,
    }
}

/// Form fields that can hold invalid input (helpers name the problem).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormField {
    BaseUrl,
    Incoming,
    Outgoing,
    DateFormat,
    AmountFormat,
    OutLanguage,
}

/// Where a credential comes from after saving the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Credential {
    /// No key needed (Ollama; a decider without its own key), no change.
    None,
    /// No key needed any more: the old reference is cleared (`""`).
    Clear,
    /// Unchanged: the saved config's key stays as written.
    Saved,
    /// The typed key, saved into `.env` as `var` (config: `${var}`).
    Typed(&'static str),
    /// Rebound to `${var}` from the environment or `.env`.
    Env(&'static str),
}

/// Key status line (and the save check) of a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// Typed: saved into `.env` as `var` on save.
    Typed(&'static str),
    /// The saved config's key or the rebound variable (maybe missing).
    Found(KeySource),
}

/// Display name of a remote decider's key owner ("Jev").
pub fn decider_service(id: &str) -> &'static str {
    match id {
        "jev" => "Jev",
        _ => "OpenAI Decisions",
    }
}

/// Sample date for live format examples (matches the profile samples).
pub fn sample_date() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2025, 11, 3).unwrap_or_default()
}

/// `date_format` applied to the sample date (core's formatter).
pub fn date_example(fmt: &str) -> String {
    autorename_core::dates::format_date(sample_date(), fmt)
}

/// `amount_format` applied to a sample amount (core's formatter).
pub fn amount_example(fmt: &str, sep: &str) -> String {
    let a = autorename_core::profiles::Amount {
        field: "x".into(),
        amount: "amount".into(),
        currency: "currency".into(),
    };
    let mut f = serde_json::Map::new();
    f.insert("amount".into(), "12,13".into());
    f.insert("currency".into(), "EUR".into());
    autorename_core::pipeline::derive_amount(&a, &mut f, fmt, sep);
    f.get("x")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn name_safe(v: &str) -> bool {
    autorename_core::naming::sanitize_filename(v) == v
}

impl SettingsForm {
    pub fn from_settings(s: &Settings) -> Self {
        SettingsForm {
            provider: PROVIDERS
                .iter()
                .position(|p| p.0 == s.ai.provider.id())
                .unwrap_or(0),
            model: s.ai.model.clone(),
            api_key: String::new(),
            show_key: false,
            base_url: s.ai.base_url.clone(),
            offline: s.privacy == Privacy::Offline,
            temperature: s.ai.temperature,
            max_retries: s.ai.max_retries,
            max_pages: s.pdf.max_pages,
            ocr: toggle_idx(s.pdf.ocr),
            vision: toggle_idx(s.pdf.vision),
            threshold: s.pdf.text_quality_threshold,
            incoming: s.pdf.incoming_invoice.clone(),
            outgoing: s.pdf.outgoing_invoice.clone(),
            ocr_language: s.ocr.language.clone(),
            ocr_threads: s.ocr.threads,
            company: s.company_names.join("\n"),
            out_language: s.output_language.clone(),
            date_format: s.date_format.clone(),
            amount_format: s.amount_format.clone(),
            decimal_separator: s.decimal_separator.clone(),
            enrich: s.enrich_enabled,
            learn: s.auto_learn,
            decider: match s.decider {
                Decider::Ai => 0,
                Decider::OpenaiDecisions => 1,
                Decider::Jev => 2,
                Decider::Off => 3,
            },
            prompt_extension: s.prompt_extension.clone(),
            profile: s.profile_id.clone(),
            decider_key: String::new(),
            show_decider_key: false,
        }
    }

    pub fn provider_id(&self) -> &'static str {
        PROVIDERS[self.provider.min(4)].0
    }

    pub fn key_var(&self) -> &'static str {
        super::model::key_var(PROVIDERS[self.provider.min(4)].0)
    }

    /// After the provider changed (to `self.provider`): its default model,
    /// no typed key, and no other provider's server address — that server
    /// would get this provider's requests and key. Back on the saved
    /// provider its saved address returns; one typed after the switch is
    /// kept (the field is edited after this).
    pub fn provider_changed(&mut self, saved: &SettingsForm) {
        self.model = autorename_core::config::default_model(self.provider_id())
            .unwrap_or_default()
            .to_string();
        // A key typed for another provider is never saved for this one.
        self.api_key.clear();
        self.show_key = false;
        self.base_url = if self.provider_id() == saved.provider_id() {
            saved.base_url.clone()
        } else {
            String::new()
        };
    }

    /// The form without view-only state (show/hide key), for dirty checks.
    pub fn values(&self) -> SettingsForm {
        SettingsForm {
            show_key: false,
            show_decider_key: false,
            ..self.clone()
        }
    }

    /// (section, key, YAML value) edits for `config_edit::set`: every key
    /// the form owns. Saving writes only the ones that differ from the
    /// saved form ([`SettingsForm::changed_edits`]).
    pub fn edits(&self) -> Vec<(Option<&'static str>, &'static str, String)> {
        use autorename_core::config_edit::{quoted, quoted_list};
        let t = |i: usize| ["auto", "true", "false"][i.min(2)].to_string();
        let mut v = vec![
            (
                None,
                "privacy",
                if self.offline { "offline" } else { "cloud" }.to_string(),
            ),
            (Some("ai"), "provider", self.provider_id().to_string()),
            // Empty = provider default (`null`; core never fills "").
            (
                Some("ai"),
                "model",
                if self.model.trim().is_empty() {
                    "null".to_string()
                } else {
                    quoted(self.model.trim())
                },
            ),
            (Some("ai"), "base_url", quoted(self.base_url.trim())),
            (
                Some("ai"),
                "temperature",
                format!("{:.1}", self.temperature),
            ),
            (Some("ai"), "max_retries", self.max_retries.to_string()),
            (Some("pdf"), "max_pages", self.max_pages.to_string()),
            (Some("pdf"), "ocr", t(self.ocr)),
            (Some("pdf"), "vision", t(self.vision)),
            (
                Some("pdf"),
                "text_quality_threshold",
                format!("{:.2}", self.threshold),
            ),
            (
                Some("pdf"),
                "incoming_invoice",
                quoted(self.incoming.trim()),
            ),
            (
                Some("pdf"),
                "outgoing_invoice",
                quoted(self.outgoing.trim()),
            ),
            (Some("ocr"), "language", quoted(self.ocr_language.trim())),
            (Some("ocr"), "threads", self.ocr_threads.to_string()),
            (Some("company"), "names", quoted_list(&self.company_names())),
            (Some("output"), "language", quoted(self.out_language.trim())),
            (Some("output"), "date_format", quoted(&self.date_format)),
            (Some("output"), "amount_format", quoted(&self.amount_format)),
            (
                Some("output"),
                "decimal_separator",
                quoted(&self.decimal_separator),
            ),
            (Some("enrich"), "enabled", self.enrich.to_string()),
            (Some("harmonization"), "auto_learn", self.learn.to_string()),
            (
                Some("harmonization"),
                "decider",
                DECIDERS[self.decider.min(3)].0.to_string(),
            ),
            (
                None,
                "prompt_extension",
                quoted(self.prompt_extension.trim()),
            ),
        ];
        if !self.profile.is_empty() {
            v.push((None, "profile", quoted(&self.profile)));
        }
        v
    }

    /// Edits that differ from `saved`: untouched keys (and their comments
    /// or `null` defaults) stay as written. Credentials follow
    /// [`SettingsForm::ai_credential`] / [`SettingsForm::decider_credential`].
    pub fn changed_edits(
        &self,
        saved: &SettingsForm,
    ) -> Vec<(Option<&'static str>, &'static str, String)> {
        use autorename_core::config_edit::quoted;
        let before = saved.edits();
        let mut v: Vec<_> = self
            .edits()
            .into_iter()
            .filter(|e| !before.contains(e))
            .collect();
        for (sec, key, cred) in [
            ("ai", "api_key", self.ai_credential(saved)),
            (
                "harmonization",
                "decider_api_key",
                self.decider_credential(saved),
            ),
        ] {
            let value = match cred {
                Credential::Typed(var) | Credential::Env(var) => quoted(&format!("${{{var}}}")),
                Credential::Clear => quoted(""),
                Credential::Saved | Credential::None => continue,
            };
            v.push((Some(sec), key, value));
        }
        v
    }

    /// Where `ai.api_key` comes from after saving (shared by the status
    /// line, Test connection and Save). A provider switch rebinds the key
    /// to the new provider's variable; another provider's key never stays.
    pub fn ai_credential(&self, saved: &SettingsForm) -> Credential {
        let var = self.key_var();
        let changed = self.provider_id() != saved.provider_id();
        if var.is_empty() {
            return if changed {
                Credential::Clear
            } else {
                Credential::None
            };
        }
        if !self.api_key.trim().is_empty() {
            Credential::Typed(var)
        } else if changed {
            Credential::Env(var)
        } else {
            Credential::Saved
        }
    }

    /// Where `harmonization.decider_api_key` comes from after saving. A
    /// decider (or provider) switch rebinds it to the new decider's
    /// variable, or clears it when the new decider has no own key.
    pub fn decider_credential(&self, saved: &SettingsForm) -> Credential {
        let changed = self.decider_id() != saved.decider_id()
            || self.decider_key_var() != saved.decider_key_var();
        let remote = |f: &SettingsForm| matches!(f.decider_id(), "openai-decisions" | "jev");
        match self.decider_key_var() {
            None if changed && remote(saved) => Credential::Clear,
            None => Credential::None,
            Some(var) if !self.decider_key.trim().is_empty() => Credential::Typed(var),
            Some(var) if changed => Credential::Env(var),
            Some(_) => Credential::Saved,
        }
    }

    /// Invalid input per field (copy for the field helper).
    /// `company.names` from the one-per-line field (blank lines dropped).
    pub fn company_names(&self) -> Vec<&str> {
        self.company
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    }

    pub fn problem(&self, f: FormField) -> Option<String> {
        let code = |v: &str| {
            let v = v.trim();
            if v.is_empty() {
                Some("Enter a code.".to_string())
            } else if !name_safe(v) {
                Some(FORBIDDEN_HINT.to_string())
            } else {
                None
            }
        };
        match f {
            FormField::BaseUrl => {
                let u = self.base_url.trim();
                (!u.is_empty() && !u.starts_with("http://") && !u.starts_with("https://"))
                    .then(|| "Start with http:// or https://".to_string())
            }
            FormField::Incoming => code(&self.incoming),
            FormField::Outgoing => code(&self.outgoing).or_else(|| {
                (self.outgoing.trim() == self.incoming.trim())
                    .then(|| "Use a different code than for bills you receive.".to_string())
            }),
            FormField::DateFormat => {
                let fmt = &self.date_format;
                if fmt.trim().is_empty() {
                    Some("Enter a date format.".into())
                } else if !fmt.contains('%') {
                    Some("Add at least one of %Y, %m or %d.".into())
                } else if !name_safe(&date_example(fmt)) {
                    Some(DATE_FORBIDDEN_HINT.into())
                } else {
                    None
                }
            }
            FormField::AmountFormat => (!self.amount_format.contains("{amount}"))
                .then(|| "Must contain {amount}.".to_string()),
            FormField::OutLanguage => self
                .out_language
                .trim()
                .is_empty()
                .then(|| "Enter a language.".to_string()),
        }
    }

    /// First reason the form can't be saved (offline rules, then invalid
    /// fields); `None` = savable.
    pub fn save_blocker(&self) -> Option<String> {
        if let Some(v) = self.offline_violation() {
            return Some(v);
        }
        let n = [
            FormField::BaseUrl,
            FormField::Incoming,
            FormField::Outgoing,
            FormField::DateFormat,
            FormField::AmountFormat,
            FormField::OutLanguage,
        ]
        .iter()
        .filter(|f| self.problem(**f).is_some())
        .count();
        (n > 0).then(|| {
            format!(
                "Fix {} to save",
                super::components::plural(n, "problem", "problems")
            )
        })
    }

    pub fn decider_id(&self) -> &'static str {
        DECIDERS[self.decider.min(3)].0
    }

    /// Env variable the chosen decider's own key is saved in (`None` = no
    /// own key needed).
    pub fn decider_key_var(&self) -> Option<&'static str> {
        super::model::decider_key_var(self.decider_id(), self.provider_id())
    }

    /// Status of the AI key after saving (`None` = no key needed).
    pub fn ai_key_state(&self, saved: &SettingsForm, cfg: &ConfigState) -> Option<KeyState> {
        match self.ai_credential(saved) {
            Credential::None | Credential::Clear => None,
            Credential::Typed(v) => Some(KeyState::Typed(v)),
            Credential::Env(v) => Some(KeyState::Found(cfg.env_source(v))),
            Credential::Saved => Some(KeyState::Found(cfg.key_source(self.key_var()))),
        }
    }

    /// Status of the decider's own key after saving (`None` = the decider
    /// needs no own key).
    pub fn decider_key_state(&self, saved: &SettingsForm, cfg: &ConfigState) -> Option<KeyState> {
        match self.decider_credential(saved) {
            Credential::None | Credential::Clear => None,
            Credential::Typed(v) => Some(KeyState::Typed(v)),
            Credential::Env(v) => Some(KeyState::Found(cfg.env_source(v))),
            Credential::Saved => cfg
                .decider_key_source(self.decider_id(), self.provider_id())
                .map(KeyState::Found),
        }
    }

    /// A selected cloud provider or decider without a key can't be saved
    /// (core would reject the config).
    pub fn key_blocker(&self, saved: &SettingsForm, cfg: &ConfigState) -> Option<String> {
        let missing =
            |s: Option<KeyState>| matches!(s, Some(KeyState::Found(KeySource::Missing(_))));
        if missing(self.ai_key_state(saved, cfg)) {
            return Some(format!(
                "Add your {} API key to save",
                PROVIDERS[self.provider.min(4)].1
            ));
        }
        if missing(self.decider_key_state(saved, cfg)) {
            let owner = match self.decider_id() {
                "jev" => "Jev",
                _ => "OpenAI",
            };
            return Some(format!("Add your {owner} API key to save"));
        }
        None
    }

    /// (env var, value) pairs to write into `.env` on save (typed keys).
    pub fn env_writes(&self, saved: &SettingsForm) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        if let Credential::Typed(var) = self.ai_credential(saved) {
            v.push((var, self.api_key.trim().to_string()));
        }
        if let Credential::Typed(dvar) = self.decider_credential(saved) {
            v.retain(|(k, _)| *k != dvar);
            v.push((dvar, self.decider_key.trim().to_string()));
        }
        v
    }

    /// Offline with a non-local base URL can't be saved (§4.7 Privacy).
    pub fn offline_violation(&self) -> Option<String> {
        if !self.offline {
            return None;
        }
        if self.provider_id() != "ollama" {
            return Some("Choose Ollama below to use Offline mode.".into());
        }
        let url = self.base_url.trim();
        if !url.is_empty() && !autorename_core::config::is_local_url(url) {
            return Some(format!(
                "Offline mode blocks requests to {url}. Use an address on this computer or your network."
            ));
        }
        if matches!(self.decider, 1 | 2) {
            return Some(
                "For name matching, choose AI on this computer or Off to use Offline mode.".into(),
            );
        }
        None
    }
}

/// GUI-only preferences (eframe storage, not config.yaml).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Prefs {
    /// 0 System, 1 Light, 2 Dark.
    pub theme: usize,
    /// 0 System, 1 On, 2 Off.
    pub reduce_motion: usize,
    /// Index into `SCALES`.
    pub scale: usize,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: 0,
            reduce_motion: 0,
            scale: 1,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SettingsState {
    /// Index into `SETTINGS_PAGES`.
    pub section: usize,
    /// Processing › Advanced is expanded (view state, not saved).
    pub advanced: bool,
    /// Naming › Date format shows the custom field (view state).
    pub date_custom: bool,
    /// Naming › Language shows the free-text field (view state).
    pub language_other: bool,
    pub form: SettingsForm,
    pub saved: SettingsForm,
    pub loaded: bool,
    pub testing: bool,
    pub test: Option<Result<u64, String>>,
    pub error: Option<String>,
    pub packs: Vec<PackStatus>,
    /// Pack id being downloaded/imported/removed.
    pub model_busy: Option<String>,
    pub model_progress: (u64, u64),
    pub model_msg: Option<Result<String, String>>,
}

impl SettingsState {
    /// First reason the form can't be saved: offline rules, a missing key,
    /// then invalid fields.
    pub fn blocker(&self, cfg: &ConfigState) -> Option<String> {
        self.form
            .offline_violation()
            .or_else(|| self.form.key_blocker(&self.saved, cfg))
            .or_else(|| self.form.save_blocker())
    }

    pub fn dirty(&self) -> bool {
        self.loaded && self.form.values() != self.saved.values()
    }
}

// ---------------------------------------------------------------------------
// Companies
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompanyFilter {
    All,
    Review,
    Learned,
    Manual,
}

#[derive(Clone, Debug, Default)]
pub struct MergeDialog {
    pub from: String,
    pub search: String,
    pub target: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CompaniesState {
    pub db: Option<NameDb>,
    pub path: Option<PathBuf>,
    /// Field of the list at `path`.
    pub field: String,
    pub filter: CompanyFilter,
    pub search: String,
    pub active: Option<usize>,
    pub merge: Option<MergeDialog>,
    /// Rename dialog: (old canonical, new name).
    pub rename: Option<(String, String)>,
    pub error: Option<String>,
    /// A load runs on the worker (results with an older `seq` are dropped).
    pub loading: bool,
    /// An edit runs on the worker; edits are disabled meanwhile.
    pub pending: bool,
    /// Reload once the pending edit is done.
    pub reload_after: bool,
    pub seq: u64,
    /// mtime of the file when it was last read (external edits reload).
    pub mtime: Option<std::time::SystemTime>,
    /// Last mtime check (at most once a second).
    pub checked: Option<std::time::Instant>,
}

impl Default for CompaniesState {
    fn default() -> Self {
        CompaniesState {
            db: None,
            path: None,
            field: String::new(),
            filter: CompanyFilter::All,
            search: String::new(),
            active: None,
            merge: None,
            rename: None,
            error: None,
            loading: false,
            pending: false,
            reload_after: false,
            seq: 0,
            mtime: None,
            checked: None,
        }
    }
}

impl CompaniesState {
    pub fn entries(&self) -> &[Entry] {
        self.db.as_ref().map_or(&[], |d| d.entries.as_slice())
    }

    pub fn review_count(&self) -> usize {
        self.entries().iter().filter(|e| e.review).count()
    }

    pub fn visible(&self) -> Vec<usize> {
        let q = self.search.to_lowercase();
        self.entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| match self.filter {
                CompanyFilter::All => true,
                CompanyFilter::Review => e.review,
                CompanyFilter::Learned => e.source == Source::Auto,
                CompanyFilter::Manual => e.source == Source::User,
            })
            .filter(|(_, e)| {
                q.is_empty()
                    || e.canonical.to_lowercase().contains(&q)
                    || e.aliases
                        .iter()
                        .chain(&e.learned)
                        .any(|a| a.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// A load or edit is running: edits are disabled.
    pub fn busy(&self) -> bool {
        self.loading || self.pending
    }

    /// Show `db` (fresh from disk), keeping the selected entry by name.
    pub fn set_db(&mut self, db: NameDb) {
        let active = self
            .active
            .and_then(|a| self.entries().get(a))
            .map(|e| e.canonical.clone());
        self.db = Some(db);
        self.active = active.and_then(|n| self.entries().iter().position(|e| e.canonical == n));
    }
}

/// "No other spellings" / "1 other spelling" / "4 other spellings".
pub fn spellings_caption(e: &Entry) -> String {
    match e.aliases.len() + e.learned.len() {
        0 => "No other spellings".to_string(),
        n => super::components::plural(n, "other spelling", "other spellings"),
    }
}

pub fn entry_caption(e: &Entry) -> String {
    let mut s = spellings_caption(e);
    match (&e.added, e.source) {
        (Some(d), Source::Auto) => s.push_str(&format!(" · added automatically {d}")),
        (Some(d), Source::User) => s.push_str(&format!(" · added by you {d}")),
        (None, Source::User) => s.push_str(" · added by you"),
        (None, Source::Auto) => s.push_str(" · added automatically"),
    }
    s
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct BatchView {
    pub id: String,
    pub timestamp: String,
    pub files: usize,
    /// Learned company-name changes in the batch.
    pub names: usize,
    pub folder: String,
    pub source: String,
    pub undone: bool,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryState {
    pub batches: Vec<BatchView>,
    pub loaded: bool,
    pub error: Option<String>,
    /// What to do about `error` (core's hint, e.g. move a corrupt journal aside).
    pub error_hint: Option<String>,
}

impl HistoryState {
    pub fn from_journal(j: &Journal) -> Self {
        // Every batch with an undoable change: renames, metadata written to
        // unchanged files, or learned company names (core journals all three).
        let mut batches: Vec<BatchView> = j
            .batches
            .iter()
            .filter(|b| !b.renames.is_empty() || !b.enrich.is_empty() || !b.aliases.is_empty())
            .map(|b| {
                let mut paths: Vec<&std::path::Path> =
                    b.renames.iter().map(|r| r.new_path.as_path()).collect();
                for e in &b.enrich {
                    if !paths.contains(&e.path.as_path()) {
                        paths.push(e.path.as_path());
                    }
                }
                BatchView {
                    id: b.batch_id.clone(),
                    timestamp: b.timestamp.clone(),
                    files: paths.len(),
                    names: b.aliases.len(),
                    folder: paths
                        .first()
                        .and_then(|p| p.parent())
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    source: b.source.clone(),
                    undone: b.undone,
                }
            })
            .collect();
        batches.reverse();
        HistoryState {
            batches,
            loaded: true,
            ..Default::default()
        }
    }

    pub fn load() -> Self {
        let Some(path) = journal::default_journal_path() else {
            return HistoryState {
                loaded: true,
                error: Some("No data folder for the undo journal.".into()),
                ..Default::default()
            };
        };
        match journal::read_journal(&path) {
            Ok(j) => HistoryState::from_journal(&j),
            Err(e) => HistoryState {
                loaded: true,
                error: Some(e.to_string()),
                error_hint: e.hint().map(str::to_string),
                ..Default::default()
            },
        }
    }
}

/// "2025-11-07 09:30" from an RFC 3339 timestamp (ISO dates in the UI, §7).
pub fn short_time(ts: &str) -> String {
    let t = ts.replace('T', " ");
    t.get(..16).unwrap_or(&t).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Batches that only wrote metadata or learned names stay undoable.
    #[test]
    fn history_lists_enrich_and_learn_only_batches() {
        let j: Journal = serde_json::from_value(serde_json::json!({
            "version": 1,
            "batches": [
                {"batch_id": "a", "timestamp": "2025-11-01T10:00:00Z", "source": "gui", "undone": false,
                 "renames": [],
                 "enrich": [{"path": "/x/doc.pdf", "orig_len": 10, "orig_sha256": "0", "post_sha256": "1"}]},
                {"batch_id": "b", "timestamp": "2025-11-02T10:00:00Z", "source": "cli", "undone": false,
                 "renames": [], "enrich": []}
            ]
        }))
        .expect("journal json");
        let h = HistoryState::from_journal(&j);
        assert_eq!(h.batches.len(), 1);
        assert_eq!(h.batches[0].files, 1);
        assert_eq!(h.batches[0].folder, "/x");
    }
}
