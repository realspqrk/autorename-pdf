#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use autorename_core::config::deep_merge;
use autorename_core::naming::render_filename;
use autorename_core::profiles::select_profile;
use serde_json::{Value, json};

/// Port of tests/conftest.py `sample_config`.
pub fn sample_config() -> Value {
    json!({
        "ai": {"provider": "openai", "model": "gpt-6-luna", "api_key": "test-key-123", "base_url": "", "temperature": 0.0, "max_retries": 2},
        "pdf": {"max_pages": 3, "ocr": false, "vision": false, "text_quality_threshold": 0.3, "outgoing_invoice": "AR", "incoming_invoice": "AP"},
        "company": {"names": ["Test Company"]},
        "output": {"language": "English", "date_format": "%Y%m%d"},
        "prompt_extension": ""
    })
}

pub fn with(base: &Value, patch: Value) -> Value {
    deep_merge(base, &patch)
}

/// Port of test_profile_filenames.py `rendered()`.
pub fn rendered(
    config: &Value,
    selected: &str,
    values: Value,
    overlay: Option<Value>,
    date_format: Option<&str>,
) -> String {
    let mut config = config.clone();
    if let Some(overlay) = overlay {
        config["profiles"] = json!({ selected: overlay });
    }
    if let Some(fmt) = date_format {
        config["output"]["date_format"] = json!(fmt);
    }
    let (_id, profile) = select_profile(&config, Some(selected))
        .unwrap_or_else(|e| panic!("select {selected}: {e}"));
    let fields = values.as_object().cloned().unwrap_or_default();
    let before = fields.clone();
    let fmt = config["output"]["date_format"]
        .as_str()
        .unwrap_or("%Y%m%d")
        .to_string();
    let name = render_filename(&profile, &fields, &fmt);
    assert_eq!(fields, before);
    name
}

pub fn tempdir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!(
        "arp-it-{}-{}-{:?}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn write_file(path: &std::path::Path, text: &str) -> PathBuf {
    std::fs::write(path, text).unwrap();
    path.to_path_buf()
}
