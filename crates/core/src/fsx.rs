//! Vetted filesystem primitives. Every mutation the app makes (journal,
//! company names, enrichment, OCR models, renames/undo) goes through here.
//!
//! - Temp files: unpredictable sibling name, exclusive create (`O_EXCL`, so an
//!   existing path — including a planted symlink — is never followed), mode
//!   `0600` on Unix from creation (via `tempfile`).
//! - [`persist_replace`]: atomic rename over the destination + parent-dir fsync
//!   (Windows: `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`).
//! - [`replace_verified`]: replace a file only if the object being displaced
//!   is still the expected one (identity + SHA-256); otherwise put it back.
//!   Document safety never relies on (advisory or mandatory) locks.
//! - [`rename_noreplace`]: OS-enforced no-overwrite rename (Linux `renameat2
//!   RENAME_NOREPLACE`, macOS `renamex_np RENAME_EXCL`, Windows `MoveFileExW`
//!   without `REPLACE_EXISTING`; `link`+`unlink` fallback). Any existing
//!   directory entry, including a dangling symlink, counts as occupied.
//! - [`lock_path`]/[`lock_exclusive`]: advisory exclusive locks (std `File::lock`:
//!   `flock` on Unix, `LockFileEx` — mandatory — on Windows).
//! - [`FileId`]: device/inode (Windows: volume serial/file index) + len + mtime.

use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::path::Path;
use std::time::SystemTime;

pub use tempfile::NamedTempFile;

/// Directory containing `path` (`.` for a bare file name).
pub fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Fresh exclusive temp file in `dir`: `.arp-<random>.tmp` (short, so it fits
/// next to any valid file name). Deleted on drop unless persisted.
pub fn temp_in(dir: &Path) -> io::Result<NamedTempFile> {
    tempfile::Builder::new()
        .prefix(".arp-")
        .suffix(".tmp")
        .rand_bytes(10)
        .tempfile_in(dir)
}

/// [`temp_in`] + write + optional permissions + fsync.
pub fn write_temp(
    dir: &Path,
    bytes: &[u8],
    perms: Option<Permissions>,
) -> io::Result<NamedTempFile> {
    let mut tmp = temp_in(dir)?;
    tmp.as_file_mut().write_all(bytes)?;
    if let Some(p) = perms {
        tmp.as_file().set_permissions(p)?;
    }
    tmp.as_file().sync_all()?;
    Ok(tmp)
}

/// Atomically replace `dst` with `tmp` and make the rename durable. On error
/// the temp file is deleted.
pub fn persist_replace(tmp: NamedTempFile, dst: &Path) -> io::Result<()> {
    let tmp = tmp.into_temp_path();
    imp::rename_replace(&tmp, dst)?;
    // Renamed away: nothing left to delete.
    let _ = tmp.keep();
    sync_dir(parent_dir(dst))
}

/// Unused-looking sibling name `.arp-<random>.bak` in `dir` (not created;
/// callers claim it with a no-replace rename).
fn random_sibling(dir: &Path) -> std::path::PathBuf {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    if let Ok(d) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        h.write_u128(d.as_nanos());
    }
    dir.join(format!(".arp-{:016x}.bak", h.finish()))
}

/// Identity and SHA-256 (hex) of the file at `path`.
pub fn identify(path: &Path) -> io::Result<(FileId, String)> {
    use sha2::{Digest, Sha256};
    let mut f = File::open(path)?;
    let id = file_id(&f)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = io::Read::read(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let hex = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok((id, hex))
}

/// A mutation that happened. `warnings` (`durability_warning: …` for a failed
/// flush/cleanup, `conflict_copy: …` for a preserved concurrent revision)
/// are about steps *after* the change took effect: callers
/// must record the change (journal) and surface the warning, never treat it
/// as "not done".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Committed {
    pub warnings: Vec<String>,
}

impl Committed {
    fn warn(&mut self, what: impl std::fmt::Display) {
        self.warnings.push(format!("durability_warning: {what}"));
    }
}

