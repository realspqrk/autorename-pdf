//! `autorename-pdf setup` (idempotency, --check, secrets never echoed or
//! written to config.yaml, decider keys) and the demo README's offline
//! commands (doc test: documented commands keep working).

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const SECRET: &str = "sk-test-SECRET-0123456789abcdef";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Isolated HOME/XDG dirs; config at `<root>/cfg/config.yaml` (not created).
struct Home {
    root: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        Home {
            root: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }
    fn config(&self) -> PathBuf {
        self.path("cfg/config.yaml")
    }
    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("autorename-pdf").unwrap();
        c.env_remove("AUTORENAME_PDF_CONFIG")
            .env_remove("OPENAI_API_KEY")
            .env_remove("JEV_API_KEY")
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_CONFIG_HOME", self.path("xdgcfg"))
            .env("HOME", self.root.path())
            .env("AUTORENAME_MODELS_DIR", self.path("models"))
            .env("NO_COLOR", "1")
            .arg("--config")
            .arg(self.config());
        c
    }
    /// Run; returns (exit, stdout JSON, stdout+stderr text).
    fn setup(
        &self,
        args: &[&str],
        envs: &[(&str, &str)],
        stdin: Option<&str>,
    ) -> (i32, Value, String) {
        let mut c = self.cmd();
        c.arg("setup").args(args).args(["-o", "json"]);
        for (k, v) in envs {
            c.env(k, v);
        }
        if let Some(s) = stdin {
            c.write_stdin(s);
        }
        let out = c.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let v =
            serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("no JSON ({e}): {text}"));
        (out.status.code().unwrap_or(-1), v, text)
    }
}

