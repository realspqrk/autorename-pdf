//! `autorename-pdf setup`: idempotent, non-interactive configuration for
//! humans and agents. Edits config.yaml line by line (comments kept, via
//! `autorename_core::config_edit`), stores API keys only in the 0600 `.env`
//! beside it, and reports every step. Secrets never reach stdout/stderr.

use std::io::Read;
use std::path::PathBuf;

use autorename_core::config::{self, Level};
use autorename_core::config_edit::{self, get_env, quoted, set, set_env, write_env};
use autorename_core::pipeline::SCHEMA_VERSION;
use autorename_core::{Error, ExitCode, Result, fsx, ocr};
use serde::Serialize;

use crate::cli::SetupArgs;
use crate::cmd::{Ctx, STARTER_CONFIG, models_root};
use crate::out::{Style, pad};

const OLLAMA_URL: &str = "http://localhost:11434";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Done,
    Unchanged,
    Failed,
    /// `--check`: needed but not there yet.
    Missing,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Step {
    pub step: &'static str,
    pub status: Status,
    /// Needed to rename (`false`: optional, e.g. OCR models, integration).
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip)]
    #[schemars(skip)]
    exit: ExitCode,
}

/// Output of `setup`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Report {
    schema_version: u32,
    check: bool,
    /// Config is valid for `rename` (provider + key resolvable).
    ready: bool,
    #[serde(with = "fsx::path_serde")]
    #[schemars(with = "fsx::path_serde::Repr")]
    config_path: PathBuf,
    #[serde(with = "fsx::path_serde")]
    #[schemars(with = "fsx::path_serde::Repr")]
    env_path: PathBuf,
    steps: Vec<Step>,
}

fn step(step: &'static str, status: Status) -> Step {
    Step {
        step,
        status,
        required: !matches!(step, "ocr_models" | "integrate"),
        detail: None,
        hint: None,
        exit: ExitCode::Success,
    }
}

impl Step {
    fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
    fn fail(mut self, exit: ExitCode) -> Self {
        self.status = Status::Failed;
        self.exit = exit;
        self
    }
}

/// The provider's standard key variable; `OPENAI_API_KEY` for a key given
/// to Ollama (an authenticating proxy).
fn default_key_var(provider: &str) -> &'static str {
    config::default_key_var(provider).unwrap_or("OPENAI_API_KEY")
}

