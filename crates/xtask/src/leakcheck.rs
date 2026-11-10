//! `cargo xtask leakcheck`: release gate against leaking secrets or data from
//! the real test documents. It scans what a push or a release makes public:
//! the lines each commit of the range adds (every commit, not only the
//! final diff: a line added and later removed stays in history), the commit
//! messages and extra files (release notes, PR body).
//!
//! - Secrets (always, also in CI): provider/GitHub/AWS keys and private key
//!   blocks; placeholders such as `sk-your-key` or `sk-1234567890…` pass.
//! - Private data (when `.todo/real-pdfs/` exists, i.e. locally): IBANs,
//!   VAT IDs, e-mail addresses, phone numbers and reference numbers found in
//!   the real PDFs (`pdftotext`) and their file names, customer names from
//!   accepted `YYYYMMDD <Customer> AR …` file names, and every line of the
//!   git-ignored `.todo/sensitive.txt`. A term that the base of the range
//!   already contains (e.g. the imprint in the About page) is public already
//!   and does not count.
//!
//! Matches are printed redacted (`We…tl`, 13 chars), so CI logs stay clean.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use regex::Regex;

pub struct Opts {
    pub range: Option<String>,
    pub files: Vec<String>,
}

impl Opts {
    pub fn parse(args: &[String]) -> Option<Opts> {
        let mut o = Opts {
            range: None,
            files: Vec::new(),
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--range" => o.range = Some(it.next()?.clone()),
                "--file" => o.files.push(it.next()?.clone()),
                _ => return None,
            }
        }
        Some(o)
    }
}

/// One finding: where, which detector, the redacted match.
#[derive(Debug, PartialEq)]
pub struct Hit {
    pub at: String,
    pub kind: &'static str,
    pub shown: String,
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `ab…yz` plus the length: enough to find it, nothing to copy.
pub fn redact(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= 6 {
        return format!("{}… ({} chars)", c.first().copied().unwrap_or(' '), c.len());
    }
    let head: String = c[..2].iter().collect();
    let tail: String = c[c.len() - 2..].iter().collect();
    format!("{head}…{tail} ({} chars)", c.len())
}

fn secret_patterns() -> Vec<(&'static str, Regex)> {
    [
        ("openai key", r"sk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}"),
        ("anthropic key", r"sk-ant-[A-Za-z0-9_-]{20,}"),
        ("google key", r"AIza[0-9A-Za-z_-]{35}"),
        ("xai key", r"xai-[A-Za-z0-9]{20,}"),
        ("github token", r"gh[pousr]_[A-Za-z0-9]{36,}"),
        ("aws key", r"AKIA[0-9A-Z]{16}"),
        ("private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    ]
    .into_iter()
    .map(|(k, p)| (k, Regex::new(p).expect("valid pattern")))
    .collect()
}

/// Test keys and examples: `sk-your-…`, `sk-test-…`, runs like `1234567890`.
fn placeholder(m: &str) -> bool {
    let l = m.to_lowercase();
    [
        "your",
        "example",
        "placeholder",
        "dummy",
        "test",
        "fake",
        "xxxx",
        "1234567890",
        "abcdef",
    ]
    .iter()
    .any(|p| l.contains(p))
}

/// Secrets in `text` (one line or a whole file).
pub fn secrets(text: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (kind, re) in secret_patterns() {
        for m in re.find_iter(text) {
            if kind == "private key" || !placeholder(m.as_str()) {
                out.push((kind, m.as_str().to_string()));
            }
        }
    }
    out
}

/// Identifiers in document text: IBAN, VAT ID, e-mail, phone, reference
/// numbers (letters/digits with `-` or `/`, at least four digits), long
/// digit runs.
pub fn identifiers(text: &str) -> BTreeSet<String> {
    let pats = [
        r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){3,7}(?: ?[A-Z0-9]{1,3})?\b",
        r"\b(?:ATU\d{8}|DE\d{9}|[A-Z]{2}\d{8,12})\b",
        r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
        r"\+\d{1,3}[\d /()-]{7,}\d",
        r"\b[A-Z0-9]+(?:[-/][A-Z0-9]+)+\b",
        r"\b\d{8,}\b",
    ];
    // Dates and periods (2026-11-10, 11/2025, 20261031) are no identifiers.
    let date_like = Regex::new(
        r"^(?:\d{4}[-/.]\d{1,2}(?:[-/.]\d{1,2})?|\d{1,2}[-/.]\d{1,2}[-/.]\d{2,4}|\d{1,2}[-/.]\d{4}|(?:19|20)\d{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12]\d|3[01]))$",
    )
    .expect("valid");
    let mut out = BTreeSet::new();
    for p in pats {
        let re = Regex::new(p).expect("valid pattern");
        for m in re.find_iter(text) {
            let t = m.as_str().trim().to_string();
            let digits = t.chars().filter(char::is_ascii_digit).count();
            if (digits >= 4 || t.contains('@')) && !date_like.is_match(&t) && t.len() >= 6 {
                out.insert(t);
            }
        }
    }
    out
}

/// Customer names from accepted outgoing-invoice file names
/// (`20261102 Example Customer AR INV-1 12,13.pdf` → `Example Customer`).
pub fn customers(file_names: &[String]) -> BTreeSet<String> {
    let re = Regex::new(r"^\d{8} (.+?) AR\b").expect("valid pattern");
    file_names
        .iter()
        .filter_map(|n| re.captures(n).map(|c| c[1].trim().to_string()))
        .filter(|n| n.chars().count() >= 4)
        .collect()
}

/// Private terms from `.todo/` (empty when the folder is missing).
fn private_terms(root: &Path) -> Result<BTreeSet<String>, String> {
    let dir = root.join(".todo/real-pdfs");
    let mut terms = BTreeSet::new();
    if let Ok(list) = std::fs::read_to_string(root.join(".todo/sensitive.txt")) {
        terms.extend(
            list.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from),
        );
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(terms);
    };
    let mut names = Vec::new();
    let mut pdftotext_missing = false;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("pdf") {
            continue;
        }
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        terms.extend(identifiers(&name));
        names.push(name);
        match Command::new("pdftotext").arg(&p).arg("-").output() {
            Ok(o) if o.status.success() => {
                terms.extend(identifiers(&String::from_utf8_lossy(&o.stdout)))
            }
            Ok(_) => {}
            Err(_) => pdftotext_missing = true,
        }
    }
    if pdftotext_missing {
        eprintln!(
            "warning: pdftotext not found: PDF text not scanned (file names and sensitive.txt are)"
        );
    }
    terms.extend(customers(&names));
    Ok(terms)
}