/// `<stem> (conflict <UTC timestamp>)<.ext>` next to `path`, not yet existing.
fn conflict_sibling(path: &Path) -> std::path::PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let dir = parent_dir(path);
    let mut n = 0;
    loop {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!(" {n}")
        };
        let p = dir.join(format!("{stem} (conflict {ts}{suffix}){ext}"));
        if std::fs::symlink_metadata(&p).is_err() {
            return p;
        }
        n += 1;
    }
}

/// Replace `path` with `tmp`, but only if the file being displaced is still
/// `expected` (same object, length, mtime) with content `expected_sha256`.
///
/// Move the current file aside to a random sibling (no-replace) → verify the
/// displaced object → move `tmp` into place (no-replace) → re-verify the
/// displaced object → delete it. A mismatch before installing moves the
/// displaced file back ([`crate::Error::Conflict`]). A mismatch *after*
/// installing (an editor wrote through a handle it held open) keeps the
/// editor's bytes as `<stem> (conflict <timestamp>).pdf`; the replacement
/// stays installed and the result carries a `conflict_copy: …` warning
/// naming the kept file (callers journal the change and report it).
///
/// Residual window (documented, accepted): on Unix, locks are advisory, so a
/// program writing through an already-open handle *between the final
/// re-verification and the unlink* (microseconds) can still lose that write.
/// No mandatory locking is used (it would block viewers/editors on Windows).
pub fn replace_verified(
    path: &Path,
    tmp: NamedTempFile,
    expected: FileId,
    expected_sha256: &str,
) -> crate::Result<Committed> {
    use crate::Error;
    let io = |e| Error::io(path, e);
    let dir = parent_dir(path);
    let backup = loop {
        let b = random_sibling(dir);
        match rename_noreplace(path, &b) {
            Ok(_) => break b,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::Conflict {
                    message: format!("{} disappeared; not replaced", path.display()),
                });
            }
            Err(e) => return Err(io(e)),
        }
    };
    let restore = |why: &str| -> Error {
        let message = match rename_noreplace(&backup, path) {
            Ok(_) => format!("{} {why}; not replaced", path.display()),
            Err(_) => format!(
                "{} {why}; the displaced file was kept at {}",
                path.display(),
                backup.display()
            ),
        };
        Error::Conflict { message }
    };
    let unchanged =
        || identify(&backup).is_ok_and(|(id, sha)| id == expected && sha == expected_sha256);
    if !unchanged() {
        return Err(restore("was changed by another program"));
    }
    let tmp = tmp.into_temp_path();
    let mut done = match rename_noreplace(&tmp, path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(restore("was re-created by another program"));
        }
        Err(e) => {
            let _ = rename_noreplace(&backup, path);
            return Err(io(e));
        }
    };
    let _ = tmp.keep();
    #[cfg(test)]
    tests::AFTER_INSTALL.with(|h| {
        if let Some(f) = h.borrow_mut().take() {
            f(&backup);
        }
    });
    // An editor may have written through a handle opened before we started.
    if !unchanged() {
        let keep = conflict_sibling(path);
        let kept = match rename_noreplace(&backup, &keep) {
            Ok(_) => keep,
            Err(_) => backup,
        };
        // Our replacement is installed (callers journal it); the editor's
        // revision is preserved next to it and reported.
        done.warnings.push(format!(
            "conflict_copy: {} was written by another program during the update; their version was kept as {}",
            path.display(),
            kept.display()
        ));
        if let Err(e) = sync_dir(dir) {
            done.warn(format!("{} not flushed to disk: {e}", path.display()));
        }
        return Ok(done);
    }
    if let Err(e) = std::fs::remove_file(&backup) {
        done.warn(format!("could not remove {}: {e}", backup.display()));
    }
    if let Err(e) = sync_dir(dir) {
        done.warn(format!("{} not flushed to disk: {e}", path.display()));
    }
    Ok(done)
}

