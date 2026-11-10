//! Extraction profiles: built-ins, overlay/extends resolution, placeholder
//! parsing, prompt interpolation and the per-profile JSON Schema (contract §3).
//!
//! Resolution runs on the loose config tree (`serde_json::Value`) exactly like
//! the Python dict code, then converts to the typed [`Profile`].

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::textutil::is_identifier;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldSpec {
    pub description: String,
}

/// A fully resolved, validated and interpolated profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Profile {
    pub intro: String,
    pub fields: IndexMap<String, FieldSpec>,
    pub template: String,
    pub truncate_field: String,
    pub harmonize_field: Option<String>,
    /// Derived direction of a document (incoming/outgoing), see [`Direction`].
    pub direction: Option<Direction>,
    /// Deterministic amount formatting, see [`Amount`].
    pub amount: Option<Amount>,
}

/// Deterministic amount formatting: when the `amount` field is non-empty,
/// `field` becomes `output.amount_format` filled with `{amount}` and
/// `{currency}` (default `"{currency} {amount}"`), so the model never
/// decides the filename's amount format. No currency → warning
/// `currency_missing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Amount {
    /// Field that receives the formatted amount (e.g. `description`).
    pub field: String,
    /// Field with the amount as printed, without currency.
    pub amount: String,
    /// Field with the currency (ISO code or printed symbol).
    pub currency: String,
}

/// Deterministic derivation of one field (e.g. `document_type` = AP/AR) from
/// two extracted company fields, instead of trusting the model's choice:
/// when the model's `field` value is `incoming` or `outgoing`, the value is
/// re-derived by matching `recipient` / `issuer` against `company.names`
/// (harmonized fuzzy match). Ambiguous (both/neither match) keeps the model's
/// value and warns `direction_uncertain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Direction {
    /// Field whose value is derived (e.g. `document_type`).
    pub field: String,
    /// Field holding the sender / issuing company.
    pub issuer: String,
    /// Field holding the recipient / billed company.
    pub recipient: String,
    /// Value when the recipient is my company (e.g. `AP`).
    pub incoming: String,
    /// Value when the issuer is my company (e.g. `AR`).
    pub outgoing: String,
    /// Field set to the side that is not my company (e.g. `company_name`);
    /// my own company never becomes the counterparty.
    pub counterparty: Option<String>,
}

pub const BUILTIN_IDS: [&str; 2] = ["business", "academic"];
const PROFILE_KEYS: [&str; 8] = [
    "intro",
    "fields",
    "template",
    "truncate_field",
    "harmonize_field",
    "direction",
    "amount",
    "extends",
];
/// Replaced wholesale by an overlay (`direction: null` disables it).
const PROFILE_SCALARS: [&str; 6] = [
    "intro",
    "template",
    "truncate_field",
    "harmonize_field",
    "direction",
    "amount",
];
const AMOUNT_KEYS: [&str; 3] = ["field", "amount", "currency"];
const DIRECTION_KEYS: [&str; 5] = ["field", "issuer", "recipient", "incoming", "outgoing"];
const DIRECTION_OPTIONAL: [&str; 1] = ["counterparty"];
fn err(msg: impl Into<String>) -> Error {
    Error::config(msg)
}

