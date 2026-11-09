//! Undo journal (`journal.json` in the OS data dir). One batch per run holds
//! renames, enrichment records and learned aliases; undo reverts all three.
//!
//! Concurrency: every read-modify-write holds an exclusive advisory lock on
//! `journal.json.lock` ([`update`], [`write_journal`], [`undo_at`]). A caller
//! holding a [`Journal`] in memory for a whole run writes with
//! [`write_journal`], which merges only the batches it touched (tracked via
//! [`batch_mut`] / [`undo`]) into the current on-disk journal.
//!
//! Every operation carries its own `undone` flag: failed undo steps stay
//! retryable; a batch is `undone` once all of its operations are.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::app_data_dir;
use crate::fsx::{self, path_serde};
use crate::harmonize::{self, Change};
use crate::naming::rename_file;
use crate::{Error, Result};

pub const JOURNAL_VERSION: u32 = 4;
pub const JOURNAL_FILE_NAME: &str = "journal.json";
pub const RETENTION_DAYS: i64 = 30;
const RENAME_RETRIES: u32 = 3;
const RENAME_DELAY: Duration = Duration::from_secs(1);

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameEntry {
    #[serde(with = "path_serde")]
    pub old_path: PathBuf,
    #[serde(with = "path_serde")]
    pub new_path: PathBuf,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub undone: bool,
}

/// Enrichment record: the incremental update appended to the original bytes.
/// `path` is where the enriched file lives (post-rename; moved back by undo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrichEntry {
    #[serde(with = "path_serde")]
    pub path: PathBuf,
    pub orig_len: u64,
    pub orig_sha256: String,
    pub post_sha256: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub undone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasEntry {
    /// Name list file that was changed (resolved when learned; undo edits
    /// this file whatever profile is active then).
    #[serde(with = "path_serde")]
    pub file: PathBuf,
    /// Field of that list (decides how its names are matched).
    pub field: String,
    #[serde(flatten)]
    pub change: Change,
    #[serde(default, skip_serializing_if = "is_false")]
    pub undone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    pub batch_id: String,
    pub timestamp: String,
    /// `cli` | `gui`
    pub source: String,
    /// All operations are undone (an empty batch: consumed by one undo).
    pub undone: bool,
    /// An apply run is still writing this batch (see [`begin_run`]); undo
    /// refuses it while that run is alive.
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_progress: bool,
    pub renames: Vec<RenameEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enrich: Vec<EnrichEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<AliasEntry>,
}

impl Batch {
    /// No recorded operation at all.
    pub fn is_empty(&self) -> bool {
        self.renames.is_empty() && self.enrich.is_empty() && self.aliases.is_empty()
    }

    fn all_undone(&self) -> bool {
        self.renames.iter().all(|e| e.undone)
            && self.enrich.iter().all(|e| e.undone)
            && self.aliases.iter().all(|e| e.undone)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub batches: Vec<Batch>,
    /// Batches changed in memory since the last read/write.
    #[serde(skip)]
    dirty: BTreeSet<String>,
}

impl PartialEq for Journal {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.batches == other.batches
    }
}

impl Eq for Journal {}

impl Default for Journal {
    fn default() -> Self {
        Journal {
            version: JOURNAL_VERSION,
            batches: Vec::new(),
            dirty: BTreeSet::new(),
        }
    }
}

pub fn default_journal_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join(JOURNAL_FILE_NAME))
}

pub fn timestamp(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Micros, false)
}

/// `YYYYMMDDTHHMMSS-<6 hex>` (UTC).
pub fn new_batch_id(now: DateTime<Utc>) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_i64(now.timestamp_nanos_opt().unwrap_or_default());
    h.write_u32(std::process::id());
    format!(
        "{}-{:06x}",
        now.format("%Y%m%dT%H%M%S"),
        h.finish() & 0xff_ffff
    )
}

// ---------------------------------------------------------------------------
// Read / write
// ---------------------------------------------------------------------------

/// Parse journal JSON (this format and version only).
pub fn parse_journal(text: &str) -> Result<Journal> {
    let journal: Journal = serde_json::from_str(text)
        .map_err(|e| Error::General(format!("undo journal is corrupt: {e}")))?;
    if journal.version != JOURNAL_VERSION {
        return Err(Error::General(format!(
            "unsupported undo journal version {}",
            journal.version
        )));
    }
    Ok(journal)
}

/// Missing → empty journal. Corrupt → error (never silently overwritten).
pub fn read_journal(path: &Path) -> Result<Journal> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_journal(&text).map_err(|e| Error::Config {
            message: format!("{}: {e}", path.display()),
            hint: Some(format!(
                "Move {} aside (e.g. rename it to journal.json.bak) to start a new history; \
                 the runs recorded in it can then no longer be undone.",
                path.display()
            )),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Journal::default()),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn parse_ts(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts)
        .map(|d| d.with_timezone(&Utc))
        .ok()
}

