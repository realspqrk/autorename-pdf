//! Explorer runs a command-line verb once per selected item, even with
//! `MultiSelectModel=Player`. Selecting 20 PDFs and choosing "Rename now"
//! starts 20 launchers within a few hundred milliseconds; this module turns
//! them into one GUI process with all 20 paths.
//!
//! Protocol (std only, per verb `key`, in a per-user temp dir). Every
//! spool access happens under an exclusive OS file lock on `<key>.lock`
//! (`File::lock`: `LockFileEx` on Windows, `flock` on Unix), so an append
//! and the leader's take can never interleave:
//! 1. Lock; append the path to `<key>.spool`; if `<key>.leader` does not
//!    exist (or is stale), create it and become the leader; unlock.
//! 2. Followers exit: the leader collects their line.
//! 3. The leader waits until the spool stops growing (QUIET, at most MAX),
//!    then, under the lock, reads and deletes the spool and the leader
//!    marker. A launcher arriving after that starts the next batch.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const TICK: Duration = Duration::from_millis(100);
/// Spool unchanged this long → every sibling launcher has arrived.
pub const QUIET: Duration = Duration::from_millis(400);
pub const MAX: Duration = Duration::from_secs(4);
/// A leader marker older than this belongs to a crashed leader.
pub const STALE: Duration = Duration::from_secs(30);

struct Paths {
    lock: PathBuf,
    spool: PathBuf,
    leader: PathBuf,
}

fn paths(dir: &Path, key: &str) -> Paths {
    Paths {
        lock: dir.join(format!("{key}.lock")),
        spool: dir.join(format!("{key}.spool")),
        leader: dir.join(format!("{key}.leader")),
    }
}

/// Run `f` while holding the exclusive lock (released when the file closes).
fn locked<T>(p: &Paths, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&p.lock)?;
    lock.lock()?;
    let r = f();
    drop(lock);
    r
}

fn stale(marker: &Path) -> bool {
    fs::metadata(marker)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age > STALE)
}

/// Step 1: append `own`; `true` when this process became the leader.
fn append(p: &Paths, own: &str) -> io::Result<bool> {
    locked(p, || {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p.spool)?;
        f.write_all(format!("{own}\n").as_bytes())?;
        f.sync_all()?;
        if p.leader.exists() && !stale(&p.leader) {
            return Ok(false);
        }
        File::create(&p.leader)?;
        Ok(true)
    })
}

/// Step 3: atomically take the batch (deduplicated, in arrival order).
fn take(p: &Paths) -> io::Result<Vec<String>> {
    locked(p, || {
        let mut text = String::new();
        match File::open(&p.spool) {
            Ok(mut f) => {
                f.read_to_string(&mut text)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let _ = fs::remove_file(&p.spool);
        let _ = fs::remove_file(&p.leader);
        let mut out: Vec<String> = Vec::new();
        for line in text.lines().filter(|l| !l.is_empty()) {
            if !out.iter().any(|o| o == line) {
                out.push(line.to_string());
            }
        }
        Ok(out)
    })
}

fn spool_len(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Returns `Some(paths)` when this process is the leader (launch the GUI with
/// them), `None` when another launcher will.
pub fn collect(dir: &Path, key: &str, own: &str) -> io::Result<Option<Vec<String>>> {
    fs::create_dir_all(dir)?;
    let p = paths(dir, key);
    if !append(&p, own)? {
        return Ok(None);
    }
    let start = Instant::now();
    let mut last = spool_len(&p.spool);
    let mut quiet_since = Instant::now();
    while start.elapsed() < MAX {
        std::thread::sleep(TICK);
        let now = spool_len(&p.spool);
        if now != last {
            last = now;
            quiet_since = Instant::now();
        } else if quiet_since.elapsed() >= QUIET {
            break;
        }
    }
    take(&p).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn concurrent_launchers_end_up_in_one_leader() {
        let dir = tmp("coalesce");
        let handles: Vec<_> = (0..12)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(i * 15));
                    collect(&dir, "rename", &format!("C:\\x\\{i}.pdf")).unwrap()
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let mut all: Vec<String> = results.iter().flatten().flatten().cloned().collect();
        all.sort();
        let mut want: Vec<String> = (0..12).map(|i| format!("C:\\x\\{i}.pdf")).collect();
        want.sort();
        assert_eq!(all, want, "every path exactly once");
        let leaders = results.iter().filter(|r| r.is_some()).count();
        assert_eq!(leaders, 1, "{results:?}");
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("rename.lock")]);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Review finding: a follower preempted between opening the spool and
    /// writing lost its path. Now the take waits for the in-flight append.
    #[test]
    fn take_waits_for_an_in_flight_append() {
        let dir = tmp("race");
        let p = paths(&dir, "k");
        assert!(append(&p, "leader.pdf").unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let follower = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let p = paths(&dir, "k");
                locked(&p, || {
                    tx.send(()).unwrap();
                    // Preempted while holding the spool.
                    std::thread::sleep(Duration::from_millis(300));
                    let mut f = OpenOptions::new().append(true).open(&p.spool)?;
                    f.write_all(b"follower.pdf\n")?;
                    Ok(p.leader.exists())
                })
                .unwrap()
            })
        };
        rx.recv().unwrap();
        let got = take(&p).unwrap();
        assert!(follower.join().unwrap(), "follower saw the leader marker");
        assert_eq!(
            got,
            vec!["leader.pdf".to_string(), "follower.pdf".to_string()]
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
