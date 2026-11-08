//! AI provider adapters: one structured-output extraction call per document.
//!
//! # Blocking by design
//!
//! The public API is synchronous (`reqwest::blocking`, rustls). Callers are a
//! CLI that processes files on a small `std::thread` pool (`--jobs`) and a GUI
//! that already runs work on worker threads, so an async API would force both
//! to own a runtime for no gain: every call is one request/response with no
//! streaming. `reqwest::blocking` keeps its own internal runtime, which means
//! these functions must NOT be called from inside an async (tokio) context.
//!
//! # Shape
//!
//! Plain functions, no trait: [`extract`] translates the canonical schema for
//! the provider, builds the provider body (`openai`, `anthropic`, `gemini`,
//! `ollama` modules; xAI reuses the OpenAI Responses adapter), sends it with
//! bounded retries by error class, parses the provider envelope (refusal /
//! truncation detection), then validates the JSON locally against the
//! canonical schema. A schema-invalid answer is retried once with the
//! validation error fed back to the model.

mod anthropic;
pub mod decide;
mod gemini;
mod ollama;
mod openai;
pub mod schema;

use std::time::Duration;

use base64::Engine as _;
use serde::Serialize;
use serde_json::Value;

use crate::config::{Privacy, Provider, Settings, is_local_ip, is_local_url};
use crate::error::Error;
use crate::profiles::{Profile, metadata_schema, system_prompt};

pub use decide::{DeciderSettings, decide_name};

/// Default `max_tokens` / `max_output_tokens` for extraction calls.
pub const DEFAULT_MAX_TOKENS: u32 = 2048;
/// Schema name sent to OpenAI/xAI (`text.format.name`), pydantic parity.
pub const SCHEMA_NAME: &str = "DocumentMetadata";

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

/// Provider-neutral extraction request.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractRequest {
    pub system_prompt: String,
    /// First user content block; images follow in page order (contract §5.1).
    pub user_text: String,
    /// Canonical JSON Schema (see [`metadata_schema`]); translated per provider
    /// and used verbatim for local validation.
    pub schema: Value,
    pub schema_name: String,
    /// Page images as PNG bytes.
    pub images: Vec<Vec<u8>>,
    pub max_tokens: u32,
}

