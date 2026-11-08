//! PDF enrichment: Info + XMP + `autorename-pdf.json` attachment, written as
//! one lopdf incremental update appended to the untouched original bytes.
//!
//! - [`plan`] decides eligibility (signed/certified/encrypted/PDF-A/e-invoice/
//!   unparseable/... → [`SkipReason`] with a stable code).
//! - [`apply`] holds an exclusive lock on the PDF, builds the update,
//!   verifies it (original is an exact prefix, lopdf re-parse, PDFium re-open,
//!   our data reads back; no PDFium → skip `verify_unavailable`), writes a
//!   private sibling temp, checks the file's identity (object, length, mtime,
//!   bytes) right before atomically replacing it. Re-runs with identical
//!   content are [`EnrichStatus::Unchanged`].
//! - [`read`] returns our attachment + XMP `arp:` properties (`inspect`).
//! - Undo is [`crate::journal::undo_enrich`] (truncate to `orig_len`).

mod payload;
mod pdf;
mod xmp;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Map, Value};

pub use payload::{
    ATTACHMENT_NAME, Attachment, Content, Extraction, Original, PageText, SCHEMA_VERSION,
};
pub use xmp::NS_ARP;

use crate::fsx::{self, FileId};
use crate::journal::{EnrichEntry, sha256_hex};
use crate::{Error, Result};

/// What the pipeline hands to enrichment (no paths, prompts or secrets).
#[derive(Debug, Clone, PartialEq)]
pub struct EnrichData {
    /// Rendered filename stem → Info `Title` / `dc:title`.
    pub title: String,
    /// e.g. `AP ACME GmbH` → Info `Subject` / `dc:description`.
    pub subject: String,
    /// Info `Keywords` (joined with `; `) / `pdf:Keywords` / `dc:subject`.
    pub keywords: Vec<String>,
    pub profile: String,
    /// Extracted fields as returned by the provider (validated).
    pub fields: Map<String, Value>,
    /// `text` | `ocr` | `vision`
    pub tier: String,
    pub provider: String,
    pub model: String,
    /// Text actually sent, one entry per page used.
    pub pages: Vec<String>,
    /// `max_pages` cut the document.
    pub partial: bool,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Encrypted,
    /// Certification signature (`/Perms /DocMDP`).
    Certified,
    /// Any signature evidence: `/ByteRange`, `/Sig` field or dictionary,
    /// `SigFlags`, usage-rights `/Perms`.
    Signed,
    /// PDF/A (any part): an extra JSON attachment would break conformance.
    PdfA,
    /// ZUGFeRD / Factur-X / XRechnung payload or source/alternative attachment.
    EInvoice,
    /// lopdf had to reconstruct the cross-reference table: an incremental
    /// update would chain onto a broken base.
    XrefRepaired,
    /// Existing XMP packet is not well-formed / has no `rdf:RDF`.
    InvalidXmp,
    ReadOnly,
    Unparseable,
    /// PDFium (the independent verification reader) cannot be loaded.
    VerifyUnavailable,
}

