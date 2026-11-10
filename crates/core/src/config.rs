//! Config v4: discovery, YAML load, defaults merge, `${VAR}` expansion,
//! schema validation, redaction and the typed [`Settings`] view.
//!
//! The config is first kept as an insertion-ordered tree (`serde_json::Value`)
//! so validation can report every problem at once (unknown keys, wrong types,
//! cross-field rules) instead of stopping at the first serde error.
//! [`settings`] converts a valid tree into typed data for the pipeline.
//! 3.x configs are not read; see `docs/migration-3-to-4.md`.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::profiles::{Profile, select_profile};
use crate::{Error, Result};

pub const CONFIG_FILE_NAME: &str = "config.yaml";
pub const NAME_LISTS_DIR_NAME: &str = "name-lists";
pub const CONFIG_ENV_VAR: &str = "AUTORENAME_PDF_CONFIG";
pub const APP_DIR_NAME: &str = "autorename-pdf";
pub const CONFIG_VERSION: u64 = 4;

pub const PROVIDERS: [&str; 5] = ["openai", "anthropic", "gemini", "xai", "ollama"];
pub const DECIDERS: [&str; 4] = ["ai", "openai-decisions", "jev", "off"];
pub const PRIVACY_MODES: [&str; 2] = ["cloud", "offline"];
pub const DECIMAL_SEPARATORS: [&str; 2] = [".", ","];
const PLACEHOLDER_API_KEYS: [&str; 2] = ["your-api-key-here", "your-api-key"];

/// The single default-model table (per provider). Values come from the
/// provider reference doc and are expected to change.
pub fn default_model(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("gpt-6-luna"),
        "anthropic" => Some("claude-haiku-5-5"),
        "gemini" => Some("gemini-3.5-flash-lite"),
        "xai" => Some("grok-4.7"),
        "ollama" => Some("gemma4"),
        _ => None,
    }
}

/// The standard API-key variable of a provider (`None` for Ollama), used
/// whenever the provider changes (`setup --provider`, `rename --provider`).
pub fn default_key_var(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("OPENAI_API_KEY"),
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "gemini" => Some("GEMINI_API_KEY"),
        "xai" => Some("XAI_API_KEY"),
        _ => None,
    }
}

/// `ai.api_key` for a newly chosen provider: `${VAR}` of its
/// [`default_key_var`], empty for Ollama.
pub fn default_key_ref(provider: &str) -> String {
    default_key_var(provider).map_or(String::new(), |v| format!("${{{v}}}"))
}

/// Defaults tree (key order = `config show` order). `ai.model` is filled
/// from [`default_model`] after merge when the user did not set it.
pub fn defaults() -> Value {
    json!({
        "config_version": CONFIG_VERSION,
        "privacy": "cloud",
        "ai": {
            "provider": "openai",
            "model": null,
            "api_key": "",
            "base_url": "",
            "temperature": 0.0,
            "max_retries": 2
        },
        "pdf": {
            "max_pages": 3,
            "ocr": "auto",
            "vision": "auto",
            "text_quality_threshold": 0.8,
            "incoming_invoice": "AP",
            "outgoing_invoice": "AR"
        },
        "ocr": {"language": "auto", "threads": 4},
        "company": {"names": []},
        "output": {"language": "English", "date_format": "%Y%m%d", "amount_format": "{currency} {amount}", "decimal_separator": "."},
        "enrich": {"enabled": true},
        "harmonization": {"auto_learn": true, "decider": "ai", "decider_api_key": ""},
        "prompt_extension": "",
        "profile": "business",
        "profiles": {}
    })
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Kind {
    Str,
    /// List of strings.
    StrList,
    Bool,
    /// `true | false | "auto"`
    Toggle,
    /// Integer ≥ 0 / ≥ 1.
    UInt,
    PosInt,
    Num,
    OneOf(&'static [&'static str]),
    Version,
}

const SECTIONS: &[(&str, &[(&str, Kind)])] = &[
    (
        "ai",
        &[
            ("provider", Kind::OneOf(&PROVIDERS)),
            ("model", Kind::Str),
            ("api_key", Kind::Str),
            ("base_url", Kind::Str),
            ("temperature", Kind::Num),
            ("max_retries", Kind::UInt),
        ],
    ),
    (
        "pdf",
        &[
            ("max_pages", Kind::PosInt),
            ("ocr", Kind::Toggle),
            ("vision", Kind::Toggle),
            ("text_quality_threshold", Kind::Num),
            ("incoming_invoice", Kind::Str),
            ("outgoing_invoice", Kind::Str),
        ],
    ),
    ("ocr", &[("language", Kind::Str), ("threads", Kind::PosInt)]),
    ("company", &[("names", Kind::StrList)]),
    (
        "output",
        &[
            ("language", Kind::Str),
            ("date_format", Kind::Str),
            ("amount_format", Kind::Str),
            ("decimal_separator", Kind::OneOf(&DECIMAL_SEPARATORS)),
        ],
    ),
    ("enrich", &[("enabled", Kind::Bool)]),
    (
        "harmonization",
        &[
            ("auto_learn", Kind::Bool),
            ("decider", Kind::OneOf(&DECIDERS)),
            ("decider_api_key", Kind::Str),
        ],
    ),
];

const TOP_SCALARS: &[(&str, Kind)] = &[
    ("config_version", Kind::Version),
    ("privacy", Kind::OneOf(&PRIVACY_MODES)),
    ("prompt_extension", Kind::Str),
    ("profile", Kind::Str),
];

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "list",
        Value::Object(_) => "mapping",
    }
}

fn check_kind(field: &str, kind: Kind, v: &Value) -> Option<Issue> {
    let ok = match kind {
        Kind::Str => v.is_string(),
        Kind::StrList => v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
        Kind::Bool => v.is_boolean(),
        Kind::Toggle => v.is_boolean() || v.as_str() == Some("auto"),
        Kind::UInt => v.as_u64().is_some_and(|n| u32::try_from(n).is_ok()),
        Kind::PosInt => v
            .as_u64()
            .is_some_and(|n| n >= 1 && u32::try_from(n).is_ok()),
        Kind::Num => v.is_number(),
        Kind::OneOf(allowed) => v.as_str().is_some_and(|s| allowed.contains(&s)),
        Kind::Version => v.as_u64() == Some(CONFIG_VERSION),
    };
    if ok {
        return None;
    }
    let expected = match kind {
        Kind::Str => "a string".to_string(),
        Kind::StrList => "a list of strings".to_string(),
        Kind::Bool => "true or false".to_string(),
        Kind::Toggle => "true, false, or \"auto\"".to_string(),
        Kind::UInt => format!("an integer from 0 to {}", u32::MAX),
        Kind::PosInt => format!("an integer from 1 to {}", u32::MAX),
        Kind::Num => "a number".to_string(),
        Kind::OneOf(allowed) => format!("one of: {}", allowed.join(", ")),
        Kind::Version => format!("{CONFIG_VERSION}"),
    };
    let got = match v {
        Value::String(s) => format!("{:?}", s),
        other => type_name(other).to_string(),
    };
    let issue = Issue::error(field, format!("{field} must be {expected} (got {got})"));
    Some(match kind {
        Kind::Version => issue.hint("3.x configs are not read; see docs/migration-3-to-4.md."),
        _ => issue,
    })
}

fn kind_schema(kind: Kind) -> Value {
    match kind {
        Kind::Str => json!({"type": "string"}),
        Kind::StrList => json!({"type": "array", "items": {"type": "string"}}),
        Kind::Bool => json!({"type": "boolean"}),
        Kind::Toggle => json!({"oneOf": [{"type": "boolean"}, {"const": "auto"}]}),
        Kind::UInt => json!({"type": "integer", "minimum": 0}),
        Kind::PosInt => json!({"type": "integer", "minimum": 1}),
        Kind::Num => json!({"type": "number"}),
        Kind::OneOf(allowed) => json!({"enum": allowed}),
        Kind::Version => json!({"const": CONFIG_VERSION}),
    }
}