/// String value of `section.key` (`""` when absent or not a string).
fn text_at(config: &Value, section: &str, key: &str) -> String {
    config
        .get(section)
        .and_then(|s| s.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// `company.names` for the prompt: `"A", "B"` (empty when none).
fn company_names(config: &Value) -> String {
    crate::config::company_names(config)
        .iter()
        .map(|n| format!("{n:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn company_name_description(config: &Value) -> &'static str {
    if company_names(config).is_empty() {
        "Find the main company in the document. Strip the legal form, such as e.U., SARL, GmbH, AG, Ltd, or Limited. Empty if no company is found."
    } else {
        "Find the counterparty company. My own company goes by these names: {company}. Exclude my own company under any of them. Strip the legal form, such as e.U., SARL, GmbH, AG, Ltd, or Limited. Empty if no company is found."
    }
}

/// Fresh built-in profile trees (verbatim text from `_profiles.py`).
pub fn builtin_profiles(config: &Value) -> Map<String, Value> {
    let business = json!({
        "intro": "Extract metadata for naming a business document. Separate the document category from its printed amount or subject. Never invent missing values.",
        "fields": {
            "document_date": {"description": "Find the most relevant document date, such as the invoice date or letter date, rather than a payment due date. Interpret the printed date according to the document's language and location. Return dd.mm.YYYY. Empty if no date is found."},
            "company_name": {"description": company_name_description(config)},
            "issuer": {"description": "Company that issued or sent the document (the seller on an invoice), as printed, including the legal form. Empty if not found."},
            "recipient": {"description": "Company or person the document is addressed to (the buyer or bill-to party on an invoice), as printed, including the legal form. Empty if not found."},
            "document_type": {"description": "For incoming invoices, use only \"{incoming_invoice}\". For outgoing invoices, use only \"{outgoing_invoice}\". Do not append an amount, invoice number, or the words Invoice or Rechnung to either code. For other documents, return a short category in {language}, such as Letter or Contract, not a generated subject summary. Empty if unknown."},
            "total_amount": {"description": "Invoices only: the printed final invoice total including tax, digits and punctuation exactly as printed, without currency, for example 12,13 or 1.234,56 or 4,258.80 (from \"TOTAL EUR 4,258.80\"). Prefer the explicitly labelled final total over subtotals, tax-only amounts, or line items. Do not calculate a missing total. Empty for other documents or if no total is printed."},
            "currency": {"description": "Currency of total_amount: the ISO 4217 code when printed or when a printed symbol is unambiguous (€ → EUR, £ → GBP), otherwise the symbol as printed. Never guess or convert. Empty if no currency is printed or total_amount is empty."},
            "description": {"description": "For invoices: empty (the amount goes into total_amount). For other documents, copy the document's own explicit subject or title verbatim in its original language. Do not summarize, translate, or invent a subject, or fall back to a generic document category or identifying number. Empty if no subject or title is present."}
        },
        "template": "{document_date} {company_name} {document_type} {description}",
        "truncate_field": "description",
        "harmonize_field": "company_name",
        "direction": {
            "field": "document_type",
            "issuer": "issuer",
            "recipient": "recipient",
            "incoming": "{incoming_invoice}",
            "outgoing": "{outgoing_invoice}",
            "counterparty": "company_name"
        },
        "amount": {"field": "description", "amount": "total_amount", "currency": "currency"}
    });
    let academic = json!({
        "intro": "You are naming an academic paper, preprint, or thesis.",
        "fields": {
            "document_date": {"description": "Publication date in dd.mm.YYYY. Prefer publication over submission or revision. If only a year is printed, use 01.01 of that year. Empty if no date or year is found."},
            "first_author_surname": {"description": "First author's surname only; no initials, given names, or et al. Empty if absent."},
            "journal_name": {"description": "Journal or conference venue as printed, including its printed abbreviation. Empty when no venue is given."},
            "title": {"description": "Paper title as printed, in its original language. Empty if absent."}
        },
        "template": "{document_date} {first_author_surname} {journal_name} {title}",
        "truncate_field": "title",
        "harmonize_field": null
    });
    let mut out = Map::new();
    out.insert("business".into(), business);
    out.insert("academic".into(), academic);
    out
}

// ---------------------------------------------------------------------------
// Placeholder syntax (shared by prompt text and filename templates)
// ---------------------------------------------------------------------------

/// A template token: literal text (braces already unescaped) or a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Literal(String),
    Field(String),
}

/// Strictly parse `text` into tokens. `{{`/`}}` escape braces; `{name}` needs
/// an identifier; anything else (`{}`, `{a.b}`, `{a!r}`, lone braces) fails.
pub fn parse_placeholders(text: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                lit.push('{');
                i += 2;
            }
            '{' => {
                let close = chars[i + 1..]
                    .iter()
                    .position(|c| *c == '}')
                    .map(|p| p + i + 1);
                let Some(close) = close else {
                    return Err(err("unclosed brace"));
                };
                let name: String = chars[i + 1..close].iter().collect();
                if !is_identifier(&name) {
                    return Err(err(format!("invalid placeholder {{{name}}}")));
                }
                if !lit.is_empty() {
                    tokens.push(Token::Literal(std::mem::take(&mut lit)));
                }
                tokens.push(Token::Field(name));
                i = close + 1;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                lit.push('}');
                i += 2;
            }
            '}' => return Err(err("unmatched brace")),
            c => {
                lit.push(c);
                i += 1;
            }
        }
    }
    if !lit.is_empty() {
        tokens.push(Token::Literal(lit));
    }
    Ok(tokens)
}