impl ExtractRequest {
    /// Document extraction request (contract §5.1 user texts). `None` when
    /// there is neither text nor images.
    pub fn document(
        profile: &Profile,
        prompt_extension: &str,
        text: &str,
        images: Vec<Vec<u8>>,
    ) -> Option<Self> {
        let has_text = !text.trim().is_empty();
        let user_text = match (has_text, images.is_empty()) {
            (true, true) => format!("Extract the information from this text:\n\n{text}"),
            (true, false) => {
                format!("Extract document metadata from this text and images:\n\n{text}")
            }
            (false, false) => "Extract document metadata from these page images:".to_string(),
            (false, true) => return None,
        };
        Some(ExtractRequest {
            system_prompt: system_prompt(profile, prompt_extension),
            user_text,
            schema: metadata_schema(profile),
            schema_name: SCHEMA_NAME.into(),
            images,
            max_tokens: DEFAULT_MAX_TOKENS,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtractResponse {
    /// Parsed model output, validated against the canonical schema.
    pub value: Value,
    /// Model id as reported by the provider (falls back to the requested one).
    pub model: String,
    pub request_id: Option<String>,
    pub usage: Usage,
    /// HTTP requests made (transport retries + schema re-ask).
    pub attempts: u32,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// Extra attempts after the first for retryable transport/HTTP errors.
    pub max_retries: u32,
    /// Exponential backoff base (attempt n waits `base * 2^n` + jitter).
    pub base_delay: Duration,
    /// Cap for any single wait, including a server `Retry-After`.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSettings {
    pub provider: Provider,
    pub model: String,
    pub api_key: String,
    /// Empty = provider default (see [`ProviderSettings::base`]). Honored for
    /// every provider. Native API roots only: OpenAI/xAI/Anthropic `…/v1`,
    /// Gemini `…/v1beta`, Ollama the server root (no `/v1`).
    pub base_url: String,
    pub temperature: f64,
    pub privacy: Privacy,
    pub timeout: Duration,
    pub retry: RetryPolicy,
}

impl ProviderSettings {
    pub fn from_settings(s: &Settings) -> Self {
        ProviderSettings {
            provider: s.ai.provider,
            model: s.ai.model.clone(),
            api_key: s.ai.api_key.clone(),
            base_url: s.ai.base_url.clone(),
            temperature: s.ai.temperature,
            privacy: s.privacy,
            timeout: Duration::from_secs(120),
            retry: RetryPolicy {
                max_retries: s.ai.max_retries,
                ..RetryPolicy::default()
            },
        }
    }

    /// Effective base URL (no trailing slash).
    pub fn base(&self) -> String {
        let custom = self.base_url.trim().trim_end_matches('/');
        if custom.is_empty() {
            default_base(self.provider).to_string()
        } else {
            custom.to_string()
        }
    }
}

fn default_base(p: Provider) -> &'static str {
    match p {
        Provider::OpenAi => "https://api.openai.com/v1",
        Provider::Anthropic => "https://api.anthropic.com/v1",
        Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta",
        Provider::Xai => "https://api.x.ai/v1",
        Provider::Ollama => "http://localhost:11434",
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// 401/403, bad or missing key → exit 11.
    Auth,
    /// 429 after retries were exhausted.
    RateLimited,
    /// Spend cap / quota / billing: never retried.
    Quota,
    /// 5xx, 529, timeouts, connection failures.
    Unavailable,
    /// Other 4xx (bad request, model not found, too large).
    BadRequest,
    /// Model refused (OpenAI refusal part, Anthropic `stop_reason: refusal`,
    /// Gemini block/safety finish).
    Refusal,
    /// Output cut off (`max_output_tokens`, `max_tokens`, `MAX_TOKENS`, `length`).
    Truncated,
    /// Output not JSON or not schema-valid (after the one re-ask).
    InvalidOutput,
    /// `privacy: offline` and a non-local URL → exit 3.
    OfflineBlocked,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}", self.render())]
pub struct ProviderError {
    pub kind: ErrorKind,
    /// `openai`, `anthropic`, …, `openai-decisions`, `jev`.
    pub provider: &'static str,
    pub message: String,
    pub status: Option<u16>,
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
    /// Next step for the user (e.g. connectivity problems).
    pub hint: Option<String>,
}

impl ProviderError {
    fn new(kind: ErrorKind, provider: &'static str, message: impl Into<String>) -> Self {
        ProviderError {
            kind,
            provider,
            message: message.into(),
            status: None,
            request_id: None,
            retry_after: None,
            hint: None,
        }
    }

    fn with_request_id(mut self, id: Option<String>) -> Self {
        if self.request_id.is_none() {
            self.request_id = id;
        }
        self
    }

    /// Worth another attempt with the same request.
    pub fn retryable(&self) -> bool {
        matches!(self.kind, ErrorKind::RateLimited | ErrorKind::Unavailable)
    }

    fn render(&self) -> String {
        let mut s = format!("{}: {}", self.provider, self.message);
        if let Some(st) = self.status {
            s.push_str(&format!(" (HTTP {st})"));
        }
        if let Some(id) = &self.request_id {
            s.push_str(&format!(" [request id {id}]"));
        }
        s
    }
}

impl From<ProviderError> for Error {
    fn from(e: ProviderError) -> Self {
        let message = e.to_string();
        match e.kind {
            ErrorKind::Auth => Error::Auth { message },
            ErrorKind::OfflineBlocked => Error::Config {
                message,
                hint: Some(
                    "privacy: offline allows only local/LAN endpoints (e.g. Ollama on localhost)."
                        .into(),
                ),
            },
            _ => Error::Provider {
                message,
                retryable: e.retryable(),
                hint: e.hint,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Run one extraction. See [`extract_detailed`] for the typed error.
pub fn extract(req: &ExtractRequest, cfg: &ProviderSettings) -> crate::Result<ExtractResponse> {
    extract_detailed(req, cfg).map_err(Error::from)
}

/// Run one extraction, keeping the provider error class, status and request id.
pub fn extract_detailed(
    req: &ExtractRequest,
    cfg: &ProviderSettings,
) -> Result<ExtractResponse, ProviderError> {
    let label = cfg.provider.id();
    let base = cfg.base();
    guard_offline(cfg.privacy, &base, label)?;
    if cfg.provider != Provider::Ollama && cfg.api_key.trim().is_empty() {
        return Err(ProviderError::new(
            ErrorKind::Auth,
            label,
            format!("API key required for provider '{label}'. Set ai.api_key in config.yaml."),
        ));
    }
    let wire_schema = schema::for_provider(cfg.provider, &req.schema);
    let client = http_client(cfg.timeout, cfg.privacy, label)?;
    let mut attempts = 0;
    let mut feedback: Option<String> = None;
    loop {
        let call = build_call(req, cfg, &base, &wire_schema, feedback.as_deref());
        let (raw, n) = send(&client, &call, cfg.provider_kind(), &cfg.retry)?;
        attempts += n;
        let parsed = parse_envelope(cfg.provider, &raw)
            .map_err(|e| e.with_request_id(raw.request_id.clone()))?;
        let checked = serde_json::from_str::<Value>(strip_fences(&parsed.text))
            .map_err(|e| format!("output is not valid JSON: {e}"))
            .and_then(|v| schema::validate(&req.schema, &v).map(|()| v));
        match checked {
            Ok(value) => {
                return Ok(ExtractResponse {
                    value,
                    model: parsed.model.unwrap_or_else(|| cfg.model.clone()),
                    request_id: raw.request_id,
                    usage: parsed.usage,
                    attempts,
                });
            }
            Err(msg) if feedback.is_none() => feedback = Some(msg),
            Err(msg) => {
                return Err(ProviderError::new(ErrorKind::InvalidOutput, label, msg)
                    .with_request_id(raw.request_id));
            }
        }
    }
}

/// Models installed in the local/LAN Ollama at `base` (`GET /api/tags`).
/// Only local URLs are contacted (offline client: no proxy, no redirects).
pub fn ollama_models(base: &str) -> Result<Vec<String>, ProviderError> {
    let base = base.trim().trim_end_matches('/');
    let base = if base.is_empty() {
        default_base(Provider::Ollama)
    } else {
        base
    };
    guard_offline(Privacy::Offline, base, "ollama")?;
    let client = http_client(Duration::from_secs(3), Privacy::Offline, "ollama")?;
    let resp = client.get(format!("{base}/api/tags")).send().map_err(|e| {
        let mut err = ProviderError::new(
            ErrorKind::Unavailable,
            "ollama",
            format!("Ollama not reachable at {base}: {}", transport_chain(&e)),
        );
        err.hint = Some("Start it with `ollama serve` (or set --ollama-url).".into());
        err
    })?;
    let body: Value = resp.json().map_err(|e| {
        ProviderError::new(
            ErrorKind::Unavailable,
            "ollama",
            format!("bad /api/tags reply: {e}"),
        )
    })?;
    Ok(body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["name"].as_str().map(str::to_string))
        .collect())
}

/// `model` is installed (`gemma4` also matches `gemma4:latest`).
pub fn ollama_has_model(installed: &[String], model: &str) -> bool {
    installed
        .iter()
        .any(|m| m == model || m.strip_suffix(":latest") == Some(model))
}

/// Refuse a non-local endpoint under `privacy: offline`.
pub fn guard_offline(
    privacy: Privacy,
    url: &str,
    label: &'static str,
) -> Result<(), ProviderError> {
    if privacy == Privacy::Offline && !is_local_url(url) {
        return Err(ProviderError::new(
            ErrorKind::OfflineBlocked,
            label,
            format!("refusing non-local endpoint {url} in offline mode"),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internals shared by adapters
// ---------------------------------------------------------------------------

/// A fully built HTTP POST.
#[derive(Debug, Clone)]
pub(crate) struct HttpCall {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

/// A 2xx response body plus its request id.
#[derive(Debug, Clone)]
pub(crate) struct RawResponse {
    pub body: Value,
    pub request_id: Option<String>,
}

/// Provider envelope, before local JSON/schema checks.
#[derive(Debug, Clone, Default)]
pub(crate) struct Envelope {
    pub text: String,
    pub model: Option<String>,
    pub usage: Usage,
}

/// Which error-classification rules apply to a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Provider(Provider),
    Decisions,
    Jev,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Provider(p) => p.id(),
            Kind::Decisions => "openai-decisions",
            Kind::Jev => "jev",
        }
    }
}

impl ProviderSettings {
    fn provider_kind(&self) -> Kind {
        Kind::Provider(self.provider)
    }
}

fn build_call(
    req: &ExtractRequest,
    cfg: &ProviderSettings,
    base: &str,
    wire_schema: &Value,
    feedback: Option<&str>,
) -> HttpCall {
    let user_text = match feedback {
        None => req.user_text.clone(),
        Some(err) => format!(
            "{}\n\nYour previous reply was rejected: {err}. Reply again with only a JSON object that matches the schema.",
            req.user_text
        ),
    };
    let input = openai::Input {
        system: &req.system_prompt,
        user_text: &user_text,
        images: &req.images,
        schema: wire_schema,
        schema_name: &req.schema_name,
        max_tokens: req.max_tokens,
    };
    match cfg.provider {
        Provider::OpenAi | Provider::Xai => openai::build(&input, cfg, base),
        Provider::Anthropic => anthropic::build(&input, cfg, base),
        Provider::Gemini => gemini::build(&input, cfg, base),
        Provider::Ollama => ollama::build(&input, cfg, base),
    }
}

fn parse_envelope(p: Provider, raw: &RawResponse) -> Result<Envelope, ProviderError> {
    match p {
        Provider::OpenAi | Provider::Xai => openai::parse(p.id(), &raw.body),
        Provider::Anthropic => anthropic::parse(&raw.body),
        Provider::Gemini => gemini::parse(&raw.body),
        Provider::Ollama => ollama::parse(&raw.body),
    }
}

/// Some local models wrap JSON in markdown fences despite `format`.
fn strip_fences(s: &str) -> &str {
    let t = s.trim();
    let Some(inner) = t.strip_prefix("```") else {
        return t;
    };
    let inner = inner.strip_prefix("json").unwrap_or(inner);
    inner.strip_suffix("```").unwrap_or(inner).trim()
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn png_data_url(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", b64(bytes))
}

/// DNS resolver for offline mode: system resolution, then every address must
/// be loopback/private/link-local, otherwise the connection is refused.
struct LocalOnlyResolver;

impl reqwest::dns::Resolve for LocalOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            use std::net::ToSocketAddrs;
            // Port is replaced by the connector; 0 is a placeholder.
            let addrs: Vec<std::net::SocketAddr> = (host.as_str(), 0).to_socket_addrs()?.collect();
            if addrs.is_empty() || addrs.iter().any(|a| !is_local_ip(a.ip())) {
                let err: Box<dyn std::error::Error + Send + Sync> =
                    format!("offline mode: {host} resolves to a non-local address").into();
                return Err(err);
            }
            let it: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok(it)
        })
    }
}

/// HTTP client. Offline: no proxies (env/system), no redirects, and DNS
/// answers restricted to local addresses ([`guard_offline`] covers IP-literal
/// hosts before any request).
pub(crate) fn http_client(
    timeout: Duration,
    privacy: Privacy,
    label: &'static str,
) -> Result<reqwest::blocking::Client, ProviderError> {
    let mut b = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("autorename-pdf/", env!("CARGO_PKG_VERSION")));
    if privacy == Privacy::Offline {
        b = b
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(std::sync::Arc::new(LocalOnlyResolver));
    }
    b.build()
        .map_err(|e| ProviderError::new(ErrorKind::Unavailable, label, format!("HTTP client: {e}")))
}

/// POST with bounded retries. Returns the 2xx body and the number of requests made.
pub(crate) fn send(
    client: &reqwest::blocking::Client,
    call: &HttpCall,
    kind: Kind,
    policy: &RetryPolicy,
) -> Result<(RawResponse, u32), ProviderError> {
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let err = match send_once(client, call, kind) {
            Ok(raw) => return Ok((raw, attempt)),
            Err(e) => e,
        };
        if !err.retryable() || attempt > policy.max_retries {
            return Err(err);
        }
        let wait = err
            .retry_after
            .unwrap_or_else(|| backoff(policy.base_delay, attempt - 1))
            .min(policy.max_delay);
        std::thread::sleep(wait);
    }
}

fn backoff(base: Duration, n: u32) -> Duration {
    let exp = base.saturating_mul(1u32 << n.min(10));
    // Cheap jitter (0–25 %) without a rand dependency.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    exp + exp.mul_f64(f64::from(nanos % 250) / 1000.0)
}

fn send_once(
    client: &reqwest::blocking::Client,
    call: &HttpCall,
    kind: Kind,
) -> Result<RawResponse, ProviderError> {
    let label = kind.label();
    let mut rb = client.post(&call.url).json(&call.body);
    for (k, v) in &call.headers {
        rb = rb.header(*k, v);
    }
    let resp = rb.send().map_err(|e| {
        let what = if e.is_timeout() {
            "request timed out"
        } else if e.is_connect() {
            "connection failed"
        } else {
            "request failed"
        };
        let msg = format!("{what}: {}", transport_chain(&e));
        let hint = if kind == Kind::Provider(Provider::Ollama) {
            "Is Ollama running and reachable at ai.base_url? Start it with `ollama serve`."
        } else {
            "Check the network connection, proxy (HTTPS_PROXY) and firewall. Working offline? Use --offline with a local Ollama model."
        };
        let mut err = ProviderError::new(ErrorKind::Unavailable, label, msg);
        err.hint = Some(hint.into());
        err
    })?;
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let request_id = ["x-request-id", "request-id"]
        .iter()
        .find_map(|h| headers.get(*h).and_then(|v| v.to_str().ok()))
        .map(str::to_string);
    let retry_after = parse_retry_after(&headers);
    let text = resp.text().map_err(|e| {
        ProviderError::new(ErrorKind::Unavailable, label, format!("reading body: {e}"))
            .with_request_id(request_id.clone())
    })?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    if (200..300).contains(&status) {
        if !body.is_object() {
            return Err(ProviderError::new(
                ErrorKind::Unavailable,
                label,
                "non-JSON response body",
            )
            .with_request_id(request_id));
        }
        return Ok(RawResponse { body, request_id });
    }
    let request_id = request_id.or_else(|| {
        body.get("request_id")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Err(classify(kind, status, retry_after, &body).with_request_id(request_id))
}

/// Upper bound for a server-requested wait (the retry policy caps further).
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// `retry-after-ms` / `retry-after` (seconds). Untrusted: non-finite,
/// negative or unparsable values are ignored (bounded backoff applies);
/// large ones are clamped. HTTP-date values are ignored.
pub(crate) fn parse_retry_after(h: &reqwest::header::HeaderMap) -> Option<Duration> {
    let get = |k: &str| {
        h.get(k)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    let secs = get("retry-after-ms")
        .map(|ms| ms / 1000.0)
        .or_else(|| get("retry-after"))?;
    Some(Duration::try_from_secs_f64(secs).map_or(MAX_RETRY_AFTER, |d| d.min(MAX_RETRY_AFTER)))
}

/// Transport error chain without URLs (query strings may carry keys), e.g.
/// `error sending request: client error (Connect): dns error: …`.
fn transport_chain(e: &reqwest::Error) -> String {
    let mut parts = Vec::new();
    let mut cur: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(err) = cur {
        let text = err.to_string();
        if !text.is_empty() && !parts.contains(&text) {
            parts.push(text);
        }
        cur = err.source();
    }
    let head = if e.is_connect() {
        "could not connect"
    } else if e.is_timeout() {
        "timed out"
    } else {
        "error sending request"
    };
    let mut out = head.to_string();
    for p in parts {
        // Drop anything that embeds a URL.
        if !p.contains("://") {
            out.push_str(": ");
            out.push_str(&p);
        }
    }
    out
}

/// Best-effort error message from any provider error body.
fn error_message(body: &Value) -> String {
    match body {
        Value::String(s) if !s.trim().is_empty() => s.chars().take(500).collect(),
        Value::Object(_) => body
            .pointer("/error/message")
            .or_else(|| body.get("error").filter(|e| e.is_string()))
            .or_else(|| body.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| body.to_string().chars().take(500).collect()),
        _ => "empty error body".into(),
    }
}

fn error_code(body: &Value) -> String {
    [
        "/error/details/error_code",
        "/error/code",
        "/error/type",
        "/error/status",
    ]
    .iter()
    .filter_map(|p| body.pointer(p))
    .filter_map(|v| v.as_str())
    .collect::<Vec<_>>()
    .join(" ")
}

const QUOTA_CODES: [&str; 5] = [
    "insufficient_quota",
    "credit_balance_exhausted",
    "spend_limit_exceeded",
    "enforced_spend_limit_reached",
    "billing_error",
];

/// Map a non-2xx response to an error class (provider reference §2).
pub(crate) fn classify(
    kind: Kind,
    status: u16,
    retry_after: Option<Duration>,
    body: &Value,
) -> ProviderError {
    let label = kind.label();
    let message = error_message(body);
    let code = error_code(body);
    let is_quota = QUOTA_CODES.iter().any(|q| code.contains(q));
    let error_kind = match status {
        401 | 403 => ErrorKind::Auth,
        // Gemini reports a bad key as 400 INVALID_ARGUMENT / API_KEY_INVALID.
        400 if kind == Kind::Provider(Provider::Gemini)
            && (message.contains("API key") || body.to_string().contains("API_KEY_INVALID")) =>
        {
            ErrorKind::Auth
        }
        402 => ErrorKind::Quota,
        // Only explicit quota/spend codes (e.g. Anthropic
        // `error.details.error_code`); every other 429 is a bounded retry.
        429 if is_quota => ErrorKind::Quota,
        429 => ErrorKind::RateLimited,
        408 | 500..=599 => ErrorKind::Unavailable,
        _ => ErrorKind::BadRequest,
    };
    let mut message = message;
    if kind == Kind::Provider(Provider::Ollama) && status == 404 {
        message.push_str(" (pull the model first: `ollama pull <model>`)");
    }
    ProviderError {
        kind: error_kind,
        provider: label,
        message,
        status: Some(status),
        request_id: None,
        retry_after,
        hint: None,
    }
}

/// `{"type":"string"}` → string description of the schema for prompt hints.
pub(crate) fn schema_hint(schema: &Value) -> String {
    format!(
        "Respond with only a JSON object matching this JSON Schema:\n{}",
        serde_json::to_string(schema).unwrap_or_default()
    )
}

/// Token count helper: JSON number → u64.
pub(crate) fn tokens(v: Option<&Value>) -> Option<u64> {
    v.and_then(Value::as_u64)
}

/// Shorthand used by adapters.
pub(crate) fn perr(kind: ErrorKind, label: &'static str, msg: impl Into<String>) -> ProviderError {
    ProviderError::new(kind, label, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base_url_default_and_custom() {
        let mut s = ProviderSettings {
            provider: Provider::Anthropic,
            model: "m".into(),
            api_key: "k".into(),
            base_url: String::new(),
            temperature: 0.0,
            privacy: Privacy::Cloud,
            timeout: Duration::from_secs(1),
            retry: RetryPolicy::default(),
        };
        assert_eq!(s.base(), "https://api.anthropic.com/v1");
        s.base_url = "http://proxy.local/v1/".into();
        assert_eq!(s.base(), "http://proxy.local/v1");
    }

    #[test]
    fn fences_are_stripped() {
        assert_eq!(strip_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_fences(" {\"a\":1} "), "{\"a\":1}");
    }

    #[test]
    fn retry_after_is_checked_and_bounded() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let h = |pairs: &[(&'static str, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.insert(*k, HeaderValue::from_str(v).unwrap());
            }
            parse_retry_after(&m)
        };
        for bad in [
            "inf",
            "-inf",
            "NaN",
            "-1",
            "soon",
            "Wed, 21 Oct 2026 07:28:00 GMT",
        ] {
            assert_eq!(h(&[("retry-after", bad)]), None, "{bad}");
            assert_eq!(h(&[("retry-after-ms", bad)]), None, "{bad}");
        }
        assert_eq!(h(&[("retry-after", "1e300")]), Some(MAX_RETRY_AFTER));
        assert_eq!(h(&[("retry-after", "2")]), Some(Duration::from_secs(2)));
        assert_eq!(
            h(&[("retry-after-ms", "1500"), ("retry-after", "9")]),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(
            h(&[("retry-after-ms", "inf"), ("retry-after", "3")]),
            Some(Duration::from_secs(3))
        );
    }

    #[test]
    fn classify_rules() {
        let k = Kind::Provider(Provider::OpenAi);
        let q = json!({"error":{"code":"insufficient_quota","message":"no"}});
        assert_eq!(classify(k, 429, None, &q).kind, ErrorKind::Quota);
        let r = json!({"error":{"code":"rate_limit_exceeded","message":"slow"}});
        assert_eq!(classify(k, 429, None, &r).kind, ErrorKind::RateLimited);
        assert_eq!(classify(k, 401, None, &r).kind, ErrorKind::Auth);
        assert_eq!(classify(k, 503, None, &r).kind, ErrorKind::Unavailable);
        assert_eq!(classify(k, 400, None, &r).kind, ErrorKind::BadRequest);
        let a = Kind::Provider(Provider::Anthropic);
        // A plain rate limit without retry-after is retried, not quota.
        let ar = json!({"type":"error","error":{"type":"rate_limit_error","message":"slow"}});
        assert_eq!(classify(a, 429, None, &ar).kind, ErrorKind::RateLimited);
        let spend = json!({"type":"error","error":{"type":"rate_limit_error","message":"cap","details":{"error_code":"enforced_spend_limit_reached"}}});
        assert_eq!(classify(a, 429, None, &spend).kind, ErrorKind::Quota);
        assert_eq!(
            classify(a, 429, Some(Duration::from_secs(1)), &r).kind,
            ErrorKind::RateLimited
        );
        assert_eq!(classify(a, 529, None, &r).kind, ErrorKind::Unavailable);
        let g = Kind::Provider(Provider::Gemini);
        let bad_key = json!({"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}});
        assert_eq!(classify(g, 400, None, &bad_key).kind, ErrorKind::Auth);
    }

    #[test]
    fn error_mapping_to_core() {
        let mut e = ProviderError::new(ErrorKind::Auth, "openai", "bad key");
        e.request_id = Some("req_1".into());
        let core: Error = e.clone().into();
        assert_eq!(core.exit_code().code(), 11);
        assert!(core.to_string().contains("req_1"));
        e.kind = ErrorKind::RateLimited;
        let core: Error = e.clone().into();
        assert_eq!(core.exit_code().code(), 10);
        assert!(core.retryable());
        e.kind = ErrorKind::OfflineBlocked;
        assert_eq!(Error::from(e).exit_code().code(), 3);
    }

    #[test]
    fn offline_guard() {
        assert!(guard_offline(Privacy::Offline, "http://localhost:11434", "ollama").is_ok());
        assert!(guard_offline(Privacy::Offline, "https://api.openai.com/v1", "openai").is_err());
        assert!(guard_offline(Privacy::Cloud, "https://api.openai.com/v1", "openai").is_ok());
    }

    #[test]
    fn user_texts_follow_contract() {
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "a".to_string(),
            crate::profiles::FieldSpec {
                description: "d".into(),
            },
        );
        let profile = Profile {
            intro: String::new(),
            fields,
            template: "{a}".into(),
            truncate_field: "a".into(),
            harmonize_field: None,
            direction: None,
            amount: None,
        };
        let r = ExtractRequest::document(&profile, "", "hello", vec![]).unwrap();
        assert_eq!(
            r.user_text,
            "Extract the information from this text:\n\nhello"
        );
        let r = ExtractRequest::document(&profile, "", "hello", vec![vec![1]]).unwrap();
        assert_eq!(
            r.user_text,
            "Extract document metadata from this text and images:\n\nhello"
        );
        let r = ExtractRequest::document(&profile, "", "  ", vec![vec![1]]).unwrap();
        assert_eq!(
            r.user_text,
            "Extract document metadata from these page images:"
        );
        assert!(ExtractRequest::document(&profile, "", "", vec![]).is_none());
    }
}
