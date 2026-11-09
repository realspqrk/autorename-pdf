//! Gemini native `generateContent` adapter
//! (`POST {base}/models/{model}:generateContent`, `x-goog-api-key`).
//!
//! Isolated on purpose: Google labels `generateContent` "legacy, still
//! supported" and pushes the Interactions API (`/v1beta/interactions`), so a
//! later swap should only touch this file.
//!
//! UNVERIFIED field names (provider reference §4): the docs' REST example uses
//! `generationConfig.responseFormat.text = {mimeType, schema}` while the API
//! reference uses `generationConfig.responseMimeType` +
//! `generationConfig.responseJsonSchema`. This implements the API-reference
//! form. A live smoke test must confirm it; if Gemini returns 400 on
//! `responseJsonSchema`, switch to the `responseFormat.text` form here.
//! `thinkingConfig.thinkingLevel: "MINIMAL"` is also unverified for every
//! Gemini 3.x model (thinking cannot be fully disabled on Gemini 3).

use serde_json::{Value, json};

use super::openai::Input;
use super::{
    Envelope, ErrorKind, HttpCall, ProviderError, ProviderSettings, Usage, b64, perr, tokens,
};

const LABEL: &str = "gemini";
const BLOCKING_FINISH: [&str; 5] = [
    "SAFETY",
    "PROHIBITED_CONTENT",
    "SPII",
    "BLOCKLIST",
    "RECITATION",
];

pub(crate) fn build(input: &Input, cfg: &ProviderSettings, base: &str) -> HttpCall {
    let mut parts = vec![json!({"text": input.user_text})];
    parts.extend(
        input
            .images
            .iter()
            .map(|png| json!({"inlineData": {"mimeType": "image/png", "data": b64(png)}})),
    );
    let model = cfg.model.strip_prefix("models/").unwrap_or(&cfg.model);
    let body = json!({
        "systemInstruction": {"parts": [{"text": input.system}]},
        "contents": [{"role": "user", "parts": parts}],
        "generationConfig": {
            "responseMimeType": "application/json",
            "responseJsonSchema": input.schema,
            "maxOutputTokens": input.max_tokens,
            "thinkingConfig": {"thinkingLevel": "MINIMAL"},
        },
    });
    HttpCall {
        url: format!("{base}/models/{model}:generateContent"),
        headers: vec![("x-goog-api-key", cfg.api_key.clone())],
        body,
    }
}

pub(crate) fn parse(body: &Value) -> Result<Envelope, ProviderError> {
    if let Some(reason) = body
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        return Err(perr(
            ErrorKind::Refusal,
            LABEL,
            format!("prompt blocked ({reason})"),
        ));
    }
    let Some(cand) = body.pointer("/candidates/0") else {
        return Err(perr(
            ErrorKind::InvalidOutput,
            LABEL,
            "response has no candidates",
        ));
    };
    match cand.get("finishReason").and_then(Value::as_str) {
        Some("MAX_TOKENS") => {
            return Err(perr(
                ErrorKind::Truncated,
                LABEL,
                "response truncated (finishReason: MAX_TOKENS)",
            ));
        }
        Some(r) if BLOCKING_FINISH.contains(&r) => {
            return Err(perr(
                ErrorKind::Refusal,
                LABEL,
                format!("response blocked (finishReason: {r})"),
            ));
        }
        _ => {}
    }
    let text: String = cand
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("thought").and_then(Value::as_bool) != Some(true))
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect();
    if text.trim().is_empty() {
        return Err(perr(
            ErrorKind::InvalidOutput,
            LABEL,
            "response has no text part",
        ));
    }
    Ok(Envelope {
        text,
        model: body
            .get("modelVersion")
            .and_then(Value::as_str)
            .map(str::to_string),
        usage: Usage {
            input_tokens: tokens(body.pointer("/usageMetadata/promptTokenCount")),
            output_tokens: tokens(body.pointer("/usageMetadata/candidatesTokenCount")),
        },
    })
}
