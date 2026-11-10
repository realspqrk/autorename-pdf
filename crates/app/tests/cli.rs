//! End-to-end CLI tests against a local mock provider (std `TcpListener`,
//! `Connection: close`, same approach as `crates/core/tests/providers.rs`;
//! reqwest::blocking must not run inside tokio, so no wiremock).
//!
//! Every test runs the binary with its own config, data and config dirs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use assert_cmd::Command;
use serde_json::Value;

const ACME_REPLY: &str = r#"{"model":"gemma4","created_at":"2026-11-07T10:00:00Z","message":{"role":"assistant","content":"{\"company_name\":\"ACME Works\",\"issuer\":\"ACME Works\",\"recipient\":\"Test Company GmbH\",\"document_date\":\"15.03.2026\",\"document_type\":\"AP\",\"total_amount\":\"1,234.56\",\"currency\":\"\",\"description\":\"1,234.56\"}"},"done":true,"done_reason":"stop","prompt_eval_count":760,"eval_count":48}"#;
const OPENAI_401: &str = r#"{"error":{"message":"Incorrect API key provided: sk-bad.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#;

type Responder = Arc<dyn Fn(&str) -> (u16, String) + Send + Sync>;

/// Serve forever; `respond(path)` → (status, body).
fn mock(respond: Responder) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let respond = respond.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() {
                        return;
                    }
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':')
                        && k.eq_ignore_ascii_case("content-length")
                    {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; len];
                let _ = reader.read_exact(&mut body);
                let (status, text) = respond(&path);
                let out = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                let mut stream = reader.into_inner();
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            });
        }
    });
    url
}