/// Drop batches older than 30 days (unparseable timestamps are kept).
pub fn prune(journal: &mut Journal, now: DateTime<Utc>) {
    let cutoff = now - TimeDelta::days(RETENTION_DAYS);
    journal
        .batches
        .retain(|b| parse_ts(&b.timestamp).is_none_or(|t| t >= cutoff));
}

/// Exclusive cross-process lock guarding `path` (held until drop).
fn lock(path: &Path) -> Result<File> {
    fsx::lock_sibling(path).map_err(|e| Error::io(path, e))
}

fn write_locked(path: &Path, journal: &mut Journal, now: DateTime<Utc>) -> Result<()> {
    prune(journal, now);
    // A batch without a single recorded change is never persisted (only the
    // in-progress preflight marker of a running apply).
    journal.batches.retain(|b| b.in_progress || !b.is_empty());
    journal.version = JOURNAL_VERSION;
    journal.dirty.clear();
    let mut text =
        serde_json::to_string_pretty(journal).map_err(|e| Error::General(e.to_string()))?;
    text.push('\n');
    fsx::write_atomic(path, text.as_bytes()).map_err(|e| Error::io(path, e))
}

/// Persist the batches `journal` changed (see module docs) into the current
/// on-disk journal under the lock; `journal` becomes the merged result.
pub fn write_journal(path: &Path, journal: &mut Journal, now: DateTime<Utc>) -> Result<()> {
    let _lock = lock(path)?;
    let mut disk = read_journal(path)?;
    for id in &journal.dirty {
        let Some(mine) = journal.batches.iter().find(|b| &b.batch_id == id) else {
            // Removed in memory (see `remove_batch`).
            disk.batches.retain(|b| &b.batch_id != id);
            continue;
        };
        match disk.batches.iter_mut().find(|b| &b.batch_id == id) {
            Some(slot) => *slot = mine.clone(),
            None => disk.batches.push(mine.clone()),
        }
    }
    write_locked(path, &mut disk, now)?;
    *journal = disk;
    Ok(())
}

/// Locked read-modify-write (one durable write per call).
pub fn update<T>(path: &Path, now: DateTime<Utc>, f: impl FnOnce(&mut Journal) -> T) -> Result<T> {
    let _lock = lock(path)?;
    let mut journal = read_journal(path)?;
    let out = f(&mut journal);
    write_locked(path, &mut journal, now)?;
    Ok(out)
}

/// Undo against the journal file, holding the lock throughout. A selected
/// batch whose apply run is still alive is refused ([`Error::Conflict`]); one
/// left `in_progress` by a crashed run is undone normally.
pub fn undo_at(path: &Path, target: &UndoTarget, now: DateTime<Utc>) -> Result<UndoReport> {
    update(path, now, |j| {
        if let UndoTarget::Batch(id) = target {
            check_newer_batches(j, id)?;
        }
        for i in select_targets(j, target) {
            let b = &mut j.batches[i];
            if b.in_progress {
                if run_alive(path, &b.batch_id) {
                    return Err(Error::Conflict {
                        message: format!(
                            "batch {} is still being applied by another autorename-pdf run",
                            b.batch_id
                        ),
                    });
                }
                b.in_progress = false;
            }
        }
        Ok(undo(j, target))
    })?
}

/// Files a batch's still-open operations touch (both rename ends, enrich).
fn open_paths(b: &Batch) -> BTreeSet<&Path> {
    let mut v = BTreeSet::new();
    for r in b.renames.iter().filter(|r| !r.undone) {
        v.insert(r.old_path.as_path());
        v.insert(r.new_path.as_path());
    }
    for e in b.enrich.iter().filter(|e| !e.undone) {
        v.insert(e.path.as_path());
    }
    v
}

/// `undo --batch X` while a newer open batch touched the same files would
/// undo out of order (wrong names, hash conflicts): refuse before changing
/// anything.
fn check_newer_batches(j: &Journal, id: &str) -> Result<()> {
    let Some(pos) = j.batches.iter().position(|b| b.batch_id == id) else {
        return Ok(());
    };
    let mine = open_paths(&j.batches[pos]);
    let blocking: Vec<&str> = j.batches[pos + 1..]
        .iter()
        .filter(|b| !b.undone && !open_paths(b).is_disjoint(&mine))
        .map(|b| b.batch_id.as_str())
        .collect();
    if blocking.is_empty() {
        return Ok(());
    }
    Err(Error::Blocked {
        message: format!(
            "batch {id} cannot be undone yet: newer batch(es) {} changed the same file(s)",
            blocking.join(", ")
        ),
        hint: Some(format!(
            "Undo the newer batch(es) first (`autorename-pdf undo --batch {}` …), or everything newest-first with `autorename-pdf undo --all`.",
            blocking.last().copied().unwrap_or_default()
        )),
    })
}

fn run_lock_path(journal: &Path, batch_id: &str) -> PathBuf {
    let mut name = journal.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{batch_id}.run"));
    journal.with_file_name(name)
}