/// Durable atomic write: temp sibling → fsync → replace → dir fsync. An
/// existing destination keeps its permissions; a new one is `0600` on Unix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = parent_dir(path);
    std::fs::create_dir_all(dir)?;
    let perms = std::fs::metadata(path).ok().map(|m| m.permissions());
    persist_replace(write_temp(dir, bytes, perms)?, path)
}

/// Durable exclusive write: fails with [`io::ErrorKind::AlreadyExists`]
/// when anything exists at `path` (never replaces), else like [`write_atomic`].
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = parent_dir(path);
    std::fs::create_dir_all(dir)?;
    let tmp = write_temp(dir, bytes, None)?.into_temp_path();
    let done = rename_noreplace(&tmp, path)?;
    let _ = tmp.keep();
    match done.warnings.first() {
        Some(w) => Err(io::Error::other(w.clone())),
        None => Ok(()),
    }
}

fn occupied(dst: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("target already exists: {}", dst.display()),
    )
}

/// Rename `src` → `dst`, failing with [`io::ErrorKind::AlreadyExists`] when
/// any entry (file, directory, dangling symlink) exists at `dst`. `Err` means
/// nothing moved; a failed directory fsync after the move is a
/// [`Committed`] warning.
pub fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<Committed> {
    if std::fs::symlink_metadata(dst).is_ok() {
        return Err(occupied(dst));
    }
    imp::rename_noreplace(src, dst).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            occupied(dst)
        } else {
            e
        }
    })?;
    let mut done = Committed::default();
    let (from, to) = (parent_dir(src), parent_dir(dst));
    for d in if from == to { vec![to] } else { vec![to, from] } {
        if let Err(e) = sync_dir(d) {
            done.warn(format!("{} not flushed to disk: {e}", d.display()));
        }
    }
    Ok(done)
}

#[allow(dead_code)] // used on platforms without a native no-replace rename
fn link_unlink(src: &Path, dst: &Path) -> io::Result<()> {
    // `link` fails when dst exists (and never follows a symlink at dst).
    std::fs::hard_link(src, dst)?;
    if let Err(e) = std::fs::remove_file(src) {
        let _ = std::fs::remove_file(dst);
        return Err(e);
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
mod imp {
    use std::io;
    use std::path::Path;

    pub use std::fs::rename as rename_replace;

    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use rustix::io::Errno;

    pub fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
        match renameat_with(CWD, src, CWD, dst, RenameFlags::NOREPLACE) {
            Ok(()) => Ok(()),
            // Kernel or filesystem without no-replace support.
            Err(e) if e == Errno::NOSYS || e == Errno::INVAL || e == Errno::NOTSUP => {
                super::link_unlink(src, dst)
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))
))]
mod imp {
    pub use super::link_unlink as rename_noreplace;
    pub use std::fs::rename as rename_replace;
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    /// NUL-terminated UTF-16 path for Win32. Paths near `MAX_PATH` (260)
    /// get the verbatim `\\?\` prefix, as std adds for its own calls; a deep
    /// folder plus a long generated name would otherwise fail to rename.
    fn wide(p: &Path) -> io::Result<Vec<u16>> {
        const SLASH: u16 = b'\\' as u16;
        let mut w: Vec<u16> = p.as_os_str().encode_wide().collect();
        let prefixed = w.len() >= 4
            && w[0] == SLASH
            && w[1] == SLASH
            && (w[2] == u16::from(b'?') || w[2] == u16::from(b'.'))
            && w[3] == SLASH;
        if w.len() >= 248 && !prefixed {
            // `absolute` (GetFullPathNameW) normalizes `/` and `..`, which
            // verbatim paths would take literally.
            let a: Vec<u16> = std::path::absolute(p)?.as_os_str().encode_wide().collect();
            w = if a.starts_with(&[SLASH, SLASH]) {
                r"\\?\UNC\"
                    .encode_utf16()
                    .chain(a[2..].iter().copied())
                    .collect()
            } else {
                r"\\?\".encode_utf16().chain(a).collect()
            };
        }
        w.push(0);
        Ok(w)
    }

