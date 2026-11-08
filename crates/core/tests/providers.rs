//! Provider adapter replay tests against a local scripted HTTP server.
//!
//! The server is hand-rolled (std `TcpListener`, one scripted reply per
//! connection, `Connection: close`) so that sequences like "429 then 200" are
//! deterministic and every request body/header can be asserted.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autorename_core::config::{Decider, Privacy, Provider};
use autorename_core::harmonize::{Candidate, Verdict};
use autorename_core::profiles::select_profile;
use autorename_core::providers::{
    DeciderSettings, ErrorKind, ExtractRequest, ProviderError, ProviderSettings, RetryPolicy,
    decide_name, extract, extract_detailed,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Mock server
// ---------------------------------------------------------------------------

struct Reply {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
}

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/providers/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn reply(status: u16, name: &str) -> Reply {
    Reply {
        status,
        headers: vec![],
        body: fixture(name),
    }
}

fn reply_h(status: u16, name: &str, headers: Vec<(&'static str, &'static str)>) -> Reply {
    Reply {
        status,
        headers,
        body: fixture(name),
    }
}

#[derive(Debug, Clone)]
struct Recorded {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

struct Mock {
    url: String,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl Mock {
    fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

fn serve(replies: Vec<Reply>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    std::thread::spawn(move || {
        for r in replies {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap();
                    }
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            sink.lock().unwrap().push(Recorded {
                path,
                headers,
                body: serde_json::from_slice(&body).unwrap_or(Value::Null),
            });
            let mut out = format!(
                "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                r.status,
                r.body.len()
            );
            for (k, v) in &r.headers {
                out.push_str(&format!("{k}: {v}\r\n"));
            }
            out.push_str("\r\n");
            out.push_str(&r.body);
            let mut stream = reader.into_inner();
            stream.write_all(out.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
    });
    Mock { url, log }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake";

fn request(images: Vec<Vec<u8>>) -> ExtractRequest {
    let (_, profile) = select_profile(&common::sample_config(), None).unwrap();
    ExtractRequest::document(&profile, "", "INVOICE\nFrom: ACME Corporation GmbH", images).unwrap()
}

fn settings(provider: Provider, base: String) -> ProviderSettings {
    ProviderSettings {
        provider,
        model: match provider {
            Provider::OpenAi => "gpt-6-luna",
            Provider::Anthropic => "claude-haiku-5-5",
            Provider::Gemini => "gemini-3.5-flash-lite",
            Provider::Xai => "grok-4.7",
            Provider::Ollama => "gemma4",
        }
        .into(),
        api_key: if provider == Provider::Ollama {
            String::new()
        } else {
            "test-key".into()
        },
        base_url: base,
        temperature: 0.0,
        privacy: Privacy::Cloud,
        timeout: Duration::from_secs(5),
        retry: RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(50),
        },
    }
}

fn base_for(provider: Provider, mock: &Mock) -> String {
    match provider {
        Provider::OpenAi | Provider::Xai | Provider::Anthropic => format!("{}/v1", mock.url),
        Provider::Gemini => format!("{}/v1beta", mock.url),
        Provider::Ollama => mock.url.clone(),
    }
}

fn run(
    provider: Provider,
    replies: Vec<Reply>,
    images: Vec<Vec<u8>>,
) -> (Result<Value, ProviderError>, Vec<Recorded>) {
    let mock = serve(replies);
    let cfg = settings(provider, base_for(provider, &mock));
    let res = extract_detailed(&request(images), &cfg).map(|r| serde_json::to_value(r).unwrap());
    (res, mock.requests())
}

fn expected_value() -> Value {
    json!({"company_name": "ACME Corp", "issuer": "ACME Corp", "recipient": "Spqrk GmbH", "document_date": "15.03.2026", "document_type": "AP", "total_amount": "1,234.56", "currency": "", "description": "1,234.56"})
}

fn err_kind(res: &Result<Value, ProviderError>) -> ErrorKind {
    res.as_ref().expect_err("expected an error").kind
}

// ---------------------------------------------------------------------------
// Success per provider (request shape asserted)
// ---------------------------------------------------------------------------

#[test]
fn openai_success_request_shape() {
    let (res, reqs) = run(
        Provider::OpenAi,
        vec![reply_h(
            200,
            "openai_success.json",
            vec![("x-request-id", "req_openai_1")],
        )],
        vec![PNG.to_vec()],
    );
    let r = res.unwrap();
    assert_eq!(r["value"], expected_value());
    assert_eq!(r["model"], "gpt-6-luna-2026-09-01");
    assert_eq!(r["request_id"], "req_openai_1");
    assert_eq!(
        r["usage"],
        json!({"input_tokens": 812, "output_tokens": 41})
    );
    assert_eq!(reqs.len(), 1);
    let q = &reqs[0];
    assert_eq!(q.path, "/v1/responses");
    assert_eq!(q.header("authorization"), Some("Bearer test-key"));
    let b = &q.body;
    assert_eq!(b["model"], "gpt-6-luna");
    assert_eq!(b["store"], false);
    assert_eq!(b["reasoning"], json!({"effort": "none"}));
    assert!(b.get("temperature").is_none());
    let f = &b["text"]["format"];
    assert_eq!(f["type"], "json_schema");
    assert_eq!(f["name"], "DocumentMetadata");
    assert_eq!(f["strict"], true);
    assert_eq!(f["schema"]["additionalProperties"], false);
    assert_eq!(
        f["schema"]["required"],
        json!([
            "document_date",
            "company_name",
            "issuer",
            "recipient",
            "document_type",
            "total_amount",
            "currency",
            "description"
        ])
    );
    assert_eq!(b["input"][0]["role"], "system");
    assert!(
        b["input"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Extract the requested fields")
    );
    let user = &b["input"][1]["content"];
    assert_eq!(user[0]["type"], "input_text");
    assert!(
        user[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Extract document metadata from this text and images:\n\n")
    );
    assert_eq!(user[1]["type"], "input_image");
    assert!(
        user[1]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
}

#[test]
fn anthropic_success_request_shape() {
    let (res, reqs) = run(
        Provider::Anthropic,
        vec![reply_h(
            200,
            "anthropic_success.json",
            vec![("request-id", "req_011CA")],
        )],
        vec![PNG.to_vec()],
    );
    let r = res.unwrap();
    assert_eq!(r["value"], expected_value());
    assert_eq!(r["request_id"], "req_011CA");
    assert_eq!(r["usage"]["output_tokens"], 52);
    let q = &reqs[0];
    assert_eq!(q.path, "/v1/messages");
    assert_eq!(q.header("x-api-key"), Some("test-key"));
    assert_eq!(q.header("anthropic-version"), Some("2026-10-21"));
    assert!(q.header("authorization").is_none());
    let b = &q.body;
    assert!(b.get("temperature").is_none());
    assert_eq!(b["max_tokens"], 2048);
    assert!(
        b["system"]
            .as_str()
            .unwrap()
            .starts_with("Extract the requested fields")
    );
    assert_eq!(b["output_config"]["format"]["type"], "json_schema");
    assert_eq!(
        b["output_config"]["format"]["schema"]["additionalProperties"],
        false
    );
    assert_eq!(b["thinking"], json!({"type": "disabled"}));
    let content = &b["messages"][0]["content"];
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["media_type"], "image/png");
}

#[test]
fn gemini_success_request_shape() {
    let (res, reqs) = run(
        Provider::Gemini,
        vec![reply(200, "gemini_success.json")],
        vec![PNG.to_vec()],
    );
    let r = res.unwrap();
    assert_eq!(r["value"], expected_value());
    assert_eq!(r["model"], "gemini-3.5-flash-lite");
    assert_eq!(
        r["usage"],
        json!({"input_tokens": 780, "output_tokens": 44})
    );
    let q = &reqs[0];
    assert_eq!(
        q.path,
        "/v1beta/models/gemini-3.5-flash-lite:generateContent"
    );
    assert_eq!(q.header("x-goog-api-key"), Some("test-key"));
    let g = &q.body["generationConfig"];
    assert_eq!(g["responseMimeType"], "application/json");
    assert!(g["responseJsonSchema"].get("title").is_none());
    assert_eq!(g["responseJsonSchema"]["type"], "object");
    assert!(q.body["systemInstruction"]["parts"][0]["text"].is_string());
    let parts = &q.body["contents"][0]["parts"];
    assert!(parts[0]["text"].is_string());
    assert_eq!(parts[1]["inlineData"]["mimeType"], "image/png");
}

#[test]
fn xai_success_request_shape() {
    let (res, reqs) = run(Provider::Xai, vec![reply(200, "xai_success.json")], vec![]);
    assert_eq!(res.unwrap()["value"], expected_value());
    let q = &reqs[0];
    assert_eq!(q.path, "/v1/responses");
    assert_eq!(q.body["model"], "grok-4.7");
    assert_eq!(q.body["store"], false);
    assert_eq!(q.body["reasoning"], json!({"effort": "low"}));
    assert_eq!(q.body["text"]["format"]["strict"], true);

    let (_, reqs) = run(
        Provider::Xai,
        vec![reply(200, "xai_success.json")],
        vec![PNG.to_vec()],
    );
    assert!(
        reqs[0].body.get("store").is_none(),
        "no store:false with images"
    );
    assert_eq!(reqs[0].body["input"][1]["content"][1]["detail"], "high");
}

#[test]
fn ollama_success_request_shape() {
    let (res, reqs) = run(
        Provider::Ollama,
        vec![reply(200, "ollama_success.json")],
        vec![PNG.to_vec()],
    );
    let r = res.unwrap();
    assert_eq!(r["value"], expected_value());
    assert_eq!(
        r["usage"],
        json!({"input_tokens": 760, "output_tokens": 48})
    );
    let q = &reqs[0];
    assert_eq!(q.path, "/api/chat");
    assert!(q.header("authorization").is_none());
    assert_eq!(q.body["stream"], false);
    assert_eq!(q.body["think"], false);
    assert_eq!(q.body["format"]["type"], "object");
    assert_eq!(q.body["options"]["temperature"], 0.0);
    assert!(
        q.body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("JSON Schema")
    );
    let img = q.body["messages"][1]["images"][0].as_str().unwrap();
    assert!(!img.starts_with("data:"));
}

// ---------------------------------------------------------------------------
// Refusal / truncation
// ---------------------------------------------------------------------------

#[test]
fn refusals_are_classified_and_not_retried() {
    for (p, f) in [
        (Provider::OpenAi, "openai_refusal.json"),
        (Provider::Anthropic, "anthropic_refusal.json"),
        (Provider::Gemini, "gemini_blocked.json"),
    ] {
        let (res, reqs) = run(p, vec![reply(200, f), reply(200, f)], vec![]);
        assert_eq!(err_kind(&res), ErrorKind::Refusal, "{f}");
        assert_eq!(reqs.len(), 1, "{f}");
    }
}

#[test]
fn truncation_is_classified() {
    for (p, f) in [
        (Provider::OpenAi, "openai_incomplete.json"),
        (Provider::Anthropic, "anthropic_max_tokens.json"),
        (Provider::Gemini, "gemini_max_tokens.json"),
        (Provider::Ollama, "ollama_length.json"),
    ] {
        let (res, reqs) = run(p, vec![reply(200, f), reply(200, f)], vec![]);
        let e = res.expect_err(f);
        assert_eq!(e.kind, ErrorKind::Truncated, "{f}");
        assert_eq!(reqs.len(), 1, "{f}");
        let core: autorename_core::Error = e.into();
        assert_eq!(core.exit_code().code(), 10);
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[test]
fn openai_401_is_auth_with_request_id() {
    let mock = serve(vec![reply_h(
        401,
        "openai_401.json",
        vec![("x-request-id", "req_bad")],
    )]);
    let cfg = settings(Provider::OpenAi, base_for(Provider::OpenAi, &mock));
    let e = extract(&request(vec![]), &cfg).unwrap_err();
    assert_eq!(e.exit_code().code(), 11);
    assert!(e.to_string().contains("req_bad"), "{e}");
    assert!(e.to_string().contains("HTTP 401"), "{e}");
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn anthropic_401_takes_request_id_from_body() {
    let (res, reqs) = run(
        Provider::Anthropic,
        vec![reply(401, "anthropic_401.json")],
        vec![],
    );
    let e = res.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    assert_eq!(e.request_id.as_deref(), Some("req_011CAuth"));
    assert_eq!(reqs.len(), 1);
}

#[test]
fn gemini_bad_key_400_is_auth() {
    let (res, _) = run(
        Provider::Gemini,
        vec![reply(400, "gemini_400_bad_key.json")],
        vec![],
    );
    assert_eq!(err_kind(&res), ErrorKind::Auth);
}

#[test]
fn missing_api_key_fails_without_request() {
    let mock = serve(vec![reply(200, "openai_success.json")]);
    let mut cfg = settings(Provider::OpenAi, base_for(Provider::OpenAi, &mock));
    cfg.api_key.clear();
    let e = extract(&request(vec![]), &cfg).unwrap_err();
    assert_eq!(e.exit_code().code(), 11);
    assert!(mock.requests().is_empty());
}

// ---------------------------------------------------------------------------
// Rate limits / 5xx
// ---------------------------------------------------------------------------

#[test]
fn openai_429_with_retry_after_then_success() {
    let (res, reqs) = run(
        Provider::OpenAi,
        vec![
            reply_h(429, "openai_429_rate.json", vec![("retry-after", "1")]),
            reply(200, "openai_success.json"),
        ],
        vec![],
    );
    let r = res.unwrap();
    assert_eq!(r["attempts"], 2);
    assert_eq!(reqs.len(), 2);
}

#[test]
fn anthropic_429_with_retry_after_then_success() {
    let (res, reqs) = run(
        Provider::Anthropic,
        vec![
            reply_h(429, "anthropic_429_rate.json", vec![("retry-after", "2")]),
            reply(200, "anthropic_success.json"),
        ],
        vec![],
    );
    assert_eq!(res.unwrap()["value"], expected_value());
    assert_eq!(reqs.len(), 2);
}

#[test]
fn anthropic_spend_cap_429_without_retry_after_is_not_retried() {
    let (res, reqs) = run(
        Provider::Anthropic,
        vec![
            reply(429, "anthropic_429_spend.json"),
            reply(200, "anthropic_success.json"),
        ],
        vec![],
    );
    let e = res.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Quota);
    assert_eq!(e.request_id.as_deref(), Some("req_011CSpend"));
    assert_eq!(reqs.len(), 1);
    let core: autorename_core::Error = e.into();
    assert_eq!(core.exit_code().code(), 10);
    assert!(!core.retryable());
}

#[test]
fn openai_insufficient_quota_is_not_retried() {
    let (res, reqs) = run(
        Provider::OpenAi,
        vec![
            reply(429, "openai_429_quota.json"),
            reply(200, "openai_success.json"),
        ],
        vec![],
    );
    assert_eq!(err_kind(&res), ErrorKind::Quota);
    assert_eq!(reqs.len(), 1);
}

#[test]
fn overload_retries_are_bounded() {
    let (res, reqs) = run(
        Provider::Anthropic,
        vec![
            reply(529, "anthropic_529.json"),
            reply(529, "anthropic_529.json"),
            reply(529, "anthropic_529.json"),
            reply(200, "anthropic_success.json"),
        ],
        vec![],
    );
    assert_eq!(err_kind(&res), ErrorKind::Unavailable);
    assert_eq!(reqs.len(), 3, "1 + max_retries(2)");
}

#[test]
fn ollama_unknown_model_hints_pull() {
    let (res, _) = run(
        Provider::Ollama,
        vec![reply(404, "ollama_404.json")],
        vec![],
    );
    let e = res.unwrap_err();
    assert_eq!(e.kind, ErrorKind::BadRequest);
    assert!(e.message.contains("ollama pull"), "{}", e.message);
}

// ---------------------------------------------------------------------------
// Schema-invalid output: one re-ask with feedback
// ---------------------------------------------------------------------------

#[test]
fn malformed_json_then_success() {
    let (res, reqs) = run(
        Provider::OpenAi,
        vec![
            reply(200, "openai_malformed.json"),
            reply(200, "openai_success.json"),
        ],
        vec![],
    );
    let r = res.unwrap();
    assert_eq!(r["value"], expected_value());
    assert_eq!(r["attempts"], 2);
    let retry_text = reqs[1].body["input"][1]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        retry_text.contains("Your previous reply was rejected: output is not valid JSON"),
        "{retry_text}"
    );
}

#[test]
fn schema_invalid_twice_fails() {
    let (res, reqs) = run(
        Provider::OpenAi,
        vec![
            reply(200, "openai_schema_invalid.json"),
            reply(200, "openai_schema_invalid.json"),
            reply(200, "openai_success.json"),
        ],
        vec![],
    );
    let e = res.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidOutput);
    assert!(e.message.contains("$.document_date"), "{}", e.message);
    assert_eq!(reqs.len(), 2);
}

// ---------------------------------------------------------------------------
// Offline guard
// ---------------------------------------------------------------------------

#[test]
fn offline_refuses_cloud_and_allows_local() {
    let mut cfg = settings(Provider::OpenAi, String::new());
    cfg.privacy = Privacy::Offline;
    let e = extract(&request(vec![]), &cfg).unwrap_err();
    assert_eq!(e.exit_code().code(), 3);

    let mock = serve(vec![reply(200, "ollama_success.json")]);
    let mut cfg = settings(Provider::Ollama, mock.url.clone());
    cfg.privacy = Privacy::Offline;
    assert!(extract(&request(vec![]), &cfg).is_ok());
}

#[test]
fn hostile_retry_after_does_not_panic() {
    let (res, reqs) = run(
        Provider::OpenAi,
        vec![
            reply_h(429, "openai_429_rate.json", vec![("retry-after", "inf")]),
            reply_h(
                429,
                "openai_429_rate.json",
                vec![("retry-after-ms", "1e309")],
            ),
            reply(200, "openai_success.json"),
        ],
        vec![],
    );
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(reqs.len(), 3);
}

#[test]
fn offline_never_follows_redirects() {
    // The "elsewhere" server must never see the document.
    let elsewhere = serve(vec![reply(200, "ollama_success.json")]);
    let location: &'static str = Box::leak(format!("{}/api/chat", elsewhere.url).into_boxed_str());
    let local = serve(vec![Reply {
        status: 307,
        headers: vec![("location", location)],
        body: "{}".into(),
    }]);
    let mut cfg = settings(Provider::Ollama, local.url.clone());
    cfg.privacy = Privacy::Offline;
    assert!(extract_detailed(&request(vec![]), &cfg).is_err());
    assert_eq!(local.requests().len(), 1);
    assert!(elsewhere.requests().is_empty(), "redirect was followed");
}

#[test]
fn offline_guard_uses_the_client_url_parser() {
    for url in [
        "http://134744072:11434",
        "http://0x08080808:11434",
        "http://8.8.8.8\\@localhost:11434",
        "http://localhost@8.8.8.8:11434",
        "ftp://localhost:11434",
    ] {
        let mut cfg = settings(Provider::Ollama, url.into());
        cfg.privacy = Privacy::Offline;
        let e = extract_detailed(&request(vec![]), &cfg).unwrap_err();
        assert_eq!(e.kind, ErrorKind::OfflineBlocked, "{url}");
    }
}

// ---------------------------------------------------------------------------
// Harmonization decider
// ---------------------------------------------------------------------------

fn candidates() -> Vec<Candidate> {
    ["ACME Corporation", "ACME Bau"]
        .iter()
        .map(|c| Candidate {
            canonical: c.to_string(),
            matched: c.to_string(),
            score: 0.8,
        })
        .collect()
}

fn decider(decider: Decider, provider: ProviderSettings, url: String) -> DeciderSettings {
    DeciderSettings {
        decider,
        provider,
        api_key: "dec-key".into(),
        url,
    }
}

#[test]
fn decider_ai_uses_enum_schema() {
    let mock = serve(vec![reply(200, "anthropic_decider.json")]);
    let p = settings(Provider::Anthropic, base_for(Provider::Anthropic, &mock));
    let v = decide_name(
        &candidates(),
        "Acme Bau GmbH",
        "company_name",
        &decider(Decider::Ai, p, String::new()),
    )
    .unwrap();
    assert_eq!(v, Some(Verdict::Existing("ACME Bau".into())));
    let schema = &mock.requests()[0].body["output_config"]["format"]["schema"];
    assert_eq!(
        schema["properties"]["choice"]["enum"],
        json!(["0", "1", "new"])
    );
}

#[test]
fn decider_openai_decisions_best_effort() {
    let mock = serve(vec![reply(200, "openai_decisions_success.json")]);
    let p = settings(Provider::OpenAi, String::new());
    let cfg = decider(
        Decider::OpenaiDecisions,
        p,
        format!("{}/v1/decisions", mock.url),
    );
    let v = decide_name(&candidates(), "ACME Corp.", "company_name", &cfg).unwrap();
    assert_eq!(v, Some(Verdict::Existing("ACME Corporation".into())));
    let q = &mock.requests()[0];
    assert_eq!(q.path, "/v1/decisions");
    assert_eq!(q.header("authorization"), Some("Bearer dec-key"));
    assert_eq!(q.body["model"], "gpt-6-luna");
    assert_eq!(q.body["type"], "choice");
    assert_eq!(q.body["options"].as_array().unwrap().len(), 3);
}

#[test]
fn decider_jev_best_effort() {
    let mock = serve(vec![reply(200, "jev_success.json")]);
    let p = settings(Provider::OpenAi, String::new());
    let cfg = decider(Decider::Jev, p, format!("{}/v1/systemone", mock.url));
    let v = decide_name(&candidates(), "Acme Bau", "company_name", &cfg).unwrap();
    assert_eq!(v, Some(Verdict::Existing("ACME Bau".into())));
    assert!(mock.requests()[0].body.get("model").is_none());
}

#[test]
fn decider_off_and_offline() {
    let p = settings(Provider::OpenAi, String::new());
    let v = decide_name(
        &candidates(),
        "x",
        "company_name",
        &decider(Decider::Off, p.clone(), String::new()),
    )
    .unwrap();
    assert_eq!(v, None);
    let mut off = p;
    off.privacy = Privacy::Offline;
    let e = decide_name(
        &candidates(),
        "x",
        "company_name",
        &decider(Decider::Jev, off, String::new()),
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::OfflineBlocked);
}

#[test]
fn connection_errors_explain_and_hide_urls() {
    let mut cfg = settings(Provider::OpenAi, "http://127.0.0.1:9/v1?key=SECRET".into());
    cfg.retry.max_retries = 0;
    let e = extract_detailed(&request(vec![]), &cfg).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unavailable);
    assert!(e.retryable());
    assert!(e.message.contains("could not connect"), "{}", e.message);
    assert!(
        !e.message.contains("SECRET") && !e.message.contains("://"),
        "{}",
        e.message
    );
    assert!(e.hint.as_deref().unwrap().contains("--offline"));
    let core: autorename_core::Error = e.into();
    assert!(core.hint().is_some());
}
