//! Canonical JSON Schema → per-provider wire schema, plus a local validator.
//!
//! Only the forms we emit are supported: an object of string properties
//! (`metadata_schema`) and a string `enum` (decider choice). Providers
//! silently ignore or reject different keywords, so every answer is
//! re-validated locally against the *canonical* schema.

use serde_json::{Map, Value};

use crate::config::Provider;

/// Keywords providers accept for the forms we emit (`title` is dropped for
/// OpenAI/xAI and Gemini; Anthropic keeps it).
const WIRE_KEYS: [&str; 6] = [
    "type",
    "properties",
    "required",
    "additionalProperties",
    "enum",
    "description",
];

/// Wire schema for `provider`.
pub fn for_provider(provider: Provider, canonical: &Value) -> Value {
    match provider {
        Provider::OpenAi | Provider::Xai => openai_strict(canonical),
        Provider::Anthropic => anthropic(canonical),
        Provider::Gemini => gemini(canonical),
        Provider::Ollama => canonical.clone(),
    }
}

/// OpenAI strict mode: every object lists all keys in `required` and sets
/// `additionalProperties: false`.
pub fn openai_strict(schema: &Value) -> Value {
    walk(schema, &|m| {
        m.retain(|k, _| WIRE_KEYS.contains(&k.as_str()));
        close_object(m, true);
    })
}

/// Anthropic: `additionalProperties` only `false`.
pub fn anthropic(schema: &Value) -> Value {
    walk(schema, &|m| close_object(m, false))
}

/// Gemini `responseJsonSchema` subset.
pub fn gemini(schema: &Value) -> Value {
    walk(schema, &|m| {
        m.retain(|k, _| WIRE_KEYS.contains(&k.as_str()))
    })
}

fn close_object(m: &mut Map<String, Value>, require_all: bool) {
    if !m.contains_key("properties") {
        return;
    }
    m.insert("additionalProperties".into(), Value::Bool(false));
    if require_all {
        let keys: Vec<Value> = m
            .get("properties")
            .and_then(Value::as_object)
            .map(|p| p.keys().cloned().map(Value::String).collect())
            .unwrap_or_default();
        m.insert("required".into(), Value::Array(keys));
    }
}

/// Apply `f` to the root and each property schema.
fn walk(schema: &Value, f: &dyn Fn(&mut Map<String, Value>)) -> Value {
    let Value::Object(src) = schema else {
        return schema.clone();
    };
    let mut m = src.clone();
    f(&mut m);
    if let Some(Value::Object(props)) = m.get_mut("properties") {
        for v in props.values_mut() {
            *v = walk(v, f);
        }
    }
    Value::Object(m)
}

// ---------------------------------------------------------------------------
// Local validation
// ---------------------------------------------------------------------------

/// Validate `value` against an emitted schema: `type: string` (+ `enum`), or
/// `type: object` with `properties` / `required` / `additionalProperties:
/// false`. Error text is fed back to the model on the re-ask, so it names the
/// JSON path.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    check(schema, value, "$")
}

fn check(schema: &Value, v: &Value, path: &str) -> Result<(), String> {
    match schema.get("type").and_then(Value::as_str) {
        Some("string") if !v.is_string() => return Err(format!("{path} must be of type string")),
        Some("object") if !v.is_object() => return Err(format!("{path} must be of type object")),
        _ => {}
    }
    if let Some(Value::Array(options)) = schema.get("enum")
        && !options.contains(v)
    {
        return Err(format!(
            "{path} must be one of {}",
            Value::Array(options.clone())
        ));
    }
    let Some(obj) = v.as_object() else {
        return Ok(());
    };
    let props = schema.get("properties").and_then(Value::as_object);
    for k in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !obj.contains_key(k) {
            return Err(format!("{path}.{k} is required"));
        }
    }
    for (k, x) in obj {
        match props.and_then(|p| p.get(k)) {
            Some(sub) => check(sub, x, &format!("{path}.{k}"))?,
            None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                return Err(format!("{path}.{k} is not an allowed field"));
            }
            None => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canonical() -> Value {
        json!({
            "title": "DocumentMetadata",
            "type": "object",
            "properties": {
                "company": {"type": "string", "title": "Company", "description": "c"},
                "date": {"type": "string", "title": "Date", "description": "d"}
            },
            "required": ["company"]
        })
    }

    #[test]
    fn openai_closes_objects_and_requires_all() {
        let s = openai_strict(&canonical());
        assert_eq!(s["required"], json!(["company", "date"]));
        assert_eq!(s["additionalProperties"], json!(false));
        assert!(s.get("title").is_none());
        assert!(s["properties"]["company"].get("title").is_none());
        assert_eq!(s["properties"]["company"]["description"], "c");
    }

    #[test]
    fn anthropic_closes_objects_keeps_title() {
        let s = anthropic(&canonical());
        assert_eq!(s["additionalProperties"], json!(false));
        assert_eq!(s["required"], json!(["company"]));
        assert_eq!(s["title"], json!("DocumentMetadata"));
    }

    #[test]
    fn gemini_subset() {
        let s = gemini(&canonical());
        assert!(s.get("title").is_none());
        assert!(s["properties"]["company"].get("title").is_none());
        assert!(s.get("additionalProperties").is_none());
    }

    #[test]
    fn validator_subset() {
        let mut s = canonical();
        s["additionalProperties"] = json!(false);
        assert!(validate(&s, &json!({"company": "Acme"})).is_ok());
        assert!(validate(&s, &json!({"company": "Acme", "date": "x"})).is_ok());
        assert!(
            validate(&s, &json!({}))
                .unwrap_err()
                .contains("$.company is required")
        );
        assert!(validate(&s, &json!({"company": 1})).is_err());
        assert!(validate(&s, &json!("x")).is_err());
        assert!(
            validate(&s, &json!({"company": "A", "x": "y"}))
                .unwrap_err()
                .contains("$.x")
        );
        let e = json!({"type": "string", "enum": ["0", "new"]});
        assert!(validate(&e, &json!("new")).is_ok());
        assert!(validate(&e, &json!("7")).is_err());
        assert!(validate(&e, &json!(0)).is_err());
    }
}