    #[allow(unsafe_code)]
    fn move_file(src: &Path, dst: &Path, flags: MOVE_FILE_FLAGS) -> io::Result<()> {
        let (s, d) = (wide(src)?, wide(dst)?);
        // SAFETY: both are NUL-terminated UTF-16 buffers alive for the call.
        if unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), flags) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// No MOVEFILE_REPLACE_EXISTING: fails with ERROR_ALREADY_EXISTS.
    pub fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
        move_file(src, dst, MOVEFILE_WRITE_THROUGH)
    }

    /// Durable replace: returns only after the move is flushed to disk.
    pub fn rename_replace(src: &Path, dst: &Path) -> io::Result<()> {
        move_file(src, dst, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
    }
}

/// fsync a directory so a rename inside it survives power loss (Unix; no-op
/// on Windows, where `MoveFileExW`/NTFS journaling covers it).
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        match File::open(dir).and_then(|d| d.sync_all()) {
            // Some filesystems cannot fsync directories at all.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
                ) =>
            {
                Ok(())
            }
            r => r,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Block until `file` holds an exclusive advisory lock (released on close).
pub fn lock_exclusive(file: &File) -> io::Result<()> {
    file.lock()
}

/// Open/create the lock file at `path` (never through a symlink on Unix) and
/// lock it exclusively. Dropping the returned handle releases the lock.
pub fn lock_path(path: &Path) -> io::Result<File> {
    std::fs::create_dir_all(parent_dir(path))?;
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let f = opts.open(path)?;
    lock_exclusive(&f)?;
    Ok(f)
}

/// Lock `<path>.lock` (the guarded file itself is replaced by renames, so it
/// cannot carry the lock). Held until the returned handle drops.
pub fn lock_sibling(path: &Path) -> io::Result<File> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    lock_path(&path.with_file_name(name))
}

/// Identity of an open file / path: same object (device + inode, or Windows
/// volume serial + file index) with the same length and mtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileId {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: Option<SystemTime>,
}

pub fn file_id(f: &File) -> io::Result<FileId> {
    let m = f.metadata()?;
    let (dev, ino) = imp_id::dev_ino(f, &m)?;
    Ok(FileId {
        dev,
        ino,
        len: m.len(),
        mtime: m.modified().ok(),
    })
}

/// [`file_id`] of whatever `path` names right now.
pub fn path_id(path: &Path) -> io::Result<FileId> {
    file_id(&File::open(path)?)
}

#[cfg(unix)]
mod imp_id {
    use std::os::unix::fs::MetadataExt;

    pub fn dev_ino(_: &std::fs::File, m: &std::fs::Metadata) -> std::io::Result<(u64, u64)> {
        Ok((m.dev(), m.ino()))
    }
}

#[cfg(windows)]
mod imp_id {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    #[allow(unsafe_code)]
    pub fn dev_ino(f: &std::fs::File, _: &std::fs::Metadata) -> std::io::Result<(u64, u64)> {
        // SAFETY: zeroed POD out-struct; the handle is valid while `f` lives.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        Ok((u64::from(info.dwVolumeSerialNumber), index))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp_id {
    pub fn dev_ino(_: &std::fs::File, _: &std::fs::Metadata) -> std::io::Result<(u64, u64)> {
        Ok((0, 0))
    }
}

/// Lossless (de)serialization of paths: a JSON string when the path is valid
/// Unicode, else `{"unix_bytes": [..]}` (Unix) / `{"windows_wide": [..]}` (Windows).
pub mod path_serde {
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Wire form of a path (JSON schema: use `#[schemars(with = "Repr")]`).
    #[derive(Serialize, Deserialize, schemars::JsonSchema)]
    #[serde(untagged)]
    pub enum Repr {
        /// Valid Unicode path.
        Str(String),
        /// Unix path that is not valid UTF-8 (raw bytes).
        Bytes { unix_bytes: Vec<u8> },
        /// Windows path that is not valid UTF-16 (raw code units).
        Wide { windows_wide: Vec<u16> },
    }