fn acme_mock() -> String {
    mock(Arc::new(|_| (200, ACME_REPLY.to_string())))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// A path as suggested commands print it: shell-quoted like
/// `out::shell_quote` (Windows paths contain `\\`, so they are quoted).
fn q(p: &Path) -> String {
    let s = p.display().to_string();
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:=,@+%".contains(c));
    if safe {
        s
    } else if cfg!(windows) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

struct Env {
    root: tempfile::TempDir,
}

impl Env {
    fn new(config: &str) -> Env {
        let root = tempfile::tempdir().unwrap();
        for d in ["cfg", "data", "xdgcfg", "docs", "models"] {
            std::fs::create_dir_all(root.path().join(d)).unwrap();
        }
        std::fs::write(root.path().join("cfg/config.yaml"), config).unwrap();
        Env { root }
    }

    fn ollama(url: &str) -> Env {
        Env::new(&format!(
            "config_version: 4\nai:\n  provider: ollama\n  model: gemma4\n  base_url: \"{url}\"\n  max_retries: 0\ncompany:\n  names: [Test Company]\n"
        ))
    }

    /// `rel` uses `/`; joined per component so Windows gets `\` like the
    /// paths the app prints.
    fn path(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.root.path().to_path_buf(), |p, c| p.join(c))
    }

    fn docs(&self) -> PathBuf {
        self.path("docs")
    }

    fn add_doc(&self, fixture_name: &str) -> PathBuf {
        let dst = self.docs().join(fixture_name);
        std::fs::copy(fixture(fixture_name), &dst).unwrap();
        dst
    }

    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("autorename-pdf").unwrap();
        c.env_remove("AUTORENAME_PDF_CONFIG")
            .env("XDG_DATA_HOME", self.path("data"))
            .env("AUTORENAME_PDF_DATA_DIR", self.path("data/autorename-pdf"))
            .env("XDG_CONFIG_HOME", self.path("xdgcfg"))
            .env("HOME", self.root.path())
            .env("AUTORENAME_MODELS_DIR", self.path("models"))
            .env("NO_COLOR", "1")
            .arg("--config")
            .arg(self.path("cfg/config.yaml"));
        c
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let out = self.cmd().args(args).output().unwrap();
        let code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let v = serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!(
                "stdout is not JSON ({e}), exit {code}\nstdout:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (code, v)
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.') && n.ends_with(".pdf"))
        .collect();
    v.sort();
    v
}

#[test]
fn dry_run_json_shape() {
    let env = Env::ollama(&acme_mock());
    env.add_doc("text_invoice_acme.pdf");
    env.add_doc("text_letter_globex.pdf");
    let before = names_in(&env.docs());
    let docs = env.docs();
    let (code, plan) = env.json(&["rename", "--dry-run", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 0, "{plan:#}");
    assert_eq!(plan["schema_version"], 4);
    assert_eq!(plan["provider"], "ollama");
    assert_eq!(plan["summary"]["total"], 2);
    let items = plan["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Same mock reply for both files: the second gets a collision suffix.
    let names: Vec<&str> = items
        .iter()
        .map(|i| i["proposed_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "20260315 ACME Works AP 1234.56.pdf",
            "20260315 ACME Works AP 1234.56_(1).pdf"
        ]
    );
    for i in items {
        assert_eq!(i["status"], "rename");
        assert_eq!(i["company"], "ACME Works");
        assert_eq!(i["date"], "2026-03-15");
        assert_eq!(i["tier"], "text");
        assert_eq!(i["fields"]["document_type"], "AP");
        assert_eq!(i["source_sha256"].as_str().unwrap().len(), 64);
        assert!(i["error"].is_null());
        assert!(i["enrich"]["action"].is_string());
    }
    assert_eq!(
        items[0]["learn"],
        serde_json::json!({"action": "new_entry", "name": "ACME Works", "review": false})
    );
    let m = &items[0]["company_match"];
    assert_eq!(
        (
            m["extracted"].as_str(),
            m["outcome"].as_str(),
            m["decided_by"].as_str()
        ),
        (Some("ACME Works"), Some("new"), Some("none"))
    );
    assert_eq!(m["normalized"], "acme works");
    assert_eq!(names_in(&env.docs()), before, "dry run changes nothing");
    assert!(!env.path("cfg/name-lists/company_name.yaml").exists());
}

#[test]
fn plan_apply_undo_round_trip_is_byte_identical() {
    let env = Env::ollama(&acme_mock());
    let a = env.add_doc("text_invoice_acme.pdf");
    let b = env.add_doc("text_outgoing_invoice_wayne.pdf");
    let originals = [std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()];
    let plan_file = env.path("plan.json");
    let docs = env.docs();
    let (code, _) = env.json(&[
        "rename",
        "--plan",
        plan_file.to_str().unwrap(),
        "-o",
        "json",
        docs.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert!(plan_file.is_file());
    assert!(a.exists() && b.exists(), "--plan changes nothing");

    let (code, report) = env.json(&["apply", plan_file.to_str().unwrap(), "-o", "json"]);
    assert_eq!(code, 0, "{report:#}");
    assert_eq!(report["schema_version"], 4);
    assert!(report["batch_id"].is_string());
    let files = report["files"].as_array().unwrap();
    assert_eq!(report["summary"]["renamed"], 2);
    for f in files {
        assert_eq!(f["rename_status"], "renamed");
        assert_eq!(f["enrich_status"], "enriched", "{f:#}");
    }
    assert_eq!(files[0]["learn_status"], "entry_added");
    assert_eq!(files[1]["learn_status"], "known");
    assert!(!a.exists() && !b.exists());
    let companies = std::fs::read_to_string(env.path("cfg/name-lists/company_name.yaml")).unwrap();
    assert!(companies.contains("\"ACME Works\""), "{companies}");

    // Enrichment is visible to `inspect`.
    let target = files[0]["target"].as_str().unwrap();
    let (code, inspected) = env.json(&["inspect", "-o", "json", target]);
    assert_eq!(code, 0);
    let embedded = &inspected["files"][0]["embedded"];
    assert_eq!(
        embedded["attachment"]["fields"]["company_name"],
        "ACME Works"
    );

    // Applying the same plan again fails per file (sources are gone).
    let (code, again) = env.json(&["apply", plan_file.to_str().unwrap(), "-o", "json"]);
    assert_eq!(code, 1, "{again:#}");

    let (code, undo) = env.json(&["undo", "-o", "json"]);
    assert_eq!(code, 0, "{undo:#}");
    assert_eq!(undo["restored"], 2);
    assert_eq!(
        std::fs::read(&a).unwrap(),
        originals[0],
        "byte-identical after undo"
    );
    assert_eq!(
        std::fs::read(&b).unwrap(),
        originals[1],
        "byte-identical after undo"
    );
    let companies = std::fs::read_to_string(env.path("cfg/name-lists/company_name.yaml")).unwrap();
    assert!(
        !companies.contains("ACME Works"),
        "learned name reverted: {companies}"
    );

    let (code, list) = env.json(&["undo", "--list", "-o", "json"]);
    assert_eq!(code, 0);
    assert!(
        list["batches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["undone"] == true)
    );
}

#[test]
fn rename_text_output_and_ndjson_events() {
    let env = Env::ollama(&acme_mock());
    env.add_doc("text_invoice_acme.pdf");
    let docs = env.docs();
    let out = env
        .cmd()
        .args(["rename", "--dry-run", "-o", "text", docs.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("text_invoice_acme.pdf  →  20260315 ACME Works AP 1234.56.pdf"),
        "{text}"
    );
    assert!(text.contains("Dry run: 1 to rename"), "{text}");

    let out = env
        .cmd()
        .args([
            "rename",
            "--dry-run",
            "-o",
            "ndjson",
            docs.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let lines: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let events: Vec<&str> = lines.iter().filter_map(|l| l["event"].as_str()).collect();
    assert_eq!(events, ["plan_started", "file_started", "file_analyzed"]);
    assert_eq!(lines.last().unwrap()["schema_version"], 4);
}

#[test]
fn no_files_exits_4() {
    let env = Env::ollama(&acme_mock());
    let docs = env.docs();
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 4);
    assert_eq!(v["schema_version"], 4);
    assert_eq!(v["error"]["code"], "no_files");
    assert!(v["error"]["hint"].is_string());
    assert_eq!(v["error"]["retryable"], false);
    // A missing explicit path is named (usage error), not "no PDFs".
    let missing = docs.join("does-not-exist");
    let (code, v) = env.json(&["rename", "-o", "json", missing.to_str().unwrap()]);
    assert_eq!(code, 2, "{v:#}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does-not-exist")
    );
}

#[test]
fn only_rename_needs_provider_credentials() {
    let env = Env::ollama(&acme_mock());
    env.add_doc("text_invoice_acme.pdf");
    let plan_file = env.path("plan.json");
    let docs = env.docs();
    let plan = plan_file.to_str().unwrap();
    let (code, _) = env.json(&[
        "rename",
        "--plan",
        plan,
        "-o",
        "json",
        docs.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    // Switch to a cloud provider without an API key.
    std::fs::write(
        env.path("cfg/config.yaml"),
        "config_version: 4\nai:\n  provider: openai\ncompany:\n  names: [Test Company]\n",
    )
    .unwrap();
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(
        (code, v["error"]["code"].as_str()),
        (3, Some("config_error"))
    );
    let (code, v) = env.json(&["apply", plan, "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    let target = v["files"][0]["target"].as_str().unwrap().to_string();
    for args in [
        vec!["inspect", "-o", "json", target.as_str()],
        vec!["names", "list", "-o", "json"],
        vec!["undo", "--list", "-o", "json"],
        vec!["undo", "-o", "json"],
    ] {
        let (code, v) = env.json(&args);
        assert_eq!(code, 0, "{args:?}: {v:#}");
    }
}

#[test]
fn bad_config_exits_3() {
    let env = Env::new("config_version: 4\npdf:\n  ocrr: true\n");
    env.add_doc("text_invoice_acme.pdf");
    let docs = env.docs();
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 3);
    assert_eq!(v["error"]["code"], "config_error");
    let (code, v) = env.json(&["config", "validate", "-o", "json"]);
    assert_eq!(code, 3);
    assert_eq!(v["valid"], false);
}

#[test]
fn auth_failure_on_every_file_exits_11() {
    let url = mock(Arc::new(|_| (401, OPENAI_401.to_string())));
    let env = Env::new(&format!(
        "config_version: 4\nai:\n  provider: openai\n  api_key: sk-bad\n  base_url: \"{url}/v1\"\n  max_retries: 0\ncompany:\n  names: [Test Company]\n"
    ));
    env.add_doc("text_invoice_acme.pdf");
    env.add_doc("text_letter_globex.pdf");
    let docs = env.docs();
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 11, "{v:#}");
    for f in v["files"].as_array().unwrap() {
        assert_eq!(f["rename_status"], "failed");
        assert_eq!(f["error"]["code"], "auth_error");
    }
    assert!(v["batch_id"].is_null(), "nothing changed, no batch");
}

#[test]
fn offline_mode_rejects_cloud_provider() {
    let env = Env::new(
        "config_version: 4\nai:\n  provider: openai\n  api_key: sk-x\ncompany:\n  names: [T]\n",
    );
    env.add_doc("text_invoice_acme.pdf");
    let docs = env.docs();
    let (code, v) = env.json(&[
        "--offline",
        "rename",
        "--provider",
        "openai",
        "--dry-run",
        "-o",
        "json",
        docs.to_str().unwrap(),
    ]);
    assert_eq!(code, 3, "{v:#}");
    assert!(v["error"]["message"].as_str().unwrap().contains("offline"));

    let env = Env::new(
        "config_version: 4\nprivacy: offline\nai:\n  provider: ollama\n  base_url: \"https://ollama.example.com\"\n",
    );
    env.add_doc("text_invoice_acme.pdf");
    let docs = env.docs();
    let (code, v) = env.json(&["rename", "--dry-run", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 3, "{v:#}");
}

#[test]
fn usage_errors_are_json_when_requested() {
    let env = Env::ollama("http://127.0.0.1:9");
    let (code, v) = env.json(&["rename", "-o", "json", "--ocr=sometimes", "x.pdf"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage_error");
    let (code, v) = env.json(&["gui", "-o", "json"]);
    assert_eq!(code, 2);
    assert!(v["error"]["message"].as_str().unwrap().contains("GUI"));
    let (code, v) = env.json(&["apply", "-o", "json", "/nonexistent/plan.json"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage_error");
}

#[test]
fn config_init_path_and_schema() {
    let env = Env::ollama("http://127.0.0.1:9");
    std::fs::remove_file(env.path("cfg/config.yaml")).unwrap();
    let (code, v) = env.json(&["config", "init", "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    assert!(env.path("cfg/config.yaml").is_file());
    let (code, v) = env.json(&["config", "init", "-o", "json"]);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "conflict");
    let (code, _) = env.json(&["config", "init", "--force", "-o", "json"]);
    assert_eq!(code, 0);

    let (code, v) = env.json(&["config", "path", "-o", "json"]);
    assert_eq!(code, 0);
    for k in ["config", "env_file", "name_lists", "journal", "models"] {
        assert!(v[k]["path"].is_string(), "{k}: {v:#}");
    }
    assert!(
        v["journal"]["path"]
            .as_str()
            .unwrap()
            .starts_with(env.path("data").to_str().unwrap())
    );

    let (code, v) = env.json(&["schema"]);
    assert_eq!(code, 0);
    for k in ["result", "plan", "config", "error", "event", "inspect"] {
        assert!(v[k].is_object(), "{k}");
    }
    assert!(v["plan"]["properties"]["items"].is_object());
    assert!(v["config"]["properties"]["ai"]["properties"]["provider"]["enum"].is_array());
    assert!(
        v["config"]["properties"]["ocr"]["properties"]
            .get("det_limit_side_len")
            .is_none()
    );
}

#[test]
fn names_add_merge_list() {
    let env = Env::ollama("http://127.0.0.1:9");
    let (code, _) = env.json(&[
        "names",
        "add",
        "ACME",
        "--alias",
        "Acme Industries",
        "-o",
        "json",
    ]);
    assert_eq!(code, 0);
    let (code, _) = env.json(&["names", "add", "Acme Holding", "-o", "json"]);
    assert_eq!(code, 0);
    let (code, v) = env.json(&["names", "merge", "Acme Holding", "ACME", "-o", "json"]);
    assert_eq!(code, 0);
    assert_eq!(v["entries"][0]["canonical"], "ACME");
    let (_, v) = env.json(&["names", "list", "-o", "json"]);
    assert_eq!(v["field"], "company_name", "business default");
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["entries"][0]["aliases"],
        serde_json::json!(["Acme Industries", "Acme Holding"])
    );
    assert!(env.path("cfg/name-lists/company_name.yaml").is_file());
    // --field (before or after the verb) picks another, separate list.
    let (code, v) = env.json(&["names", "add", "Nature Physics", "--field", "journal_name"]);
    assert_eq!(
        (code, v["field"].as_str()),
        (0, Some("journal_name")),
        "{v:#}"
    );
    let (_, v) = env.json(&["names", "--field", "journal_name", "list", "-o", "json"]);
    assert_eq!(v["entries"][0]["canonical"], "Nature Physics");
    assert!(env.path("cfg/name-lists/journal_name.yaml").is_file());
    let (_, v) = env.json(&["names", "list", "-o", "json"]);
    assert_eq!(v["entries"].as_array().unwrap().len(), 1, "{v:#}");
    let (code, v) = env.json(&["names", "--field", "../x", "list", "-o", "json"]);
    assert_eq!(
        (code, v["error"]["code"].as_str()),
        (2, Some("usage_error"))
    );
    // Ids that could share a file (device names, case) are refused.
    for bad in ["con", "_con", "Company_Name"] {
        let (code, _) = env.json(&["names", "--field", bad, "list", "-o", "json"]);
        assert_eq!(code, 2, "{bad}");
    }
    // Decomposed input (U+0308) is stored as NFC and reported, not a crash.
    let (code, v) = env.json(&["names", "add", "Mu\u{308}ller GmbH", "-o", "json"]);
    assert_eq!(
        (code, v["entries"][0]["canonical"].as_str()),
        (0, Some("Müller GmbH")),
        "{v:#}"
    );
    let (code, v) = env.json(&[
        "names",
        "rename",
        "Müller GmbH",
        "Mo\u{308}bius GmbH",
        "-o",
        "json",
    ]);
    assert_eq!(
        (code, v["entries"][0]["canonical"].as_str()),
        (0, Some("Möbius GmbH")),
        "{v:#}"
    );
    // Adding an existing spelling reports that entry, not the last one.
    let (code, v) = env.json(&["names", "add", "Mo\u{308}bius GmbH", "-o", "json"]);
    assert_eq!(
        (code, v["entries"][0]["canonical"].as_str()),
        (0, Some("Möbius GmbH")),
        "{v:#}"
    );
}

#[test]
fn names_without_a_default_list_needs_field() {
    let env = Env::new(
        "config_version: 4\nai:\n  provider: ollama\n  model: gemma4\nprofile: academic\n",
    );
    let (code, v) = env.json(&["names", "list", "-o", "json"]);
    assert_eq!(code, 2, "{v:#}");
    assert_eq!(v["error"]["code"], "usage_error");
    let text = format!("{} {}", v["error"]["message"], v["error"]["hint"]);
    assert!(text.contains("--field"), "{text}");
    let (code, v) = env.json(&["names", "--field", "journal_name", "list", "-o", "json"]);
    assert_eq!(
        (code, v["field"].as_str()),
        (0, Some("journal_name")),
        "{v:#}"
    );
}

fn help(args: &[&str]) -> String {
    let out = Command::cargo_bin("autorename-pdf")
        .unwrap()
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn help_snapshots() {
    insta::assert_snapshot!("top_level_help", help(&["--help"]));
    insta::assert_snapshot!("rename_help", help(&["rename", "--help"]));
}

/// `--help` is pre-wrapped (clap does not wrap): no line over 80 columns.
#[test]
fn long_help_fits_80_columns() {
    let commands: &[&[&str]] = &[
        &[],
        &["setup"],
        &["rename"],
        &["apply"],
        &["undo"],
        &["inspect"],
        &["names"],
        &["names", "list"],
        &["names", "review"],
        &["names", "rename"],
        &["names", "merge"],
        &["names", "add"],
        &["config"],
        &["config", "show"],
        &["config", "validate"],
        &["config", "path"],
        &["config", "init"],
        &["models"],
        &["models", "list"],
        &["models", "fetch"],
        &["models", "import"],
        &["models", "remove"],
        &["schema"],
        &["integrate"],
        &["integrate", "install"],
        &["integrate", "remove"],
        &["integrate", "status"],
        &["gui"],
    ];
    let mut long = Vec::new();
    for cmd in commands {
        let mut args = cmd.to_vec();
        args.push("--help");
        for line in help(&args).lines() {
            if line.chars().count() > 80 {
                long.push(format!("{args:?}: {line}"));
            }
        }
    }
    assert!(
        long.is_empty(),
        "help lines over 80 columns:\n{}",
        long.join("\n")
    );
}

#[test]
fn every_command_has_long_help() {
    for args in [
        &["rename", "--help"][..],
        &["apply", "--help"],
        &["undo", "--help"],
        &["inspect", "--help"],
        &["names", "--help"],
        &["config", "--help"],
        &["models", "--help"],
        &["schema", "--help"],
    ] {
        let text = help(args);
        assert!(
            text.lines().count() > 12,
            "{args:?} help too short:\n{text}"
        );
    }
    for args in [
        &["rename", "--help"][..],
        &["apply", "--help"],
        &["undo", "--help"],
    ] {
        let text = help(args);
        assert!(
            text.contains("EXAMPLES:") && text.contains("NETWORK:"),
            "{args:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Review round 2 regressions
// ---------------------------------------------------------------------------

/// A real plan for one ACME doc; returns (plan path, plan JSON, source).
fn make_plan(env: &Env) -> (PathBuf, Value, PathBuf) {
    let src = env.add_doc("text_invoice_acme.pdf");
    let plan_file = env.path("plan.json");
    let docs = env.docs();
    let (code, _) = env.json(&[
        "rename",
        "--plan",
        plan_file.to_str().unwrap(),
        "-o",
        "json",
        docs.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let plan: Value = serde_json::from_str(&std::fs::read_to_string(&plan_file).unwrap()).unwrap();
    (plan_file, plan, src)
}

#[test]
fn tampered_plan_is_rejected_before_any_change() {
    let env = Env::ollama(&acme_mock());
    let (plan_file, plan, src) = make_plan(&env);
    let outside = env.path("arbitrary.conf");
    for name in ["../escape.pdf", "x.conf", "sub/x.pdf", "/abs/x.pdf"] {
        let mut p = plan.clone();
        p["items"][0]["proposed_name"] = serde_json::json!(name);
        std::fs::write(&plan_file, serde_json::to_string(&p).unwrap()).unwrap();
        let (code, v) = env.json(&["apply", plan_file.to_str().unwrap(), "-o", "json"]);
        assert_eq!(code, 2, "{name}: {v:#}");
        assert_eq!(v["error"]["code"], "usage_error");
        let hint = v["error"]["hint"].as_str().unwrap();
        assert!(hint.contains("items[0].proposed_name"), "{hint}");
        assert!(hint.contains(&plan_file.display().to_string()), "{hint}");
        assert!(
            hint.contains(&format!(
                "autorename-pdf --config {} apply {}",
                q(&env.path("cfg/config.yaml")),
                q(&plan_file)
            )),
            "{hint}"
        );
        assert!(!hint.contains("rename --plan"), "no re-analysis: {hint}");
        assert!(src.exists());
    }
    // A tampered source names the source and suggests re-creating the plan.
    let mut p = plan.clone();
    p["items"][0]["source"] = serde_json::json!("relative/doc.pdf");
    std::fs::write(&plan_file, serde_json::to_string(&p).unwrap()).unwrap();
    let (code, v) = env.json(&["apply", plan_file.to_str().unwrap(), "-o", "json"]);
    assert_eq!(code, 2, "{v:#}");
    let hint = v["error"]["hint"].as_str().unwrap();
    assert!(
        hint.contains("items[0].source") && hint.contains("rename --plan"),
        "{hint}"
    );
    assert!(!hint.contains("proposed_name"), "{hint}");
    assert!(!env.path("data/autorename-pdf/journal.json").exists());
    // `target` is informational: a tampered target is ignored, the file is
    // renamed to its proposed_name inside its own folder.
    let mut p = plan.clone();
    p["items"][0]["target"] = serde_json::json!(outside);
    std::fs::write(&plan_file, serde_json::to_string(&p).unwrap()).unwrap();
    let (code, v) = env.json(&["apply", plan_file.to_str().unwrap(), "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    assert!(!outside.exists());
    let name = plan["items"][0]["proposed_name"].as_str().unwrap();
    assert!(env.docs().join(name).exists());
}

#[test]
fn editing_only_proposed_name_is_enough() {
    let env = Env::ollama(&acme_mock());
    let (plan_file, plan, src) = make_plan(&env);
    let mut p = plan.clone();
    p["items"][0]["proposed_name"] = serde_json::json!("My own name.pdf");
    std::fs::write(&plan_file, serde_json::to_string(&p).unwrap()).unwrap();
    let (code, v) = env.json(&["apply", plan_file.to_str().unwrap(), "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    assert!(!src.exists() && env.docs().join("My own name.pdf").exists());
}

#[test]
fn rerunning_on_renamed_duplicates_changes_nothing() {
    let env = Env::ollama(&acme_mock());
    let a = env.add_doc("text_invoice_acme.pdf");
    std::fs::copy(&a, env.docs().join("copy.pdf")).unwrap();
    let docs = env.docs();
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 0, "{v:#}");
    let mut names = names_in(&docs);
    names.sort();
    assert_eq!(
        names,
        [
            "20260315 ACME Works AP 1234.56.pdf",
            "20260315 ACME Works AP 1234.56_(1).pdf"
        ]
    );
    let hashes = |d: &Path| -> Vec<Vec<u8>> {
        let mut n = names_in(d);
        n.sort();
        n.iter()
            .map(|f| std::fs::read(d.join(f)).unwrap())
            .collect()
    };
    let before = hashes(&docs);
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(v["summary"]["renamed"], 0, "{v:#}");
    assert_eq!(v["summary"]["unchanged"], 2);
    for f in v["files"].as_array().unwrap() {
        assert_eq!(f["enrich_status"], "unchanged", "{f:#}");
    }
    assert_eq!(hashes(&docs), before, "bytes identical");
}

#[test]
fn unwritable_journal_stops_before_any_change() {
    let env = Env::ollama(&acme_mock());
    let (plan_file, _, src) = make_plan(&env);
    // The journal directory cannot be created: its parent is a file.
    let data = env.path("data");
    std::fs::remove_dir_all(&data).unwrap();
    std::fs::write(&data, b"not a dir").unwrap();
    let (code, v) = env.json(&["apply", plan_file.to_str().unwrap(), "-o", "json"]);
    assert_eq!(code, 1, "{v:#}");
    assert!(v["error"]["message"].is_string());
    assert!(src.exists(), "nothing renamed without an undo record");
}

#[test]
fn plan_output_never_clobbers_inputs_or_existing_files() {
    let env = Env::ollama(&acme_mock());
    let src = env.add_doc("text_invoice_acme.pdf");
    let original = std::fs::read(&src).unwrap();
    let s = src.to_str().unwrap();
    let (code, v) = env.json(&["rename", s, "--plan", s, "-o", "json"]);
    assert_eq!(
        (code, v["error"]["code"].as_str()),
        (2, Some("usage_error"))
    );
    assert_eq!(std::fs::read(&src).unwrap(), original);
    let existing = env.path("existing.json");
    std::fs::write(&existing, b"keep").unwrap();
    let e = existing.to_str().unwrap();
    let (code, _) = env.json(&["rename", s, "--plan", e, "-o", "json"]);
    assert_eq!(code, 2);
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep");
    let (code, _) = env.json(&["rename", s, "--plan", e, "--force", "-o", "json"]);
    assert_eq!(code, 0);
    assert_ne!(std::fs::read(&existing).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn dangling_symlink_counts_as_taken_when_planning() {
    let env = Env::ollama(&acme_mock());
    env.add_doc("text_invoice_acme.pdf");
    std::os::unix::fs::symlink(
        env.path("nowhere"),
        env.docs().join("20260315 ACME Works AP 1234.56.pdf"),
    )
    .unwrap();
    let docs = env.docs();
    let (code, plan) = env.json(&["rename", "--dry-run", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(
        plan["items"][0]["proposed_name"],
        "20260315 ACME Works AP 1234.56_(1).pdf"
    );
}

#[cfg(unix)]
#[test]
fn aliased_directories_share_reservations() {
    let env = Env::ollama(&acme_mock());
    let real = env.docs().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::copy(fixture("text_invoice_acme.pdf"), real.join("a.pdf")).unwrap();
    std::fs::copy(fixture("text_letter_globex.pdf"), real.join("b.pdf")).unwrap();
    let alias = env.docs().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let (a, b) = (real.join("a.pdf"), alias.join("b.pdf"));
    let (code, plan) = env.json(&[
        "rename",
        "--dry-run",
        "-o",
        "json",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let names: Vec<&str> = plan["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["proposed_name"].as_str().unwrap())
        .collect();
    assert_ne!(names[0], names[1], "same physical folder: distinct targets");
}

// APFS rejects non-UTF-8 file names (EILSEQ).
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn non_utf8_paths_survive_machine_output() {
    use std::os::unix::ffi::OsStrExt;
    let env = Env::ollama(&acme_mock());
    let name = std::ffi::OsStr::from_bytes(b"scan-\xff.pdf");
    let src = env.docs().join(name);
    std::fs::copy(fixture("text_invoice_acme.pdf"), &src).unwrap();
    let out = env
        .cmd()
        .arg("rename")
        .arg("--dry-run")
        .args(["-o", "json"])
        .arg(&src)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let bytes: Vec<u8> = v["items"][0]["source"]["unix_bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(bytes, src.as_os_str().as_bytes());
}

#[test]
fn ndjson_usage_errors_are_one_line() {
    let env = Env::ollama("http://127.0.0.1:9");
    let out = env
        .cmd()
        .args(["rename", "--ocr=invalid", "-o", "ndjson", "x.pdf"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    let v: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["error"]["code"], "usage_error");
}

#[test]
fn offline_refuses_model_downloads() {
    let env = Env::ollama("http://127.0.0.1:9");
    let (code, v) = env.json(&["--offline", "models", "fetch", "latin", "-o", "json"]);
    assert_eq!(code, 3, "{v:#}");
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("models import")
    );
}

/// Free-first route: a scan, no OCR models installed, default `ocr/vision:
/// auto` → `ocr_models_missing` warning, then vision via the (mock) model.
#[test]
fn scan_without_ocr_models_is_renamed_via_vision() {
    let env = Env::ollama(&acme_mock());
    let src = env.add_doc("image_invoice_springfield.pdf");
    let docs = env.docs();
    let (code, report) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 0, "{report:#}");
    let f = &report["files"][0];
    assert_eq!(f["rename_status"], "renamed", "{f:#}");
    assert_eq!(f["tier"], "vision_needed");
    let warned = f["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().starts_with("ocr_models_missing"));
    assert!(warned, "{f:#}");
    assert!(!src.exists());
}

/// The missing-models warning names a command the user can run as is: with
/// this run's `--config`, and offline (`privacy: offline`) an import from a
/// local copy instead of a refused download. Human preview marks pages
/// sent as images.
#[test]
fn missing_model_hint_is_scoped_and_privacy_aware() {
    let warning = |env: &Env| {
        let docs = env.docs();
        let args = ["rename", "--dry-run", "-o", "json", docs.to_str().unwrap()];
        let (code, plan) = env.json(&args);
        assert_eq!(code, 0, "{plan:#}");
        plan["items"][0]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap().to_string())
            .find(|w| w.starts_with("ocr_models_missing"))
            .unwrap_or_else(|| panic!("{plan:#}"))
    };
    let config = |env: &Env| q(&env.path("cfg/config.yaml"));
    let env = Env::ollama(&acme_mock());
    env.add_doc("image_invoice_springfield.pdf");
    let w = warning(&env);
    assert!(
        w.contains(&format!(
            "autorename-pdf --config {} models fetch latin",
            config(&env)
        )),
        "{w}"
    );
    let text = env
        .cmd()
        .args(["rename", "--dry-run", "-o", "text"])
        .arg(env.docs())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("page 1 sent to AI as an image"), "{text}");

    let env = Env::new(&format!(
        "config_version: 4\nprivacy: offline\nai:\n  provider: ollama\n  model: gemma4\n  base_url: \"{}\"\n  max_retries: 0\ncompany:\n  names: [Test Company]\n",
        acme_mock()
    ));
    env.add_doc("image_invoice_springfield.pdf");
    let w = warning(&env);
    assert!(
        w.contains(&format!(
            "autorename-pdf --config {} models import <dir>",
            config(&env)
        )),
        "{w}"
    );
    assert!(!w.contains("models fetch latin"), "{w}");
}

// APFS rejects non-UTF-8 file names (EILSEQ).
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn non_utf8_config_dir_gives_structured_documents() {
    use std::os::unix::ffi::OsStrExt;
    let env = Env::ollama("http://127.0.0.1:9");
    let dir = env
        .path("cfg")
        .join(std::ffi::OsStr::from_bytes(b"conf-\xff"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(env.path("cfg/config.yaml"), dir.join("config.yaml")).unwrap();
    for args in [["config", "path"], ["config", "show"], ["names", "list"]] {
        let out = Command::cargo_bin("autorename-pdf")
            .unwrap()
            .env_remove("AUTORENAME_PDF_CONFIG")
            .env("XDG_DATA_HOME", env.path("data"))
            .env("AUTORENAME_PDF_DATA_DIR", env.path("data/autorename-pdf"))
            .env("XDG_CONFIG_HOME", env.path("xdgcfg"))
            .env("HOME", env.path(""))
            .arg("--config")
            .arg(dir.join("config.yaml"))
            .args(args)
            .args(["-o", "json"])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(!v.to_string().contains("serialize_error"), "{args:?}: {v}");
        assert!(v.to_string().contains("unix_bytes"), "{args:?}: {v}");
    }
}

#[test]
fn ndjson_schema_and_missing_command_are_structured() {
    let env = Env::ollama("http://127.0.0.1:9");
    let out = env
        .cmd()
        .args(["schema", "event", "-o", "ndjson"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().count(), 1);
    let _: Value = serde_json::from_str(stdout.trim()).unwrap();
    for fmt in ["json", "ndjson"] {
        let out = Command::cargo_bin("autorename-pdf")
            .unwrap()
            .args(["--output", fmt])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["error"]["code"], "usage_error", "{fmt}");
    }
}

#[test]
fn review_entries_carry_evidence_and_commands() {
    let env = Env::new(&format!(
        "config_version: 4\nai:\n  provider: ollama\n  model: gemma4\n  base_url: \"{}\"\n  max_retries: 0\ncompany:\n  names: [Test Company]\nharmonization:\n  decider: \"off\"\n",
        acme_mock()
    ));
    std::fs::create_dir_all(env.path("cfg/name-lists")).unwrap();
    std::fs::write(
        env.path("cfg/name-lists/company_name.yaml"),
        "ACME Holding: {}\n",
    )
    .unwrap();
    // Empty state names the total.
    let out = env
        .cmd()
        .args(["names", "review", "-o", "text"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("No companies need review (1 total)"));
    env.add_doc("text_invoice_acme.pdf");
    let docs = env.docs();
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 0, "{v:#}");
    let (_, v) = env.json(&["names", "review", "-o", "json"]);
    let e = &v["entries"][0];
    assert_eq!(e["canonical"], "ACME Works", "{v:#}");
    let ev = &e["evidence"];
    assert_eq!(ev["candidates"][0]["name"], "ACME Holding");
    assert_eq!(ev["source"], "text_invoice_acme.pdf", "file name only");
    assert!(ev["batch"].as_str().unwrap().len() > 10);
    let cmds: Vec<&str> = e["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    // Suggested commands repeat the non-default --config, shell-quoted.
    let pre = format!(
        "autorename-pdf --config {} ",
        q(&env.path("cfg/config.yaml"))
    );
    assert!(
        cmds.contains(&format!("{pre}names merge 'ACME Works' 'ACME Holding'").as_str()),
        "{cmds:?}"
    );
    assert!(cmds.contains(&format!("{pre}names review --accept 'ACME Works'").as_str()));
    let file = std::fs::read_to_string(env.path("cfg/name-lists/company_name.yaml")).unwrap();
    assert!(
        file.contains("  review:\n    reason: "),
        "block style: {file}"
    );
    assert!(
        file.contains("    candidates:\n      - name: \"ACME Holding\"\n"),
        "{file}"
    );
    assert!(!file.contains(docs.to_str().unwrap()), "no absolute paths");
    // Accept → user-owned; undo keeps it and says so.
    let (code, _) = env.json(&["names", "review", "--accept", "ACME Works", "-o", "json"]);
    assert_eq!(code, 0);
    let (code, v) = env.json(&["undo", "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(v["aliases"][0]["note"], "kept: accepted by user", "{v:#}");
    let (_, v) = env.json(&["names", "list", "-o", "json"]);
    assert_eq!(v["entries"].as_array().unwrap().len(), 2);
    // Spelling fix via rename; merge into a missing target hints rename.
    let (code, v) = env.json(&["names", "merge", "ACME Works", "ACME", "-o", "json"]);
    assert_eq!(code, 2);
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("names --field company_name rename")
    );
    let (code, v) = env.json(&["names", "rename", "ACME Works", "ACME", "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(v["entries"][0]["canonical"], "ACME");
}

#[test]
fn undo_reports_entries_renamed_by_the_user_as_kept() {
    let env = Env::ollama(&acme_mock());
    let doc = env.add_doc("text_invoice_acme.pdf");
    let (code, v) = env.json(&["rename", "-o", "json", doc.to_str().unwrap()]);
    assert_eq!(code, 0, "{v:#}");
    let (code, v) = env.json(&["names", "rename", "ACME Works", "Acme Works", "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(v["entries"][0]["source"], "user");
    let (code, v) = env.json(&["undo", "-o", "json"]);
    assert_eq!(code, 0, "{v:#}");
    let a = &v["aliases"][0];
    assert_eq!(a["reverted"], false);
    assert!(
        a["note"]
            .as_str()
            .unwrap()
            .starts_with("kept: renamed by user"),
        "{v:#}"
    );
    let (_, v) = env.json(&["names", "list", "-o", "json"]);
    assert_eq!(v["entries"][0]["canonical"], "Acme Works");
}

#[test]
fn data_dir_env_isolates_journal_and_models() {
    let env = Env::ollama(&acme_mock());
    let data = env.path("isolated-state");
    let doc = env.add_doc("text_invoice_acme.pdf");
    let run = |args: &[&str]| {
        let out = env
            .cmd()
            .env("AUTORENAME_PDF_DATA_DIR", &data)
            .env_remove("AUTORENAME_MODELS_DIR")
            .args(args)
            .args(["-o", "json"])
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        (out.status.code().unwrap_or(-1), v)
    };
    let (code, v) = run(&["config", "path"]);
    assert_eq!(code, 0);
    let s = |k: &str| v[k]["path"].as_str().unwrap().to_string();
    assert_eq!(PathBuf::from(s("data")), data);
    assert!(PathBuf::from(s("journal")).starts_with(&data));
    assert!(PathBuf::from(s("models")).starts_with(&data));
    let (code, v) = run(&["rename", doc.to_str().unwrap()]);
    assert_eq!(code, 0, "{v:#}");
    assert!(data.join("journal.json").is_file());
    assert!(!env.path("data/autorename-pdf/journal.json").exists());
}

#[test]
fn own_company_is_never_the_counterparty_nor_learned() {
    // The model wrongly names my company as the counterparty and says AR.
    let reply = r#"{"model":"gemma4","message":{"role":"assistant","content":"{\"company_name\":\"Test Company GmbH\",\"issuer\":\"ACME Works\",\"recipient\":\"Test Company GmbH\",\"document_date\":\"15.03.2026\",\"document_type\":\"AR\",\"total_amount\":\"1,234.56\",\"currency\":\"EUR\",\"description\":\"\"}"},"done":true,"done_reason":"stop"}"#;
    let env = Env::ollama(&mock(Arc::new(move |_| (200, reply.to_string()))));
    env.add_doc("text_invoice_acme.pdf");
    let docs = env.docs();
    let (code, v) = env.json(&["rename", "-o", "json", docs.to_str().unwrap()]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(
        // The issuer as printed, minus its legal form.
        v["files"][0]["name"],
        "20260315 ACME Works AP EUR 1234.56.pdf",
        "{v:#}"
    );
    let names = std::fs::read_to_string(env.path("cfg/name-lists/company_name.yaml")).unwrap();
    assert!(!names.contains("Test Company"), "{names}");
}

#[test]
fn provider_override_uses_that_providers_key_variable() {
    let env = Env::new(
        "config_version: 4\nai:\n  provider: openai\n  api_key: \"${OPENAI_API_KEY}\"\ncompany:\n  names: [Test Company]\n",
    );
    std::fs::write(env.path("cfg/.env"), "OPENAI_API_KEY=sk-openai-test\n").unwrap();
    env.add_doc("text_invoice_acme.pdf");
    let out = env
        .cmd()
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .args(["rename", "--provider", "anthropic", "-o", "json"])
        .arg(env.docs())
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    // Never the OpenAI key sent to Anthropic: a config error naming the variable.
    assert_eq!(out.status.code(), Some(3), "{v:#}");
    let hint = v["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("ANTHROPIC_API_KEY"), "{hint}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("OPENAI_API_KEY"), "{stderr}");
}

#[test]
fn hints_repeat_the_effective_config() {
    let env = Env::ollama("http://127.0.0.1:9");
    let (code, v) = env.json(&["undo", "--batch", "nope", "-o", "json"]);
    assert_eq!(code, 2);
    let hint = v["error"]["hint"].as_str().unwrap();
    assert!(
        hint.contains(&format!(
            "autorename-pdf --config {} undo --list",
            q(&env.path("cfg/config.yaml"))
        )),
        "{hint}"
    );
}
