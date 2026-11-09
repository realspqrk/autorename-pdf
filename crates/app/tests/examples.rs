//! Every shipped config (`config.yaml.example`, `examples/*.yaml`, the
//! `config init` starter) must pass `config validate` without issues other
//! than the "no company name" warning, and `name-lists/company_name.yaml.example`
//! must load through `names list`. Keeps the docs from drifting.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn shipped_configs() -> Vec<PathBuf> {
    let mut v = vec![
        repo().join("config.yaml.example"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/starter-config.yaml"),
    ];
    let mut examples: Vec<PathBuf> = std::fs::read_dir(repo().join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    examples.sort();
    assert!(
        examples.len() >= 5,
        "examples/ lost its recipes: {examples:?}"
    );
    v.extend(examples);
    v
}

/// Runs the binary on a copy of `config` with the example company name list
/// beside it, isolated from the user's dirs, with dummy API keys.
fn run(config: &Path, args: &[&str]) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.yaml");
    std::fs::copy(config, &cfg).unwrap();
    std::fs::create_dir_all(dir.path().join("name-lists")).unwrap();
    std::fs::copy(
        repo().join("name-lists/company_name.yaml.example"),
        dir.path().join("name-lists/company_name.yaml"),
    )
    .unwrap();
    let out = Command::cargo_bin("autorename-pdf")
        .unwrap()
        .env_remove("AUTORENAME_PDF_CONFIG")
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env(
            "AUTORENAME_PDF_DATA_DIR",
            dir.path().join("data/autorename-pdf"),
        )
        .env("XDG_CONFIG_HOME", dir.path().join("xdgcfg"))
        .env("HOME", dir.path())
        .env("OPENAI_API_KEY", "sk-test-1234567890abcdef")
        .env("ANTHROPIC_API_KEY", "sk-ant-test-1234567890")
        .env("GEMINI_API_KEY", "test-gemini-key-1234567890")
        .env("XAI_API_KEY", "xai-test-1234567890")
        .arg("--config")
        .arg(&cfg)
        .args(["-o", "json"])
        .args(args)
        .output()
        .unwrap();
    let doc = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{}: no JSON ({e}): {}",
            config.display(),
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out.status.code().unwrap_or(-1), doc)
}

#[test]
fn every_shipped_config_validates() {
    for config in shipped_configs() {
        let (code, doc) = run(&config, &["config", "validate"]);
        assert_eq!(code, 0, "{}: {doc:#}", config.display());
        assert_eq!(doc["valid"], true, "{}: {doc:#}", config.display());
        let unexpected: Vec<&Value> = doc["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["field"] != "company.name")
            .collect();
        assert!(
            unexpected.is_empty(),
            "{}: {unexpected:#?}",
            config.display()
        );
    }
}

#[test]
fn company_names_example_loads() {
    let (code, doc) = run(&repo().join("config.yaml.example"), &["names", "list"]);
    assert_eq!(code, 0, "{doc:#}");
    let text = doc.to_string();
    for name in ["Alphabet", "Microsoft", "Amazon", "Apple"] {
        assert!(text.contains(name), "{name} missing: {doc:#}");
    }
}