fn valid_var(name: &str) -> bool {
    let mut c = name.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// Current scalar of `section.key` in config text (best effort, for
/// decisions like "provider changed"), from the parsed YAML.
fn current(text: &str, section: &str, key: &str) -> String {
    config::parse_yaml(text)
        .ok()
        .and_then(|v| v.get(section)?.get(key)?.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Config text + pending .env text, with per-step bookkeeping.
struct Draft {
    text: String,
    env: String,
    steps: Vec<Step>,
}

impl Draft {
    /// Apply `edits` as one step (done if anything changed).
    fn edit(&mut self, name: &'static str, edits: &[(Option<&str>, &str, String)]) {
        let before = self.text.clone();
        for (sec, key, value) in edits {
            self.text = set(&self.text, *sec, key, value);
        }
        let status = if self.text == before {
            Status::Unchanged
        } else {
            Status::Done
        };
        self.steps.push(step(name, status));
    }

    /// Store `value` under `var` in .env (never echoed).
    fn secret(&mut self, name: &'static str, var: &str, value: &str, config_edit: (&str, &str)) {
        let before = (self.text.clone(), self.env.clone());
        if get_env(&self.env, var).as_deref() != Some(value) {
            self.env = set_env(&self.env, var, value);
        }
        self.text = set(
            &self.text,
            Some(config_edit.0),
            config_edit.1,
            &quoted(&format!("${{{var}}}")),
        );
        let status = if (self.text.clone(), self.env.clone()) == before {
            Status::Unchanged
        } else {
            Status::Done
        };
        self.steps
            .push(step(name, status).detail(format!("stored as {var} in .env")));
    }
}

/// Read a key from env var `var` (process environment only).
fn key_from_env(var: &str) -> std::result::Result<String, String> {
    match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        _ => Err(format!("environment variable {var} is not set")),
    }
}

fn usage(message: impl Into<String>, hint: &str) -> Error {
    Error::Usage {
        message: message.into(),
        hint: Some(hint.into()),
    }
}

pub fn run(a: &SetupArgs, ctx: &Ctx<'_>) -> Result<ExitCode> {
    for var in [&a.api_key_env, &a.decider_key_env].into_iter().flatten() {
        if !valid_var(var) {
            return Err(usage(
                format!("{var:?} is not a valid environment variable name"),
                "Use letters, digits and `_`, e.g. OPENAI_API_KEY.",
            ));
        }
    }
    if a.cloud && ctx.g.offline {
        return Err(usage(
            "--cloud and --offline contradict each other",
            "Use --cloud to leave offline mode, --offline to enter it.",
        ));
    }
    if a.check && (a.api_key_stdin || a.api_key_env.is_some()) {
        return Err(usage(
            "--check changes nothing; it cannot store a key",
            "Run `setup --check` and `setup --api-key-env VAR` separately.",
        ));
    }
    let loc = ctx.location();
    let path = loc.path.clone();
    let env_path = config::env_path(&path);
    let existing = std::fs::read_to_string(&path).ok();
    let env_text = std::fs::read_to_string(&env_path).unwrap_or_default();
    let mut d = Draft {
        text: existing
            .clone()
            .unwrap_or_else(|| STARTER_CONFIG.to_string()),
        env: env_text.clone(),
        steps: Vec::new(),
    };
    d.steps.push(match (&existing, a.check) {
        (Some(_), _) => step("config", Status::Unchanged),
        (None, true) => step("config", Status::Missing).hint(format!(
            "Run `{}` (or `config init`) to create it.",
            ctx.out.command(&["setup"])
        )),
        (None, false) => step("config", Status::Done).detail("created from the starter config"),
    });

    if !a.check {
        plan_edits(a, ctx, &mut d)?;
    }

    // Validate the result in memory first: a rejected run changes nothing.
    let ready = if a.check && existing.is_none() {
        false
    } else {
        validate(&d.text, &d.env, a.check, ctx.out, &mut d.steps)
    };
    let edit_failed = d.steps.iter().any(|s| s.status == Status::Failed);
    let changes = existing.as_deref() != Some(d.text.as_str()) || d.env != env_text;
    if !a.check && changes && (edit_failed || !ready) {
        for s in d.steps.iter_mut().filter(|s| s.status == Status::Done) {
            s.status = Status::Unchanged;
            s.detail =
                Some("not applied: the resulting config is invalid (nothing was written)".into());
        }
    }
    // Persist (never in --check, only a valid result).
    if !a.check && !edit_failed && ready {
        if existing.as_deref() != Some(d.text.as_str()) {
            let r = match existing {
                Some(_) => fsx::write_atomic(&path, d.text.as_bytes()),
                None => fsx::write_new(&path, d.text.as_bytes()),
            };
            r.map_err(|e| Error::io(&path, e))?;
        }
        if d.env != env_text {
            write_env(&env_path, &d.env).map_err(|e| Error::io(&env_path, e))?;
        }
    }

    let offline = ctx.g.offline
        || config::parse_yaml(&d.text)
            .ok()
            .and_then(|v| v.get("privacy")?.as_str().map(|p| p == "offline"))
            .unwrap_or(false);
    let mut ready = ready;
    if ready && let Some(s) = ollama_step(&d.text) {
        ready = s.status == Status::Unchanged;
        d.steps.push(s);
    }
    if a.ocr_models || a.check {
        d.steps.push(ocr_step(&d.text, a.check, offline, ctx));
    }
    if a.integrate {
        d.steps.push(integrate_step(a.check));
    }

    let failed = d.steps.iter().find(|s| s.status == Status::Failed);
    let code = match failed {
        Some(s) => s.exit,
        None if !ready => ExitCode::Config,
        None => ExitCode::Success,
    };
    for s in &mut d.steps {
        s.hint = s.hint.take().map(|h| ctx.out.fix_hint(&h));
    }
    let report = Report {
        schema_version: SCHEMA_VERSION,
        check: a.check,
        ready,
        config_path: path,
        env_path,
        steps: d.steps,
    };
    print(&report, ctx);
    Ok(code)
}

fn plan_edits(a: &SetupArgs, ctx: &Ctx<'_>, d: &mut Draft) -> Result<()> {
    // Every value is a free-text string: always quoted (`1234`, `true`).
    let q = quoted;
    if a.cloud {
        // The only way out of saved offline mode: explicit.
        d.edit("privacy", &[(None, "privacy", q("cloud"))]);
    }
    if let Some(p) = a.provider {
        let mut edits = vec![(Some("ai"), "provider", q(p.id()))];
        if current(&d.text, "ai", "provider") != p.id() {
            // Another provider's model would be wrong: use the new default.
            if a.model.is_none() {
                edits.push((Some("ai"), "model", "null".into()));
            }
            // Its endpoint too: it would receive the new provider's requests
            // and key. Empty = the new provider's default.
            if a.base_url.is_none() {
                edits.push((Some("ai"), "base_url", q("")));
            }
            // The key follows the provider (like the GUI): the new
            // provider's variable, empty for Ollama. --api-key-* below may
            // set it again.
            let key = config::default_key_ref(p.id());
            edits.push((Some("ai"), "api_key", q(&key)));
        }
        d.edit("provider", &edits);
    }
    if let Some(url) = &a.base_url {
        d.edit("base_url", &[(Some("ai"), "base_url", q(url.trim()))]);
    }
    if let Some(m) = &a.model {
        d.edit("model", &[(Some("ai"), "model", q(m))]);
    }
    if !a.company.is_empty() {
        // Always strings: a name like "1234" must not become a number.
        let names: Vec<&str> = a
            .company
            .iter()
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();
        d.edit(
            "company",
            &[(Some("company"), "names", config_edit::quoted_list(&names))],
        );
    }
    if let Some(p) = &a.profile {
        d.edit("profile", &[(None, "profile", q(p))]);
    }
    if ctx.g.offline {
        // Keep a model already chosen for Ollama unless one is given.
        let existing_model = (current(&d.text, "ai", "provider") == "ollama")
            .then(|| current(&d.text, "ai", "model"))
            .filter(|m| !m.is_empty());
        let model = a
            .ollama_model
            .clone()
            .or(existing_model)
            .unwrap_or_else(|| config::default_model("ollama").unwrap_or("").to_string());
        let switching = current(&d.text, "ai", "provider") != "ollama";
        let mut edits = vec![
            (None, "privacy", q("offline")),
            (Some("ai"), "provider", q("ollama")),
            (Some("ai"), "model", q(&model)),
            // No cloud key reference (no "${OPENAI_API_KEY} is not set" notices).
            (Some("ai"), "api_key", q("")),
        ];
        // Only touch base_url when given, when it belongs to the previous
        // provider, or when it cannot work offline (a cloud URL); an empty
        // value already means localhost.
        let saved_url = current(&d.text, "ai", "base_url");
        match a.ollama_url.as_ref().or(a.base_url.as_ref()) {
            Some(url) => edits.push((Some("ai"), "base_url", q(url.trim()))),
            None if switching && !saved_url.is_empty() => {
                edits.push((Some("ai"), "base_url", q("")));
            }
            None if !saved_url.is_empty() && !config::is_local_url(&saved_url) => {
                edits.push((Some("ai"), "base_url", q(OLLAMA_URL)));
            }
            None => {}
        }
        let dec = current(&d.text, "harmonization", "decider");
        if matches!(dec.as_str(), "openai-decisions" | "jev") && a.decider.is_none() {
            edits.push((Some("harmonization"), "decider", q("ai")));
        }
        d.edit("offline", &edits);
    } else if a.ollama_url.is_some() || a.ollama_model.is_some() {
        return Err(usage(
            "--ollama-url / --ollama-model need --offline",
            "For a LAN/local Ollama without offline mode use --provider ollama --base-url URL --model M.",
        ));
    }

    // API key.
    if a.api_key_stdin || a.api_key_env.is_some() {
        let provider = current(&d.text, "ai", "provider");
        let var = a
            .api_key_env
            .clone()
            .unwrap_or_else(|| default_key_var(&provider).to_string());
        let key = if a.api_key_stdin {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| Error::io("<stdin>", e))?;
            let s = s.trim().to_string();
            if s.is_empty() {
                return Err(usage(
                    "--api-key-stdin: no key on stdin",
                    "Pipe the key, e.g. `printf %s \"$KEY\" | autorename-pdf setup --api-key-stdin`.",
                ));
            }
            Ok(s)
        } else {
            key_from_env(&var)
        };
        match key {
            Ok(k) => d.secret("api_key", &var, &k, ("ai", "api_key")),
            Err(e) => d.steps.push(
                step("api_key", Status::Failed)
                    .fail(ExitCode::Config)
                    .detail(e)
                    .hint(format!(
                        "Export {var} first, or pipe the key with --api-key-stdin."
                    )),
            ),
        }
    }

    if let Some(dec) = a.decider {
        d.edit(
            "decider",
            &[(Some("harmonization"), "decider", q(dec.id()))],
        );
    }
    if let Some(var) = &a.decider_key_env {
        match key_from_env(var) {
            Ok(k) => d.secret("decider_key", var, &k, ("harmonization", "decider_api_key")),
            Err(e) => d.steps.push(
                step("decider_key", Status::Failed)
                    .fail(ExitCode::Config)
                    .detail(e)
                    .hint(format!("Export {var} first.")),
            ),
        }
    }
    Ok(())
}

/// `validate` step: errors → failed (or missing with --check).
/// Validate config `text` with the process environment, then `env_text`
/// (the `.env` as it would be written). Nothing is read from disk.
fn validate(
    text: &str,
    env_text: &str,
    check: bool,
    out: &crate::out::Out,
    steps: &mut Vec<Step>,
) -> bool {
    let env = |name: &str| std::env::var(name).ok().or_else(|| get_env(env_text, name));
    let loaded = config::parse_yaml(text)
        .map_err(|e| {
            Error::config_hint(
                format!("config.yaml is not valid YAML: {e}"),
                "Fix config.yaml, then re-run `autorename-pdf setup`.",
            )
        })
        .and_then(|raw| config::from_raw(raw, &env))
        .map(|(tree, notices)| config::LoadedConfig {
            path: PathBuf::new(),
            tree,
            notices,
        });
    let loaded = match loaded {
        Ok(l) => l,
        Err(e) => {
            let s = step("validate", Status::Failed)
                .detail(e.to_string())
                .hint(e.hint().unwrap_or("Fix config.yaml, then re-run `setup`."));
            steps.push(if check {
                Step {
                    status: Status::Missing,
                    ..s
                }
            } else {
                s.fail(ExitCode::Config)
            });
            return false;
        }
    };
    let v = config::validate_loaded(&loaded, None);
    match v.issues.iter().find(|i| i.level == Level::Error) {
        Some(i) => {
            let hint = match i.field.as_str() {
                "ai.provider" | "ai.base_url" if i.message.starts_with("privacy: offline") => {
                    format!(
                        "Offline mode is saved in config.yaml. To use a cloud provider, leave it explicitly: `{} --provider P`; to stay offline: `{}`.",
                        out.command(&["setup", "--cloud"]),
                        out.command(&["setup", "--offline"])
                    )
                }
                "ai.api_key" => format!(
                    "Export the key and run `{} VAR`, pipe it to `{}`, or run offline: `{}`.",
                    out.command(&["setup", "--api-key-env"]),
                    out.command(&["setup", "--api-key-stdin"]),
                    out.command(&["setup", "--offline"])
                ),
                _ => i
                    .hint
                    .clone()
                    .unwrap_or_else(|| "Fix config.yaml, then re-run `setup`.".into()),
            };
            let s = step("validate", Status::Failed)
                .detail(format!("{} ({})", i.message, i.field))
                .hint(hint);
            steps.push(if check {
                Step {
                    status: Status::Missing,
                    ..s
                }
            } else {
                s.fail(ExitCode::Config)
            });
            false
        }
        None => {
            let mut s = step("validate", Status::Unchanged);
            if let Some(w) = v
                .issues
                .iter()
                .find(|i| i.level == Level::Warning && i.field == "company.names")
            {
                s = s.detail(w.message.clone()).hint(format!(
                    "Run `{} \"Your Company\"`.",
                    out.command(&["setup", "--company"])
                ));
            }
            steps.push(s);
            true
        }
    }
}

/// Ollama provider: the server answers and the model is pulled (local
/// `/api/tags` only). `None` for other providers.
fn ollama_step(text: &str) -> Option<Step> {
    let v = config::parse_yaml(text).ok()?;
    let ai = v.get("ai")?;
    if ai.get("provider")?.as_str()? != "ollama" {
        return None;
    }
    let base = ai.get("base_url").and_then(|b| b.as_str()).unwrap_or("");
    let model = ai
        .get("model")
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .or(config::default_model("ollama"))
        .unwrap_or("")
        .to_string();
    Some(match autorename_core::providers::ollama_models(base) {
        Ok(models) if autorename_core::providers::ollama_has_model(&models, &model) => {
            step("ollama", Status::Unchanged).detail(format!("model {model} installed"))
        }
        Ok(_) => step("ollama", Status::Missing)
            .detail(format!("model {model} is not installed in Ollama"))
            .hint(format!("ollama pull {model}")),
        Err(e) => step("ollama", Status::Missing)
            .detail(e.message.clone())
            .hint(
                e.hint
                    .clone()
                    .unwrap_or_else(|| "Start Ollama: `ollama serve`.".into()),
            ),
    })
}

fn ocr_step(text: &str, check: bool, offline: bool, ctx: &Ctx<'_>) -> Step {
    let lang = {
        let l = current(text, "ocr", "language");
        if l.is_empty() {
            ocr::AUTO_LANGUAGE.to_string()
        } else {
            l
        }
    };
    let pack = match ocr::pack_for_language_or_err(&lang) {
        Ok(p) => p,
        Err(e) => {
            return step("ocr_models", Status::Failed)
                .fail(ExitCode::Config)
                .detail(e.to_string());
        }
    };
    let root = match models_root() {
        Ok(r) => r,
        Err(e) => {
            return step("ocr_models", Status::Failed)
                .fail(ExitCode::General)
                .detail(e.to_string());
        }
    };
    let installed = ocr::models::list(&root)
        .iter()
        .any(|s| s.pack == pack.id && s.installed);
    if installed {
        return step("ocr_models", Status::Unchanged).detail(format!("pack {}", pack.id));
    }
    let out = ctx.out;
    // Offline never downloads: the only way in is a local copy.
    let import_hint = crate::cmd::models_hint(out, true, pack);
    if check {
        let s = step("ocr_models", Status::Missing).detail(format!(
            "pack {} not installed (optional; scans fall back to vision)",
            pack.id
        ));
        return if offline {
            s.hint(import_hint)
        } else {
            s.hint(format!(
                "`{}` (or `{}`)",
                out.command(&["setup", "--ocr-models"]),
                out.command(&["models", "fetch", pack.id])
            ))
        };
    }
    if offline {
        return step("ocr_models", Status::Failed)
            .fail(ExitCode::Config)
            .detail("privacy: offline forbids downloading OCR models")
            .hint(import_hint);
    }
    match ocr::models::fetch(
        &root,
        pack,
        &mut |p| out.progress_bytes(&format!("Downloading {}", p.file), p.downloaded, p.total),
        None,
    ) {
        Ok(()) => {
            out.finish_progress();
            step("ocr_models", Status::Done).detail(format!("pack {}", pack.id))
        }
        Err(e) => {
            out.finish_progress();
            step("ocr_models", Status::Failed)
                .fail(e.exit_code())
                .detail(e.to_string())
                .hint("Retry later, or `models import <dir>` from a local copy.")
        }
    }
}

fn integrate_step(check: bool) -> Step {
    use crate::integrate::{self, Action, State};
    let action = if check {
        Action::Status
    } else {
        Action::Install
    };
    match integrate::run(action, None) {
        Ok(r) => {
            let missing = r
                .items
                .iter()
                .any(|i| matches!(i.state, State::Missing | State::Outdated));
            let wrote = r.items.iter().any(|i| i.action == Some("written"));
            let status = match (check, missing, wrote) {
                (true, true, _) => Status::Missing,
                (true, false, _) | (false, _, false) => Status::Unchanged,
                (false, _, true) => Status::Done,
            };
            step("integrate", status).detail(format!("{} ({} entries)", r.platform, r.items.len()))
        }
        Err(e) => step("integrate", Status::Failed)
            .fail(e.exit_code())
            .detail(e.to_string()),
    }
}

fn print(r: &Report, ctx: &Ctx<'_>) {
    let out = ctx.out;
    if !out.is_text() {
        out.document(r);
        return;
    }
    out.line(&out.paint(&format!("# {}", r.config_path.display()), Style::Dim));
    for s in &r.steps {
        let (word, style) = match s.status {
            Status::Done => ("done", Style::Green),
            Status::Unchanged => ("ok", Style::Dim),
            Status::Failed => ("failed", Style::Red),
            Status::Missing => ("missing", Style::Yellow),
        };
        let detail = s.detail.as_deref().unwrap_or("");
        out.line(&format!(
            "  {} {}  {detail}",
            out.paint(&pad(word, 8), style),
            pad(s.step, 12)
        ));
        if let Some(h) = &s.hint {
            out.line(&format!("{}{}", " ".repeat(25), out.paint(h, Style::Dim)));
        }
    }
    out.line(&if r.ready {
        format!(
            "{} try: {} <folder>",
            out.paint("Ready.", Style::Green),
            out.command(&["rename", "--dry-run"])
        )
    } else {
        out.paint("Not ready yet (see above).", Style::Yellow)
    });
}