impl SkipReason {
    pub fn code(self) -> &'static str {
        match self {
            SkipReason::Encrypted => "encrypted",
            SkipReason::Certified => "certified",
            SkipReason::Signed => "signed",
            SkipReason::PdfA => "pdf_a",
            SkipReason::EInvoice => "e_invoice",
            SkipReason::XrefRepaired => "xref_repaired",
            SkipReason::InvalidXmp => "invalid_xmp",
            SkipReason::ReadOnly => "read_only",
            SkipReason::Unparseable => "unparseable",
            SkipReason::VerifyUnavailable => "verify_unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "reason")]
pub enum Assessment {
    Eligible,
    Skip(SkipReason),
}

/// Bytes identity of one enrichment, for the undo journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnrichOutcome {
    pub orig_len: u64,
    pub orig_sha256: String,
    pub post_sha256: String,
    /// `durability_warning: …` after a committed change (still journal it).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl EnrichOutcome {
    /// Journal record; `path` = where the file lives after the batch.
    pub fn journal_entry(&self, path: &Path) -> EnrichEntry {
        EnrichEntry {
            path: path.to_path_buf(),
            orig_len: self.orig_len,
            orig_sha256: self.orig_sha256.clone(),
            post_sha256: self.post_sha256.clone(),
            undone: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichStatus {
    Enriched(EnrichOutcome),
    /// Our attachment already carries identical content.
    Unchanged,
    Skipped(SkipReason),
}

impl EnrichStatus {
    /// `enriched` | `unchanged` | `skipped:<reason>` (`enrich_status` in JSON output).
    pub fn code(&self) -> String {
        match self {
            EnrichStatus::Enriched(_) => "enriched".into(),
            EnrichStatus::Unchanged => "unchanged".into(),
            EnrichStatus::Skipped(r) => format!("skipped:{}", r.code()),
        }
    }
}

/// Embedded AutoRename data of a PDF.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmbeddedData {
    pub attachment: Attachment,
    /// XMP `arp:` leaves (`fields.company`, `profile`, …).
    pub xmp: BTreeMap<String, String>,
}

fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| Error::io(path, e))
}

fn is_read_only(path: &Path) -> Result<bool> {
    let m = std::fs::metadata(path).map_err(|e| Error::io(path, e))?;
    Ok(m.permissions().readonly())
}

/// Eligibility only (no writes). I/O errors are errors; everything about the
/// document itself is an [`Assessment`].
pub fn plan(path: &Path) -> Result<Assessment> {
    let bytes = read_bytes(path)?;
    if is_read_only(path)? {
        return Ok(Assessment::Skip(SkipReason::ReadOnly));
    }
    Ok(match pdf::parse(&bytes) {
        Ok(_) => Assessment::Eligible,
        Err(r) => Assessment::Skip(r),
    })
}

fn xmp_props(data: &EnrichData, att: &Attachment) -> xmp::Props {
    let ts = att.timestamp.clone();
    let c = &att.content;
    xmp::Props {
        title: data.title.clone(),
        description: data.subject.clone(),
        keywords: data.keywords.clone(),
        modify_date: data.timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        arp: vec![
            ("schemaVersion", att.schema_version.to_string()),
            (
                "generator",
                format!("{} {}", att.generator, att.generator_version),
            ),
            ("timestamp", ts),
            ("profile", c.profile.clone()),
            ("tier", c.extraction.tier.clone()),
            ("provider", c.extraction.provider.clone()),
            ("model", c.extraction.model.clone()),
            ("pagesUsed", c.extraction.pages_used.to_string()),
            ("partial", c.extraction.partial.to_string()),
            ("attachment", ATTACHMENT_NAME.into()),
            ("contentSha256", att.content_sha256.clone()),
            ("originalSha256", att.original.sha256.clone()),
            ("originalLength", att.original.length.to_string()),
        ],
        fields: c
            .fields
            .iter()
            .filter(|(k, _)| xmp::is_ncname(k))
            .filter_map(|(k, v)| xmp::field_text(v).map(|t| (k.clone(), t)))
            .collect(),
    }
}

fn read_handle(mut f: &File) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    f.seek(SeekFrom::Start(0))?;
    f.read_to_end(&mut out)?;
    Ok(out)
}

/// Independent page counter for verification: `None` = reader unavailable,
/// `Some(Err)` = cannot open the bytes.
type PageCounter = dyn Fn(&[u8]) -> Option<std::result::Result<usize, String>>;

fn pdfium_pages(bytes: &[u8]) -> Option<std::result::Result<usize, String>> {
    crate::extract::pdfium::with_pdfium(|pdfium| {
        pdfium
            .load_pdf_from_byte_slice(bytes, None)
            .map(|d| d.pages().len() as usize)
            .map_err(|e| e.to_string())
    })
    .ok()
}

/// Enrich `path` in place (see module docs). Skips are `Ok`.
pub fn apply(path: &Path, data: &EnrichData) -> Result<EnrichStatus> {
    apply_with(path, data, None, &pdfium_pages)
}

/// [`apply`], refusing ([`Error::Conflict`]) unless the file's bytes hash to
/// `expected_sha256` (e.g. the hash a plan was made from).
pub fn apply_expect(path: &Path, data: &EnrichData, expected_sha256: &str) -> Result<EnrichStatus> {
    apply_with(path, data, Some(expected_sha256), &pdfium_pages)
}

