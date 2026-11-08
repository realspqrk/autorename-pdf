//! oar-ocr inference (feature `ocr`). The engine (5 ONNX sessions, ~150–190 ms
//! to build) is cached per process and reused across documents.

use std::sync::Mutex;

use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};

use super::models::PackPaths;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOptions {
    /// ORT intra-op threads; `None` = ORT default (all cores).
    pub threads: Option<usize>,
}

type Cached = (PackPaths, EngineOptions, OAROCR);
static ENGINE: Mutex<Option<Cached>> = Mutex::new(None);

fn ocr_failed(e: impl std::fmt::Display) -> Error {
    Error::Extract {
        code: "ocr_failed",
        message: format!("OCR failed: {e}"),
        hint: None,
    }
}

fn build(paths: &PackPaths, opts: &EngineOptions) -> Result<OAROCR> {
    use oar_ocr::domain::tasks::TextDetectionConfig;
    let mut b = OAROCRBuilder::new(paths.det.clone(), paths.rec.clone(), paths.dict.clone())
        // Official PP-OCRv6 thresholds (same as `OAROCRBuilder::pp_ocrv6`).
        .text_detection_config(TextDetectionConfig {
            score_threshold: 0.2,
            box_threshold: 0.45,
            unclip_ratio: 1.4,
            max_candidates: 3000,
            // Detection resize stays at oar-ocr's min-side default: a max-side
            // cap (3.x bridge: 736) destroyed small-font scans in the spike.
            ..Default::default()
        });
    if let Some(p) = &paths.doc_orientation {
        b = b.with_document_image_orientation_classification(p.clone());
    }
    if let Some(p) = &paths.textline_orientation {
        b = b.with_text_line_orientation_classification(p.clone());
    }
    if let Some(t) = opts.threads {
        b = b.ort_session(oar_ocr::core::config::OrtSessionConfig {
            intra_threads: Some(t),
            ..Default::default()
        });
    }
    b.build().map_err(ocr_failed)
}

/// OCR page images; returns one text per page (recognized lines in sorted-box
/// order, joined `"\n"`).
pub fn recognize(
    paths: &PackPaths,
    opts: &EngineOptions,
    pages: Vec<image::RgbImage>,
) -> Result<Vec<String>> {
    let mut slot = ENGINE.lock().unwrap_or_else(|p| p.into_inner());
    let reuse = matches!(&*slot, Some((p, o, _)) if p == paths && o == opts);
    if !reuse {
        *slot = Some((paths.clone(), opts.clone(), build(paths, opts)?));
    }
    let (_, _, engine) = slot.as_ref().expect("engine initialized above");
    pages
        .into_iter()
        .map(|img| {
            let res = engine.predict(vec![img]).map_err(ocr_failed)?;
            Ok(res
                .first()
                .map(|r| r.concatenated_text("\n"))
                .unwrap_or_default())
        })
        .collect()
}
