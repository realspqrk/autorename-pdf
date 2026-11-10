//! Config loading from files: YAML strictness, env/.env, schema, examples.

mod common;

use std::path::Path;

use autorename_core::config::{self, Level, load_with_env, validate_loaded};
use autorename_core::naming::render_filename;
use autorename_core::profiles::select_profile;
use common::{tempdir, write_file};
use serde_json::json;

fn write(dir: &Path, text: &str) -> std::path::PathBuf {
    write_file(&dir.join("config.yaml"), text)
}

fn no_env(_: &str) -> Option<String> {
    None
}

/// A complete 4.x config in the shape `config init` will write.
const FULL: &str = r#"
config_version: 4
privacy: cloud
ai:
  provider: openai
  model: gpt-6-luna
  api_key: "${OPENAI_API_KEY}"
  base_url: ""
  temperature: 0.0
  max_retries: 2
pdf:
  max_pages: 3
  ocr: auto
  vision: false
  text_quality_threshold: 0.3
  incoming_invoice: AP
  outgoing_invoice: AR
ocr:
  language: en
  threads: 4
company:
  names: [Example GmbH]
output:
  language: English
  date_format: "%Y%m%d"
enrich:
  enabled: true
harmonization:
  auto_learn: true
  decider: ai
prompt_extension: ""
profile: business
profiles:
  business:
    fields:
      invoice_id:
        description: "Copy the printed invoice number exactly. Empty if absent."
    template: "{document_date} {company_name} {document_type} {description} {invoice_id}"
"#;

#[test]
fn full_config_validates_and_renders() {
    let dir = tempdir();
    let p = write(&dir, FULL);
    let env = |n: &str| (n == "OPENAI_API_KEY").then(|| "sk-test-1234567890".to_string());
    let loaded = load_with_env(&p, &env).unwrap();
    let v = validate_loaded(&loaded, None);
    assert!(v.valid && v.issues.is_empty(), "{:?}", v.issues);
    let s = config::settings(&loaded.tree, None).unwrap();
    assert_eq!(s.ai.api_key, "sk-test-1234567890");
    assert_eq!(s.pdf.ocr, config::Toggle::Auto);
    let fields = json!({"document_date": "06.09.2026", "company_name": "ACME", "document_type": "AP", "description": "12,13", "invoice_id": "12345"});
    assert_eq!(
        render_filename(&s.profile, fields.as_object().unwrap(), &s.date_format),
        "20260906 ACME AP 12,13 12345.pdf"
    );
    let s = config::settings(&loaded.tree, Some("academic")).unwrap();
    let fields = json!({"document_date": "01.01.2026", "first_author_surname": "Smith", "journal_name": "", "title": "Original title"});
    assert_eq!(
        render_filename(&s.profile, fields.as_object().unwrap(), &s.date_format),
        "20260101 Smith Original title.pdf"
    );
}