fn apply_with(
    path: &Path,
    data: &EnrichData,
    expect: Option<&str>,
    pages: &PageCounter,
) -> Result<EnrichStatus> {
    let io = |e| Error::io(path, e);
    let file = File::open(path).map_err(io)?;
    let id = fsx::file_id(&file).map_err(io)?;
    let bytes = read_handle(&file).map_err(io)?;
    drop(file);
    let orig_sha256 = sha256_hex(&bytes);
    if expect.is_some_and(|e| e != orig_sha256) {
        return Err(Error::Conflict {
            message: format!(
                "{} changed since it was planned; not enriched",
                path.display()
            ),
        });
    }
    if std::fs::metadata(path)
        .map_err(io)?
        .permissions()
        .readonly()
    {
        return Ok(EnrichStatus::Skipped(SkipReason::ReadOnly));
    }
    let parsed = match pdf::parse(&bytes) {
        Ok(p) => p,
        Err(r) => return Ok(EnrichStatus::Skipped(r)),
    };
    let content = Content::from_data(data);
    let content_sha256 = content.sha256();
    let original = match &parsed.ours {
        Some(o) if o.attachment.content_sha256 == content_sha256 => {
            return Ok(EnrichStatus::Unchanged);
        }
        // Keep the pre-AutoRename identity while it is still a true prefix.
        Some(o)
            if usize::try_from(o.attachment.original.length)
                .ok()
                .filter(|&n| n <= bytes.len())
                .is_some_and(|n| sha256_hex(&bytes[..n]) == o.attachment.original.sha256) =>
        {
            o.attachment.original.clone()
        }
        _ => Original {
            sha256: sha256_hex(&bytes),
            length: bytes.len() as u64,
        },
    };
    let attachment = Attachment {
        schema_version: SCHEMA_VERSION,
        generator: payload::GENERATOR.into(),
        generator_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: data.timestamp.to_rfc3339_opts(SecondsFormat::Secs, true),
        content_sha256: content_sha256.clone(),
        original,
        content,
    };
    let json = attachment.to_json();
    let desc = xmp::description(&xmp_props(data, &attachment));
    let keywords = data.keywords.join("; ");
    let page_count = parsed.page_count;
    let out = pdf::build(
        &bytes,
        parsed,
        &pdf::Update {
            title: &data.title,
            subject: &data.subject,
            keywords: &keywords,
            timestamp: data.timestamp,
            json: &json,
            xmp_desc: &desc,
        },
    )
    .map_err(|e| Error::General(format!("{}: enrichment failed: {e}", path.display())))?;
    match verify(&bytes, &out, &content_sha256, page_count, pages) {
        Ok(()) => {}
        Err(VerifyError::Unavailable) => {
            return Ok(EnrichStatus::Skipped(SkipReason::VerifyUnavailable));
        }
        Err(VerifyError::Failed(m)) => {
            return Err(Error::General(format!(
                "{}: enrichment verification failed: {m}",
                path.display()
            )));
        }
    }
    let done = commit(path, id, &orig_sha256, &out)?;
    Ok(EnrichStatus::Enriched(EnrichOutcome {
        orig_len: bytes.len() as u64,
        orig_sha256: sha256_hex(&bytes),
        post_sha256: sha256_hex(&out),
        warnings: done.warnings,
    }))
}

enum VerifyError {
    /// No independent reader: never commit unverified output.
    Unavailable,
    Failed(String),
}

impl From<String> for VerifyError {
    fn from(m: String) -> Self {
        VerifyError::Failed(m)
    }
}

impl From<&str> for VerifyError {
    fn from(m: &str) -> Self {
        VerifyError::Failed(m.into())
    }
}

fn verify(
    orig: &[u8],
    out: &[u8],
    content_sha256: &str,
    page_count: usize,
    pages: &PageCounter,
) -> std::result::Result<(), VerifyError> {
    if out.len() <= orig.len() || !out.starts_with(orig) {
        return Err("original bytes are not a prefix of the result".into());
    }
    let p =
        pdf::parse(out).map_err(|r| format!("result does not re-parse cleanly ({})", r.code()))?;
    if p.page_count != page_count {
        return Err(format!("page count changed {page_count} → {}", p.page_count).into());
    }
    match &p.ours {
        Some(o) if o.attachment.content_sha256 == content_sha256 => {}
        _ => return Err("attachment does not read back".into()),
    }
    if !p.xmp.as_deref().is_some_and(|x| x.contains(NS_ARP)) {
        return Err("XMP does not read back".into());
    }
    // Independent reader; required.
    match pages(out) {
        Some(Ok(n)) if n == page_count => Ok(()),
        Some(Ok(n)) => Err(format!("PDFium sees {n} pages, expected {page_count}").into()),
        Some(Err(e)) => Err(format!("PDFium cannot open the result: {e}").into()),
        None => Err(VerifyError::Unavailable),
    }
}

/// Private temp sibling (original permissions applied, fsynced) →
/// [`fsx::replace_verified`]: a concurrent save by another program (new
/// object, length, mtime or bytes) is a conflict and is kept.
fn commit(path: &Path, id: FileId, orig_sha256: &str, out: &[u8]) -> Result<fsx::Committed> {
    let io = |e| Error::io(path, e);
    let perms = std::fs::metadata(path).map_err(io)?.permissions();
    let tmp = fsx::write_temp(fsx::parent_dir(path), out, Some(perms)).map_err(io)?;
    fsx::replace_verified(path, tmp, id, orig_sha256)
}

