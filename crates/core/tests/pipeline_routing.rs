//! Page routing and review warnings through `pipeline::plan_files`
//! (business profile): one AI request per document; the visual cap sends a
//! logo page to vision when OCR models are missing. Local Ollama mock (std
//! `TcpListener`, `Connection: close`); needs PDFium like the extraction
//! tests.

#[allow(dead_code)]
#[path = "common/pdf.rs"]
mod testpdf;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use autorename_core::config::{self, Settings};
use autorename_core::extract::Tier;
use autorename_core::pipeline::{self, PlanItem, PlanOptions};
use serde_json::{Value, json};
use testpdf::{Extra, INVOICE, page_pdf};

const ME: &str = "Petermeir Digital Solutions e.U.";

type Responder = Arc<dyn Fn(&Value) -> (u16, String) + Send + Sync>;

struct Mock {
    url: String,
    log: Arc<Mutex<Vec<Value>>>,
}

impl Mock {
    fn requests(&self) -> Vec<Value> {
        self.log.lock().unwrap().clone()
    }
}

/// Serve forever; `respond(request body)` → (status, body).
fn serve(respond: Responder) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() {
                    break;
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
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let (status, text) = respond(&body);
            sink.lock().unwrap().push(body);
            let out = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            let mut stream = reader.into_inner();
            let _ = stream.write_all(out.as_bytes());
            let _ = stream.flush();
        }
    });
    Mock { url, log }
}

/// Ollama `/api/chat` reply carrying the business fields.
fn reply(company: &str, issuer: &str, recipient: &str, doc_type: &str) -> String {
    let content = json!({
        "document_date": "15.03.2024",
        "company_name": company,
        "issuer": issuer,
        "recipient": recipient,
        "document_type": doc_type,
        "total_amount": "4,258.80",
        "currency": "EUR",
        "description": ""
    });
    json!({
        "model": "gemma4",
        "message": {"role": "assistant", "content": content.to_string()},
        "done": true,
        "done_reason": "stop"
    })
    .to_string()
}

fn images_in(body: &Value) -> usize {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["images"].as_array())
        .map(Vec::len)
        .sum()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.pdf"))
}

fn settings(url: &str, vision: Value, threshold: f64) -> Settings {
    let raw = json!({
        "config_version": 4,
        "ai": {"provider": "ollama", "model": "gemma4", "base_url": url, "max_retries": 0},
        "pdf": {"ocr": "auto", "vision": vision, "text_quality_threshold": threshold},
        "company": {"names": [ME]},
        "enrich": {"enabled": false},
        "harmonization": {"decider": "off"}
    });
    let (tree, _) = config::from_raw(raw, &|_| None).unwrap();
    config::settings(&tree, None).unwrap()
}

/// Plan one PDF with an empty OCR models dir (no local OCR).
fn plan(pdf: &Path, s: &Settings) -> PlanItem {
    let models = tempfile::tempdir().unwrap();
    let opts = PlanOptions {
        jobs: 1,
        models_dir: Some(models.path().to_path_buf()),
        ..PlanOptions::default()
    };
    let p = pipeline::plan_files(&[pdf.to_path_buf()], s, &opts, &|_| {}).unwrap();
    p.items.into_iter().next().unwrap()
}

fn has(item: &PlanItem, prefix: &str) -> bool {
    item.warnings.iter().any(|w| w.starts_with(prefix))
}

/// No counterparty in the answer: a review warning, no second request,
/// nothing learned.
#[test]
fn missing_counterparty_is_flagged_for_review() {
    let mock = serve(Arc::new(|_| (200, reply("", "", ME, "AP"))));
    let item = plan(
        &fixture("text_invoice_acme"),
        &settings(&mock.url, json!("auto"), 0.8),
    );
    assert_eq!(mock.requests().len(), 1);
    assert_eq!(images_in(&mock.requests()[0]), 0);
    assert!(item.error.is_none(), "{:?}", item.error);
    assert!(has(&item, "counterparty_missing"), "{:?}", item.warnings);
    assert!(item.learn.is_none());
    assert_eq!(item.tier, Some(Tier::Text));
}

/// My company as issuer with a recipient is a normal outgoing invoice; the
/// model's counterparty (no legal form) is kept over the printed recipient.
#[test]
fn outgoing_invoice_needs_no_review() {
    let mock = serve(Arc::new(|_| {
        (
            200,
            reply("Wayne Enterprises", ME, "Wayne Enterprises Inc.", "AR"),
        )
    }));
    let item = plan(
        &fixture("text_outgoing_invoice_wayne"),
        &settings(&mock.url, json!("auto"), 0.8),
    );
    assert_eq!(mock.requests().len(), 1);
    assert_eq!(item.company.as_deref(), Some("Wayne Enterprises"));
    assert_eq!(item.fields["document_type"], "AR");
    assert!(item.warnings.is_empty(), "{:?}", item.warnings);
    assert_eq!(item.tier, Some(Tier::Text));
}

/// A logo page scores 0.75. Without OCR models its clean text layer is
/// sent as text (no image) at any limit, with `ocr_models_missing` at 0.8.
/// The plan records the page's score and visual flag.
#[test]
fn logo_page_stays_text_without_ocr_models() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = page_pdf(
        dir.path(),
        "logo",
        INVOICE,
        &[Extra::RasterLogo("Zenith Robotics")],
    );
    let mock = serve(Arc::new(|_| {
        (
            200,
            reply("Zenith Robotics", "Zenith Robotics GmbH", ME, "AP"),
        )
    }));
    let item = plan(&pdf, &settings(&mock.url, json!("auto"), 0.8));
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(images_in(&reqs[0]), 0);
    assert_eq!(item.tier, Some(Tier::Text));
    assert_eq!(item.quality, Some(0.75));
    let pq = &item.page_quality[0];
    assert_eq!(
        (pq.text, pq.visual, pq.ocr, pq.image),
        (0.75, true, None, false)
    );
    if autorename_core::ocr::ENGINE_AVAILABLE {
        assert!(has(&item, "ocr_models_missing"), "{:?}", item.warnings);
    }
    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["page_quality"][0]["visual"], true);

    let mock = serve(Arc::new(|_| {
        (
            200,
            reply("Zenith Robotics", "Zenith Robotics GmbH", ME, "AP"),
        )
    }));
    let item = plan(&pdf, &settings(&mock.url, json!("auto"), 0.75));
    assert_eq!(images_in(&mock.requests()[0]), 0);
    assert_eq!(item.tier, Some(Tier::Text));
    assert!(item.warnings.is_empty(), "{:?}", item.warnings);
}
