//! Harmonization decider for the ambiguous band
//! ([`crate::harmonize::Decision::AskDecider`]): pick one candidate or "new".
//!
//! Backends (`harmonization.decider`):
//! - `ai` (default): a tiny structured call on the main provider whose schema
//!   is `{"choice": enum["0".."k-1","new"]}`.
//! - `openai-decisions`: `POST /v1/decisions` with one `choice` question
//!   (`questions[].choices[]`, 2–255 options), answer in `answers[0].choice`
//!   (developers.openai.com/api/docs/guides/decisions, read 2026-11-10).
//! - `jev`: BEST-EFFORT shape from abridged notes, not live-checked; its
//!   response parsing is deliberately tolerant.
//! - `off`: no call, `Ok(None)`.

use std::time::Duration;

use serde_json::{Value, json};

use super::{
    ErrorKind, ExtractRequest, HttpCall, Kind, ProviderError, ProviderSettings, extract_detailed,
    guard_offline, http_client, perr, send,
};
use crate::config::{Decider, Privacy, Provider, Settings};
use crate::harmonize::{self, Candidate, NameKind, Verdict};

pub const DECISIONS_URL: &str = "https://api.openai.com/v1/decisions";
pub const DECISIONS_MODEL: &str = "gpt-6-luna";
pub const JEV_URL: &str = "https://api.typesafe.ai/v1/systemone";

#[derive(Debug, Clone, PartialEq)]
pub struct DeciderSettings {
    pub decider: Decider,
    /// Main provider (used by `ai`; privacy/timeout/retry reused by the others).
    pub provider: ProviderSettings,
    /// Key for `openai-decisions` / `jev` (unused by `ai`).
    pub api_key: String,
    /// Endpoint override for `openai-decisions` / `jev` (empty = default).
    pub url: String,
}

impl DeciderSettings {
    /// Key: `harmonization.decider_api_key`; empty → `openai-decisions` uses
    /// `ai.api_key` when the main provider is OpenAI, else `OPENAI_API_KEY`;
    /// `jev` uses `JEV_API_KEY`.
    pub fn from_settings(s: &Settings, env: &dyn Fn(&str) -> Option<String>) -> Self {
        let provider = ProviderSettings::from_settings(s);
        let api_key = match s.decider {
            _ if !s.decider_api_key.is_empty() => s.decider_api_key.clone(),
            Decider::OpenaiDecisions if s.ai.provider == Provider::OpenAi => s.ai.api_key.clone(),
            Decider::OpenaiDecisions => env("OPENAI_API_KEY").unwrap_or_default(),
            Decider::Jev => env("JEV_API_KEY").unwrap_or_default(),
            _ => String::new(),
        };
        DeciderSettings {
            decider: s.decider,
            provider,
            api_key,
            url: String::new(),
        }
    }
}

/// Decider wording for one name list. Company lists ask about the same
/// legal company; any other field asks about the same thing, named by the
/// field's label ([`harmonize::field_label`]), without assuming invoices.
struct Wording {
    question: String,
    system: String,
    known: String,
    new_option: String,
    schema_name: &'static str,
}

fn wording(field: &str, name: &str) -> Wording {
    match NameKind::for_field(field) {
        NameKind::Company => Wording {
            question: format!(
                "An invoice names the company \"{name}\". Is it the same legal company as one of the \
                 known companies listed, or a different (new) company? Spelling, OCR noise, legal \
                 suffixes and abbreviations do not make a different company; a different business does."
            ),
            system: "You match company names. Answer with the index of the matching known company, or \"new\"."
                .into(),
            known: "Known companies".into(),
            new_option: "None of these (new company)".into(),
            schema_name: "CompanyChoice",
        },
        NameKind::Generic => {
            let (singular, plural) = harmonize::field_label(field);
            let (one, many) = (singular.to_lowercase(), plural.to_lowercase());
            Wording {
                question: format!(
                    "A document names the {one} \"{name}\". Is it the same {one} as one of the \
                     known {many} listed, or a different (new) one? Spelling, OCR noise, \
                     capitalization and abbreviations do not make a different {one}."
                ),
                system: format!(
                    "You match {one} names. Answer with the index of the matching known {one}, or \"new\"."
                ),
                known: format!("Known {many}"),
                new_option: format!("None of these (new {one})"),
                schema_name: "NameChoice",
            }
        }
    }
}