    pub fn serialize<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
        if let Some(text) = p.to_str() {
            return Repr::Str(text.to_string()).serialize(s);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Repr::Bytes {
                unix_bytes: p.as_os_str().as_bytes().to_vec(),
            }
            .serialize(s)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Repr::Wide {
                windows_wide: p.as_os_str().encode_wide().collect(),
            }
            .serialize(s)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Repr::Str(p.to_string_lossy().into_owned()).serialize(s)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        use serde::de::Error;
        match Repr::deserialize(d)? {
            Repr::Str(s) => Ok(PathBuf::from(s)),
            #[cfg(unix)]
            Repr::Bytes { unix_bytes } => {
                use std::os::unix::ffi::OsStringExt;
                Ok(PathBuf::from(std::ffi::OsString::from_vec(unix_bytes)))
            }
            #[cfg(windows)]
            Repr::Wide { windows_wide } => {
                use std::os::windows::ffi::OsStringExt;
                Ok(PathBuf::from(std::ffi::OsString::from_wide(&windows_wide)))
            }
            #[allow(unreachable_patterns)]
            _ => Err(D::Error::custom("path was recorded on another platform")),
        }
    }

    /// Lossless JSON value of a path (for ad-hoc `json!` documents).
    pub fn to_value(p: &Path) -> serde_json::Value {
        serialize(p, serde_json::value::Serializer).unwrap_or(serde_json::Value::Null)
    }

    /// `Option<PathBuf>` variant (`null` for `None`).
    pub mod opt {
        use std::path::PathBuf;

        use serde::{Deserialize, Deserializer, Serializer};

