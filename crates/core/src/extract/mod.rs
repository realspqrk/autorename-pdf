//! Content extraction pipeline (behavior contract §4, rebuild spec §Pipeline 1–3).
//!
//! Everything is decided **per page** (the first `pdf.max_pages` pages),
//! against one limit `L` (`pdf.text_quality_threshold`):
//!
//! 1. Text (always): PDFium → pdfplumber-identical text, NFC, mojibake
//!    repair, then readability `R` ([`quality::page_quality`]). A page with a
//!    significant picture or drawing ([`visual::scan_page`]) scores at most
//!    [`visual::VISUAL_CAP`]: `Qtext = min(R, 0.75)`, else `Qtext = R`.
//! 2. OCR: `on` ⇒ every page, `off` ⇒ none, `auto` ⇒ pages with
//!    `Qtext < L` (strict). A successful OCR sets the page's result
//!    `Q = R(full OCR text)` (0 when empty; no cap, no max with `Qtext`).
//!    Failed or unavailable OCR keeps `Qtext` and adds a warning. The OCR
//!    evidence of a page is every non-blank line when `R < L`, else only the
//!    lines that its text layer does not already have ([`image_only_lines`]).
//! 3. Vision: `on` ⇒ every page image, `off` ⇒ none, `auto` ⇒ pages whose
//!    result `Q` is still `< L`.
//!
//! Pages are rendered once at [`render::RENDER_SCALE`], only when OCR or
//! vision may need them; PNGs are encoded only for the pages that go to the
//! model. OCR runs offline and never downloads: missing models are an error
//! for `ocr: on`, a warning (`ocr_models_missing: …`) for `auto`. Vision
//! only *prepares* PNGs; the provider call happens later (offline: the
//! local Ollama model).
//!
//! The document summary `quality` is the worst page's result `Q`.

pub mod mojibake;
pub mod pdfium;
pub mod quality;
pub mod render;
pub mod text;
pub mod visual;

use std::path::{Path, PathBuf};

use pdfium_render::prelude::*;
use serde::Serialize;

use crate::config::{Settings, Toggle};
use crate::ocr;
use crate::{Error, Result};

const OCR_HEADER: &str = "--- OCR Text ---";

/// Inputs of the pipeline (decoupled from the config tree).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractConfig {
    pub max_pages: usize,
    pub ocr: Toggle,
    pub vision: Toggle,
    pub text_quality_threshold: f64,
    /// `ocr.language`: `auto` (installed packs) or a code → its pack.
    pub ocr_language: String,
    pub ocr_threads: Option<usize>,
    /// Models root; `None` ⇒ [`ocr::models::default_root`].
    pub models_dir: Option<PathBuf>,
}

impl ExtractConfig {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            max_pages: s.pdf.max_pages as usize,
            ocr: s.pdf.ocr,
            vision: s.pdf.vision,
            text_quality_threshold: s.pdf.text_quality_threshold,
            ocr_language: s.ocr.language.clone(),
            ocr_threads: (s.ocr.threads > 0).then_some(s.ocr.threads as usize),
            models_dir: None,
        }
    }
}

impl Default for ExtractConfig {
    /// Config defaults.
    fn default() -> Self {
        Self {
            max_pages: 3,
            ocr: Toggle::Auto,
            vision: Toggle::Auto,
            text_quality_threshold: 0.8,
            ocr_language: ocr::AUTO_LANGUAGE.into(),
            ocr_threads: None,
            models_dir: None,
        }
    }
}

/// Highest tier that contributed content.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Text,
    Ocr,
    /// Page images prepared; the AI call must be multimodal.
    VisionNeeded,
}

/// One selected page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    /// 1-based page number in the PDF.
    pub number: usize,
    /// Text layer (NFC, mojibake-repaired); empty when blank or failed.
    pub text: String,
    /// Readability `R` of `text` ([`quality::page_quality`]).
    pub text_quality: f64,
    /// The page shows a significant picture or drawing ([`visual`]).
    pub visual: bool,
    /// Full local OCR text; `None` when OCR did not run (or failed).
    pub ocr_text: Option<String>,
    /// Readability of the full OCR text, when OCR ran.
    pub ocr_quality: Option<f64>,
    /// OCR lines added to the AI evidence for this page (see module docs).
    pub ocr_evidence: String,
    /// Result `Q`: `ocr_quality` after a successful OCR, else [`Page::text_score`].
    pub quality: f64,
    /// The page image goes to the model.
    pub image: bool,
    /// Text, render or OCR failures of this page.
    pub errors: Vec<String>,
}

