//! Anthropic Messages API adapter (`POST {base}/messages`) with structured
//! output via `output_config.format` (GA, no beta header).

use serde_json::{Value, json};

use super::openai::Input;
use super::{
    Envelope, ErrorKind, HttpCall, ProviderError, ProviderSettings, Usage, b64, perr, tokens,
};

/// Pinned API version (provider reference §1).
pub(crate) const ANTHROPIC_VERSION: &str = "2026-10-21";
const LABEL: &str = "anthropic";

pub(crate) fn build(input: &Input, cfg: &ProviderSettings, base: &str) -> HttpCall {
    let mut content = vec![json!({"type": "text", "text": input.user_text})];
    content.extend(input.images.iter().map(|png| {
        json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(png)}})
    }));
    // No temperature/top_p/top_k (rejected on Claude 5 models). Thinking off
    // + low effort: extraction needs no reasoning. UNVERIFIED for models other
    // than claude-haiku-5-5 (reference only states Haiku 5.5 accepts
    // `thinking: disabled` at effort ≤ high).
    let body = json!({
        "model": cfg.model,
        "max_tokens": input.max_tokens,
        "system": input.system,
        "thinking": {"type": "disabled"},
        "output_config": {
            "effort": "low",
            "format": {"type": "json_schema", "schema": input.schema},
        },
        "messages": [{"role": "user", "content": content}],
    });
    HttpCall {
        url: format!("{base}/messages"),
        headers: vec![
            ("x-api-key", cfg.api_key.clone()),
            ("anthropic-version", ANTHROPIC_VERSION.to_string()),
        ],
        body,
    }
}

pub(crate) fn parse(body: &Value) -> Result<Envelope, ProviderError> {
    match body.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => {
            return Err(perr(
                ErrorKind::Refusal,
                LABEL,
                "model refused (stop_reason: refusal)",
            ));
        }
        Some("max_tokens") => {
            return Err(perr(
                ErrorKind::Truncated,
                LABEL,
                "response truncated (stop_reason: max_tokens)",
            ));
        }
        _ => {}
    }
    let text: String = body
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    if text.trim().is_empty() {
        return Err(perr(
            ErrorKind::InvalidOutput,
            LABEL,
            "response has no text block",
        ));
    }
    Ok(Envelope {
        text,
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        usage: Usage {
            input_tokens: tokens(body.pointer("/usage/input_tokens")),
            output_tokens: tokens(body.pointer("/usage/output_tokens")),
        },
    })
}
