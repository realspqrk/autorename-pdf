//! Offline OCR: hash-pinned model packs (always available) and the oar-ocr
//! PP-OCRv6 engine (cargo feature `ocr`).

#[cfg(feature = "ocr")]
pub mod engine;
pub mod manifest;
pub mod models;

pub use manifest::{AUTO_LANGUAGE, PACKS, Pack, is_auto, pack, pack_for_language};
pub use models::{PackPaths, PackStatus, Progress};

use std::path::Path;

use crate::{Error, Result};

/// True when this build contains the OCR engine.
pub const ENGINE_AVAILABLE: bool = cfg!(feature = "ocr");

/// Pack for `ocr.language`, or `ocr_language_unsupported`.
pub fn pack_for_language_or_err(lang: &str) -> Result<&'static Pack> {
    pack_for_language(lang).ok_or_else(|| Error::Extract {
        code: "ocr_language_unsupported",
        message: format!("No OCR model pack for language '{lang}'"),
        hint: Some(format!(
            "Supported packs: {}. Set ocr.language to auto or a language code (e.g. en, de, fr).",
            PACKS.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")
        )),
    })
}

/// Pack to run for `ocr.language` with models under `root`. A code maps to
/// its pack. `auto` uses the installed pack: the first installed one in
/// manifest order, else the default pack (whose `require` then reports
/// `ocr_models_missing`). Today `latin` is the only pack; with several
/// script packs, `auto` needs script detection (followup).
pub fn pack_for_setting(lang: &str, root: &Path) -> Result<&'static Pack> {
    if !is_auto(lang) {
        return pack_for_language_or_err(lang);
    }
    let installed = models::list(root);
    PACKS
        .iter()
        .find(|p| installed.iter().any(|s| s.pack == p.id && s.installed))
        .or(PACKS.first())
        .ok_or_else(|| Error::General("No OCR model packs are known".into()))
}