impl Page {
    /// `Qtext`: readability, capped at [`visual::VISUAL_CAP`] when `visual`.
    pub fn text_score(&self) -> f64 {
        visual::capped(self.text_quality, self.visual)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Extraction {
    /// Tier-1 text (`Page N:` blocks of non-blank pages).
    pub text: String,
    /// OCR evidence (`Page N:` blocks of pages with OCR lines), empty if none.
    pub ocr_text: String,
    pub tier: Tier,
    /// Worst page's result `Q` ([`quality::document_quality`]).
    pub quality: f64,
    pub page_count: usize,
    /// `min(max_pages, page_count)`.
    pub pages_used: usize,
    /// Per selected page, in page order.
    pub pages: Vec<Page>,
    /// PNG per page with `image: true`, in page order.
    #[serde(skip)]
    pub page_images: Vec<Vec<u8>>,
    /// `text`, `ocr`, `vision` in the order they contributed.
    pub sources: Vec<&'static str>,
    pub warnings: Vec<String>,
}

/// Non-blank `(header, body)` parts joined by blank lines. The text layer
/// has no header; OCR text without a text layer before it has none either
/// (§4.7).
fn join_blocks(parts: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (header, body) in parts.iter().filter(|(_, b)| !quality::is_blank(b)) {
        let first = out.is_empty();
        if !first {
            out.push_str("\n\n");
        }
        if !header.is_empty() && !(first && *header == OCR_HEADER) {
            out.push_str(header);
            out.push_str("\n\n");
        }
        out.push_str(body);
    }
    out
}

impl Extraction {
    /// Text handed to the AI (§4.7): text, then OCR evidence under a separator.
    pub fn combined_text(&self) -> String {
        join_blocks(&[("", &self.text), (OCR_HEADER, &self.ocr_text)])
    }

    /// Per used page: the text the AI saw for it (embedded on enrichment).
    pub fn page_texts(&self) -> Vec<String> {
        (1..=self.pages_used)
            .map(|n| {
                let Some(p) = self.pages.iter().find(|p| p.number == n) else {
                    return String::new();
                };
                join_blocks(&[("", &p.text), (OCR_HEADER, &p.ocr_evidence)])
            })
            .collect()
    }

    /// Content gate (§4.7): anything for the AI to look at.
    pub fn has_content(&self) -> bool {
        !self.combined_text().is_empty() || !self.page_images.is_empty()
    }
}

/// Auto-routing decision (§4.3).
pub fn should_run(setting: Toggle, quality: f64, threshold: f64) -> bool {
    match setting {
        Toggle::On => true,
        Toggle::Off => false,
        Toggle::Auto => quality < threshold,
    }
}

/// Extract with settings from the loaded config.
pub fn extract_document(path: &Path, settings: &Settings) -> Result<Extraction> {
    extract_with(path, &ExtractConfig::from_settings(settings))
}

fn resolve_ocr(cfg: &ExtractConfig) -> Result<ocr::PackPaths> {
    if !ocr::ENGINE_AVAILABLE {
        return Err(Error::Extract {
            code: "ocr_unavailable",
            message: "This build has no OCR engine".into(),
            hint: Some(
                "Disable OCR (pdf.ocr: false) or use a build with the `ocr` feature.".into(),
            ),
        });
    }
    let root = cfg
        .models_dir
        .clone()
        .or_else(ocr::models::default_root)
        .ok_or_else(|| Error::General("No data directory for OCR models".into()))?;
    let pack = ocr::pack_for_setting(&cfg.ocr_language, &root)?;
    ocr::models::require(&root, pack)
}

/// Per-page plan made under the PDFium lock.
struct Routed {
    page: Page,
    /// Rendered image when OCR or vision may need it.
    image: Option<image::RgbImage>,
    ocr: bool,
    vision_candidate: bool,
}

struct PdfStage {
    pages: Vec<Routed>,
    page_count: usize,
    warnings: Vec<String>,
    ocr_paths: Option<ocr::PackPaths>,
}

fn load<'a>(p: &'a Pdfium, bytes: &'a [u8], warnings: &mut Vec<String>) -> Option<PdfDocument<'a>> {
    match p.load_pdf_from_byte_slice(bytes, None) {
        Ok(d) => Some(d),
        Err(e) => {
            warnings.push(format!("Could not open PDF: {e}"));
            None
        }
    }
}

/// Visual scan of page `n` (1-based); a failed lookup counts as visual.
fn scan(doc: Option<&PdfDocument>, n: usize) -> visual::Scan {
    match doc.map(|d| d.pages().get((n - 1) as PdfPageIndex)) {
        Some(Ok(pg)) => visual::scan_page(&pg),
        Some(Err(e)) => visual::Scan {
            visual: true,
            warning: Some(format!("{e:?}")),
        },
        None => visual::Scan {
            visual: false,
            warning: None,
        },
    }
}

/// Full pipeline with explicit inputs.
pub fn extract_with(path: &Path, cfg: &ExtractConfig) -> Result<Extraction> {
    // Read ourselves: avoids non-ASCII path issues in native libraries.
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let thr = cfg.text_quality_threshold;
    let stage = pdfium::with_pdfium(|p| -> Result<PdfStage> {
        let mut warnings = Vec::new();
        let doc = load(p, &bytes, &mut warnings);
        let page_count = doc.as_ref().map_or(0, |d| d.pages().len().max(0) as usize);
        let texts = doc
            .as_ref()
            .map(|d| text::document_pages(d, cfg.max_pages))
            .unwrap_or_default();
        let mut pages: Vec<Routed> = texts
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let (text, errors) = match t {
                    Ok(t) => (t, Vec::new()),
                    Err(w) => (String::new(), vec![w]),
                };
                let sc = scan(doc.as_ref(), i + 1);
                if let Some(w) = sc.warning {
                    warnings.push(format!(
                        "visual_scan_incomplete: page {}: {w}; counted as a page with pictures",
                        i + 1
                    ));
                }
                let r = quality::page_quality(&text);
                let q = visual::capped(r, sc.visual);
                let ocr = should_run(cfg.ocr, q, thr);
                Routed {
                    page: Page {
                        number: i + 1,
                        text,
                        text_quality: r,
                        visual: sc.visual,
                        ocr_text: None,
                        ocr_quality: None,
                        ocr_evidence: String::new(),
                        quality: q,
                        image: false,
                        errors,
                    },
                    image: None,
                    ocr,
                    // Decided on the result after OCR; render when it may be needed.
                    vision_candidate: match cfg.vision {
                        Toggle::On => true,
                        Toggle::Off => false,
                        Toggle::Auto => ocr || q < thr,
                    },
                }
            })
            .collect();
        for r in &pages {
            warnings.extend(r.page.errors.iter().cloned());
        }
        let mut ocr_paths = None;
        if pages.iter().any(|r| r.ocr) {
            match resolve_ocr(cfg) {
                Ok(p) => ocr_paths = Some(p),
                // Explicit `on`: fail before rendering, never a silent fallback.
                Err(e) if cfg.ocr == Toggle::On => return Err(e),
                Err(e) => {
                    // `auto`: note it and continue to vision with `Qtext`.
                    // The missing-models hint depends on privacy and the
                    // caller's flags: the CLI adds it (`cmd::models_hint`).
                    let hint = e
                        .hint()
                        .filter(|_| e.code() != "ocr_models_missing")
                        .map(|h| format!(" {h}"))
                        .unwrap_or_default();
                    warnings.push(format!("{}: {e}{hint}", e.code()));
                    for r in pages.iter_mut() {
                        r.ocr = false;
                        r.vision_candidate &= should_run(cfg.vision, r.page.quality, thr);
                    }
                }
            }
        }
        if let Some(d) = &doc {
            for r in pages.iter_mut().filter(|r| r.ocr || r.vision_candidate) {
                let n = r.page.number;
                match d
                    .pages()
                    .get((n - 1) as PdfPageIndex)
                    .and_then(|pg| render::render_page(&pg, render::RENDER_SCALE))
                {
                    Ok(img) => r.image = Some(img),
                    Err(e) => {
                        let w = format!("Could not render page {n}: {e}");
                        warnings.push(w.clone());
                        r.page.errors.push(w);
                        r.ocr = false;
                        r.vision_candidate = false;
                    }
                }
            }
        }
        Ok(PdfStage {
            pages,
            page_count,
            warnings,
            ocr_paths,
        })
    })??;

