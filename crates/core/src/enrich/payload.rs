//! `autorename-pdf.json` attachment, schema version 1.
//!
//! Holds extraction results only: no secrets, prompts or file-system paths.
//! `content_sha256` hashes [`Content`] (everything except timestamp, generator
//! version and original-file identity) and drives re-run idempotence.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::EnrichData;
use crate::journal::sha256_hex;

pub const SCHEMA_VERSION: u32 = 1;
pub const GENERATOR: &str = "autorename-pdf";
pub const ATTACHMENT_NAME: &str = "autorename-pdf.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extraction {
    /// `text` | `ocr` | `vision`
    pub tier: String,
    pub provider: String,
    pub model: String,
    pub pages_used: usize,
    /// True when `max_pages` cut the document.
    pub partial: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageText {
    /// 1-based page number.
    pub page: usize,
    pub text: String,
}

/// The semantic part of the attachment (hashed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Content {
    pub profile: String,
    pub title: String,
    pub subject: String,
    pub keywords: Vec<String>,
    pub fields: Map<String, Value>,
    pub extraction: Extraction,
    pub pages: Vec<PageText>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Original {
    pub sha256: String,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub schema_version: u32,
    pub generator: String,
    pub generator_version: String,
    /// RFC 3339 UTC.
    pub timestamp: String,
    pub content_sha256: String,
    /// The document before AutoRename-PDF first touched it (carried across
    /// re-runs while still a verifiable prefix).
    pub original: Original,
    #[serde(flatten)]
    pub content: Content,
}

impl Content {
    pub fn from_data(d: &EnrichData) -> Self {
        Content {
            profile: d.profile.clone(),
            title: d.title.clone(),
            subject: d.subject.clone(),
            keywords: d.keywords.clone(),
            fields: d.fields.clone(),
            extraction: Extraction {
                tier: d.tier.clone(),
                provider: d.provider.clone(),
                model: d.model.clone(),
                pages_used: d.pages.len(),
                partial: d.partial,
            },
            pages: d
                .pages
                .iter()
                .enumerate()
                .map(|(i, t)| PageText {
                    page: i + 1,
                    text: t.clone(),
                })
                .collect(),
        }
    }

    pub fn sha256(&self) -> String {
        // Struct field order is fixed; `fields` keeps insertion order.
        sha256_hex(&serde_json::to_vec(self).unwrap_or_default())
    }
}

impl Attachment {
    /// Parse bytes as our attachment; `None` for anything else (foreign file
    /// with the same name, other schema versions).
    pub fn parse(bytes: &[u8]) -> Option<Attachment> {
        let a: Attachment = serde_json::from_slice(bytes).ok()?;
        (a.generator == GENERATOR && a.schema_version == SCHEMA_VERSION).then_some(a)
    }

    pub fn to_json(&self) -> Vec<u8> {
        let mut v = serde_json::to_vec_pretty(self).unwrap_or_default();
        v.push(b'\n');
        v
    }
}