/// JSON Schema (2020-12) of `config.yaml`, generated from the validator's
/// own key table so the two never drift. `null` anywhere means "default".
pub fn json_schema() -> Value {
    let d = defaults();
    let with_default = |mut s: Value, default: Option<&Value>| {
        if let (Some(obj), Some(dv)) = (s.as_object_mut(), default)
            && !dv.is_null()
        {
            obj.insert("default".into(), dv.clone());
        }
        s
    };
    let mut props = Map::new();
    for (key, kind) in TOP_SCALARS {
        props.insert((*key).into(), with_default(kind_schema(*kind), d.get(*key)));
    }
    for (section, keys) in SECTIONS {
        let mut inner = Map::new();
        for (k, kind) in *keys {
            inner.insert(
                (*k).into(),
                with_default(kind_schema(*kind), d.get(*section).and_then(|s| s.get(*k))),
            );
        }
        props.insert(
            (*section).into(),
            json!({"type": "object", "additionalProperties": false, "properties": inner}),
        );
    }
    props.insert(
        "profiles".into(),
        json!({
            "type": "object",
            "description": "Profile overlays (business, academic) and custom profiles: intro, fields, template, truncate_field, harmonize_field, direction, amount, extends.",
            "additionalProperties": {"type": "object"}
        }),
    );
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AutoRename-PDF config.yaml",
        "type": "object",
        "additionalProperties": false,
        "properties": props,
    })
}

// ---------------------------------------------------------------------------
// Issues
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub field: String,
    pub level: Level,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Issue {
    fn error(field: &str, message: impl Into<String>) -> Self {
        Issue {
            field: field.into(),
            level: Level::Error,
            message: message.into(),
            hint: None,
        }
    }
    fn warning(field: &str, message: impl Into<String>) -> Self {
        Issue {
            field: field.into(),
            level: Level::Warning,
            message: message.into(),
            hint: None,
        }
    }
    fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Discovery & paths
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    /// `--config PATH`
    Cli,
    /// `AUTORENAME_PDF_CONFIG`
    Env,
    /// OS config dir (`%APPDATA%`, `~/.config`, `~/Library/Application Support`)
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocation {
    pub path: PathBuf,
    pub source: ConfigSource,
}

/// Pure discovery: `--config` > env var > OS config dir (existing or not;
/// `config init` writes there). Without any OS dir: `./config.yaml`.
pub fn discover_config_with(
    cli: Option<&Path>,
    env: Option<PathBuf>,
    user_config_dir: Option<&Path>,
) -> ConfigLocation {
    if let Some(p) = cli {
        return ConfigLocation {
            path: p.to_path_buf(),
            source: ConfigSource::Cli,
        };
    }
    if let Some(p) = env.filter(|p| !p.as_os_str().is_empty()) {
        return ConfigLocation {
            path: p,
            source: ConfigSource::Env,
        };
    }
    let path = user_config_dir.map_or_else(
        || PathBuf::from(CONFIG_FILE_NAME),
        |d| d.join(CONFIG_FILE_NAME),
    );
    ConfigLocation {
        path,
        source: ConfigSource::User,
    }
}

pub fn user_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR_NAME))
}

/// Per-machine data dir (undo journal, OCR models). Local, not roaming.
/// Env override for every piece of app state (undo journal, OCR models,
/// locks): isolates agents, tests and portable installs.
pub const DATA_DIR_ENV: &str = "AUTORENAME_PDF_DATA_DIR";

/// App state directory: `$AUTORENAME_PDF_DATA_DIR`, else the OS local data
/// dir + `autorename-pdf`.
pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os(DATA_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|d| d.join(APP_DIR_NAME)))
}

pub fn discover_config(cli: Option<&Path>) -> ConfigLocation {
    discover_config_with(
        cli,
        std::env::var_os(CONFIG_ENV_VAR).map(PathBuf::from),
        user_config_dir().as_deref(),
    )
}

/// `name-lists/` beside the config file: one name list per harmonized field.
pub fn name_lists_dir(config_path: &Path) -> PathBuf {
    config_dir_of(config_path).join(NAME_LISTS_DIR_NAME)
}

/// `name-lists/<field>.yaml` beside the config file (see [`name_list_in`]).
pub fn name_list_path(config_path: &Path, field: &str) -> PathBuf {
    name_list_in(&name_lists_dir(config_path), field)
}

/// `<dir>/<field>.yaml`, the field id verbatim. Callers pass only ids that
/// passed [`crate::harmonize::is_field_id`] (lowercase identifier, no
/// Windows device name), so the name never leaves `dir`.
pub fn name_list_in(dir: &Path, field: &str) -> PathBuf {
    debug_assert!(
        crate::harmonize::is_field_id(field),
        "unchecked name-list field {field:?}"
    );
    dir.join(format!("{field}.yaml"))
}

/// `.env` beside the config (API keys referenced as `${VAR}`).
pub fn env_path(config_path: &Path) -> PathBuf {
    config_dir_of(config_path).join(".env")
}

fn config_dir_of(config_path: &Path) -> PathBuf {
    let abs = std::path::absolute(config_path).unwrap_or_else(|_| config_path.to_path_buf());
    abs.parent().map(Path::to_path_buf).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    /// Merged, env-expanded tree (not yet validated).
    pub tree: Value,
    /// Load-time notices (unset `${VAR}`s), reported by [`validate_loaded`].
    pub notices: Vec<Issue>,
}

/// Parse YAML 1.2 text (duplicate keys rejected at every depth).
pub fn parse_yaml(text: &str) -> std::result::Result<Value, String> {
    let options = serde_saphyr::options! { strict_booleans: true };
    serde_saphyr::from_str_with_options::<Value>(text, options).map_err(|e| e.to_string())
}

/// `KEY=VALUE` pairs from `.env` beside the config (never applied to the process env).
pub fn read_dotenv(config_path: &Path) -> HashMap<String, String> {
    let path = config_dir_of(config_path).join(".env");
    dotenvy::from_path_iter(&path)
        .map(|it| it.flatten().collect())
        .unwrap_or_default()
}

/// Load with the process environment; `.env` beside the config fills gaps only.
pub fn load(path: &Path) -> Result<LoadedConfig> {
    load_with_overrides(path, &Overrides::default())
}

/// [`load`] with one-run overrides applied to the file's values before
/// `${VAR}` expansion, as if the file said so (a new provider brings its own
/// key variable, see [`apply_overrides`]).
pub fn load_with_overrides(path: &Path, o: &Overrides) -> Result<LoadedConfig> {
    let dotenv = read_dotenv(path);
    load_inner(path, o, &|name| {
        std::env::var(name)
            .ok()
            .or_else(|| dotenv.get(name).cloned())
    })
}

/// Load with an injected variable lookup (tests).
pub fn load_with_env(path: &Path, env: &dyn Fn(&str) -> Option<String>) -> Result<LoadedConfig> {
    load_inner(path, &Overrides::default(), env)
}