/// Liveness token of an apply run: an exclusive lock on
/// `journal.json.<batch>.run`, held until [`RunGuard`] drops (process exit
/// releases it too, so a crash never blocks undo forever).
pub struct RunGuard {
    path: PathBuf,
    _lock: File,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Start an apply run for `batch_id` (see [`RunGuard`]).
pub fn begin_run(journal: &Path, batch_id: &str) -> Result<RunGuard> {
    let path = run_lock_path(journal, batch_id);
    let lock = fsx::lock_path(&path).map_err(|e| Error::io(&path, e))?;
    Ok(RunGuard { path, _lock: lock })
}

fn run_alive(journal: &Path, batch_id: &str) -> bool {
    let path = run_lock_path(journal, batch_id);
    match File::open(&path) {
        Ok(f) => matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        Err(_) => false,
    }
}

/// Drop a batch (e.g. a run that changed nothing); persisted by
/// [`write_journal`].
pub fn remove_batch(journal: &mut Journal, batch_id: &str) {
    journal.batches.retain(|b| b.batch_id != batch_id);
    journal.dirty.insert(batch_id.to_string());
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

/// Find or lazily create the batch (marks it changed for [`write_journal`]).
pub fn batch_mut<'a>(
    journal: &'a mut Journal,
    batch_id: &str,
    source: &str,
    now: DateTime<Utc>,
) -> &'a mut Batch {
    journal.dirty.insert(batch_id.to_string());
    let pos = match journal.batches.iter().position(|b| b.batch_id == batch_id) {
        Some(p) => p,
        None => {
            journal.batches.push(Batch {
                batch_id: batch_id.into(),
                timestamp: timestamp(now),
                source: source.into(),
                undone: false,
                in_progress: false,
                renames: Vec::new(),
                enrich: Vec::new(),
                aliases: Vec::new(),
            });
            journal.batches.len() - 1
        }
    };
    &mut journal.batches[pos]
}

pub fn record_rename(
    journal: &mut Journal,
    batch_id: &str,
    old: &Path,
    new: &Path,
    now: DateTime<Utc>,
) {
    batch_mut(journal, batch_id, "cli", now)
        .renames
        .push(RenameEntry {
            old_path: old.to_path_buf(),
            new_path: new.to_path_buf(),
            timestamp: timestamp(now),
            undone: false,
        });
}

pub fn record_enrich(
    journal: &mut Journal,
    batch_id: &str,
    entry: EnrichEntry,
    now: DateTime<Utc>,
) {
    batch_mut(journal, batch_id, "cli", now).enrich.push(entry);
}

