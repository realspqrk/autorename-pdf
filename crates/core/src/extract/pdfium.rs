//! Runtime binding of the pinned PDFium shared library and serialized access.
//!
//! PDFium is not thread-safe. All access goes through [`with_pdfium`], which holds
//! one process-wide mutex, so callers never touch PDFium concurrently.
//! The library is bound once (pdfium-render keeps global bindings) on first use.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use pdfium_render::prelude::*;

use crate::{Error, Result};

/// Env override: directory containing `pdfium.dll` / `libpdfium.so` / `libpdfium.dylib`.
pub const PDFIUM_DIR_ENV: &str = "AUTORENAME_PDFIUM_DIR";

/// Target triple used for `vendor/pdfium/<triple>/` (dev natives, see `cargo xtask fetch-natives`).
pub fn host_triple() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("aarch64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Some("aarch64-pc-windows-msvc")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else {
        None
    }
}

/// Candidate directories in search order: env > exe dir > dev vendor dir.
/// (The system library search path is tried last, by name only.)
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os(PDFIUM_DIR_ENV).filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(d));
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(d);
    }
    if let Some(t) = host_triple() {
        // crates/core -> workspace root
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.ancestors().nth(2).unwrap_or(manifest);
        dirs.push(root.join("vendor").join("pdfium").join(t));
    }
    dirs
}

fn bind() -> std::result::Result<Pdfium, String> {
    let mut tried = Vec::new();
    for dir in search_dirs() {
        let lib = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if lib.is_file() {
            match Pdfium::bind_to_library(&lib) {
                Ok(b) => return Ok(Pdfium::new(b)),
                Err(e) => tried.push(format!("{}: {e}", lib.display())),
            }
        } else {
            tried.push(format!("{}: not found", lib.display()));
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(b) => Ok(Pdfium::new(b)),
        Err(e) => {
            tried.push(format!("system library: {e}"));
            Err(tried.join("; "))
        }
    }
}

static PDFIUM: OnceLock<std::result::Result<Mutex<Pdfium>, String>> = OnceLock::new();

/// Run `f` with exclusive access to the bound PDFium instance.
pub fn with_pdfium<T>(f: impl FnOnce(&Pdfium) -> T) -> Result<T> {
    let slot = PDFIUM.get_or_init(|| bind().map(Mutex::new));
    match slot {
        Ok(m) => {
            // A panic inside a previous closure poisons the lock; PDFium state is
            // per-document, so continuing is safe.
            let guard = m.lock().unwrap_or_else(|p| p.into_inner());
            Ok(f(&guard))
        }
        Err(detail) => Err(Error::Extract {
            code: "pdfium_missing",
            message: format!("PDFium library could not be loaded ({detail})"),
            hint: Some(format!(
                "Place the PDFium library next to the executable or set {PDFIUM_DIR_ENV}; \
                 developers: `cargo xtask fetch-natives`."
            )),
        }),
    }
}
