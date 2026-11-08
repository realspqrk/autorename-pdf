//! Ollama native chat adapter (`POST {base}/api/chat`), structured output via
//! `format: <JSON Schema>`. No API key; base is the server root.

use serde_json::{Value, json};

use super::openai::Input;
use super::{
    Envelope, ErrorKind, HttpCall, ProviderError, ProviderSettings, Usage, b64, perr, schema_hint,
    tokens,
};

const LABEL: &str = "ollama";

pub(crate) fn build(input: &Input, cfg: &ProviderSettings, base: &str) -> HttpCall {
    // Ollama docs recommend repeating the schema in the prompt.
    let system = format!("{}\n\n{}", input.system, schema_hint(input.schema));
    let mut user = json!({"role": "user", "content": input.user_text});
    if !input.images.is_empty() {
        user["images"] = Value::Array(input.images.iter().map(|p| Value::String(b64(p))).collect());
    }
    let body = json!({
        "model": cfg.model,
        "stream": false,
        "think": false,
        "format": input.schema,
        "options": {"temperature": cfg.temperature, "num_predict": input.max_tokens},
        "messages": [{"role": "system", "content": system}, user],
    });
    let mut headers = Vec::new();
    if !cfg.api_key.trim().is_empty() {
        headers.push(("authorization", format!("Bearer {}", cfg.api_key)));
    }
    HttpCall {
        url: format!("{base}/api/chat"),
        headers,
        body,
    }
}

pub(crate) fn parse(body: &Value) -> Result<Envelope, ProviderError> {
    // UNVERIFIED: `done_reason` values are not documented; "length" = cut off.
    if body.get("done_reason").and_then(Value::as_str) == Some("length") {
        return Err(perr(
            ErrorKind::Truncated,
            LABEL,
            "response truncated (done_reason: length)",
        ));
    }
    let text = body
        .pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if text.trim().is_empty() {
        return Err(perr(
            ErrorKind::InvalidOutput,
            LABEL,
            "response has no message content",
        ));
    }
    Ok(Envelope {
        text,
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        usage: Usage {
            input_tokens: tokens(body.get("prompt_eval_count")),
            output_tokens: tokens(body.get("eval_count")),
        },
    })
}