    let PdfStage {
        pages,
        page_count,
        mut warnings,
        ocr_paths,
    } = stage;
    let mut out_pages = Vec::with_capacity(pages.len());
    let mut page_images = Vec::new();
    let mut ocr_ran = false;
    let mut ocr_read = false;
    for Routed {
        mut page,
        image,
        ocr,
        vision_candidate,
    } in pages
    {
        if ocr && let (Some(paths), Some(img)) = (&ocr_paths, &image) {
            match ocr_page(paths, cfg, img.clone()) {
                Ok(t) => {
                    let q = quality::page_quality(&t);
                    page.quality = q;
                    page.ocr_quality = Some(q);
                    page.ocr_evidence = if page.text_quality < thr {
                        non_blank_lines(&t)
                    } else {
                        image_only_lines(&t, &page.text)
                    };
                    ocr_ran = true;
                    ocr_read |= !quality::is_blank(&t);
                    page.ocr_text = Some(t);
                }
                Err(e) => {
                    let w = format!("OCR failed on page {}: {e}", page.number);
                    warnings.push(w.clone());
                    page.errors.push(w);
                }
            }
        }
        let send = vision_candidate && should_run(cfg.vision, page.quality, thr);
        if send && let Some(img) = &image {
            let png = render::encode_png(img)
                .map_err(|e| Error::General(format!("PNG encoding failed: {e}")))?;
            page_images.push(png);
            page.image = true;
        }
        out_pages.push(page);
    }
    if ocr_ran && !ocr_read {
        warnings.push("OCR returned no text".into());
    }
    let ocr_text = out_pages
        .iter()
        .filter(|p| !quality::is_blank(&p.ocr_evidence))
        .map(|p| format!("Page {}:\n{}", p.number, p.ocr_evidence))
        .collect::<Vec<_>>()
        .join("\n\n");
    let text = out_pages
        .iter()
        .filter(|p| !quality::is_blank(&p.text))
        .map(|p| format!("Page {}:\n{}", p.number, p.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut sources = vec!["text"];
    if !ocr_text.is_empty() {
        sources.push("ocr");
    }
    if !page_images.is_empty() {
        sources.push("vision");
    }
    let tier = if !page_images.is_empty() {
        Tier::VisionNeeded
    } else if sources.contains(&"ocr") {
        Tier::Ocr
    } else {
        Tier::Text
    };
    Ok(Extraction {
        text,
        ocr_text,
        tier,
        quality: quality::document_quality(out_pages.iter().map(|p| p.quality)),
        page_count,
        pages_used: page_count.min(cfg.max_pages),
        pages: out_pages,
        page_images,
        sources,
        warnings,
    })
}

/// Every non-blank OCR line, trimmed (pages whose text layer reads poorly).
fn non_blank_lines(ocr: &str) -> String {
    ocr.lines()
        .map(str::trim)
        .filter(|l| !quality::is_blank(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Indel similarity at which an OCR word counts as a text-layer word.
const SAME_WORD: f64 = 0.8;

/// OCR lines of a page that its text layer does not already have: what
/// only images (logos, stamps) show. A line is new as soon as one of its
/// words (2+ characters, casefolded) has no close match (Indel similarity
/// ≥ [`SAME_WORD`]) anywhere in the text layer. Reading order and small OCR
/// slips (`lnvoice`) on known text add nothing, while a logo name is kept
/// even when it shares legal-form words with other text (`ACME GmbH & Co.
/// KG` next to `Spqrk GmbH & Co. KG`).
fn image_only_lines(ocr: &str, text: &str) -> String {
    fn words(s: &str) -> Vec<String> {
        crate::harmonize::casefold(s)
            .split(|c: char| !quality::py_isalnum(c))
            .filter(|w| w.chars().count() >= 2)
            .map(unconfuse)
            .collect()
    }
    // Typical OCR digit/letter swaps inside words with letters (`t0` → `to`);
    // pure numbers (`4711`) stay as they are.
    fn unconfuse(w: &str) -> String {
        if !w.chars().any(char::is_alphabetic) {
            return w.to_string();
        }
        w.chars()
            .map(|c| match c {
                '0' => 'o',
                '1' => 'l',
                '5' => 's',
                c => c,
            })
            .collect()
    }
    let layer: std::collections::HashSet<String> = words(text).into_iter().collect();
    let known = |w: &str| {
        layer.contains(w)
            || layer.iter().any(|l| {
                rapidfuzz::distance::indel::normalized_similarity(w.chars(), l.chars()) >= SAME_WORD
            })
    };
    ocr.lines()
        .filter(|line| words(line).iter().any(|w| !known(w)))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(feature = "ocr")]
fn ocr_page(paths: &ocr::PackPaths, cfg: &ExtractConfig, img: image::RgbImage) -> Result<String> {
    let opts = ocr::engine::EngineOptions {
        threads: cfg.ocr_threads,
    };
    Ok(ocr::engine::recognize(paths, &opts, vec![img])?
        .into_iter()
        .next()
        .unwrap_or_default())
}

#[cfg(not(feature = "ocr"))]
fn ocr_page(
    _paths: &ocr::PackPaths,
    _cfg: &ExtractConfig,
    _img: image::RgbImage,
) -> Result<String> {
    unreachable!("resolve_ocr rejects OCR when the engine is not compiled in")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_is_strict_less_than() {
        assert!(should_run(Toggle::Auto, 0.79, 0.8));
        assert!(!should_run(Toggle::Auto, 0.8, 0.8));
        assert!(should_run(Toggle::On, 1.0, 0.8));
        assert!(!should_run(Toggle::Off, 0.0, 0.8));
    }

    fn page(number: usize, text: &str, evidence: &str) -> Page {
        Page {
            number,
            text: text.into(),
            text_quality: 1.0,
            visual: false,
            ocr_text: (!evidence.is_empty()).then(|| evidence.into()),
            ocr_quality: (!evidence.is_empty()).then_some(1.0),
            ocr_evidence: evidence.into(),
            quality: 1.0,
            image: false,
            errors: Vec::new(),
        }
    }

    fn ex(text: &str, ocr: &str) -> Extraction {
        Extraction {
            text: text.into(),
            ocr_text: ocr.into(),
            tier: Tier::Text,
            quality: 0.0,
            page_count: 1,
            pages_used: 1,
            pages: Vec::new(),
            page_images: Vec::new(),
            sources: vec!["text"],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn combined_text_layout() {
        assert_eq!(ex("A", "B").combined_text(), "A\n\n--- OCR Text ---\n\nB");
        assert_eq!(ex(" ", "B").combined_text(), "B");
        assert_eq!(ex("A", "").combined_text(), "A");
        assert!(!ex("", " ").has_content());
    }

    #[test]
    fn page_texts_follow_page_numbers() {
        let mut e = ex("", "");
        e.pages_used = 3;
        e.pages = vec![
            page(1, "one", "Logo"),
            page(2, "", "two"),
            page(3, "three", ""),
        ];
        assert_eq!(
            e.page_texts(),
            vec![
                format!("one\n\n{OCR_HEADER}\n\nLogo"),
                "two".to_string(),
                "three".to_string(),
            ]
        );
    }

    #[test]
    fn image_only_lines_keep_only_new_text() {
        let layer = "Invoice 4711\nBill to: Spqrk GmbH\nTotal EUR 12,13";
        // Same words in another order / case, one OCR slip: nothing new.
        assert_eq!(
            image_only_lines(
                "BILL TO: Spqrk GmbH\nlnvoice 4711\nBILL T0: Spqrk GmbH",
                layer
            ),
            ""
        );
        // The logo line is the only addition.
        let ocr = "ACME Industries\nInvoice 4711\nBill to: Spqrk GmbH";
        assert_eq!(image_only_lines(ocr, layer), "ACME Industries");
        // A logo sharing a legal suffix with the recipient is still new;
        // a line cut differently (part of a text line, two lines merged)
        // is known.
        let ocr =
            "ACME GmbH\nSpqrk GmbH\nBill to: Spqrk GmbH Total EUR 12,13\nACME Holding Spqrk GmbH";
        assert_eq!(
            image_only_lines(ocr, layer),
            "ACME GmbH\nACME Holding Spqrk GmbH"
        );
        // Long shared legal forms never hide the distinctive name.
        let kg = "Bill to: Spqrk GmbH & Co. KG\nSpqrk International GmbH";
        assert_eq!(
            image_only_lines(
                "ACME GmbH & Co. KG\nACME International GmbH\nSpqrk GmbH & Co. KG",
                kg
            ),
            "ACME GmbH & Co. KG\nACME International GmbH"
        );
        // Only known words, even from different lines: nothing new.
        assert_eq!(image_only_lines("Invoice GmbH", layer), "");
        assert_eq!(non_blank_lines(" a \n\n  \nb"), "a\nb");
    }
}