#[test]
fn duplicate_yaml_keys_rejected_at_every_depth() {
    let dir = tempdir();
    for source in [
        "profile: business\nprofile: academic\n",
        "profiles:\n  business: {}\n  business: {}\n",
        "profiles:\n  business:\n    fields:\n      description: {description: first}\n      description: {description: second}\n",
        "profiles:\n  business:\n    fields:\n      description: {description: first, description: second}\n",
        "ai: {model: one, model: two}\n",
    ] {
        let p = write(&dir, source);
        let err = load_with_env(&p, &no_env).expect_err(source);
        assert_eq!(err.exit_code().code(), 3);
        assert!(err.to_string().contains("unreadable"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            source,
            "file untouched"
        );
    }
}

#[test]
fn load_errors_and_empty_file() {
    let dir = tempdir();
    assert!(
        load_with_env(&dir.join("nope.yaml"), &no_env)
            .unwrap_err()
            .to_string()
            .starts_with("Config file not found")
    );
    for src in [": invalid: yaml: {{{}}", "- a\n- b\n"] {
        let p = write(&dir, src);
        assert_eq!(
            load_with_env(&p, &no_env).unwrap_err().code(),
            "config_error",
            "{src:?}"
        );
    }
    // Empty file = all defaults; validation then asks for the API key.
    let p = write(&dir, "# nothing yet\n");
    let v = validate_loaded(&load_with_env(&p, &no_env).unwrap(), None);
    assert!(!v.valid);
    assert!(v.issues.iter().any(|i| i.field == "ai.api_key"));
}

#[test]
fn env_expansion_before_profile_interpolation() {
    let dir = tempdir();
    let src = "company: {names: [ACME]}\nprofiles:\n  business:\n    fields:\n      description: {description: '${PROFILE_CITY} {company}'}\n";
    let p = write(&dir, src);
    let env = |n: &str| (n == "PROFILE_CITY").then(|| "Wien".to_string());
    let loaded = load_with_env(&p, &env).unwrap();
    assert_eq!(
        select_profile(&loaded.tree, None).unwrap().1.fields["description"].description,
        "Wien \"ACME\""
    );
    // Unset var stays literal → placeholder parse fails, notice names the var.
    let loaded = load_with_env(&p, &no_env).unwrap();
    assert!(
        loaded
            .notices
            .iter()
            .any(|i| i.message.contains("${PROFILE_CITY}"))
    );
    assert!(select_profile(&loaded.tree, None).is_err());
}

#[test]
fn dotenv_fills_gaps_but_never_overrides_real_env() {
    let dir = tempdir();
    let p = write(
        &dir,
        "ai:\n  api_key: \"${ARP_TEST_KEY}\"\n  base_url: \"https://${ARP_TEST_HOST}/v1\"\ncompany:\n  names: [Test Corp]\n",
    );
    write_file(
        &dir.join(".env"),
        "ARP_TEST_KEY=from-dotenv\nARP_TEST_HOST=dotenv.example\n",
    );
    let dotenv = config::read_dotenv(&p);
    let env = |n: &str| {
        if n == "ARP_TEST_KEY" {
            Some("from-real-env".to_string())
        } else {
            dotenv.get(n).cloned()
        }
    };
    let loaded = load_with_env(&p, &env).unwrap();
    assert_eq!(loaded.tree["ai"]["api_key"], "from-real-env");
    assert_eq!(loaded.tree["ai"]["base_url"], "https://dotenv.example/v1");
    let v = validate_loaded(&load_with_env(&p, &no_env).unwrap(), None);
    assert!(
        v.issues
            .iter()
            .any(|i| i.field == "ai.api_key" && i.level == Level::Error)
    );
}

#[test]
fn strict_schema_in_files() {
    let dir = tempdir();
    // YAML 1.2: `yes`/`off` are strings → plain type errors; unknown keys are errors.
    let p = write(
        &dir,
        "ai: {provider: ollama}\ncompany: {names: [X]}\npdf:\n  ocr: yes\n  vision: off\npaddleocr:\n  language: de\n",
    );
    let v = validate_loaded(&load_with_env(&p, &no_env).unwrap(), None);
    let errors: Vec<&str> = v
        .issues
        .iter()
        .filter(|i| i.level == Level::Error)
        .map(|i| i.field.as_str())
        .collect();
    assert_eq!(errors, ["pdf.ocr", "pdf.vision", "paddleocr"]);
    let p = write(&dir, "config_version: 2\nai: {provider: ollama}\n");
    let v = validate_loaded(&load_with_env(&p, &no_env).unwrap(), None);
    assert_eq!(v.issues[0].field, "config_version");
}

#[test]
fn redacted_tree_for_show() {
    let dir = tempdir();
    let p = write(
        &dir,
        "ai: {api_key: sk-1234567890abcdef}\ncompany: {names: [X]}\n",
    );
    let loaded = load_with_env(&p, &no_env).unwrap();
    assert_eq!(config::redact(&loaded.tree)["ai"]["api_key"], "sk-1...cdef");
    assert_eq!(loaded.tree["ai"]["api_key"], "sk-1234567890abcdef");
}
