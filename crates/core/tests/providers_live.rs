//! Live smoke tests against real providers. Ignored by default and gated by
//! `AUTORENAME_LIVE=1`; keys come from the environment or the repo `.env`.
//!
//! ```text
//! AUTORENAME_LIVE=1 cargo test -p autorename-core --test providers_live -- --ignored --nocapture
//! ```

mod common;

use std::time::Duration;

use autorename_core::config::{Privacy, Provider, default_model};
use autorename_core::profiles::select_profile;
use autorename_core::providers::{ExtractRequest, ProviderSettings, RetryPolicy, extract};

/// Text of `tests/fixtures/text_invoice_acme.pdf` (abridged).
const ACME_TEXT: &str = "INVOICE\nInvoice No: INV-2024-3038228\nDate: 15.03.2024\n\
From: ACME Corporation GmbH\nIndustriestrasse 42, 8010 Graz, Austria\nVAT: ATU12345678\n\
To: Petermeir Digital Solutions e.U.\nHauptplatz 1, 8530 Bad Gleichenberg, Austria\n\
Web Development Services 40 EUR 85.00 EUR 3,400.00\nHosting & Maintenance 1 EUR 120.00\n\
SSL Certificate 1 EUR 29.00\nSubtotal EUR 3,549.00\nVAT 20% EUR 709.80\nTOTAL EUR 4,258.80\n\
Payment due: 14.04.2024";

fn key(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        return Some(v).filter(|v| !v.is_empty());
    }
    let env_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env");
    dotenvy::from_path_iter(env_path)
        .ok()?
        .flatten()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

fn smoke(provider: Provider, key_var: &str) {
    if std::env::var("AUTORENAME_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped: set AUTORENAME_LIVE=1");
        return;
    }
    let Some(api_key) = key(key_var) else {
        eprintln!("skipped: {key_var} not set");
        return;
    };
    let (_, profile) = select_profile(&common::sample_config(), None).unwrap();
    let req = ExtractRequest::document(&profile, "", ACME_TEXT, vec![]).unwrap();
    let cfg = ProviderSettings {
        provider,
        model: default_model(provider.id()).unwrap().to_string(),
        api_key,
        base_url: String::new(),
        temperature: 0.0,
        privacy: Privacy::Cloud,
        timeout: Duration::from_secs(60),
        retry: RetryPolicy::default(),
    };
    let resp = extract(&req, &cfg).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("{}", serde_json::to_string_pretty(&resp).unwrap());
    let company = resp.value["company_name"].as_str().unwrap().to_lowercase();
    assert!(company.contains("acme"), "{company}");
    assert!(
        resp.value["document_date"]
            .as_str()
            .unwrap()
            .contains("2024")
    );
}

#[test]
#[ignore = "live: AUTORENAME_LIVE=1, needs OPENAI_API_KEY"]
fn live_openai_text_extraction() {
    smoke(Provider::OpenAi, "OPENAI_API_KEY");
}

#[test]
#[ignore = "live: AUTORENAME_LIVE=1, needs ANTHROPIC_API_KEY"]
fn live_anthropic_text_extraction() {
    smoke(Provider::Anthropic, "ANTHROPIC_API_KEY");
}
