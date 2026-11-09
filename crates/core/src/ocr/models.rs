//! OCR model store: `<models root>/<pack>/<file>`, hash-pinned by [`super::manifest`].
//!
//! Nothing here downloads implicitly. [`fetch`] is called explicitly by the CLI
//! (`autorename-pdf models fetch`) or GUI; extraction only calls [`require`].

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::manifest::{ModelFile, PACKS, Pack};
use crate::{Error, Result};

/// Env override for the models root (tests, portable installs).
pub const MODELS_DIR_ENV: &str = "AUTORENAME_MODELS_DIR";

/// Default models root: `$AUTORENAME_MODELS_DIR` > `<OS data dir>/autorename-pdf/models`.
pub fn default_root() -> Option<PathBuf> {
    std::env::var_os(MODELS_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::config::app_data_dir().map(|d| d.join("models")))
}

pub fn pack_dir(root: &Path, pack: &Pack) -> PathBuf {
    root.join(pack.id)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileStatus {
    pub name: &'static str,
    pub size: u64,
    /// Present with the expected size (hash is checked by fetch/import/verify).
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PackStatus {
    pub pack: &'static str,
    pub description: &'static str,
    #[serde(with = "crate::fsx::path_serde")]
    pub dir: PathBuf,
    pub installed: bool,
    pub total_size: u64,
    pub files: Vec<FileStatus>,
}

/// Resolved local paths of an installed pack (input to the engine).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackPaths {
    pub pack: &'static str,
    pub det: PathBuf,
    pub rec: PathBuf,
    pub dict: PathBuf,
    pub doc_orientation: Option<PathBuf>,
    pub textline_orientation: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress<'a> {
    pub file: &'a str,
    pub downloaded: u64,
    pub total: u64,
}

fn size_ok(path: &Path, f: &ModelFile) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == f.size)
}

fn status(root: &Path, pack: &'static Pack) -> PackStatus {
    let dir = pack_dir(root, pack);
    let files: Vec<FileStatus> = pack
        .files()
        .into_iter()
        .map(|f| FileStatus {
            name: f.name,
            size: f.size,
            present: size_ok(&dir.join(f.name), &f),
        })
        .collect();
    PackStatus {
        pack: pack.id,
        description: pack.description,
        installed: files.iter().all(|f| f.present),
        total_size: pack.total_size(),
        dir,
        files,
    }
}

/// Status of every pack in the manifest.
pub fn list(root: &Path) -> Vec<PackStatus> {
    PACKS.iter().map(|p| status(root, p)).collect()
}

fn missing_error(pack: &Pack, dir: &Path) -> Error {
    Error::Extract {
        code: "ocr_models_missing",
        message: format!(
            "OCR models for pack '{}' are not installed in {}",
            pack.id,
            dir.display()
        ),
        // No runnable command: whether to fetch or import depends on the
        // privacy mode, and the command on the caller's flags (`--config`).
        // The CLI replaces it (`cmd::models_hint`); the GUI offers a button.
        hint: Some(format!(
            "Install the '{}' pack (~{} MB) online with `models fetch`, or from a copy of another machine's `models` folder with `models import <dir>`; or disable OCR (pdf.ocr: false).",
            pack.id,
            pack.total_size().div_ceil(1_000_000)
        )),
    }
}

/// Paths of an installed pack, or `ocr_models_missing`. Size check only (cheap);
/// hashes were verified when the files were written.
pub fn require(root: &Path, pack: &'static Pack) -> Result<PackPaths> {
    let dir = pack_dir(root, pack);
    if !status(root, pack).installed {
        return Err(missing_error(pack, &dir));
    }
    Ok(PackPaths {
        pack: pack.id,
        det: dir.join(pack.det.name),
        rec: dir.join(pack.rec.name),
        dict: dir.join(pack.dict.name),
        doc_orientation: pack.doc_orientation.map(|f| dir.join(f.name)),
        textline_orientation: pack.textline_orientation.map(|f| dir.join(f.name)),
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_verified(path: &Path, f: &ModelFile) -> Result<bool> {
    Ok(size_ok(path, f) && sha256_file(path)? == f.sha256)
}

/// Full hash verification of an installed pack; returns names of bad/missing files.
pub fn verify(root: &Path, pack: &'static Pack) -> Result<Vec<&'static str>> {
    let dir = pack_dir(root, pack);
    let mut bad = Vec::new();
    for f in pack.files() {
        if !file_verified(&dir.join(f.name), &f)? {
            bad.push(f.name);
        }
    }
    Ok(bad)
}

fn integrity_error(name: &str, detail: String) -> Error {
    Error::Extract {
        code: "ocr_model_integrity",
        message: format!("{name}: {detail}"),
        hint: Some("Retry; if it persists the pinned manifest needs a bump.".into()),
    }
}

/// Copy `bytes` from `reader` into an exclusive temp in `dir`, check size +
/// sha256, then atomically rename into place (temp deleted on any error).
fn write_verified(
    dir: &Path,
    f: &ModelFile,
    reader: &mut dyn Read,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<()> {
    let dest = dir.join(f.name);
    let mut part = crate::fsx::temp_in(dir).map_err(|e| Error::io(dir, e))?;
    {
        let out = part.as_file_mut();
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| integrity_error(f.name, format!("read failed: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| Error::io(&dest, e))?;
            done += n as u64;
            progress(Progress {
                file: f.name,
                downloaded: done,
                total: f.size,
            });
        }
        out.sync_all().map_err(|e| Error::io(&dest, e))?;
        let got = hex(&hasher.finalize());
        if done != f.size || got != f.sha256 {
            return Err(integrity_error(
                f.name,
                format!(
                    "expected {} bytes sha256 {}, got {done} bytes sha256 {got}",
                    f.size, f.sha256
                ),
            ));
        }
    }
    crate::fsx::persist_replace(part, &dest).map_err(|e| Error::io(&dest, e))
}

fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))
}

/// Reader that fails once `cancel` is set (download cancellation).
struct Cancellable<'a, R> {
    inner: R,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
}

impl<R: Read> Read for Cancellable<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self
            .cancel
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        self.inner.read(buf)
    }
}