/// Ask the configured decider whether `name` (a value of `field`) is one of
/// `candidates` from that field's name list. `Ok(None)` = decider off
/// (caller falls back to [`crate::harmonize::Decision::resolve`] with `None`).
pub fn decide_name(
    candidates: &[Candidate],
    name: &str,
    field: &str,
    cfg: &DeciderSettings,
) -> Result<Option<Verdict>, ProviderError> {
    if candidates.is_empty() {
        return Ok(Some(Verdict::New));
    }
    let w = wording(field, name);
    match cfg.decider {
        Decider::Off => Ok(None),
        Decider::Ai => decide_ai(candidates, &w, &cfg.provider).map(Some),
        Decider::OpenaiDecisions => decide_remote(candidates, &w, cfg, Kind::Decisions).map(Some),
        Decider::Jev => decide_remote(candidates, &w, cfg, Kind::Jev).map(Some),
    }
}

fn decide_ai(
    candidates: &[Candidate],
    w: &Wording,
    cfg: &ProviderSettings,
) -> Result<Verdict, ProviderError> {
    let mut options: Vec<Value> = (0..candidates.len())
        .map(|i| json!(i.to_string()))
        .collect();
    options.push(json!("new"));
    let listing: String = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{i}: {}", c.canonical))
        .collect::<Vec<_>>()
        .join("\n");
    let req = ExtractRequest {
        system_prompt: w.system.clone(),
        user_text: format!("{}\n\n{}:\n{listing}", w.question, w.known),
        schema: json!({
            "type": "object",
            "properties": {"choice": {"type": "string", "enum": options}},
            "required": ["choice"],
            "additionalProperties": false,
        }),
        schema_name: w.schema_name.into(),
        images: Vec::new(),
        max_tokens: 256,
    };
    let resp = extract_detailed(&req, cfg)?;
    let choice = resp.value["choice"].as_str().unwrap_or("new");
    Ok(match choice.parse::<usize>() {
        Ok(i) if i < candidates.len() => Verdict::Existing(candidates[i].canonical.clone()),
        _ => Verdict::New,
    })
}

fn decide_remote(
    candidates: &[Candidate],
    w: &Wording,
    cfg: &DeciderSettings,
    kind: Kind,
) -> Result<Verdict, ProviderError> {
    let label = kind.label();
    if cfg.provider.privacy == Privacy::Offline {
        return Err(perr(
            ErrorKind::OfflineBlocked,
            label,
            format!("decider {label} is a cloud service and is refused in offline mode"),
        ));
    }
    if cfg.api_key.trim().is_empty() {
        return Err(perr(
            ErrorKind::Auth,
            label,
            format!("API key required for decider '{label}'"),
        ));
    }
    let default_url = if kind == Kind::Jev {
        JEV_URL
    } else {
        DECISIONS_URL
    };
    let url = if cfg.url.is_empty() {
        default_url.to_string()
    } else {
        cfg.url.clone()
    };
    guard_offline(cfg.provider.privacy, &url, label)?;
    // The API allows 255 choices: the best 254 candidates plus "new".
    let candidates = &candidates[..candidates.len().min(254)];
    let mut options: Vec<String> = candidates.iter().map(|c| c.canonical.clone()).collect();
    options.push(w.new_option.clone());
    let body = match kind {
        Kind::Decisions => decisions_body(candidates, w),
        // BEST-EFFORT (abridged notes): a `choice` over given options.
        _ => json!({"type": "choice", "input": w.question, "options": options}),
    };
    let call = HttpCall {
        url,
        headers: vec![("authorization", format!("Bearer {}", cfg.api_key))],
        body,
    };
    let client = http_client(
        cfg.provider.timeout.min(Duration::from_secs(60)),
        cfg.provider.privacy,
        label,
    )?;
    let (raw, _) = send(&client, &call, kind, &cfg.provider.retry)?;
    let picked = match kind {
        Kind::Decisions => decisions_choice(&raw.body, candidates.len()),
        _ => parse_choice(&raw.body, &options),
    };
    match picked {
        Some(i) if i < candidates.len() => Ok(Verdict::Existing(candidates[i].canonical.clone())),
        Some(_) => Ok(Verdict::New),
        None => Err(perr(
            ErrorKind::InvalidOutput,
            label,
            "decision response has no recognizable choice",
        )
        .with_request_id(raw.request_id)),
    }
}