/// Our embedded data, if any. Unloadable PDFs are an error; PDFs we would
/// skip (signed, …) are still read.
pub fn read(path: &Path) -> Result<Option<EmbeddedData>> {
    let bytes = read_bytes(path)?;
    let parsed = match pdf::parse(&bytes) {
        Ok(p) => p,
        Err(SkipReason::Encrypted | SkipReason::Unparseable) => {
            return Err(Error::General(format!(
                "{}: cannot read PDF structure",
                path.display()
            )));
        }
        // Skip reasons do not prevent reading; fall back to a plain load.
        Err(_) => return read_lenient(&bytes),
    };
    Ok(parsed.ours.map(|o| EmbeddedData {
        attachment: o.attachment,
        xmp: parsed.xmp.as_deref().map(xmp::read_arp).unwrap_or_default(),
    }))
}

fn read_lenient(bytes: &[u8]) -> Result<Option<EmbeddedData>> {
    pdf::read_lenient(bytes)
        .map(|r| {
            r.map(|(attachment, packet)| EmbeddedData {
                attachment,
                xmp: packet.as_deref().map(xmp::read_arp).unwrap_or_default(),
            })
        })
        .map_err(|e| Error::General(format!("cannot read PDF structure: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> std::path::PathBuf {
        let src =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/minimal_text.pdf");
        let p = dir.join("doc.pdf");
        std::fs::copy(src, &p).unwrap();
        p
    }

    fn data() -> EnrichData {
        EnrichData {
            title: "t".into(),
            subject: "s".into(),
            keywords: vec![],
            profile: "business".into(),
            fields: Map::new(),
            tier: "text".into(),
            provider: "p".into(),
            model: "m".into(),
            pages: vec![],
            partial: false,
            timestamp: "2026-11-07T10:11:12Z".parse().unwrap(),
        }
    }

    #[test]
    fn unavailable_or_disagreeing_verifier_never_commits() {
        let d = tempfile::tempdir().unwrap();
        let p = sample(d.path());
        let before = std::fs::read(&p).unwrap();
        let status = apply_with(&p, &data(), None, &|_| None).unwrap();
        assert_eq!(status, EnrichStatus::Skipped(SkipReason::VerifyUnavailable));
        assert_eq!(status.code(), "skipped:verify_unavailable");
        assert!(apply_with(&p, &data(), None, &|_| Some(Ok(99))).is_err());
        assert!(apply_with(&p, &data(), None, &|_| Some(Err("broken".into()))).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), before, "untouched");
        assert_eq!(
            std::fs::read_dir(d.path()).unwrap().count(),
            1,
            "no temp left"
        );
    }

    #[test]
    fn commit_keeps_a_concurrent_save_and_replaces_only_the_checked_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("doc.pdf");
        std::fs::write(&p, b"orig").unwrap();
        let sha = sha256_hex(b"orig");
        let id = fsx::path_id(&p).unwrap();
        // An editor's atomic save (new object) between read and commit.
        fsx::write_atomic(&p, b"EDITOR SAVE").unwrap();
        let e = commit(&p, id, &sha, b"orig+update").unwrap_err();
        assert_eq!(e.code(), "conflict");
        assert_eq!(std::fs::read(&p).unwrap(), b"EDITOR SAVE");
        // An in-place write (same object, new bytes) is caught as well.
        let id = fsx::path_id(&p).unwrap();
        std::fs::write(&p, b"EDITOR SAVE 2").unwrap();
        let e = commit(&p, id, &sha256_hex(b"EDITOR SAVE"), b"x").unwrap_err();
        assert_eq!(e.code(), "conflict");
        assert_eq!(std::fs::read(&p).unwrap(), b"EDITOR SAVE 2");
        // Unchanged → replaced; no temp/backup files left behind.
        let id = fsx::path_id(&p).unwrap();
        commit(&p, id, &sha256_hex(b"EDITOR SAVE 2"), b"orig+update").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"orig+update");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn apply_expect_refuses_unplanned_bytes() {
        let d = tempfile::tempdir().unwrap();
        let p = sample(d.path());
        let e = apply_with(&p, &data(), Some("0000"), &|_| Some(Ok(1))).unwrap_err();
        assert_eq!(e.code(), "conflict");
    }
}