/// Download every missing or corrupt file of `pack` (explicit user action only).
/// Setting `cancel` stops between chunks; the partial file is discarded and
/// the error code is `ocr_model_download_cancelled`.
pub fn fetch(
    root: &Path,
    pack: &'static Pack,
    progress: &mut dyn FnMut(Progress<'_>),
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let cancelled = || cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst));
    let cancel_err = || Error::Extract {
        code: "ocr_model_download_cancelled",
        message: "Download cancelled".into(),
        hint: None,
    };
    let dir = pack_dir(root, pack);
    ensure_dir(&dir)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(None)
        .user_agent(concat!("autorename-pdf/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::General(format!("HTTP client: {e}")))?;
    for f in pack.files() {
        if file_verified(&dir.join(f.name), &f)? {
            continue;
        }
        if cancelled() {
            return Err(cancel_err());
        }
        let resp = client
            .get(f.url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| Error::Extract {
                code: "ocr_model_download",
                message: format!("Downloading {} failed: {e}", f.url),
                hint: Some("Check network access, or use `models import <dir>`.".into()),
            })?;
        let mut reader = Cancellable {
            inner: resp,
            cancel,
        };
        if let Err(e) = write_verified(&dir, &f, &mut reader, progress) {
            return Err(if cancelled() { cancel_err() } else { e });
        }
    }
    Ok(())
}

/// Install `pack` from a local directory holding the manifest's file names
/// (offline machines). Every file is hash-verified.
/// Install `pack` from `src`: the pack folder itself (holding its files) or
/// a models root containing `<pack>/` (e.g. a copied `models` folder).
pub fn import(root: &Path, pack: &'static Pack, src: &Path) -> Result<()> {
    let has_files = |d: &Path| pack.files().iter().all(|f| d.join(f.name).is_file());
    let src = if has_files(src) {
        src.to_path_buf()
    } else if has_files(&src.join(pack.id)) {
        src.join(pack.id)
    } else {
        let names: Vec<&str> = pack.files().iter().map(|f| f.name).collect();
        return Err(Error::Extract {
            code: "ocr_models_missing",
            message: format!(
                "{} does not contain the '{}' pack files ({})",
                src.display(),
                pack.id,
                names.join(", ")
            ),
            hint: Some(format!(
                "Pass the pack folder or its parent, e.g. `autorename-pdf models import /media/usb/models/{}` or `.../models` (copy it from `autorename-pdf config path` → models on a machine that ran `models fetch`).",
                pack.id
            )),
        });
    };
    let dir = pack_dir(root, pack);
    ensure_dir(&dir)?;
    for f in pack.files() {
        let from = src.join(f.name);
        let mut reader = fs::File::open(&from).map_err(|e| Error::io(&from, e))?;
        write_verified(&dir, &f, &mut reader, &mut |_| {})?;
    }
    Ok(())
}

/// Delete an installed pack. Missing pack dir is not an error.
pub fn remove(root: &Path, pack: &'static Pack) -> Result<()> {
    let dir = pack_dir(root, pack);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(dir, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::manifest::LATIN;

    #[test]
    fn missing_pack_reports_code_and_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let err = require(tmp.path(), &LATIN).unwrap_err();
        assert_eq!(err.code(), "ocr_models_missing");
        let hint = err.hint().unwrap();
        assert!(hint.contains("models fetch") && hint.contains("models import <dir>"));
        assert!(!hint.contains("autorename-pdf"), "unscoped command: {hint}");
        let st = list(tmp.path());
        assert!(!st[0].installed);
        assert!(st[0].files.iter().all(|f| !f.present));
    }

    #[test]
    fn write_verified_rejects_bad_hash_and_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let f = ModelFile {
            name: "x.bin",
            url: "",
            sha256: "00",
            size: 3,
        };
        let err = write_verified(tmp.path(), &f, &mut &b"abc"[..], &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "ocr_model_integrity");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn import_accepts_the_models_root_and_explains_the_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let e = import(&tmp.path().join("dst"), &LATIN, tmp.path()).unwrap_err();
        assert_eq!(e.code(), "ocr_models_missing");
        assert!(e.hint().unwrap().contains("models import"));
        assert!(e.to_string().contains(LATIN.files()[0].name));
        // A root holding `latin/` with the (wrong-content) files is detected
        // and then rejected by the hash check, not by "file not found".
        let pack = tmp.path().join("root").join(LATIN.id);
        fs::create_dir_all(&pack).unwrap();
        for f in LATIN.files() {
            fs::write(pack.join(f.name), b"x").unwrap();
        }
        let e = import(&tmp.path().join("dst"), &LATIN, &tmp.path().join("root")).unwrap_err();
        assert_eq!(e.code(), "ocr_model_integrity");
    }

    #[test]
    fn write_verified_accepts_good_file() {
        let tmp = tempfile::tempdir().unwrap();
        let f = ModelFile {
            name: "x.bin",
            url: "",
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            size: 3,
        };
        let mut seen = 0;
        write_verified(tmp.path(), &f, &mut &b"abc"[..], &mut |p| {
            seen = p.downloaded
        })
        .unwrap();
        assert_eq!(seen, 3);
        assert!(file_verified(&tmp.path().join("x.bin"), &f).unwrap());
        remove(tmp.path(), &LATIN).unwrap(); // absent pack dir is fine
    }
}