        #[derive(Deserialize)]
        struct W(#[serde(with = "super")] PathBuf);

        pub fn serialize<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
            match p {
                Some(p) => super::serialize(p, s),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
            Ok(Option::<W>::deserialize(d)?.map(|w| w.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Hook = Box<dyn FnOnce(&Path)>;
    thread_local! {
        /// Runs once right after the replacement is installed (race tests).
        pub(super) static AFTER_INSTALL: std::cell::RefCell<Option<Hook>> =
            std::cell::RefCell::new(None);
    }

    #[test]
    fn write_through_an_open_handle_during_replace_is_kept() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("doc.pdf");
        std::fs::write(&p, b"v1").unwrap();
        let (id, sha) = identify(&p).unwrap();
        // An editor holds the file open from before the replace started.
        let mut editor = OpenOptions::new().write(true).open(&p).unwrap();
        AFTER_INSTALL.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_backup: &Path| {
                editor.write_all(b"EDITOR NEW REVISION").unwrap();
                editor.sync_all().unwrap();
            }))
        });
        let tmp = write_temp(d.path(), b"ours", None).unwrap();
        // Committed (so it gets journaled/undoable) + a conflict_copy warning.
        let done = replace_verified(&p, tmp, id, &sha).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"ours");
        let kept: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains("(conflict "))
            .collect();
        assert_eq!(kept.len(), 1, "editor's revision kept");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), b"EDITOR NEW REVISION");
        assert!(kept[0].to_string_lossy().ends_with(").pdf"));
        assert_eq!(done.warnings.len(), 1, "{:?}", done.warnings);
        assert!(done.warnings[0].starts_with("conflict_copy: "));
        assert!(done.warnings[0].contains(&kept[0].display().to_string()));
    }

    #[test]
    fn noreplace_rename_refuses_files_and_dangling_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let e = rename_noreplace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&b).unwrap(), b"b");
        #[cfg(unix)]
        {
            let link = d.path().join("dangling");
            std::os::unix::fs::symlink(d.path().join("nowhere"), &link).unwrap();
            let e = rename_noreplace(&a, &link).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            // The OS-level primitive also refuses (no check-then-act window).
            assert_eq!(
                imp::rename_noreplace(&a, &link).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
        }
        let c = d.path().join("c");
        rename_noreplace(&a, &c).unwrap();
        assert!(!a.exists() && std::fs::read(&c).unwrap() == b"a");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_ignores_planted_temp_symlinks_and_starts_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let victim = d.path().join("victim");
        std::fs::write(&victim, b"precious").unwrap();
        let dst = d.path().join("journal.json");
        // The old predictable names, planted as symlinks to the victim.
        for name in [
            format!(".journal.json.tmp-{}", std::process::id()),
            format!(".journal.json.arp-{}-0.tmp", std::process::id()),
        ] {
            std::os::unix::fs::symlink(&victim, d.path().join(name)).unwrap();
        }
        write_atomic(&dst, b"new").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        let m = std::fs::symlink_metadata(&dst).unwrap();
        assert!(m.file_type().is_file());
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        let t = temp_in(d.path()).unwrap();
        assert_eq!(
            t.as_file().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Existing permissions are kept on replace.
        std::fs::set_permissions(&dst, Permissions::from_mode(0o640)).unwrap();
        write_atomic(&dst, b"newer").unwrap();
        assert_eq!(
            std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn replace_verified_only_displaces_the_expected_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("doc.pdf");
        std::fs::write(&p, b"v1").unwrap();
        let (id, sha) = identify(&p).unwrap();
        // A concurrent writer changes the file after it was checked.
        std::fs::write(&p, b"v2 by editor").unwrap();
        let tmp = write_temp(d.path(), b"ours", None).unwrap();
        let e = replace_verified(&p, tmp, id, &sha).unwrap_err();
        assert_eq!(e.code(), "conflict");
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"v2 by editor",
            "editor's save kept"
        );
        assert_eq!(
            std::fs::read_dir(d.path()).unwrap().count(),
            1,
            "no backup/temp left"
        );
        // Unchanged → replaced, nothing left behind.
        let (id, sha) = identify(&p).unwrap();
        let tmp = write_temp(d.path(), b"ours", None).unwrap();
        replace_verified(&p, tmp, id, &sha).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"ours");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
        // Missing file → conflict, temp cleaned.
        let gone = d.path().join("gone.pdf");
        let tmp = write_temp(d.path(), b"x", None).unwrap();
        assert_eq!(
            replace_verified(&gone, tmp, id, &sha).unwrap_err().code(),
            "conflict"
        );
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn write_new_never_replaces() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("plan.json");
        write_new(&p, b"1").unwrap();
        assert_eq!(
            write_new(&p, b"2").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&p).unwrap(), b"1");
    }

    #[test]
    fn file_id_tracks_replacement() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x");
        std::fs::write(&p, b"1").unwrap();
        let f = File::open(&p).unwrap();
        let id = file_id(&f).unwrap();
        assert_eq!(path_id(&p).unwrap(), id);
        drop(f); // Windows refuses to replace a file that is still open
        write_atomic(&p, b"1").unwrap();
        assert_ne!(path_id(&p).unwrap(), id);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_round_trip_through_json() {
        use std::os::unix::ffi::OsStrExt;
        #[derive(serde::Serialize, serde::Deserialize)]
        struct W(#[serde(with = "path_serde")] std::path::PathBuf);
        let p = Path::new(std::ffi::OsStr::from_bytes(b"/in/scan-\xff.pdf")).to_path_buf();
        let json = serde_json::to_string(&W(p.clone())).unwrap();
        assert!(json.contains("unix_bytes"));
        assert_eq!(serde_json::from_str::<W>(&json).unwrap().0, p);
        let plain = serde_json::to_string(&W("/in/ok.pdf".into())).unwrap();
        assert_eq!(plain, r#""/in/ok.pdf""#);
    }
}
