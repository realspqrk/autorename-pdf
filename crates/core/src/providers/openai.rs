//! OpenAI Responses API adapter (`POST {base}/responses`); xAI reuses it
//! (OpenAI-compatible Responses endpoint at `https://api.x.ai/v1`).

use serde_json::{Value, json};

use super::{
    Envelope, ErrorKind, HttpCall, ProviderError, ProviderSettings, Usage, perr, png_data_url,
    tokens,
};
use crate::config::Provider;

/// Provider-neutral inputs shared by all adapters' `build`.
pub(crate) struct Input<'a> {
    pub system: &'a str,
    pub user_text: &'a str,
    pub images: &'a [Vec<u8>],
    /// Already translated for the provider.
    pub schema: &'a Value,
    pub schema_name: &'a str,
    pub max_tokens: u32,
}

/// `reasoning.effort: "none"` is accepted by the GPT-6 family and the
/// `*-luna` tier (provider reference §1). Older non-reasoning models reject
/// the `reasoning` object, so it is only sent where known to work.
fn supports_reasoning_none(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("luna") || m.starts_with("gpt-6") || m.starts_with("gpt-5.6")
}

pub(crate) fn build(input: &Input, cfg: &ProviderSettings, base: &str) -> HttpCall {
    let xai = cfg.provider == Provider::Xai;
    let detail = if xai { "high" } else { "auto" };
    let mut user = vec![json!({"type": "input_text", "text": input.user_text})];
    user.extend(input.images.iter().map(
        |png| json!({"type": "input_image", "image_url": png_data_url(png), "detail": detail}),
    ));
    let mut body = json!({
        "model": cfg.model,
        "max_output_tokens": input.max_tokens,
        "input": [
            {"role": "system", "content": [{"type": "input_text", "text": input.system}]},
            {"role": "user", "content": user},
        ],
        "text": {"format": {
            "type": "json_schema",
            "name": input.schema_name,
            "strict": true,
            "schema": input.schema,
        }},
    });
    if xai {
        // xAI: no "none" effort; "low" is the cheapest documented level.
        body["reasoning"] = json!({"effort": "low"});
        // UNVERIFIED (provider reference §1/§5): xAI warns that `store:false`
        // with images "may fail", so it is only sent for text-only requests.
        if input.images.is_empty() {
            body["store"] = json!(false);
        }
    } else {
        body["store"] = json!(false);
        if supports_reasoning_none(&cfg.model) {
            body["reasoning"] = json!({"effort": "none"});
        }
    }
    HttpCall {
        url: format!("{base}/responses"),
        headers: vec![("authorization", format!("Bearer {}", cfg.api_key))],
        body,
    }
}

pub(crate) fn parse(label: &'static str, body: &Value) -> Result<Envelope, ProviderError> {
    let status = body.get("status").and_then(Value::as_str).unwrap_or("");
    if status == "incomplete" {
        let reason = body
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(perr(
            ErrorKind::Truncated,
            label,
            format!("response incomplete ({reason})"),
        ));
    }
    if status == "failed" {
        let msg = body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("response failed");
        return Err(perr(ErrorKind::Unavailable, label, msg.to_string()));
    }
    let mut text = String::new();
    for item in body
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        for part in item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match part.get("type").and_then(Value::as_str) {
                Some("refusal") => {
                    let why = part.get("refusal").and_then(Value::as_str).unwrap_or("");
                    return Err(perr(
                        ErrorKind::Refusal,
                        label,
                        format!("model refused: {why}"),
                    ));
                }
                Some("output_text") => {
                    text.push_str(part.get("text").and_then(Value::as_str).unwrap_or(""));
                }
                _ => {}
            }
        }
    }
    if text.trim().is_empty() {
        return Err(perr(
            ErrorKind::InvalidOutput,
            label,
            "response has no output text",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_none_models() {
        assert!(supports_reasoning_none("gpt-6-luna"));
        assert!(supports_reasoning_none("gpt-6.1-terra"));
        assert!(!supports_reasoning_none("gpt-4.1-mini"));
    }
}