fn statuses(v: &Value) -> Vec<(String, String)> {
    v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["step"].as_str().unwrap().to_string(),
                s["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn step<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["step"] == name)
        .unwrap_or_else(|| panic!("no step {name}: {v:#}"))
}

#[test]
fn check_on_an_empty_home_changes_nothing() {
    let h = Home::new();
    let (code, v, _) = h.setup(&["--check"], &[], None);
    assert_eq!(code, 3, "{v:#}");
    assert_eq!(v["ready"], false);
    assert_eq!(step(&v, "config")["status"], "missing");
    assert!(step(&v, "config")["hint"].is_string());
    assert!(!h.config().exists() && !h.path("cfg/.env").exists());
}

#[test]
fn setup_is_idempotent_and_keeps_the_key_out_of_config_and_output() {
    let h = Home::new();
    let args = [
        "--provider",
        "openai",
        "--api-key-env",
        "MY_KEY",
        "--company",
        "Spqrk",
    ];
    let env = [("MY_KEY", SECRET)];
    let (code, v, text) = h.setup(&args, &env, None);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(v["ready"], true);
    assert!(!text.contains(SECRET), "key echoed: {text}");
    let config = std::fs::read_to_string(h.config()).unwrap();
    assert!(!config.contains(SECRET));
    assert!(config.contains("\"${MY_KEY}\""));
    assert!(config.contains("# YOUR company"), "comments kept");
    let dotenv = std::fs::read_to_string(h.path("cfg/.env")).unwrap();
    // Encoded for dotenvy (quoted, escaped); read back exactly.
    assert_eq!(
        autorename_core::config_edit::get_env(&dotenv, "MY_KEY").as_deref(),
        Some(SECRET)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(h.path("cfg/.env"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // Second run: nothing to do.
    let before = (config, dotenv);
    let (code, v, text) = h.setup(&args, &env, None);
    assert_eq!(code, 0);
    assert!(!text.contains(SECRET));
    assert!(
        statuses(&v).iter().all(|(_, s)| s == "unchanged"),
        "{:?}",
        statuses(&v)
    );
    let after = (
        std::fs::read_to_string(h.config()).unwrap(),
        std::fs::read_to_string(h.path("cfg/.env")).unwrap(),
    );
    assert_eq!(before, after);
    // --check now reports ready; `config show` redacts.
    let (code, v, _) = h.setup(&["--check"], &[], None);
    assert_eq!((code, v["ready"].as_bool()), (0, Some(true)), "{v:#}");
    let out = h
        .cmd()
        .args(["config", "show", "-o", "json"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains(SECRET));
}

#[test]
fn key_from_stdin_and_missing_key_hint() {
    let h = Home::new();
    // No key anywhere: validation fails with an actionable hint (exit 3).
    let (code, v, _) = h.setup(
        &["--provider", "anthropic", "--company", "Spqrk"],
        &[],
        None,
    );
    assert_eq!(code, 3, "{v:#}");
    assert_eq!(step(&v, "validate")["status"], "failed");
    assert!(
        step(&v, "validate")["hint"]
            .as_str()
            .unwrap()
            .contains("--api-key-stdin")
    );
    // A rejected run changes nothing.
    assert!(!h.config().exists() && !h.path("cfg/.env").exists());
    let (code, v, text) = h.setup(
        &[
            "--provider",
            "anthropic",
            "--company",
            "Spqrk",
            "--api-key-stdin",
        ],
        &[],
        Some(&format!("{SECRET}\n")),
    );
    assert_eq!(code, 0, "{v:#}");
    assert!(!text.contains(SECRET));
    let dotenv = std::fs::read_to_string(h.path("cfg/.env")).unwrap();
    assert_eq!(
        autorename_core::config_edit::get_env(&dotenv, "ANTHROPIC_API_KEY").as_deref(),
        Some(SECRET)
    );
    // Unset env var → failed step, exit 3, nothing stored.
    let (code, v, _) = h.setup(&["--api-key-env", "NOT_SET_ANYWHERE"], &[], None);
    assert_eq!(code, 3);
    assert_eq!(step(&v, "api_key")["status"], "failed");
}

/// Changing the provider rebinds `ai.api_key` to the new provider's
/// variable (empty for Ollama), like the GUI; the same provider keeps a
/// custom variable.
#[test]
fn provider_change_rebinds_the_api_key() {
    let h = Home::new();
    let key = |h: &Home| {
        let text = std::fs::read_to_string(h.config()).unwrap();
        text.lines()
            .find(|l| l.trim_start().starts_with("api_key:"))
            .unwrap()
            .split('#')
            .next()
            .unwrap()
            .trim()
            .to_string()
    };
    let env = [("MY_KEY", SECRET), ("ANTHROPIC_API_KEY", SECRET)];
    let args = [
        "--provider",
        "openai",
        "--api-key-env",
        "MY_KEY",
        "--company",
        "Spqrk",
    ];
    assert_eq!(h.setup(&args, &env, None).0, 0);
    assert_eq!(key(&h), r#"api_key: "${MY_KEY}""#);
    // Same provider again: the custom variable stays.
    let (code, v, _) = h.setup(&["--provider", "openai"], &env, None);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(key(&h), r#"api_key: "${MY_KEY}""#);
    let (code, v, _) = h.setup(&["--provider", "anthropic"], &env, None);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(step(&v, "provider")["status"], "done");
    assert_eq!(key(&h), r#"api_key: "${ANTHROPIC_API_KEY}""#);
    h.setup(&["--provider", "ollama"], &env, None);
    assert_eq!(key(&h), r#"api_key: """#);
}

fn ai(h: &Home, key: &str) -> Value {
    let text = std::fs::read_to_string(h.config()).unwrap();
    let v = autorename_core::config::parse_yaml(&text).unwrap();
    v["ai"][key].clone()
}

/// A provider change never keeps the previous provider's endpoint (its
/// requests and key would go to the old server): `ai.base_url` is cleared
/// unless `--base-url` is given in the same run.
#[test]
fn provider_change_clears_the_old_endpoint() {
    let h = Home::new();
    let env = [("MY_KEY", SECRET)];
    let args = [
        "--provider",
        "anthropic",
        "--api-key-env",
        "MY_KEY",
        "--company",
        "Spqrk",
    ];
    let (code, v, _) = h.setup(&args, &env, None);
    assert_eq!(code, 0, "{v:#}");
    // An endpoint written by hand (or by an older run).
    let text = std::fs::read_to_string(h.config()).unwrap();
    let url = autorename_core::config_edit::quoted("https://api.anthropic.com/v1");
    let text = autorename_core::config_edit::set(&text, Some("ai"), "base_url", &url);
    std::fs::write(h.config(), text).unwrap();
    assert_eq!(ai(&h, "base_url"), "https://api.anthropic.com/v1");
    // Same provider: the endpoint stays.
    let (code, v, _) = h.setup(&["--provider", "anthropic"], &env, None);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(ai(&h, "base_url"), "https://api.anthropic.com/v1");
    // Switch: cleared (the new provider's default endpoint).
    let (code, v, _) = h.setup(
        &["--provider", "openai", "--api-key-env", "MY_KEY"],
        &env,
        None,
    );
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(ai(&h, "provider"), "openai");
    assert_eq!(ai(&h, "base_url"), "");
    // Switch with an explicit endpoint (shared gateway): that one.
    let (code, v, _) = h.setup(
        &[
            "--provider",
            "xai",
            "--base-url",
            "https://gw.example/v1",
            "--api-key-env",
            "MY_KEY",
        ],
        &env,
        None,
    );
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(ai(&h, "base_url"), "https://gw.example/v1");
    let (code, v, _) = h.setup(&["--base-url", ""], &env, None);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(ai(&h, "base_url"), "");
}

/// Offline mode is left only explicitly: `--provider <cloud>` alone stays
/// refused (exit 3, nothing written, hint names `--cloud`); `--cloud`
/// switches privacy back and the cloud provider gets its own endpoint.
#[test]
fn cloud_flag_leaves_offline_mode_explicitly() {
    let h = Home::new();
    let (code, v, _) = h.setup(
        &[
            "--offline",
            "--ollama-url",
            "http://127.0.0.1:9",
            "--company",
            "Spqrk",
        ],
        &[],
        None,
    );
    assert_eq!(step(&v, "validate")["status"], "unchanged", "{v:#} {code}");
    let before = std::fs::read_to_string(h.config()).unwrap();
    let env = [("MY_KEY", SECRET)];
    let (code, v, _) = h.setup(
        &["--provider", "anthropic", "--api-key-env", "MY_KEY"],
        &env,
        None,
    );
    assert_eq!(code, 3, "{v:#}");
    let hint = step(&v, "validate")["hint"].as_str().unwrap().to_string();
    assert!(hint.contains("setup --cloud"), "{hint}");
    assert_eq!(std::fs::read_to_string(h.config()).unwrap(), before);
    let (code, v, _) = h.setup(
        &[
            "--cloud",
            "--provider",
            "anthropic",
            "--api-key-env",
            "MY_KEY",
        ],
        &env,
        None,
    );
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(step(&v, "privacy")["status"], "done");
    let text = std::fs::read_to_string(h.config()).unwrap();
    let tree = autorename_core::config::parse_yaml(&text).unwrap();
    assert_eq!(tree["privacy"], "cloud");
    assert_eq!(ai(&h, "provider"), "anthropic");
    assert_eq!(ai(&h, "base_url"), "", "the Ollama URL must not stay");
    // Idempotent, and --cloud with --offline is a usage error.
    let (code, v, _) = h.setup(&["--cloud"], &env, None);
    assert_eq!(
        (code, step(&v, "privacy")["status"].as_str()),
        (0, Some("unchanged"))
    );
    let (code, _, _) = h.setup(&["--offline", "--cloud"], &env, None);
    assert_eq!(code, 2);
}

#[test]
fn decider_key_is_validated_and_stored_in_env() {
    let h = Home::new();
    let base = [("OPENAI_API_KEY", "sk-main-1234567890")];
    let (code, _, _) = h.setup(
        &["--api-key-env", "OPENAI_API_KEY", "--company", "Spqrk"],
        &base,
        None,
    );
    assert_eq!(code, 0);
    let before = std::fs::read_to_string(h.config()).unwrap();
    let (code, v, _) = h.setup(&["--decider", "jev"], &base, None);
    assert_eq!(code, 3, "{v:#}");
    assert_eq!(
        std::fs::read_to_string(h.config()).unwrap(),
        before,
        "rejected run writes nothing"
    );
    let hint = step(&v, "validate")["hint"].as_str().unwrap().to_string();
    assert!(hint.contains("JEV_API_KEY"), "{hint}");
    let (code, v, text) = h.setup(
        &["--decider", "jev", "--decider-key-env", "JEV_API_KEY"],
        &[("JEV_API_KEY", SECRET)],
        None,
    );
    assert_eq!(code, 0, "{v:#}");
    assert!(!text.contains(SECRET));
    let config = std::fs::read_to_string(h.config()).unwrap();
    assert!(
        config.contains("decider_api_key: \"${JEV_API_KEY}\""),
        "{config}"
    );
    let out = h
        .cmd()
        .args(["config", "show", "-o", "json"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains(SECRET));
}

#[test]
fn offline_setup_refuses_model_downloads() {
    let h = Home::new();
    let (code, v, _) = h.setup(
        &["--offline", "--company", "Spqrk", "--ocr-models"],
        &[],
        None,
    );
    assert_eq!(code, 3, "{v:#}");
    assert_eq!(step(&v, "offline")["status"], "done");
    assert_eq!(step(&v, "validate")["status"], "unchanged");
    let ocr = step(&v, "ocr_models");
    assert_eq!(ocr["status"], "failed");
    assert!(ocr["hint"].as_str().unwrap().contains("models import"));
    let config = std::fs::read_to_string(h.config()).unwrap();
    assert!(config.contains("privacy: \"offline\""));
    assert!(config.contains("provider: \"ollama\""));
    // No cloud key reference → no "${OPENAI_API_KEY} is not set" notice.
    assert!(!config.contains("${OPENAI_API_KEY}"), "{config}");
    let out = h
        .cmd()
        .args(["config", "validate", "-o", "json"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!v.to_string().contains("OPENAI_API_KEY"), "{v:#}");
}

/// One-shot local Ollama stand-in answering `/api/tags`.
fn fake_ollama(models: &'static str) -> String {
    use std::io::{BufRead, BufReader, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut r = BufReader::new(s);
            let mut line = String::new();
            while r.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let body = format!("{{\"models\":[{models}]}}");
            let _ = r.get_mut().write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    .as_bytes(),
            );
        }
    });
    url
}

#[test]
fn offline_setup_checks_ollama_and_the_model() {
    let h = Home::new();
    let url = fake_ollama(r#"{"name":"gemma4:latest"}"#);
    let (code, v, _) = h.setup(
        &[
            "--offline",
            "--company",
            "Spqrk",
            "--ollama-url",
            &url,
            "--ollama-model",
            "gemma4",
        ],
        &[],
        None,
    );
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(step(&v, "ollama")["status"], "unchanged");
    assert_eq!(step(&v, "ollama")["required"], true);
    // Model not pulled → missing + `ollama pull` hint, not ready (exit 3).
    let (code, v, _) = h.setup(
        &[
            "--offline",
            "--ollama-url",
            &url,
            "--ollama-model",
            "gemma4:e2b",
        ],
        &[],
        None,
    );
    assert_eq!(code, 3, "{v:#}");
    assert_eq!(step(&v, "ollama")["hint"], "ollama pull gemma4:e2b");
    // --offline without --ollama-model / --ollama-url keeps the saved values.
    let (_, v, _) = h.setup(&["--offline"], &[], None);
    assert_eq!(step(&v, "offline")["status"], "unchanged", "{v:#}");
    let config = std::fs::read_to_string(h.config()).unwrap();
    assert!(config.contains("model: \"gemma4:e2b\""), "{config}");
    assert!(
        config.contains(&format!("base_url: \"{url}\"")),
        "custom URL kept: {config}"
    );
    // Nothing listening → missing with a start hint.
    let (code, v, _) = h.setup(
        &["--offline", "--ollama-url", "http://127.0.0.1:9"],
        &[],
        None,
    );
    assert_eq!(code, 3);
    assert!(
        step(&v, "ollama")["hint"]
            .as_str()
            .unwrap()
            .contains("ollama serve")
    );
}

// ---------------------------------------------------------------------------
// examples/demo/README.md doc test
// ---------------------------------------------------------------------------

fn demo_readme() -> String {
    std::fs::read_to_string(repo().join("examples/demo/README.md")).unwrap()
}

/// Lines of the ```sh offline blocks (commands that need no network/key).
fn offline_commands(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in md.lines() {
        match line.trim() {
            "```sh offline" => inside = true,
            "```" if inside => inside = false,
            l if inside && !l.is_empty() => out.push(l.to_string()),
            _ => {}
        }
    }
    out
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        std::fs::copy(&p, to.join(p.file_name().unwrap())).unwrap();
    }
}

#[test]
fn demo_readme_offline_commands_work() {
    let md = demo_readme();
    let cmds = offline_commands(&md);
    assert!(cmds.len() >= 3, "{cmds:?}");
    let h = Home::new();
    let work = h.path("work");
    copy_dir(&repo().join("examples/demo"), &work.join("demo"));
    for cmd in &cmds {
        let args: Vec<&str> = cmd
            .strip_prefix("autorename-pdf ")
            .unwrap_or_else(|| panic!("not an autorename-pdf command: {cmd}"))
            .split_whitespace()
            .collect();
        let out = h.cmd().current_dir(&work).args(&args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{cmd}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // Every demo PDF named in the README exists (and vice versa).
    let mut files: Vec<String> = std::fs::read_dir(repo().join("examples/demo"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".pdf"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 7);
    for f in &files {
        assert!(md.contains(&format!("`{f}`")), "{f} not documented");
    }
    // The documented inspect output for the unrenamed invoice is exact.
    let out = h
        .cmd()
        .current_dir(&work)
        .args(["inspect", "demo/acme-invoice.pdf", "-o", "text"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines().skip(1) {
        assert!(md.contains(line), "README out of date, missing: {line:?}");
    }
}