/// Terms the base tree already contains are public already.
fn drop_public(root: &Path, base: &str, terms: BTreeSet<String>) -> BTreeSet<String> {
    terms
        .into_iter()
        .filter(|t| {
            let st = Command::new("git")
                .current_dir(root)
                .args(["grep", "-q", "-F", "-i", "-e", t, base, "--"])
                .status();
            !matches!(st, Ok(s) if s.success())
        })
        .collect()
}

/// Check `text` (from `at`) against the secret patterns and `terms`.
pub fn scan(at: &str, text: &str, terms: &BTreeSet<String>, hits: &mut Vec<Hit>) {
    for (kind, m) in secrets(text) {
        hits.push(Hit {
            at: at.to_string(),
            kind,
            shown: redact(&m),
        });
    }
    let lower = text.to_lowercase();
    let squashed: String = lower.split_whitespace().collect();
    for t in terms {
        let tl = t.to_lowercase();
        let ts: String = tl.split_whitespace().collect();
        if lower.contains(&tl) || (ts.len() >= 8 && squashed.contains(&ts)) {
            hits.push(Hit {
                at: at.to_string(),
                kind: "private data",
                shown: redact(t),
            });
        }
    }
}

fn default_range(root: &Path) -> String {
    match git(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]) {
        Ok(u) => format!("{}..HEAD", u.trim()),
        Err(_) => "origin/main..HEAD".into(),
    }
}