fn load_inner(
    path: &Path,
    o: &Overrides,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<LoadedConfig> {
    let display = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if !path.is_file() {
        return Err(Error::config_hint(
            format!("Config file not found: {}", display.display()),
            "Run `autorename-pdf config init`.",
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let raw = parse_yaml(&text).map_err(|e| {
        Error::config_hint(
            format!("Config file unreadable: {}: {e}", display.display()),
            "Fix the YAML syntax (duplicate keys are errors).",
        )
    })?;
    let (tree, notices) = from_raw_with(raw, o, env)?;
    Ok(LoadedConfig {
        path: path.to_path_buf(),
        tree,
        notices,
    })
}

/// Parsed YAML → merged, env-expanded tree + notices. An empty document is `{}`.
pub fn from_raw(raw: Value, env: &dyn Fn(&str) -> Option<String>) -> Result<(Value, Vec<Issue>)> {
    from_raw_with(raw, &Overrides::default(), env)
}

/// [`from_raw`] with one-run overrides applied before expansion.
pub fn from_raw_with(
    raw: Value,
    o: &Overrides,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(Value, Vec<Issue>)> {
    let raw = match raw {
        Value::Null => Value::Object(Map::new()),
        v @ Value::Object(_) => v,
        _ => return Err(Error::config("Config file must be a YAML mapping")),
    };
    let mut tree = deep_merge(&defaults(), &raw);
    // The effective provider decides whether an override changes it
    // (`provider: ${PROVIDER}`); an unset variable stays literal and still
    // gets its notice below.
    let provider = str_at(&tree, "ai", "provider");
    if provider.contains("${") {
        let provider = expand_env(provider, env).0;
        set_in(&mut tree, "ai", "provider", json!(provider));
    }
    apply_overrides(&mut tree, o);
    let mut notices = Vec::new();
    // Expand on the parsed tree (no YAML injection) before provider-dependent
    // defaults, so `provider: ${PROVIDER}` picks the right default model.
    expand_env_tree(&mut tree, "", env, &mut notices);
    fill_default_model(&mut tree);
    fill_decider_key(&mut tree, env);
    Ok((tree, notices))
}

/// Env variable a remote decider falls back to when
/// `harmonization.decider_api_key` is empty (`None` = uses `ai.api_key`).
pub fn decider_key_fallback(decider: &str, provider: &str) -> Option<&'static str> {
    match decider {
        "openai-decisions" if provider == "openai" => None,
        "openai-decisions" => Some("OPENAI_API_KEY"),
        "jev" => Some("JEV_API_KEY"),
        _ => None,
    }
}

/// Empty `decider_api_key` → the documented fallback (`ai.api_key` for
/// openai-decisions on OpenAI, else `OPENAI_API_KEY` / `JEV_API_KEY`).
fn fill_decider_key(tree: &mut Value, env: &dyn Fn(&str) -> Option<String>) {
    let decider = str_at(tree, "harmonization", "decider").to_string();
    let provider = str_at(tree, "ai", "provider").to_string();
    if !str_at(tree, "harmonization", "decider_api_key").is_empty()
        || !matches!(decider.as_str(), "openai-decisions" | "jev")
    {
        return;
    }
    let key = match decider_key_fallback(&decider, &provider) {
        None => str_at(tree, "ai", "api_key").to_string(),
        Some(var) => env(var).unwrap_or_default(),
    };
    if let Some(h) = tree.get_mut("harmonization").and_then(Value::as_object_mut) {
        h.insert("decider_api_key".into(), Value::String(key));
    }
}

/// Overlay `overrides` on `defaults`: mapping sections merge key by key, a
/// `null` value means "use the default", anything else replaces. `profiles`
/// is taken verbatim (its `null`s mean "delete field").
pub fn deep_merge(defaults: &Value, overrides: &Value) -> Value {
    match (defaults, overrides) {
        (Value::Object(d), Value::Object(o)) => {
            let mut out = d.clone();
            for (k, v) in o {
                let merged = match (out.get(k), v) {
                    (_, Value::Null) if out.contains_key(k) => continue,
                    (Some(dv @ Value::Object(_)), Value::Object(_)) if k != "profiles" => {
                        deep_merge(dv, v)
                    }
                    _ => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        (_, o) => o.clone(),
    }
}

fn fill_default_model(tree: &mut Value) {
    let provider = tree
        .pointer("/ai/provider")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(ai) = tree.get_mut("ai").and_then(Value::as_object_mut)
        && ai.get("model").is_none_or(Value::is_null)
    {
        ai.insert(
            "model".into(),
            default_model(&provider).map_or(json!(""), Value::from),
        );
    }
}

/// Replace `${VAR}`; unset vars stay literal (validation flags an api_key).
pub fn expand_env(value: &str, env: &dyn Fn(&str) -> Option<String>) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut missing = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                out.push_str(&rest[..start]);
                match env(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push_str(&rest[start..start + 3 + end]);
                        missing.push(name.to_string());
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..start + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    (out, missing)
}

fn expand_env_tree(
    v: &mut Value,
    path: &str,
    env: &dyn Fn(&str) -> Option<String>,
    notices: &mut Vec<Issue>,
) {
    match v {
        Value::Object(m) => {
            for (k, child) in m.iter_mut() {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                expand_env_tree(child, &p, env, notices);
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|c| expand_env_tree(c, path, env, notices)),
        Value::String(s) if s.contains("${") => {
            let (expanded, missing) = expand_env(s, env);
            for name in missing {
                notices.push(Issue::warning(
                    path,
                    format!("Environment variable ${{{name}}} is not set"),
                ));
            }
            *s = expanded;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Validation {
    pub valid: bool,
    pub issues: Vec<Issue>,
    pub profile: Option<String>,
    pub resolved_profile: Option<Profile>,
}

/// `company.names`, trimmed, empty entries dropped.
pub fn company_names(config: &Value) -> Vec<String> {
    config
        .get("company")
        .and_then(|c| c.get("names"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect()
}

fn str_at<'a>(config: &'a Value, section: &str, key: &str) -> &'a str {
    config
        .get(section)
        .and_then(|s| s.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Keys renamed during the 4.0 betas: (old key, hint naming the new one).
const RENAMED: [(&str, &str); 1] = [(
    "company.name",
    "company.names takes a list of your names, e.g. names: [\"Spqrk GmbH\", \"Jane Doe\"].",
)];

fn unknown_key(field: &str) -> Issue {
    let issue = Issue::error(field, format!("unknown key {field:?}"));
    match RENAMED.iter().find(|(old, _)| *old == field) {
        Some((_, hint)) => issue.hint(*hint),
        None => issue,
    }
}

/// Schema check: unknown keys, section shapes, value types.
fn check_schema(config: &Value, issues: &mut Vec<Issue>) -> bool {
    let Some(root) = config.as_object() else {
        issues.push(Issue::error("", "config must be a mapping"));
        return false;
    };
    let mut shapes_ok = true;
    for (key, value) in root {
        if let Some((_, kind)) = TOP_SCALARS.iter().find(|(k, _)| k == key) {
            issues.extend(check_kind(key, *kind, value));
        } else if let Some((_, keys)) = SECTIONS.iter().find(|(s, _)| s == key) {
            let Some(section) = value.as_object() else {
                issues.push(Issue::error(
                    key,
                    format!("{key} must be a mapping (got {})", type_name(value)),
                ));
                shapes_ok = false;
                continue;
            };
            for (k, v) in section {
                let field = format!("{key}.{k}");
                match keys.iter().find(|(name, _)| name == k) {
                    Some((_, kind)) => issues.extend(check_kind(&field, *kind, v)),
                    None => issues.push(unknown_key(&field)),
                }
            }
        } else if key != "profiles" {
            issues.push(Issue::error(key, format!("unknown key {:?}", key)));
        }
    }
    shapes_ok
}

/// Full validation (`config validate`): [`validate_base`] plus
/// [`provider_issues`].
pub fn validate(config: &Value, profile_override: Option<&str>) -> Validation {
    let mut v = validate_base(config, profile_override);
    v.issues.extend(provider_issues(config));
    v.valid = !v.issues.iter().any(|i| i.level == Level::Error);
    v
}

/// Requirements for calling the configured provider: API key, and under
/// `privacy: offline` a local Ollama endpoint and no cloud decider. Only
/// planning (`rename`) needs these ([`Settings::require_provider`]).
pub fn provider_issues(config: &Value) -> Vec<Issue> {
    let mut issues = Vec::new();
    // Offline: only the offline policy applies; cloud credentials (API key,
    // decider keys) are irrelevant and never suggested.
    if config.get("privacy").and_then(Value::as_str) == Some("offline") {
        validate_offline(config, &mut issues);
        return issues;
    }
    let provider = str_at(config, "ai", "provider");
    let api_key = str_at(config, "ai", "api_key");
    if provider != "ollama" && api_key.is_empty() {
        issues.push(
            Issue::error("ai.api_key", "API key required for non-ollama providers")
                .hint("Set ai.api_key, e.g. \"${OPENAI_API_KEY}\" with the key in .env beside config.yaml."),
        );
    } else if provider != "ollama" && api_key.contains("${") {
        issues.push(
            Issue::error(
                "ai.api_key",
                "API key still contains an unexpanded ${VAR} reference",
            )
            .hint(format!(
                "Set {} in the environment or in .env beside config.yaml.",
                expand_env(api_key, &|_| None).1.join(", ")
            )),
        );
    }
    if PLACEHOLDER_API_KEYS.contains(&api_key.trim().to_lowercase().as_str()) {
        issues.push(Issue::warning(
            "ai.api_key",
            "Placeholder API key; replace before calling a cloud provider",
        ));
    }
    let decider = str_at(config, "harmonization", "decider");
    if matches!(decider, "openai-decisions" | "jev") {
        let key = str_at(config, "harmonization", "decider_api_key");
        if key.is_empty() || key.contains("${") {
            let var = decider_key_fallback(decider, provider).unwrap_or("OPENAI_API_KEY");
            issues.push(
                Issue::error(
                    "harmonization.decider_api_key",
                    format!("decider '{decider}' needs an API key"),
                )
                .hint(format!(
                    "Set harmonization.decider_api_key: \"${{{var}}}\" and put {var}=... into .env beside config.yaml (or `autorename-pdf setup --decider {decider} --decider-key-env {var}`), or use harmonization.decider: ai."
                )),
            );
        }
    }
    issues
}

/// Everything every command needs: schema, value types, cross-field
/// warnings and the profile. No provider credentials.
pub fn validate_base(config: &Value, profile_override: Option<&str>) -> Validation {
    let mut issues = Vec::new();
    let shapes_ok = check_schema(config, &mut issues);
    if str_at(config, "ai", "model").is_empty() {
        issues.push(Issue::warning("ai.model", "No model set"));
    }
    if !str_at(config, "output", "amount_format").contains("{amount}") {
        issues.push(
            Issue::error(
                "output.amount_format",
                "output.amount_format must contain {amount}",
            )
            .hint("Default: \"{currency} {amount}\"."),
        );
    }
    let fmt = str_at(config, "output", "date_format");
    let sample = chrono::NaiveDate::from_ymd_opt(2025, 11, 3).expect("valid date");
    let rendered = crate::dates::format_date(sample, fmt);
    if crate::naming::sanitize_filename(&rendered) != rendered {
        issues.push(
            Issue::error(
                "output.date_format",
                format!("output.date_format renders {rendered:?}, which contains characters not allowed in file names"),
            )
            .hint("Use separators like - _ . or none, e.g. \"%Y-%m-%d\"."),
        );
    }
    if company_names(config).is_empty() {
        issues.push(Issue::warning(
            "company.names",
            "Your company names are not configured. AI cannot distinguish incoming vs outgoing invoices.",
        ));
    }

    let (profile, resolved_profile) = if shapes_ok {
        match select_profile(config, profile_override) {
            Ok((id, p)) => (Some(id), Some(p)),
            Err(e) => {
                issues.push(Issue::error("profile", e.to_string()));
                (None, None)
            }
        }
    } else {
        (None, None)
    };
    let valid = !issues.iter().any(|i| i.level == Level::Error);
    Validation {
        valid,
        issues,
        profile,
        resolved_profile,
    }
}

fn validate_offline(config: &Value, issues: &mut Vec<Issue>) {
    if str_at(config, "ai", "provider") != "ollama" {
        issues.push(
            Issue::error(
                "ai.provider",
                "privacy: offline requires ai.provider: ollama",
            )
            .hint("Offline runs on a local Ollama: set ai.provider: ollama with harmonization.decider: ai or off (`autorename-pdf setup --offline`), or leave offline mode: privacy: cloud (`autorename-pdf setup --cloud`)."),
        );
    }
    let base_url = str_at(config, "ai", "base_url");
    if !base_url.is_empty() && !is_local_url(base_url) {
        issues.push(
            Issue::error(
                "ai.base_url",
                format!("privacy: offline requires a local ai.base_url, got {base_url}"),
            )
            .hint("Use localhost, a loopback address or a private LAN host."),
        );
    }
    let decider = str_at(config, "harmonization", "decider");
    if matches!(decider, "openai-decisions" | "jev") {
        issues.push(
            Issue::error(
                "harmonization.decider",
                format!("privacy: offline forbids the cloud decider '{decider}'"),
            )
            .hint("Use harmonization.decider: ai (local model) or off."),
        );
    }
}

/// Validation including load-time notices (unset env vars).
pub fn validate_loaded(loaded: &LoadedConfig, profile_override: Option<&str>) -> Validation {
    let mut v = validate(&loaded.tree, profile_override);
    let mut issues = loaded.notices.clone();
    issues.append(&mut v.issues);
    v.issues = issues;
    v
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// Copy with `ai.api_key` redacted (`sk-1...cdef`, or `***` when ≤ 8 chars).
pub fn redact(config: &Value) -> Value {
    let mut out = config.clone();
    for ptr in ["/ai/api_key", "/harmonization/decider_api_key"] {
        redact_at(&mut out, ptr);
    }
    out
}

fn redact_at(out: &mut Value, ptr: &str) {
    if let Some(Value::String(key)) = out.pointer_mut(ptr)
        && !key.is_empty()
    {
        let chars: Vec<char> = key.chars().collect();
        *key = if chars.len() > 8 {
            let head: String = chars[..4].iter().collect();
            let tail: String = chars[chars.len() - 4..].iter().collect();
            format!("{head}...{tail}")
        } else {
            "***".into()
        };
    }
}

// ---------------------------------------------------------------------------
// CLI overrides
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Toggle {
    Off,
    On,
    Auto,
}

impl Toggle {
    /// CLI spelling: `auto | on | off | true | false` (case-insensitive).
    pub fn parse(s: &str) -> Option<Toggle> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Toggle::Auto),
            "on" | "true" => Some(Toggle::On),
            "off" | "false" => Some(Toggle::Off),
            _ => None,
        }
    }
    fn to_value(self) -> Value {
        match self {
            Toggle::Off => json!(false),
            Toggle::On => json!(true),
            Toggle::Auto => json!("auto"),
        }
    }
    fn from_value(v: &Value) -> Option<Toggle> {
        match v {
            Value::Bool(true) => Some(Toggle::On),
            Value::Bool(false) => Some(Toggle::Off),
            Value::String(s) if s == "auto" => Some(Toggle::Auto),
            _ => None,
        }
    }
}

/// One-run overrides, applied before validation.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub ocr: Option<Toggle>,
    pub vision: Option<Toggle>,
    pub no_enrich: bool,
    pub no_learn: bool,
    pub offline: bool,
}

fn set_in(tree: &mut Value, section: &str, key: &str, value: Value) {
    if let Value::Object(root) = tree {
        let s = root
            .entry(section)
            .or_insert_with(|| Value::Object(Map::new()));
        if !s.is_object() {
            *s = Value::Object(Map::new());
        }
        s.as_object_mut().expect("object").insert(key.into(), value);
    }
}

/// Apply one-run overrides. A provider different from the configured one
/// brings its own defaults, exactly like `setup --provider`: its default
/// model (unless `model` is given), its default endpoint and its standard
/// key variable (`${ANTHROPIC_API_KEY}`; none for Ollama). Apply before
/// `${VAR}` expansion ([`load_with_overrides`]) so that reference resolves;
/// on an expanded tree only the Ollama switch (`offline`) is complete.
pub fn apply_overrides(tree: &mut Value, o: &Overrides) {
    if o.offline {
        apply_offline_override(tree);
    }
    if let Some(p) = &o.provider
        && str_at(tree, "ai", "provider") != p
    {
        set_in(tree, "ai", "provider", json!(p));
        set_in(
            tree,
            "ai",
            "model",
            default_model(p).map_or(json!(""), Value::from),
        );
        set_in(tree, "ai", "base_url", json!(""));
        set_in(tree, "ai", "api_key", json!(default_key_ref(p)));
    }
    if let Some(m) = &o.model {
        set_in(tree, "ai", "model", json!(m));
    }
    if let Some(v) = o.ocr {
        set_in(tree, "pdf", "ocr", v.to_value());
    }
    if let Some(v) = o.vision {
        set_in(tree, "pdf", "vision", v.to_value());
    }
    if o.no_enrich {
        set_in(tree, "enrich", "enabled", json!(false));
    }
    if o.no_learn {
        set_in(tree, "harmonization", "auto_learn", json!(false));
    }
}

/// `--offline`: privacy offline + provider ollama (from another provider:
/// its default model, local endpoint, no key). Remaining violations (a
/// remote Ollama base_url, cloud decider) still fail validation.
pub fn apply_offline_override(tree: &mut Value) {
    if str_at(tree, "ai", "provider") != "ollama" {
        apply_overrides(
            tree,
            &Overrides {
                provider: Some("ollama".into()),
                ..Default::default()
            },
        );
    }
    if let Value::Object(root) = tree {
        root.insert("privacy".into(), json!("offline"));
    }
}

// ---------------------------------------------------------------------------
// Typed settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAi,
    Anthropic,
    Gemini,
    Xai,
    Ollama,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Provider> {
        Some(match s {
            "openai" => Provider::OpenAi,
            "anthropic" => Provider::Anthropic,
            "gemini" => Provider::Gemini,
            "xai" => Provider::Xai,
            "ollama" => Provider::Ollama,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Gemini => "gemini",
            Provider::Xai => "xai",
            Provider::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decider {
    Ai,
    OpenaiDecisions,
    Jev,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Privacy {
    Cloud,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AiSettings {
    pub provider: Provider,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub temperature: f64,
    pub max_retries: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PdfSettings {
    pub max_pages: u32,
    pub ocr: Toggle,
    pub vision: Toggle,
    pub text_quality_threshold: f64,
    pub incoming_invoice: String,
    pub outgoing_invoice: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OcrSettings {
    pub language: String,
    pub threads: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    pub privacy: Privacy,
    pub ai: AiSettings,
    pub pdf: PdfSettings,
    pub ocr: OcrSettings,
    /// `company.names`: every name of my company (trimmed, non-empty).
    pub company_names: Vec<String>,
    pub output_language: String,
    pub date_format: String,
    /// `{currency}` and `{amount}` → invoice amount in filenames.
    pub amount_format: String,
    /// `.` or `,`: decimal mark of amounts in filenames.
    pub decimal_separator: String,
    pub prompt_extension: String,
    pub enrich_enabled: bool,
    pub auto_learn: bool,
    pub decider: Decider,
    /// Resolved key of a remote decider (`harmonization.decider_api_key`, or
    /// the fallback filled in at load time).
    pub decider_api_key: String,
    pub profile_id: String,
    pub profile: Profile,
    /// [`provider_issues`] of the tree (checked by [`Settings::require_provider`]).
    pub provider_issues: Vec<Issue>,
}

impl Settings {
    /// First provider-requirement error (missing API key, offline rules) as
    /// a config error. Call before anything that talks to the provider.
    pub fn require_provider(&self) -> Result<()> {
        let errors: Vec<&Issue> = self
            .provider_issues
            .iter()
            .filter(|i| i.level == Level::Error)
            .collect();
        let Some(first) = errors.first() else {
            return Ok(());
        };
        let mut message = first.message.clone();
        if errors.len() > 1 {
            let rest: Vec<&str> = errors[1..].iter().map(|i| i.message.as_str()).collect();
            message.push_str(&format!(" (also: {})", rest.join("; ")));
        }
        Err(Error::Config {
            message,
            hint: first.hint.clone(),
        })
    }
}

/// Base-validate ([`validate_base`]), then build typed settings. Errors carry
/// the first validation error; provider requirements are deferred to
/// [`Settings::require_provider`].
pub fn settings(tree: &Value, profile_override: Option<&str>) -> Result<Settings> {
    let v = validate_base(tree, profile_override);
    if let Some(issue) = v.issues.iter().find(|i| i.level == Level::Error) {
        return Err(Error::Config {
            message: issue.message.clone(),
            hint: issue.hint.clone(),
        });
    }
    // Schema-valid and defaults-merged: every key below exists with its type.
    let get = |s: &str, k: &str| {
        tree.get(s)
            .and_then(|x| x.get(k))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let text = |s: &str, k: &str| str_at(tree, s, k).to_string();
    let num = |s: &str, k: &str| get(s, k).as_f64().unwrap_or_default();
    let uint = |s: &str, k: &str| -> Result<u32> {
        get(s, k)
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| {
                Error::config(format!("{s}.{k} must be an integer from 0 to {}", u32::MAX))
            })
    };
    let flag = |s: &str, k: &str| get(s, k).as_bool().unwrap_or_default();
    let toggle = |s: &str, k: &str| Toggle::from_value(&get(s, k)).unwrap_or(Toggle::Off);
    let top = |k: &str| tree.get(k).and_then(Value::as_str).unwrap_or("");
    Ok(Settings {
        privacy: if top("privacy") == "offline" {
            Privacy::Offline
        } else {
            Privacy::Cloud
        },
        ai: AiSettings {
            provider: Provider::parse(str_at(tree, "ai", "provider")).unwrap_or(Provider::OpenAi),
            model: text("ai", "model"),
            api_key: text("ai", "api_key"),
            base_url: text("ai", "base_url"),
            temperature: num("ai", "temperature"),
            max_retries: uint("ai", "max_retries")?,
        },
        pdf: PdfSettings {
            max_pages: uint("pdf", "max_pages")?,
            ocr: toggle("pdf", "ocr"),
            vision: toggle("pdf", "vision"),
            text_quality_threshold: num("pdf", "text_quality_threshold"),
            incoming_invoice: text("pdf", "incoming_invoice"),
            outgoing_invoice: text("pdf", "outgoing_invoice"),
        },
        ocr: OcrSettings {
            language: text("ocr", "language"),
            threads: uint("ocr", "threads")?,
        },
        company_names: company_names(tree),
        output_language: text("output", "language"),
        date_format: text("output", "date_format"),
        amount_format: text("output", "amount_format"),
        decimal_separator: text("output", "decimal_separator"),
        prompt_extension: top("prompt_extension").to_string(),
        enrich_enabled: flag("enrich", "enabled"),
        auto_learn: flag("harmonization", "auto_learn"),
        decider: match str_at(tree, "harmonization", "decider") {
            "openai-decisions" => Decider::OpenaiDecisions,
            "jev" => Decider::Jev,
            "off" => Decider::Off,
            _ => Decider::Ai,
        },
        decider_api_key: text("harmonization", "decider_api_key"),
        provider_issues: provider_issues(tree),
        profile_id: v.profile.unwrap_or_default(),
        profile: v
            .resolved_profile
            .ok_or_else(|| Error::config("no profile"))?,
    })
}

// ---------------------------------------------------------------------------
// Network locality
// ---------------------------------------------------------------------------

/// Loopback, private (RFC 1918 / ULA) or link-local address.
pub fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_v4(v4);
            }
            let seg0 = v6.segments()[0];
            v6 == Ipv6Addr::LOCALHOST || (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

/// Local host *name* (not an IP literal): `localhost`, `*.localhost`, mDNS
/// `*.local`, `*.lan`, `*.home.arpa`, `*.internal`, or a single-label LAN
/// hostname (e.g. `nas`, `ollama`). Names still resolve through the offline
/// resolver, which rejects non-local addresses.
pub fn is_local_host_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if host == "localhost"
        || [".localhost", ".local", ".lan", ".home.arpa", ".internal"]
            .iter()
            .any(|s| host.ends_with(s))
    {
        return true;
    }
    !host.contains('.') && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// True when `url` is an `http(s)` URL whose host — as the HTTP client
/// parses it (WHATWG URL: `http://134744072` is `8.8.8.8`, userinfo
/// stripped) — is a local IP ([`is_local_ip`]) or a local name
/// ([`is_local_host_name`]).
pub fn is_local_url(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url.trim()) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    // `host_str` is canonical: IPv4 dotted-decimal, IPv6 in brackets.
    let Some(host) = u.host_str() else {
        return false;
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(ip) => is_local_ip(ip),
        Err(_) => is_local_host_name(host),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn merged(raw: Value) -> Value {
        from_raw(raw, &no_env).unwrap().0
    }

    fn valid_base() -> Value {
        merged(json!({"ai": {"api_key": "key", "model": "m"}, "company": {"names": ["Acme"]}}))
    }

    fn with(base: Value, patch: Value) -> Value {
        deep_merge(&base, &patch)
    }

    fn fields_at(v: &Validation, level: Level) -> Vec<String> {
        v.issues
            .iter()
            .filter(|i| i.level == level)
            .map(|i| i.field.clone())
            .collect()
    }

    #[test]
    fn local_url_table() {
        let table = [
            ("http://localhost:11434/v1", true),
            ("http://127.0.0.1:11434", true),
            ("http://127.8.0.1", true),
            ("http://[::1]:11434/v1", true),
            ("http://192.168.1.20:11434", true),
            ("http://10.0.0.5", true),
            ("http://172.16.4.1", true),
            ("http://172.32.0.1", false),
            ("http://169.254.1.1", true),
            ("http://[fd00::1]/", true),
            ("http://[fe80::1]/", true),
            ("http://[::ffff:192.168.0.1]/", true),
            ("http://nas.local:11434", true),
            ("http://box.lan", true),
            ("http://ollama:11434", true),
            ("http://user:pw@localhost/", true),
            ("HTTP://LOCALHOST:11434", true),
            ("http://localhost.:11434", true),
            // Not HTTP(S) / not a URL.
            ("localhost:11434", false),
            ("ftp://localhost/", false),
            ("file:///etc/passwd", false),
            // The client's parser decides the destination (all are 8.8.8.8).
            ("http://134744072", false),
            ("http://0x08080808", false),
            ("http://8.8.8.8\\@localhost", false),
            ("http://010.010.010.010", false),
            ("http://8.8.8.8#@localhost", false),
            ("http://localhost@8.8.8.8", false),
            ("http://[::ffff:8.8.8.8]/", false),
            ("http://0x7f000001", true),
            ("http://2130706433", true),
            ("https://api.openai.com/v1", false),
            ("http://8.8.8.8", false),
            ("http://[2001:db8::1]/", false),
            ("http://localhost.evil.com", false),
            ("", false),
        ];
        for (url, expected) in table {
            assert_eq!(is_local_url(url), expected, "{url}");
        }
    }

    #[test]
    fn offline_reports_only_the_policy_never_cloud_credentials() {
        for decider in ["jev", "openai-decisions"] {
            let t = merged(
                json!({"privacy": "offline", "ai": {"provider": "openai", "api_key": ""}, "harmonization": {"decider": decider}}),
            );
            let issues = provider_issues(&t);
            let fields: Vec<&str> = issues.iter().map(|i| i.field.as_str()).collect();
            assert_eq!(
                fields,
                ["ai.provider", "harmonization.decider"],
                "{decider}"
            );
            let text = format!("{issues:?}");
            assert!(
                !text.contains("API key") && !text.contains("API_KEY"),
                "{text}"
            );
            let e = settings(&t, None).unwrap().require_provider().unwrap_err();
            assert!(
                e.to_string()
                    .starts_with("privacy: offline requires ai.provider: ollama")
            );
            assert!(e.hint().unwrap().contains("ollama") && e.hint().unwrap().contains("decider"));
        }
        // A valid offline config has no credential issue at all.
        let ok = merged(
            json!({"privacy": "offline", "ai": {"provider": "ollama", "api_key": "${NOPE}"}}),
        );
        assert!(provider_issues(&ok).is_empty());
    }

    #[test]
    fn decider_key_resolution_and_validation() {
        let env = |n: &str| (n == "DEC").then(|| "dec-secret-123456".to_string());
        let tree = |v: Value| from_raw(v, &env).unwrap().0;
        let errs = |t: &Value| fields_at(&validate(t, None), Level::Error);
        let base = json!({"ai": {"provider": "anthropic", "api_key": "k"}, "company": {"names": ["Spqrk"]}});
        // jev without key → error with hint; explicit ${VAR} key → ok.
        let t = tree(with(
            base.clone(),
            json!({"harmonization": {"decider": "jev"}}),
        ));
        assert_eq!(errs(&t), ["harmonization.decider_api_key"]);
        assert!(
            provider_issues(&t)[0]
                .hint
                .as_deref()
                .unwrap()
                .contains("JEV_API_KEY")
        );
        let t = tree(with(
            base.clone(),
            json!({"harmonization": {"decider": "jev", "decider_api_key": "${DEC}"}}),
        ));
        assert!(errs(&t).is_empty());
        let s = settings(&t, None).unwrap();
        assert_eq!(s.decider_api_key, "dec-secret-123456");
        assert_eq!(
            redact(&t)["harmonization"]["decider_api_key"],
            "dec-...3456"
        );
        // openai-decisions on the OpenAI provider falls back to ai.api_key.
        let t = tree(
            json!({"ai": {"provider": "openai", "api_key": "main-key"}, "harmonization": {"decider": "openai-decisions"}}),
        );
        assert!(errs(&t).is_empty());
        assert_eq!(settings(&t, None).unwrap().decider_api_key, "main-key");
        // Unset ${VAR} stays literal → error; decider ai needs nothing.
        let t = tree(with(
            base.clone(),
            json!({"harmonization": {"decider": "openai-decisions", "decider_api_key": "${NOPE}"}}),
        ));
        assert_eq!(errs(&t), ["harmonization.decider_api_key"]);
        assert!(errs(&tree(base)).is_empty());
    }

    #[test]
    fn provider_requirements_are_separate_from_base_settings() {
        let no_key = merged(json!({"company": {"names": ["Acme"]}}));
        let s = settings(&no_key, None).expect("apply/undo/inspect need no API key");
        assert_eq!(s.require_provider().unwrap_err().exit_code().code(), 3);
        assert!(!validate(&no_key, None).valid, "config validate reports it");
        assert!(validate_base(&no_key, None).valid);
        assert!(
            settings(&valid_base(), None)
                .unwrap()
                .require_provider()
                .is_ok()
        );
    }

    #[test]
    fn env_expands_before_provider_default_model() {
        let env = |n: &str| (n == "PROVIDER").then(|| "ollama".to_string());
        let (tree, _) = from_raw(json!({"ai": {"provider": "${PROVIDER}"}}), &env).unwrap();
        assert_eq!(tree["ai"]["provider"], "ollama");
        assert_eq!(tree["ai"]["model"], json!(default_model("ollama").unwrap()));
    }

    #[test]
    fn env_expansion() {
        let env = |n: &str| (n == "HOST").then(|| "api.example.com".to_string());
        assert_eq!(
            expand_env("https://${HOST}/v1", &env).0,
            "https://api.example.com/v1"
        );
        assert_eq!(
            expand_env("${MISSING}", &env),
            ("${MISSING}".to_string(), vec!["MISSING".to_string()])
        );
        assert_eq!(expand_env("plain", &env).0, "plain");
        assert_eq!(
            expand_env("${HOST}${HOST}", &env).0,
            "api.example.comapi.example.com"
        );
        assert_eq!(expand_env("a ${ b", &env).0, "a ${ b");
        assert_eq!(expand_env("${}", &env).0, "${}");
    }

    #[test]
    fn merge_rules() {
        let d = json!({"ai": {"provider": "openai", "model": "x"}, "profiles": {}});
        let m = deep_merge(&d, &json!({"ai": {"model": "y", "provider": null}}));
        assert_eq!(
            m["ai"],
            json!({"provider": "openai", "model": "y"}),
            "null = default"
        );
        assert_eq!(deep_merge(&d, &json!({"ai": null}))["ai"], d["ai"]);
        assert_eq!(
            deep_merge(&d, &json!({"ai": "s"}))["ai"],
            "s",
            "wrong shape is kept for validation"
        );
        let p = deep_merge(
            &d,
            &json!({"profiles": {"business": {"fields": {"description": null}}}}),
        );
        assert_eq!(
            p["profiles"]["business"]["fields"]["description"],
            Value::Null,
            "profiles verbatim"
        );
    }

    #[test]
    fn defaults_and_model_table() {
        let tree = merged(Value::Null);
        assert_eq!(tree["config_version"], 4);
        assert_eq!(tree["ai"]["model"], "gpt-6-luna");
        assert_eq!(
            tree["harmonization"],
            json!({"auto_learn": true, "decider": "ai", "decider_api_key": ""})
        );
        assert_eq!(tree["profiles"], json!({}));
        assert_eq!(
            merged(json!({"ai": {"provider": "anthropic"}}))["ai"]["model"],
            "claude-haiku-5-5"
        );
        assert_eq!(
            merged(json!({"ai": {"provider": "ollama", "model": ""}}))["ai"]["model"],
            ""
        );
        assert!(PROVIDERS.iter().all(|p| default_model(p).is_some()));
        assert!(from_raw(json!(["x"]), &no_env).is_err());
    }

    #[test]
    fn validation_table() {
        // (patch, expected error fields, expected warning fields)
        let table: Vec<(Value, Vec<&str>, Vec<&str>)> = vec![
            (json!({}), vec![], vec![]),
            (json!({"ai": {"provider": ""}}), vec!["ai.provider"], vec![]),
            (
                json!({"ai": {"provider": "not-a-vendor"}}),
                vec!["ai.provider"],
                vec![],
            ),
            (json!({"ai": {"api_key": ""}}), vec!["ai.api_key"], vec![]),
            (
                json!({"ai": {"provider": "ollama", "api_key": ""}}),
                vec![],
                vec![],
            ),
            (json!({"ai": {"model": ""}}), vec![], vec!["ai.model"]),
            (
                json!({"company": {"names": [""]}}),
                vec![],
                vec!["company.names"],
            ),
            (
                json!({"ai": {"api_key": "${MISSING_KEY}"}}),
                vec!["ai.api_key"],
                vec![],
            ),
            (
                json!({"ai": {"api_key": " Your-API-Key "}}),
                vec![],
                vec!["ai.api_key"],
            ),
            (
                json!({"ai": {"temperature": "hot", "max_retries": -1}}),
                vec!["ai.temperature", "ai.max_retries"],
                vec![],
            ),
            (json!({"pdf": {"ocr": "banana"}}), vec!["pdf.ocr"], vec![]),
            (
                json!({"pdf": {"ocr": "yes", "vision": 0}}),
                vec!["pdf.ocr", "pdf.vision"],
                vec![],
            ),
            (
                json!({"pdf": {"ocr": "auto", "vision": true}}),
                vec![],
                vec![],
            ),
            (
                json!({"pdf": {"max_pages": 0}}),
                vec!["pdf.max_pages"],
                vec![],
            ),
            (
                json!({"output": {"date_format": "%d/%m/%Y"}}),
                vec!["output.date_format"],
                vec![],
            ),
            (
                json!({"pdf": {"max_pages": true}}),
                vec!["pdf.max_pages"],
                vec![],
            ),
            (
                json!({"pdf": {"max_pages": 2.5}}),
                vec!["pdf.max_pages"],
                vec![],
            ),
            (
                json!({"pdf": {"max_pages": 4294967296u64}, "ocr": {"threads": 4294967296u64}, "ai": {"max_retries": 4294967296u64}}),
                vec!["ai.max_retries", "pdf.max_pages", "ocr.threads"],
                vec![],
            ),
            (
                json!({"pdf": {"extraction_mode": "auto"}}),
                vec!["pdf.extraction_mode"],
                vec![],
            ),
            (json!({"paddleocr": {}}), vec!["paddleocr"], vec![]),
            (json!({"filename": "x"}), vec!["filename"], vec![]),
            (json!({"config_version": 2}), vec!["config_version"], vec![]),
            (
                json!({"enrich": {"enabled": "no"}}),
                vec!["enrich.enabled"],
                vec![],
            ),
            (
                json!({"harmonization": {"decider": "magic"}}),
                vec!["harmonization.decider"],
                vec![],
            ),
            (
                json!({"harmonization": {"decider": "jev", "decider_api_key": "k"}}),
                vec![],
                vec![],
            ),
            (
                json!({"harmonization": {"decider": "jev"}}),
                vec!["harmonization.decider_api_key"],
                vec![],
            ),
            (json!({"privacy": "stealth"}), vec!["privacy"], vec![]),
            (
                json!({"prompt_extension": 3}),
                vec!["prompt_extension"],
                vec![],
            ),
            (json!({"profile": "missing"}), vec!["profile"], vec![]),
            (
                json!({"profiles": {"business": {"template": ""}}}),
                vec!["profile"],
                vec![],
            ),
        ];
        for (patch, errors, warnings) in table {
            let v = validate(&with(valid_base(), patch.clone()), None);
            assert_eq!(fields_at(&v, Level::Error), errors, "errors for {patch}");
            assert_eq!(
                fields_at(&v, Level::Warning),
                warnings,
                "warnings for {patch}"
            );
            assert_eq!(v.valid, errors.is_empty());
        }
    }

    #[test]
    fn validation_shapes_and_messages() {
        let v = validate(&with(valid_base(), json!({"ai": [], "company": 1})), None);
        let errs = fields_at(&v, Level::Error);
        assert!(errs.contains(&"ai".to_string()) && errs.contains(&"company".to_string()));
        assert!(v.profile.is_none() && v.resolved_profile.is_none());
        let v = validate(&valid_base(), None);
        assert_eq!(v.profile.as_deref(), Some("business"));
        let v = validate(&valid_base(), Some("academic"));
        assert_eq!(v.profile.as_deref(), Some("academic"));
        let v = validate(
            &with(valid_base(), json!({"ai": {"provider": "nope"}})),
            None,
        );
        assert_eq!(
            v.issues[0].message,
            "ai.provider must be one of: openai, anthropic, gemini, xai, ollama (got \"nope\")"
        );
        let v = validate(&with(valid_base(), json!({"config_version": 2})), None);
        assert!(
            v.issues[0]
                .hint
                .as_deref()
                .unwrap()
                .contains("migration-3-to-4")
        );
        let v = validate(&with(valid_base(), json!({"pdf": {"ocrr": true}})), None);
        assert_eq!(v.issues[0].message, "unknown key \"pdf.ocrr\"");
        let v = validate(
            &with(valid_base(), json!({"company": {"name": "Acme"}})),
            None,
        );
        assert!(v.issues[0].hint.as_deref().unwrap().contains("company.names"));
    }

    #[test]
    fn offline_privacy_rules() {
        let ok = json!({"privacy": "offline", "ai": {"provider": "ollama", "api_key": ""}});
        assert!(validate(&with(valid_base(), ok.clone()), None).valid);
        let lan = json!({"ai": {"base_url": "http://192.168.0.9:11434/v1"}});
        assert!(validate(&with(with(valid_base(), ok.clone()), lan), None).valid);
        let table = [
            (json!({"privacy": "offline"}), "ai.provider"),
            (
                json!({"ai": {"base_url": "https://ollama.example.com"}}),
                "ai.base_url",
            ),
            (
                json!({"harmonization": {"decider": "openai-decisions"}}),
                "harmonization.decider",
            ),
            (
                json!({"harmonization": {"decider": "jev"}}),
                "harmonization.decider",
            ),
        ];
        for (patch, field) in table {
            let base = if field == "ai.provider" {
                valid_base()
            } else {
                with(valid_base(), ok.clone())
            };
            let v = validate(&with(base, patch.clone()), None);
            assert!(
                fields_at(&v, Level::Error).contains(&field.to_string()),
                "{patch} -> {:?}",
                v.issues
            );
            assert!(v.issues.iter().any(|i| i.hint.is_some()));
        }
        // --offline on a cloud config: provider forced, default model follows.
        let mut tree = merged(json!({"ai": {"api_key": "key"}, "company": {"names": ["Acme"]}}));
        apply_overrides(
            &mut tree,
            &Overrides {
                offline: true,
                ..Default::default()
            },
        );
        assert_eq!(
            (
                tree["ai"]["provider"].as_str(),
                tree["ai"]["model"].as_str()
            ),
            (Some("ollama"), Some("gemma4"))
        );
        assert_eq!(settings(&tree, None).unwrap().privacy, Privacy::Offline);
        let mut remote = with(
            tree.clone(),
            json!({"ai": {"base_url": "https://x.example.com"}}),
        );
        apply_offline_override(&mut remote);
        assert!(!validate(&remote, None).valid);
    }

    #[test]
    fn provider_override_brings_its_own_key_endpoint_and_model() {
        let env = |n: &str| match n {
            "OPENAI_API_KEY" => Some("sk-openai".to_string()),
            "ANTHROPIC_API_KEY" => Some("sk-ant".to_string()),
            _ => None,
        };
        let raw = json!({"ai": {"provider": "openai", "model": "gpt-custom", "api_key": "${OPENAI_API_KEY}", "base_url": "https://gw.example.com/v1"}, "company": {"names": ["Acme"]}});
        let run = |o: Overrides| from_raw_with(raw.clone(), &o, &env).unwrap();
        let ai = |t: &Value| {
            let a = &t["ai"];
            (
                a["provider"].as_str().unwrap().to_string(),
                a["model"].as_str().unwrap().to_string(),
                a["api_key"].as_str().unwrap().to_string(),
                a["base_url"].as_str().unwrap().to_string(),
            )
        };
        let anthropic = |model: Option<&str>| Overrides {
            provider: Some("anthropic".into()),
            model: model.map(str::to_string),
            ..Default::default()
        };
        assert_eq!(
            ai(&run(anthropic(None)).0),
            (
                "anthropic".into(),
                "claude-haiku-5-5".into(),
                "sk-ant".into(),
                "".into()
            )
        );
        assert_eq!(ai(&run(anthropic(Some("claude-x"))).0).1, "claude-x");
        // The same provider changes nothing.
        let same = run(Overrides {
            provider: Some("openai".into()),
            ..Default::default()
        });
        assert_eq!(
            ai(&same.0),
            (
                "openai".into(),
                "gpt-custom".into(),
                "sk-openai".into(),
                "https://gw.example.com/v1".into()
            )
        );
        // Missing variable: the error names it; no notice about the old one.
        let (tree, notices) = run(Overrides {
            provider: Some("gemini".into()),
            ..Default::default()
        });
        assert!(
            notices.iter().all(|n| !n.message.contains("OPENAI")),
            "{notices:?}"
        );
        let err = settings(&tree, None)
            .unwrap()
            .require_provider()
            .unwrap_err();
        assert!(err.to_string().contains("unexpanded"), "{err}");
        assert!(
            err.hint().unwrap_or_default().contains("GEMINI_API_KEY"),
            "{err:?}"
        );
        // An env-selected provider equal to the override is no change:
        // literal key, custom model and endpoint stay.
        let env2 = |n: &str| (n == "PROVIDER").then(|| "anthropic".to_string());
        let raw2 = json!({"ai": {"provider": "${PROVIDER}", "model": "claude-x", "api_key": "sk-literal", "base_url": "https://gw.example.com"}});
        let (t, _) = from_raw_with(raw2, &anthropic(None), &env2).unwrap();
        assert_eq!(
            ai(&t),
            (
                "anthropic".into(),
                "claude-x".into(),
                "sk-literal".into(),
                "https://gw.example.com".into()
            )
        );
        let env3 = |n: &str| (n == "PROVIDER").then(|| "ollama".to_string());
        let raw3 = json!({"ai": {"provider": "${PROVIDER}", "model": "qwen9", "base_url": "http://192.168.1.5:11434"}});
        let ollama = Overrides {
            provider: Some("ollama".into()),
            ..Default::default()
        };
        let (t, _) = from_raw_with(raw3.clone(), &ollama, &env3).unwrap();
        assert_eq!(
            ai(&t),
            (
                "ollama".into(),
                "qwen9".into(),
                "".into(),
                "http://192.168.1.5:11434".into()
            )
        );
        let offline = Overrides {
            offline: true,
            ..Default::default()
        };
        let (t, _) = from_raw_with(raw3, &offline, &env3).unwrap();
        assert_eq!(ai(&t).3, "http://192.168.1.5:11434");
        // Ollama needs no key.
        let (tree, _) = run(Overrides {
            provider: Some("ollama".into()),
            ..Default::default()
        });
        assert_eq!(tree["ai"]["api_key"], "");
        assert!(settings(&tree, None).unwrap().require_provider().is_ok());
    }

    #[test]
    fn redaction() {
        assert_eq!(
            redact(&json!({"ai": {"api_key": "sk-1234567890abcdef"}}))["ai"]["api_key"],
            "sk-1...cdef"
        );
        assert_eq!(
            redact(&json!({"ai": {"api_key": "12345678"}}))["ai"]["api_key"],
            "***"
        );
        assert_eq!(redact(&json!({"ai": {"api_key": ""}}))["ai"]["api_key"], "");
        assert!(
            redact(&json!({"ai": {"provider": "ollama"}}))["ai"]
                .get("api_key")
                .is_none()
        );
        let original = json!({"ai": {"api_key": "sk-1234567890abcdef"}});
        let _ = redact(&original);
        assert_eq!(original["ai"]["api_key"], "sk-1234567890abcdef");
    }

    #[test]
    fn overrides_and_toggle_parse() {
        let mut tree = merged(json!({"ai": {"api_key": "key"}, "company": {"names": ["Acme"]}}));
        apply_overrides(
            &mut tree,
            &Overrides {
                provider: Some("anthropic".into()),
                ..Default::default()
            },
        );
        assert_eq!(tree["ai"]["model"], "claude-haiku-5-5");
        apply_overrides(
            &mut tree,
            &Overrides {
                provider: Some("gemini".into()),
                model: Some("custom".into()),
                ..Default::default()
            },
        );
        assert_eq!(tree["ai"]["model"], "custom", "explicit model wins");
        apply_overrides(
            &mut tree,
            &Overrides {
                provider: Some("xai".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            tree["ai"]["model"], "grok-4.7",
            "another provider's model never follows"
        );
        apply_overrides(
            &mut tree,
            &Overrides {
                ocr: Some(Toggle::On),
                vision: Some(Toggle::Off),
                ..Default::default()
            },
        );
        assert_eq!(
            (tree["pdf"]["ocr"].clone(), tree["pdf"]["vision"].clone()),
            (json!(true), json!(false))
        );
        apply_overrides(
            &mut tree,
            &Overrides {
                vision: Some(Toggle::Auto),
                no_enrich: true,
                no_learn: true,
                ..Default::default()
            },
        );
        assert_eq!(tree["pdf"]["vision"], "auto");
        assert_eq!(
            (
                tree["enrich"]["enabled"].as_bool(),
                tree["harmonization"]["auto_learn"].as_bool()
            ),
            (Some(false), Some(false))
        );
        apply_overrides(
            &mut tree,
            &Overrides {
                provider: Some("bogus".into()),
                ..Default::default()
            },
        );
        assert!(settings(&tree, None).is_err());
        for (s, t) in [
            ("auto", Toggle::Auto),
            ("ON", Toggle::On),
            ("true", Toggle::On),
            ("off", Toggle::Off),
            ("False", Toggle::Off),
        ] {
            assert_eq!(Toggle::parse(s), Some(t));
        }
        assert_eq!(Toggle::parse("yes"), None);
    }

    #[test]
    fn typed_settings() {
        let s = settings(&valid_base(), None).unwrap();
        assert_eq!(
            (s.ai.provider, s.ai.max_retries, s.pdf.ocr, s.pdf.vision),
            (Provider::OpenAi, 2, Toggle::Auto, Toggle::Auto),
            "free-first route: OCR then vision, both auto by default"
        );
        assert_eq!(
            (s.pdf.incoming_invoice.as_str(), s.ocr.threads, s.decider),
            ("AP", 4, Decider::Ai)
        );
        assert!(s.enrich_enabled && s.auto_learn);
        assert_eq!(s.profile_id, "business");
        assert_eq!(s.date_format, "%Y%m%d");
        let err = settings(
            &with(valid_base(), json!({"ai": {"temperature": "hot"}})),
            None,
        )
        .unwrap_err();
        assert_eq!(err.code(), "config_error");
    }

    #[test]
    fn discovery_order() {
        let user = Path::new("/home/u/.config/autorename-pdf");
        let d = discover_config_with(
            Some(Path::new("/x/c.yaml")),
            Some("/env.yaml".into()),
            Some(user),
        );
        assert_eq!(
            (d.path, d.source),
            (PathBuf::from("/x/c.yaml"), ConfigSource::Cli)
        );
        let d = discover_config_with(None, Some("/env.yaml".into()), Some(user));
        assert_eq!(
            (d.path, d.source),
            (PathBuf::from("/env.yaml"), ConfigSource::Env)
        );
        let d = discover_config_with(None, Some("".into()), Some(user));
        assert_eq!(
            (d.path, d.source),
            (user.join("config.yaml"), ConfigSource::User)
        );
        assert_eq!(
            discover_config_with(None, None, None).path,
            PathBuf::from("config.yaml")
        );
        assert_eq!(
            name_list_path(Path::new("/x/c.yaml"), "company_name"),
            // absolute: Windows adds the current drive (`D:\x\…`)
            std::path::absolute("/x/name-lists/company_name.yaml").unwrap()
        );
        let file = |f: &str| {
            name_list_in(Path::new("/d"), f)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(file("company_name"), "company_name.yaml");
        assert_eq!(file("com0"), "com0.yaml");
        // Only list-safe ids reach the path: injective, no device names.
        for bad in [
            "CON",
            "con",
            "com1",
            "Lpt9",
            "COM¹",
            "../x",
            "",
            "größe",
            "Company_Name",
        ] {
            assert!(!crate::harmonize::is_field_id(bad), "{bad}");
        }
    }
}