/// One `choice` question: values `"0".."k-1"` (candidate index, the name as
/// description) and `"new"`.
fn decisions_body(candidates: &[Candidate], w: &Wording) -> Value {
    let mut choices: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| json!({"value": i.to_string(), "description": c.canonical}))
        .collect();
    choices.push(json!({"value": "new", "description": w.new_option}));
    json!({
        "model": DECISIONS_MODEL,
        "input": [
            {"role": "developer", "content": w.system},
            {"role": "user", "content": w.question},
        ],
        "questions": [{
            "type": "choice",
            "name": "match",
            "instructions": format!("Pick the matching entry of the {} or \"new\".", w.known.to_lowercase()),
            "choices": choices,
        }],
    })
}

/// Index picked in a Decisions response (`answers[0].choice`); `k` = "new".
/// A refusal or a value outside the choices is `None`.
fn decisions_choice(body: &Value, k: usize) -> Option<usize> {
    let answer = body.get("answers")?.get(0)?;
    match answer.get("choice")?.as_str()? {
        "new" => Some(k),
        v => v.parse::<usize>().ok().filter(|i| *i < k),
    }
}

/// Find the chosen option index in a tolerant way: `choice` / `answer` as an
/// option string or index, at the top level or under `output`/`result`/`decision`.
fn parse_choice(body: &Value, options: &[String]) -> Option<usize> {
    let scopes = [
        Some(body),
        body.get("output"),
        body.get("result"),
        body.get("decision"),
    ];
    let v = scopes
        .into_iter()
        .flatten()
        .flat_map(|s| ["choice", "answer", "index"].map(|k| s.get(k)))
        .flatten()
        .next()?;
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(|n| n as usize)
            .filter(|i| *i < options.len()),
        Value::String(s) => options
            .iter()
            .position(|o| o.eq_ignore_ascii_case(s.trim())),
        Value::Object(o) => o
            .get("index")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .or_else(|| {
                o.get("value")
                    .and_then(Value::as_str)
                    .and_then(|s| options.iter().position(|x| x.eq_ignore_ascii_case(s)))
            }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerant_choice_parsing() {
        let opts = vec![
            "Acme".to_string(),
            "None of these (new company)".to_string(),
        ];
        assert_eq!(parse_choice(&json!({"choice": "acme"}), &opts), Some(0));
        assert_eq!(
            parse_choice(&json!({"output": {"choice": 1}}), &opts),
            Some(1)
        );
        assert_eq!(
            parse_choice(&json!({"result": {"choice": {"index": 0}}}), &opts),
            Some(0)
        );
        assert_eq!(parse_choice(&json!({"choice": "Globex"}), &opts), None);
        assert_eq!(parse_choice(&json!({}), &opts), None);
    }

    #[test]
    fn decisions_request_and_answer() {
        let cands = [Candidate {
            canonical: "OpenAI".into(),
            matched: "OpenAI".into(),
            score: 0.9,
        }];
        let w = wording("company_name", "OpenAI OpCo");
        let body = decisions_body(&cands, &w);
        assert_eq!(body["model"], DECISIONS_MODEL);
        let q = &body["questions"][0];
        assert_eq!(q["type"], "choice");
        assert_eq!(
            q["choices"][0],
            json!({"value": "0", "description": "OpenAI"})
        );
        assert_eq!(q["choices"][1]["value"], "new");
        let answer =
            |c: &str| json!({"answers": [{"type": "choice", "name": "match", "choice": c}]});
        assert_eq!(decisions_choice(&answer("0"), 1), Some(0));
        assert_eq!(decisions_choice(&answer("new"), 1), Some(1));
        assert_eq!(decisions_choice(&answer("7"), 1), None);
        assert_eq!(
            decisions_choice(
                &json!({"answers": [{"type": "refusal", "name": "match"}]}),
                1
            ),
            None
        );
    }

    #[test]
    fn wording_follows_the_list_kind() {
        let c = wording("company_name", "ACME");
        assert!(
            c.question
                .starts_with("An invoice names the company \"ACME\"")
        );
        assert_eq!(c.schema_name, "CompanyChoice");
        let v = wording("journal_name", "Nature Phys.");
        for text in [&v.question, &v.system, &v.known, &v.new_option] {
            let t = text.to_lowercase();
            assert!(!t.contains("compan") && !t.contains("invoice"), "{text}");
        }
        assert!(
            v.question.contains("the venue \"Nature Phys.\""),
            "{}",
            v.question
        );
        assert_eq!(v.known, "Known venues");
        assert_eq!(
            wording("vendor", "x").new_option,
            "None of these (new vendor)"
        );
    }
}