/// Tokens of an already validated template (invalid input yields one literal).
pub fn template_tokens(template: &str) -> Vec<Token> {
    parse_placeholders(template).unwrap_or_else(|_| vec![Token::Literal(template.to_string())])
}

fn field_names(tokens: &[Token]) -> Vec<&str> {
    tokens
        .iter()
        .filter_map(|t| match t {
            Token::Field(n) => Some(n.as_str()),
            Token::Literal(_) => None,
        })
        .collect()
}

/// Expand the four prompt placeholders once; substituted values stay literal.
pub fn interpolate_profile_text(text: &str, config: &Value) -> Result<String> {
    let values = [
        ("company", company_names(config)),
        ("language", text_at(config, "output", "language")),
        (
            "incoming_invoice",
            text_at(config, "pdf", "incoming_invoice"),
        ),
        (
            "outgoing_invoice",
            text_at(config, "pdf", "outgoing_invoice"),
        ),
    ];
    let tokens = parse_placeholders(text)?;
    let mut out = String::new();
    for token in &tokens {
        match token {
            Token::Literal(l) => out.push_str(l),
            Token::Field(name) => {
                let Some((_, v)) = values.iter().find(|(k, _)| k == name) else {
                    return Err(err(format!("unknown prompt placeholder {:?}", name)));
                };
                out.push_str(v);
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

fn check_id(v: &Value) -> Result<&str> {
    match v {
        Value::String(s) if !s.is_empty() && s.trim() == s => Ok(s),
        other => Err(err(format!("invalid profile id {}", other))),
    }
}

/// Field ids become JSON-schema property names: ASCII letter first, then
/// ASCII letters, digits or `_`.
pub fn valid_field_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn validate_overlay_shape(id: &str, overlay: &Value, allow_extends: bool) -> Result<()> {
    let rid = format!("{:?}", id);
    let map = match overlay {
        Value::Null => return Err(err(format!("profile {rid} is null"))),
        Value::Object(m) => m,
        _ => return Err(err(format!("profile {rid} must be a mapping"))),
    };
    let mut extra: Vec<&String> = map
        .keys()
        .filter(|k| !PROFILE_KEYS.contains(&k.as_str()))
        .collect();
    if !extra.is_empty() {
        extra.sort();
        let list: Vec<String> = extra.iter().map(|k| format!("{:?}", k)).collect();
        return Err(err(format!(
            "unknown keys on profile {rid}: [{}]",
            list.join(", ")
        )));
    }
    if let Some(parent) = map.get("extends") {
        if !allow_extends {
            return Err(err(format!("built-in profile {rid} cannot set extends")));
        }
        match parent {
            Value::String(s) if !s.is_empty() && s.trim() == s => {}
            _ => return Err(err(format!("profile {rid} extends must be a single id"))),
        }
    }
    match map.get("intro") {
        Some(Value::Null) => return Err(err(format!("profile {rid} intro is null"))),
        Some(Value::String(_)) | None => {}
        Some(_) => return Err(err(format!("profile {rid} intro must be a string"))),
    }
    if let Some(t) = map.get("template") {
        match t {
            Value::String(s) if !s.is_empty() => {}
            _ => return Err(err(format!("profile {rid} template is empty"))),
        }
    }
    if let Some(fields) = map.get("fields") {
        let Some(fields) = fields.as_object() else {
            return Err(err(format!("profile {rid} fields must be a mapping")));
        };
        for (fid, spec) in fields {
            if spec.is_null() {
                continue;
            }
            let rf = format!("{:?}", fid);
            if !valid_field_id(fid) {
                return Err(err(format!("invalid field id {rf}")));
            }
            let Some(spec) = spec.as_object() else {
                return Err(err(format!("field {rf} must be a mapping")));
            };
            if spec.keys().any(|k| k != "description") {
                return Err(err(format!("unknown keys on field {rf}")));
            }
            match spec.get("description") {
                Some(Value::String(d)) if !d.trim().is_empty() => {}
                _ => return Err(err(format!("field {rf} description is empty"))),
            }
        }
    }
    Ok(())
}

fn apply_overlay(base: &Value, overlay: &Map<String, Value>) -> Result<Value> {
    let mut result = base.as_object().cloned().unwrap_or_default();
    for key in PROFILE_SCALARS {
        if let Some(v) = overlay.get(key) {
            result.insert(key.into(), v.clone());
        }
    }
    result
        .entry("intro")
        .or_insert_with(|| Value::String(String::new()));
    result.entry("harmonize_field").or_insert(Value::Null);
    result.entry("direction").or_insert(Value::Null);
    result.entry("amount").or_insert(Value::Null);
    if let Some(Value::Object(fields_overlay)) = overlay.get("fields") {
        let fields = result
            .entry("fields")
            .or_insert_with(|| Value::Object(Map::new()));
        let fields = fields
            .as_object_mut()
            .ok_or_else(|| err("fields must be a mapping"))?;
        for (fid, spec) in fields_overlay {
            if spec.is_null() {
                if !fields.contains_key(fid) {
                    return Err(err(format!("cannot delete missing field {:?}", fid)));
                }
                if fid == "document_date" {
                    return Err(err("document_date cannot be deleted"));
                }
                fields.shift_remove(fid);
            } else {
                if !valid_field_id(fid) {
                    return Err(err(format!("invalid field id {:?}", fid)));
                }
                // Existing ids are replaced in place; new ids append.
                fields.insert(fid.clone(), spec.clone());
            }
        }
    }
    // An inherited direction/amount whose fields were deleted no longer applies.
    for (key, refs) in [
        ("direction", &["field", "issuer", "recipient"][..]),
        ("amount", &AMOUNT_KEYS[..]),
    ] {
        if overlay.contains_key(key) {
            continue;
        }
        let Some(Value::Object(d)) = result.get(key) else {
            continue;
        };
        let fields = result.get("fields").and_then(Value::as_object);
        let gone = refs.iter().any(|k| {
            d.get(*k)
                .and_then(Value::as_str)
                .is_none_or(|f| !fields.is_some_and(|m| m.contains_key(f)))
        });
        if gone {
            result.insert(key.into(), Value::Null);
        }
    }
    // A deleted counterparty field only drops the counterparty part.
    if !overlay.contains_key("direction") {
        let has = |f: &str| {
            result
                .get("fields")
                .and_then(Value::as_object)
                .is_some_and(|m| m.contains_key(f))
        };
        let cp = result
            .get("direction")
            .and_then(|d| d.get("counterparty"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(cp) = cp
            && !has(&cp)
            && let Some(Value::Object(d)) = result.get_mut("direction")
        {
            d.shift_remove("counterparty");
        }
    }
    result.shift_remove("extends");
    Ok(Value::Object(result))
}

fn standalone_defaults(def: &Map<String, Value>) -> Result<Value> {
    let mut result = def.clone();
    result.shift_remove("extends");
    result
        .entry("intro")
        .or_insert_with(|| Value::String(String::new()));
    result.entry("harmonize_field").or_insert(Value::Null);
    result.entry("direction").or_insert(Value::Null);
    result.entry("amount").or_insert(Value::Null);
    match result.get("fields") {
        None | Some(Value::Null) => {
            result.insert("fields".into(), Value::Object(Map::new()));
        }
        Some(Value::Object(fields)) => {
            if let Some((fid, _)) = fields.iter().find(|(_, s)| s.is_null()) {
                return Err(err(format!("cannot delete missing field {:?}", fid)));
            }
        }
        Some(_) => return Err(err("fields must be a mapping")),
    }
    Ok(Value::Object(result))
}

fn validate_resolved(id: &str, profile: &Value) -> Result<()> {
    let rid = format!("{:?}", id);
    let fields = profile.get("fields").and_then(Value::as_object);
    let Some(fields) = fields.filter(|f| f.contains_key("document_date")) else {
        return Err(err(format!("profile {rid} must include document_date")));
    };
    if !fields["document_date"]
        .get("description")
        .is_some_and(Value::is_string)
    {
        return Err(err(format!("profile {rid} document_date is invalid")));
    }
    if let Some((fid, _)) = fields.iter().find(|(_, s)| !s.is_object()) {
        return Err(err(format!("field {:?} must be a mapping", fid)));
    }
    let template = match profile.get("template") {
        Some(Value::String(t)) if !t.is_empty() => t,
        _ => return Err(err(format!("profile {rid} is missing template"))),
    };
    if template.contains('/') || template.contains('\\') {
        return Err(err("template cannot contain path separators"));
    }
    if template.to_lowercase().ends_with(".pdf") {
        return Err(err("template must not include .pdf"));
    }
    let tokens = parse_placeholders(template)?;
    let names = field_names(&tokens);
    if let Some(name) = names.iter().find(|n| !fields.contains_key(**n)) {
        return Err(err(format!(
            "template placeholder {:?} is not a field",
            name
        )));
    }
    let truncate = profile.get("truncate_field").unwrap_or(&Value::Null);
    match truncate.as_str() {
        Some(t)
            if t != "document_date"
                && fields.contains_key(t)
                && template.contains(&format!("{{{t}}}")) =>
        {
            if !names.contains(&t) {
                return Err(err(format!(
                    "truncate_field {:?} is not in the template",
                    t
                )));
            }
        }
        _ => {
            return Err(err(format!("invalid truncate_field {}", truncate)));
        }
    }
    match profile.get("harmonize_field").unwrap_or(&Value::Null) {
        Value::Null => {}
        Value::String(h)
            if h != "document_date"
                && fields.contains_key(h)
                && crate::harmonize::is_field_id(h) => {}
        other => {
            return Err(err(format!(
                "invalid harmonize_field {other}: needs a template field with a lowercase ASCII id (a-z, 0-9, _), not document_date"
            )));
        }
    }
    if !profile.get("intro").is_none_or(Value::is_string) {
        return Err(err(format!("profile {rid} intro must be a string")));
    }
    match profile.get("direction").unwrap_or(&Value::Null) {
        Value::Null => {}
        Value::Object(d) => {
            if let Some(k) = d.keys().find(|k| {
                !DIRECTION_KEYS.contains(&k.as_str()) && !DIRECTION_OPTIONAL.contains(&k.as_str())
            }) {
                return Err(err(format!("unknown key {k:?} in profile {rid} direction")));
            }
            if let Some(cp) = d.get("counterparty")
                && !cp.as_str().is_some_and(|c| fields.contains_key(c))
            {
                return Err(err(format!(
                    "profile {rid} direction.counterparty {cp} is not a field"
                )));
            }
            for k in DIRECTION_KEYS {
                let Some(v) = d.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()) else {
                    return Err(err(format!(
                        "profile {rid} direction.{k} must be a non-empty string"
                    )));
                };
                if matches!(k, "field" | "issuer" | "recipient") && !fields.contains_key(v) {
                    return Err(err(format!(
                        "profile {rid} direction.{k} {v:?} is not a field"
                    )));
                }
            }
        }
        _ => {
            return Err(err(format!(
                "profile {rid} direction must be a mapping or null"
            )));
        }
    }
    match profile.get("amount").unwrap_or(&Value::Null) {
        Value::Null => {}
        Value::Object(d) => {
            if let Some(k) = d.keys().find(|k| !AMOUNT_KEYS.contains(&k.as_str())) {
                return Err(err(format!("unknown key {k:?} in profile {rid} amount")));
            }
            for k in AMOUNT_KEYS {
                match d.get(k).and_then(Value::as_str) {
                    Some(v) if fields.contains_key(v) => {}
                    other => {
                        return Err(err(format!(
                            "profile {rid} amount.{k} {:?} is not a field",
                            other.unwrap_or("")
                        )));
                    }
                }
            }
        }
        _ => {
            return Err(err(format!(
                "profile {rid} amount must be a mapping or null"
            )));
        }
    }
    Ok(())
}

fn finalize(profile: &Value, config: &Value) -> Result<Profile> {
    let intro = profile.get("intro").and_then(Value::as_str).unwrap_or("");
    let mut fields = IndexMap::new();
    for (fid, spec) in profile["fields"].as_object().into_iter().flatten() {
        let desc = spec["description"].as_str().unwrap_or("");
        fields.insert(
            fid.clone(),
            FieldSpec {
                description: interpolate_profile_text(desc, config)?,
            },
        );
    }
    Ok(Profile {
        intro: interpolate_profile_text(intro, config)?,
        fields,
        template: profile["template"].as_str().unwrap_or("").to_string(),
        truncate_field: profile["truncate_field"].as_str().unwrap_or("").to_string(),
        harmonize_field: profile
            .get("harmonize_field")
            .and_then(Value::as_str)
            .map(str::to_string),
        direction: match profile.get("direction") {
            Some(Value::Object(d)) => {
                let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                Some(Direction {
                    field: s("field"),
                    issuer: s("issuer"),
                    recipient: s("recipient"),
                    incoming: interpolate_profile_text(&s("incoming"), config)?,
                    outgoing: interpolate_profile_text(&s("outgoing"), config)?,
                    counterparty: d
                        .get("counterparty")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            }
            _ => None,
        },
        amount: match profile.get("amount") {
            Some(Value::Object(d)) => {
                let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                Some(Amount {
                    field: s("field"),
                    amount: s("amount"),
                    currency: s("currency"),
                })
            }
            _ => None,
        },
    })
}

/// Resolve every profile (built-ins, overlays, customs in declaration order).
/// Every declared profile is validated, used or not.
pub fn resolve_profiles(config: &Value) -> Result<IndexMap<String, Profile>> {
    let mut resolved = builtin_profiles(config);
    let empty = Map::new();
    let declarations = match config.get("profiles") {
        None => &empty,
        Some(Value::Object(m)) => m,
        Some(_) => return Err(err("profiles must be a mapping")),
    };
    for id in declarations.keys() {
        check_id(&Value::String(id.clone()))?;
    }
    for (id, overlay) in declarations
        .iter()
        .filter(|(id, _)| BUILTIN_IDS.contains(&id.as_str()))
    {
        validate_overlay_shape(id, overlay, false)?;
        let merged = apply_overlay(
            &resolved[id.as_str()],
            overlay.as_object().expect("shape checked"),
        )?;
        resolved.insert(id.clone(), merged);
    }
    for (id, def) in declarations
        .iter()
        .filter(|(id, _)| !BUILTIN_IDS.contains(&id.as_str()))
    {
        validate_overlay_shape(id, def, true)?;
        let def = def.as_object().expect("shape checked");
        let merged = if let Some(parent) = def.get("extends") {
            let parent = parent.as_str().unwrap_or("");
            let Some(base) = resolved.get(parent) else {
                return Err(err(format!("unknown parent {parent:?} for profile {id:?}")));
            };
            apply_overlay(base, def)?
        } else {
            if !def.contains_key("template") || !def.contains_key("truncate_field") {
                return Err(err(format!(
                    "standalone profile {:?} is missing settings",
                    id
                )));
            }
            standalone_defaults(def)?
        };
        resolved.insert(id.clone(), merged);
    }
    let mut out = IndexMap::new();
    for (id, profile) in &resolved {
        validate_resolved(id, profile)?;
        out.insert(id.clone(), finalize(profile, config)?);
    }
    Ok(out)
}

/// Select a resolved profile: `override_id` (CLI `--profile`) or `config.profile`.
pub fn select_profile(config: &Value, override_id: Option<&str>) -> Result<(String, Profile)> {
    let mut profiles = resolve_profiles(config)?;
    let selected = match override_id {
        Some(id) => Value::String(id.to_string()),
        None => config
            .get("profile")
            .cloned()
            .unwrap_or_else(|| Value::String("business".into())),
    };
    let id = check_id(&selected)?.to_string();
    match profiles.shift_remove(&id) {
        Some(p) => Ok((id, p)),
        None => Err(err(format!("unknown profile {:?}", id))),
    }
}

// ---------------------------------------------------------------------------
// Structured output schema + prompt
// ---------------------------------------------------------------------------

/// Canonical strict JSON Schema for a profile (OpenAI/Anthropic strict-mode compatible).
pub fn metadata_schema(profile: &Profile) -> Value {
    let mut props = Map::new();
    for (id, spec) in &profile.fields {
        props.insert(
            id.clone(),
            json!({"description": spec.description, "type": "string"}),
        );
    }
    json!({
        "additionalProperties": false,
        "properties": props,
        "required": profile.fields.keys().collect::<Vec<_>>(),
        "title": "DocumentMetadata",
        "type": "object",
    })
}

/// System prompt (contract §5.1).
pub fn system_prompt(profile: &Profile, prompt_extension: &str) -> String {
    let mut parts: Vec<String> = vec!["Extract the requested fields from the document content. Due to OCR text detection, the text may be noisy and contain spelling and detection errors. Handle those as well as possible.".into()];
    if !profile.intro.is_empty() {
        parts.push(profile.intro.clone());
    }
    parts.push(
        profile
            .fields
            .iter()
            .map(|(n, s)| format!("{n}: {}", s.description))
            .collect::<Vec<_>>()
            .join("\n\n"),
    );
    parts.push("Return every requested field as a string. If a value is not found, return an empty string. Do not invent missing values or return undeclared fields.".into());
    let mut prompt = parts.join("\n\n");
    if !prompt_extension.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(prompt_extension);
    }
    prompt
}

/// Check a model result against the strict schema: every field a string,
/// nothing undeclared.
pub fn validate_metadata(profile: &Profile, value: &Value) -> Result<Map<String, Value>> {
    let Some(map) = value.as_object() else {
        return Err(Error::General("model output is not an object".into()));
    };
    if let Some(extra) = map.keys().find(|k| !profile.fields.contains_key(*k)) {
        return Err(Error::General(format!(
            "model returned undeclared field {:?}",
            extra
        )));
    }
    for id in profile.fields.keys() {
        match map.get(id) {
            Some(Value::String(_)) => {}
            Some(_) => {
                return Err(Error::General(format!("field {:?} must be a string", id)));
            }
            None => {
                return Err(Error::General(format!("missing field {:?}", id)));
            }
        }
    }
    Ok(map.clone())
}