pub fn record_alias(
    journal: &mut Journal,
    batch_id: &str,
    file: &Path,
    field: &str,
    change: Change,
    now: DateTime<Utc>,
) {
    batch_mut(journal, batch_id, "cli", now)
        .aliases
        .push(AliasEntry {
            file: file.to_path_buf(),
            field: field.to_string(),
            change,
            undone: false,
        });
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchSummary {
    pub batch_id: String,
    pub timestamp: String,
    /// Renamed files.
    pub file_count: usize,
    /// Enriched files (metadata embedded), renamed or not.
    pub metadata_count: usize,
    /// Learned names (all name lists).
    pub name_count: usize,
    pub undone: bool,
}

pub fn list_batches(journal: &Journal) -> Vec<BatchSummary> {
    journal
        .batches
        .iter()
        .map(|b| BatchSummary {
            batch_id: b.batch_id.clone(),
            timestamp: b.timestamp.clone(),
            file_count: b.renames.len(),
            metadata_count: b.enrich.len(),
            name_count: b.aliases.len(),
            undone: b.undone,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Enrichment hashes + undo
// ---------------------------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Journal record for an enrichment: original bytes are a prefix of `post`.
pub fn enrich_entry(path: &Path, original: &[u8], post: &[u8]) -> EnrichEntry {
    EnrichEntry {
        path: path.to_path_buf(),
        orig_len: original.len() as u64,
        orig_sha256: sha256_hex(original),
        post_sha256: sha256_hex(post),
        undone: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnrichOutcome {
    /// Truncated back to the original bytes.
    Restored,
    /// File already equals the original.
    AlreadyOriginal,
}

fn conflict(path: &Path, what: &str) -> Error {
    Error::Conflict {
        message: format!("{} {what}; not restoring", path.display()),
    }
}

/// Restore the first `orig_len` bytes iff the file still hashes to
/// `post_sha256` and its prefix to `orig_sha256`; otherwise
/// [`Error::Conflict`].
///
/// Never truncates in place: the original bytes go to a private temp
/// sibling, then [`fsx::replace_verified`] swaps it in only if the displaced
/// file is still the exact object and bytes that were checked (a concurrent
/// save is kept and reported as a conflict).
pub fn undo_enrich(path: &Path, entry: &EnrichEntry) -> Result<EnrichOutcome> {
    undo_enrich_full(path, entry).map(|(o, _)| o)
}

/// [`undo_enrich`] plus `durability_warning`s of a committed restore.
fn undo_enrich_full(path: &Path, entry: &EnrichEntry) -> Result<(EnrichOutcome, Vec<String>)> {
    let io = |e| Error::io(path, e);
    let mut f = File::open(path).map_err(io)?;
    let id = fsx::file_id(&f).map_err(io)?;
    let perms = f.metadata().map_err(io)?.permissions();
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(io)?;
    drop(f);
    let full = sha256_hex(&bytes);
    let orig_len = usize::try_from(entry.orig_len).unwrap_or(usize::MAX);
    if bytes.len() == orig_len && full == entry.orig_sha256 {
        return Ok((EnrichOutcome::AlreadyOriginal, Vec::new()));
    }
    if full != entry.post_sha256
        || bytes.len() < orig_len
        || sha256_hex(&bytes[..orig_len]) != entry.orig_sha256
    {
        return Err(conflict(path, "changed after enrichment"));
    }
    let tmp =
        fsx::write_temp(fsx::parent_dir(path), &bytes[..orig_len], Some(perms)).map_err(io)?;
    let done = fsx::replace_verified(path, tmp, id, &full)?;
    Ok((EnrichOutcome::Restored, done.warnings))
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoTarget {
    /// Last batch that is not fully undone (an empty batch is a no-op that
    /// consumes it).
    Last,
    Batch(String),
    /// Every open batch, newest first.
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UndoStatus {
    Restored,
    Failed,
}

/// Per-file undo result:
/// `old_path` = original name (restored), `new_path` = renamed path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct UndoFile {
    #[serde(with = "path_serde")]
    #[schemars(with = "path_serde::Repr")]
    pub old_path: PathBuf,
    #[serde(with = "path_serde")]
    #[schemars(with = "path_serde::Repr")]
    pub new_path: PathBuf,
    pub status: UndoStatus,
    pub error: Option<String>,
    /// `durability_warning: …` (the restore happened).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct UndoEnrich {
    #[serde(with = "path_serde")]
    #[schemars(with = "path_serde::Repr")]
    pub path: PathBuf,
    pub status: Option<EnrichOutcome>,
    pub error: Option<String>,
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct UndoAlias {
    #[serde(with = "path_serde")]
    #[schemars(with = "path_serde::Repr")]
    pub file: PathBuf,
    /// Field of the name list.
    pub field: String,
    #[serde(flatten)]
    pub change: Change,
    pub reverted: bool,
    /// Why nothing was reverted although no error happened, e.g.
    /// `kept: accepted by user`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct UndoReport {
    pub batch_ids: Vec<String>,
    pub restored: usize,
    /// Failed operations: renames, enrichment restores and learned names.
    pub failed: usize,
    pub files: Vec<UndoFile>,
    pub enrich: Vec<UndoEnrich>,
    pub aliases: Vec<UndoAlias>,
}

impl UndoReport {
    pub fn has_failures(&self) -> bool {
        self.failed > 0
            || self.enrich.iter().any(|e| e.error.is_some())
            || self.aliases.iter().any(|a| a.error.is_some())
    }
}

/// Newest first.
fn select_targets(journal: &Journal, target: &UndoTarget) -> Vec<usize> {
    let mut open = journal
        .batches
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, b)| !b.undone);
    match target {
        UndoTarget::All => open.map(|(i, _)| i).collect(),
        UndoTarget::Batch(id) => open
            .filter(|(_, b)| &b.batch_id == id)
            .map(|(i, _)| i)
            .collect(),
        UndoTarget::Last => open.next().map(|(i, _)| i).into_iter().collect(),
    }
}

fn undo_rename(entry: &RenameEntry) -> UndoFile {
    let (new, old) = (&entry.new_path, &entry.old_path);
    let result = if std::fs::symlink_metadata(new).is_err() {
        Err(format!("File not found: {}", new.display()))
    } else if std::fs::symlink_metadata(old).is_ok() {
        Err(format!("Original path is occupied: {}", old.display()))
    } else {
        rename_file(new, old, RENAME_RETRIES, RENAME_DELAY).map_err(|e| e.to_string())
    };
    UndoFile {
        old_path: old.clone(),
        new_path: new.clone(),
        status: if result.is_ok() {
            UndoStatus::Restored
        } else {
            UndoStatus::Failed
        },
        warnings: result
            .as_ref()
            .map(|d| d.warnings.clone())
            .unwrap_or_default(),
        error: result.err(),
    }
}

/// Revert open alias entries newest first, one load/save per file. Entries
/// already gone count as done; errors/conflicts stay open.
fn undo_aliases(entries: &mut [AliasEntry]) -> Vec<UndoAlias> {
    let mut by_file: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate().rev().filter(|(_, e)| !e.undone) {
        by_file.entry(e.file.clone()).or_default().push(i);
    }
    let mut out = Vec::new();
    for (path, idxs) in by_file {
        // One file = one list; its field decides how names match.
        let field = entries[idxs[0]].field.clone();
        // Locked reload-modify-save: concurrent learning is never lost.
        let changed = harmonize::update_db(&path, &field, |db| {
            Ok(idxs
                .iter()
                .map(|&i| {
                    let r = harmonize::unlearn(db, &entries[i].change).map_err(|e| e.to_string());
                    (i, r)
                })
                .collect::<Vec<_>>())
        });
        let results =
            changed.unwrap_or_else(|e| idxs.iter().map(|&i| (i, Err(e.to_string()))).collect());
        for (i, r) in results {
            let e = &mut entries[i];
            e.undone = r.is_ok();
            out.push(UndoAlias {
                file: e.file.clone(),
                field: e.field.clone(),
                change: e.change.clone(),
                // Already gone (user edited the file) is not a failure.
                reverted: r == Ok(harmonize::Unlearned::Reverted),
                note: match &r {
                    Ok(harmonize::Unlearned::Kept(why)) => Some(format!("kept: {why}")),
                    Ok(harmonize::Unlearned::Gone) => Some("already removed".into()),
                    _ => None,
                },
                error: r.err(),
            });
        }
    }
    out
}

/// Revert the selected batches newest first: per batch enrichment
/// (truncate), renames (reverse order; enrichment records follow the file
/// back), then learned aliases. Each successful operation is marked undone;
/// failed ones stay open for a retry. Persist with [`write_journal`] (or use
/// [`undo_at`]).
pub fn undo(journal: &mut Journal, target: &UndoTarget) -> UndoReport {
    let mut report = UndoReport::default();
    // Files whose newer operation failed (or was blocked): older operations
    // on them must not run (they would act on the wrong bytes or name).
    let mut failed: BTreeSet<PathBuf> = BTreeSet::new();
    let blocked = |p: &Path| Error::Blocked {
        message: format!(
            "blocked_by_failed_newer: a newer change to {} could not be undone; this older one waits (retry after fixing it)",
            p.display()
        ),
        hint: None,
    };
    for i in select_targets(journal, target) {
        let id = journal.batches[i].batch_id.clone();
        journal.dirty.insert(id.clone());
        report.batch_ids.push(id);
        let batch = &mut journal.batches[i];
        for e in batch.enrich.iter_mut().rev().filter(|e| !e.undone) {
            let r = if failed.contains(&e.path) {
                Err(blocked(&e.path))
            } else {
                undo_enrich_full(&e.path, e)
            };
            e.undone = r.is_ok();
            if r.is_err() {
                failed.insert(e.path.clone());
            }
            report.enrich.push(UndoEnrich {
                path: e.path.clone(),
                status: r.as_ref().ok().map(|(o, _)| *o),
                warnings: r.as_ref().map(|(_, w)| w.clone()).unwrap_or_default(),
                code: r.as_ref().err().map(|e| match e {
                    Error::Blocked { .. } => "blocked_by_failed_newer",
                    other => other.code(),
                }),
                error: r.err().map(|e| e.to_string()),
            });
        }
        for k in (0..batch.renames.len()).rev() {
            if batch.renames[k].undone {
                continue;
            }
            let r = batch.renames[k].clone();
            let f = if failed.contains(&r.new_path) || failed.contains(&r.old_path) {
                UndoFile {
                    old_path: r.old_path.clone(),
                    new_path: r.new_path.clone(),
                    status: UndoStatus::Failed,
                    error: Some(blocked(&r.new_path).to_string()),
                    warnings: Vec::new(),
                }
            } else {
                undo_rename(&r)
            };
            match f.status {
                UndoStatus::Restored => {
                    report.restored += 1;
                    let r = &mut batch.renames[k];
                    r.undone = true;
                    for e in batch.enrich.iter_mut().filter(|e| !e.undone) {
                        if e.path == r.new_path {
                            e.path = r.old_path.clone();
                        }
                    }
                }
                UndoStatus::Failed => {
                    report.failed += 1;
                    failed.insert(r.new_path.clone());
                    failed.insert(r.old_path.clone());
                }
            }
            report.files.push(f);
        }
        report.aliases.extend(undo_aliases(&mut batch.aliases));
        batch.undone = batch.all_undone();
    }
    // Every failed operation counts (rename, enrichment, learned name).
    report.failed += report.enrich.iter().filter(|e| e.error.is_some()).count()
        + report.aliases.iter().filter(|a| a.error.is_some()).count();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-11-07T10:11:12.345678Z".parse().unwrap()
    }

    #[test]
    fn batch_id_format_and_uniqueness() {
        let ids: std::collections::HashSet<String> =
            (0..100).map(|_| new_batch_id(now())).collect();
        assert_eq!(ids.len(), 100);
        let id = new_batch_id(now());
        assert_eq!(id.len(), 22);
        assert_eq!(&id[..16], "20261107T101112-");
        assert!(id[16..].bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn roundtrip_and_strict_format() {
        assert!(parse_journal("{corrupt").is_err());
        assert!(parse_journal(r#"[{"old_path":"a","new_path":"b","timestamp":"t"}]"#).is_err());
        assert!(parse_journal(r#"{"version":3,"batches":[]}"#).is_err());
        assert_eq!(
            parse_journal(r#"{"version":4,"batches":[]}"#).unwrap(),
            Journal::default()
        );
        // Round trip keeps enrich + aliases.
        let mut j = Journal::default();
        record_enrich(
            &mut j,
            "b",
            EnrichEntry {
                path: "p".into(),
                orig_len: 1,
                orig_sha256: "a".into(),
                post_sha256: "b".into(),
                undone: false,
            },
            now(),
        );
        record_alias(
            &mut j,
            "b",
            Path::new("n.yaml"),
            "company_name",
            Change::EntryAdded {
                canonical: "X".into(),
            },
            now(),
        );
        let text = serde_json::to_string(&j).unwrap();
        assert!(text.contains(r#""kind":"entry_added""#));
        assert_eq!(parse_journal(&text).unwrap(), j);
    }

    #[test]
    fn pruning_compares_real_timestamps() {
        let mut j = Journal::default();
        for (id, ts) in [
            ("old", timestamp(now() - TimeDelta::days(31))),
            ("recent", timestamp(now() - TimeDelta::days(29))),
            // Different offset: lexicographic compare would get this wrong.
            ("offset", "2026-10-08T23:30:00-05:00".to_string()),
            ("garbage", "t".to_string()),
        ] {
            batch_mut(&mut j, id, "cli", now()).timestamp = ts;
        }
        prune(&mut j, now());
        let ids: Vec<&str> = j.batches.iter().map(|b| b.batch_id.as_str()).collect();
        assert_eq!(ids, ["recent", "offset", "garbage"]);
    }

    #[test]
    fn enrich_undo_truncates_only_matching_hash() {
        let dir = tempdir();
        let path = dir.join("doc.pdf");
        let original = b"%PDF-1.7 original bytes".to_vec();
        let mut post = original.clone();
        post.extend_from_slice(b"\n% incremental update\n");
        std::fs::write(&path, &post).unwrap();
        let entry = enrich_entry(&path, &original, &post);
        assert_eq!(undo_enrich(&path, &entry).unwrap(), EnrichOutcome::Restored);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            undo_enrich(&path, &entry).unwrap(),
            EnrichOutcome::AlreadyOriginal
        );
        // Modified after enrichment → conflict, file untouched.
        let mut edited = post.clone();
        edited.push(b'!');
        std::fs::write(&path, &edited).unwrap();
        let err = undo_enrich(&path, &entry).unwrap_err();
        assert_eq!(err.code(), "conflict");
        assert_eq!(std::fs::read(&path).unwrap(), edited);
        assert_eq!(
            undo_enrich(&dir.join("missing.pdf"), &entry)
                .unwrap_err()
                .code(),
            "io_error"
        );
    }

    #[test]
    fn undo_follows_learned_aliases_through_merge_and_keeps_accepted_entries() {
        use harmonize::Choice;
        const F: &str = harmonize::COMPANY_FIELD;
        let dir = tempdir();
        let (log, names) = (dir.join("journal.json"), dir.join("names.yaml"));
        std::fs::write(&names, "ACME: {aliases: [ACME Corporation]}\nGlobex: {}\n").unwrap();
        let today = now().date_naive();
        let learn = |c: Choice| {
            harmonize::update_db(&names, F, |db| Ok(harmonize::learn(db, &c, today)))
                .unwrap()
                .unwrap()
        };
        let alias = learn(Choice::Alias {
            canonical: "ACME".into(),
            alias: "Acme Hldg".into(),
        });
        let entry = learn(Choice::New {
            name: "Initech".into(),
            review: true,
        });
        update(&log, now(), |j| {
            record_alias(j, "run", &names, F, alias, now());
            record_alias(j, "run", &names, F, entry, now());
        })
        .unwrap();
        // The user merges ACME into Globex (the learned alias moves along)
        // and accepts Initech.
        harmonize::update_db(&names, F, |db| {
            harmonize::merge(db, "ACME", "Globex")?;
            harmonize::set_review(db, "Initech", false)
        })
        .unwrap();
        let r = undo_at(&log, &UndoTarget::Last, now()).unwrap();
        assert!(!r.has_failures(), "{r:?}");
        let by_kind = |k: &str| {
            r.aliases
                .iter()
                .find(|a| serde_json::to_value(&a.change).unwrap()["kind"] == k)
                .unwrap()
                .clone()
        };
        assert!(by_kind("alias_added").reverted);
        let kept = by_kind("entry_added");
        assert!(!kept.reverted);
        assert_eq!(kept.note.as_deref(), Some("kept: accepted by user"));
        let db = harmonize::load_db(&names, F).unwrap();
        assert!(
            db.entries
                .iter()
                .all(|e| !e.learned.contains(&"Acme Hldg".to_string()))
        );
        assert!(db.find("Initech").is_some());
    }

    #[test]
    fn concurrent_updates_never_lose_batches() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let log = log.clone();
                std::thread::spawn(move || {
                    for i in 0..5 {
                        // Long-lived in-memory journals (pipeline style) …
                        let mut j = read_journal(&log).unwrap();
                        let id = format!("t{t}-{i}");
                        record_rename(&mut j, &id, Path::new("a"), Path::new("b"), now());
                        write_journal(&log, &mut j, now()).unwrap();
                        // … and locked read-modify-write.
                        update(&log, now(), |j| {
                            record_rename(
                                j,
                                &format!("u{t}-{i}"),
                                Path::new("c"),
                                Path::new("d"),
                                now(),
                            )
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(read_journal(&log).unwrap().batches.len(), 80);
    }

    #[test]
    fn stale_in_memory_journal_does_not_revert_other_writers() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&b, b"x").unwrap();
        update(&log, now(), |j| record_rename(j, "old", &a, &b, now())).unwrap();
        let mut run = read_journal(&log).unwrap(); // a long run starts
        undo_at(&log, &UndoTarget::Last, now()).unwrap(); // another process undoes "old"
        record_rename(&mut run, "new", Path::new("c"), Path::new("d"), now());
        write_journal(&log, &mut run, now()).unwrap();
        let j = read_journal(&log).unwrap();
        assert!(j.batches[0].undone, "undo by the other process survives");
        assert_eq!(j.batches.len(), 2);
    }

    #[test]
    fn undo_refuses_a_batch_whose_run_is_alive() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        let run = begin_run(&log, "live").unwrap();
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&b, b"x").unwrap();
        update(&log, now(), |j| {
            record_rename(j, "live", &a, &b, now());
            batch_mut(j, "live", "cli", now()).in_progress = true
        })
        .unwrap();
        let e = undo_at(&log, &UndoTarget::Last, now()).unwrap_err();
        assert_eq!(e.code(), "conflict");
        assert!(!read_journal(&log).unwrap().batches[0].undone);
        // Run finished or crashed (lock released): undo proceeds.
        drop(run);
        let r = undo_at(&log, &UndoTarget::Last, now()).unwrap();
        assert_eq!(r.batch_ids, ["live"]);
        assert!(read_journal(&log).unwrap().batches[0].undone);
    }

    #[test]
    fn undo_of_an_older_batch_is_blocked_by_newer_changes_to_the_same_file() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        let (a, b, c) = (dir.join("a.pdf"), dir.join("b.pdf"), dir.join("c.pdf"));
        std::fs::write(&c, b"x").unwrap();
        update(&log, now(), |j| {
            record_rename(j, "one", &a, &b, now());
            record_rename(j, "two", &b, &c, now());
            record_rename(j, "other", &dir.join("x.pdf"), &dir.join("y.pdf"), now());
        })
        .unwrap();
        let before = std::fs::read_to_string(&log).unwrap();
        let e = undo_at(&log, &UndoTarget::Batch("one".into()), now()).unwrap_err();
        assert_eq!(e.code(), "undo_blocked");
        assert!(e.to_string().contains("two") && !e.to_string().contains("other"));
        assert!(e.hint().unwrap().contains("undo --all"));
        assert!(c.exists() && !a.exists());
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            before,
            "nothing changed"
        );
        // Newest first works; then the older one is free.
        undo_at(&log, &UndoTarget::Batch("two".into()), now()).unwrap();
        let r = undo_at(&log, &UndoTarget::Batch("one".into()), now()).unwrap();
        assert_eq!(r.restored, 1);
        assert!(a.exists());
    }

    #[test]
    fn empty_batches_are_never_persisted() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        let mut j = Journal::default();
        batch_mut(&mut j, "nothing", "cli", now());
        write_journal(&log, &mut j, now()).unwrap();
        assert!(read_journal(&log).unwrap().batches.is_empty());
    }

    #[test]
    fn removed_batches_are_deleted_on_write() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        update(&log, now(), |j| {
            record_rename(j, "keep", Path::new("a"), Path::new("b"), now())
        })
        .unwrap();
        let mut j = read_journal(&log).unwrap();
        record_rename(&mut j, "drop", Path::new("c"), Path::new("d"), now());
        write_journal(&log, &mut j, now()).unwrap();
        remove_batch(&mut j, "drop");
        write_journal(&log, &mut j, now()).unwrap();
        let ids: Vec<String> = read_journal(&log)
            .unwrap()
            .batches
            .into_iter()
            .map(|b| b.batch_id)
            .collect();
        assert_eq!(ids, ["keep"]);
    }

    #[test]
    fn undo_enrich_never_destroys_a_concurrent_save() {
        // Undo restores via temp + verified replace: a file changed after the
        // hash check is a conflict (exercised in fsx); here the plain path.
        let dir = tempdir();
        let p = dir.join("a.pdf");
        std::fs::write(&p, b"orig+post").unwrap();
        let entry = enrich_entry(&p, b"orig", b"orig+post");
        assert_eq!(undo_enrich(&p, &entry).unwrap(), EnrichOutcome::Restored);
        assert_eq!(std::fs::read(&p).unwrap(), b"orig");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "no temp/backup left"
        );
    }

    #[test]
    fn undo_all_runs_newest_first_and_failures_stay_retryable() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        let (a, b, c) = (dir.join("a.pdf"), dir.join("b.pdf"), dir.join("c.pdf"));
        std::fs::write(&c, b"x").unwrap();
        update(&log, now(), |j| {
            record_rename(j, "one", &a, &b, now());
            record_rename(j, "two", &b, &c, now());
        })
        .unwrap();
        let r = undo_at(&log, &UndoTarget::All, now()).unwrap();
        assert_eq!((r.restored, r.failed), (2, 0), "{r:?}");
        assert_eq!(r.batch_ids, ["two", "one"]);
        assert!(a.exists() && !b.exists() && !c.exists());

        // Occupied original → failed, batch stays open; retry after clearing.
        let (d, e) = (dir.join("d.pdf"), dir.join("e.pdf"));
        std::fs::write(&d, b"blocker").unwrap();
        std::fs::write(&e, b"y").unwrap();
        update(&log, now(), |j| record_rename(j, "three", &d, &e, now())).unwrap();
        let r = undo_at(&log, &UndoTarget::Last, now()).unwrap();
        assert_eq!(r.failed, 1);
        assert!(!read_journal(&log).unwrap().batches[2].undone);
        std::fs::remove_file(&d).unwrap();
        let r = undo_at(&log, &UndoTarget::Last, now()).unwrap();
        assert_eq!(
            (r.restored, r.batch_ids.clone()),
            (1, vec!["three".to_string()])
        );
        assert!(read_journal(&log).unwrap().batches[2].undone);
    }

    #[test]
    fn a_failed_newer_change_blocks_older_changes_to_the_same_file() {
        let dir = tempdir();
        let log = dir.join("journal.json");
        let (a, b, c) = (dir.join("a.pdf"), dir.join("b.pdf"), dir.join("c.pdf"));
        let mut post = b"orig".to_vec();
        post.extend_from_slice(b"+post");
        // Run 1 renamed a→b; run 2 renamed b→c and enriched c; then an
        // editor saved c.
        std::fs::write(&c, b"orig+post+user-edit").unwrap();
        std::fs::write(dir.join("other.pdf"), b"x").unwrap();
        update(&log, now(), |j| {
            record_rename(j, "one", &a, &b, now());
            record_rename(j, "one", &dir.join("x.pdf"), &dir.join("other.pdf"), now());
            record_rename(j, "two", &b, &c, now());
            record_enrich(j, "two", enrich_entry(&c, b"orig", &post), now());
        })
        .unwrap();
        let r = undo_at(&log, &UndoTarget::All, now()).unwrap();
        assert_eq!(r.enrich[0].code, Some("conflict"));
        let blocked: Vec<&UndoFile> = r
            .files
            .iter()
            .filter(|f| {
                f.error
                    .as_deref()
                    .is_some_and(|e| e.starts_with("blocked_by_failed_newer"))
            })
            .collect();
        assert_eq!(blocked.len(), 2, "c→b and b→a both wait: {r:?}");
        assert_eq!(r.restored, 1, "the unrelated file is restored");
        assert_eq!(
            std::fs::read(&c).unwrap(),
            b"orig+post+user-edit",
            "never touched"
        );
        assert!(!a.exists() && !b.exists());
        // Everything stays retryable: after the editor's change is reverted,
        // undo --all finishes the job newest-first.
        std::fs::write(&c, &post).unwrap();
        let r = undo_at(&log, &UndoTarget::All, now()).unwrap();
        assert!(!r.has_failures(), "{r:?}");
        assert_eq!(std::fs::read(&a).unwrap(), b"orig");
    }

    // APFS rejects non-UTF-8 file names (EILSEQ).
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn non_utf8_paths_are_restored_exactly() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempdir();
        let log = dir.join("journal.json");
        let orig = dir.join(std::ffi::OsStr::from_bytes(b"scan-\xff.pdf"));
        let renamed = dir.join("renamed.pdf");
        std::fs::write(&renamed, b"x").unwrap();
        update(&log, now(), |j| {
            record_rename(j, "b", &orig, &renamed, now())
        })
        .unwrap();
        let r = undo_at(&log, &UndoTarget::Last, now()).unwrap();
        assert_eq!(r.restored, 1);
        assert!(
            std::fs::symlink_metadata(&orig).is_ok(),
            "exact bytes restored"
        );
    }

    /// Unique temp dir under the target dir (no extra crate).
    pub(crate) fn tempdir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "arp-core-{}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