pub fn run(root: &Path, o: &Opts) -> Result<(), String> {
    let range = o.range.clone().unwrap_or_else(|| default_range(root));
    let base = range.split("..").next().unwrap_or("HEAD").to_string();
    let terms = private_terms(root)?;
    let n_terms = terms.len();
    let terms = drop_public(root, &base, terms);
    let mut hits = Vec::new();
    // Every commit's own added lines and message (`git log -p`).
    let log = git(
        root,
        &[
            "log",
            "-p",
            "--no-color",
            "--no-ext-diff",
            "--format=@@commit %h%n%B",
            &range,
        ],
    )?;
    let mut commit = String::new();
    let mut file = String::new();
    for line in log.lines() {
        if let Some(c) = line.strip_prefix("@@commit ") {
            commit = c.to_string();
            file = "(message)".into();
        } else if let Some(f) = line.strip_prefix("+++ b/") {
            file = f.to_string();
        } else if file == "(message)" && !line.starts_with("diff --git") {
            scan(&format!("{commit} message"), line, &terms, &mut hits);
        } else if let Some(added) = line.strip_prefix('+') {
            if !added.starts_with("++") {
                scan(&format!("{commit} {file}"), added, &terms, &mut hits);
            }
        } else if line.starts_with("diff --git") {
            file.clear();
        }
    }
    for f in &o.files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
        scan(f, &text, &terms, &mut hits);
    }
    let commits = git(root, &["rev-list", "--count", &range])?
        .trim()
        .to_string();
    println!(
        "leakcheck {range}: {commits} commits, {} extra files, {} private terms ({} already public)",
        o.files.len(),
        terms.len(),
        n_terms - terms.len()
    );
    if hits.is_empty() {
        println!("no secrets or private data found");
        return Ok(());
    }
    for h in &hits {
        println!("  {}  {}: {}", h.at, h.kind, h.shown);
    }
    Err(format!(
        "{} finding(s): remove them before pushing or publishing (history: known.md, filter-repo with --refs <base>..main)",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_but_not_placeholders() {
        // Built at run time so this file itself passes the gate.
        let real = format!("key = sk-proj-{}", "Q7wLx9Vb2KmN4pRs8TuYz3AcDeFgHiJk");
        assert_eq!(secrets(&real).len(), 1);
        for fake in [
            "OPENAI_API_KEY=sk-your-key-here-0000000000000",
            "ai: {api_key: sk-1234567890abcdef}",
            "sk-test-1234567890abcdefghij",
        ] {
            assert!(secrets(fake).is_empty(), "{fake}");
        }
        let key = format!("-----BEGIN {} PRIVATE KEY-----", "OPENSSH");
        assert_eq!(secrets(&key).len(), 1);
    }

    #[test]
    fn identifiers_from_documents() {
        let ids = identifiers(
            "IBAN AT61 1904 3002 3457 3201 · UID ATU12345678 · billing@example.org · Tel +43 316 123456 · Rechnung RE-2099-001-07 vom 2026-11-02 · Kd.-Nr. 4815162342",
        );
        for want in [
            "AT61 1904 3002 3457 3201",
            "ATU12345678",
            "billing@example.org",
            "RE-2099-001-07",
            "4815162342",
        ] {
            assert!(ids.contains(want), "{want} in {ids:?}");
        }
        for date in ["2026-11-02", "11/2025", "20261031"] {
            assert!(!identifiers(date).contains(date), "{date} is a date");
        }
        assert!(!ids.contains("2026-11-02"));
        assert!(ids.iter().any(|i| i.starts_with("+43 316")));
    }

    #[test]
    fn customers_and_redacted_hits() {
        let names = [
            "20261102 Example Customer AR INV-1 12,13.pdf".to_string(),
            "20240311 Fuel Station ER 92,86.pdf".to_string(),
        ];
        assert_eq!(
            customers(&names),
            BTreeSet::from(["Example Customer".to_string()])
        );
        let terms = BTreeSet::from([
            "Example Customer".to_string(),
            "AT61 1904 3002 3457 3201".to_string(),
        ]);
        let mut hits = Vec::new();
        scan("abc README.md", "to example   customer", &terms, &mut hits);
        assert_eq!(
            hits.len(),
            1,
            "case and spacing do not hide a name: {hits:?}"
        );
        let mut hits = Vec::new();
        scan(
            "abc README.md",
            "invoice to Example Customer, IBAN AT611904300234573201",
            &terms,
            &mut hits,
        );
        assert_eq!(
            hits.len(),
            2,
            "an IBAN without spaces matches too: {hits:?}"
        );
        assert!(!hits[0].shown.contains("Customer"));
        assert_eq!(redact("Example Customer"), "Ex…er (16 chars)");
    }
}
