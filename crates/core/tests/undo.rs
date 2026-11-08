//! Undo journal: target selection, safety rules, full-batch revert.

mod common;

use std::path::{Path, PathBuf};

use autorename_core::harmonize::{self, Choice};
use autorename_core::journal::{self, Journal, UndoReport, UndoStatus, UndoTarget};
use chrono::{NaiveDate, TimeDelta, Utc};
use common::tempdir;

const F: &str = harmonize::COMPANY_FIELD;

fn touch(p: &Path) {
    std::fs::write(p, b"pdf").unwrap();
}

/// Write a journal with one batch per `(id, [(old, new)])`.
fn seed(log: &Path, batches: &[(&str, Vec<(PathBuf, PathBuf)>)]) {
    let now = Utc::now();
    let mut j = Journal::default();
    for (id, renames) in batches {
        for (old, new) in renames {
            journal::record_rename(&mut j, id, old, new, now);
        }
    }
    journal::write_journal(log, &mut j, now).unwrap();
}

fn undo_file(log: &Path, target: UndoTarget) -> UndoReport {
    let mut j = journal::read_journal(log).unwrap();
    let r = journal::undo(&mut j, &target);
    journal::write_journal(log, &mut j, Utc::now()).unwrap();
    r
}

#[test]
fn undo_success_marks_batch_and_keeps_log() {
    let d = tempdir();
    let (old, new) = (d.join("original.pdf"), d.join("20240315 ACME ER.pdf"));
    touch(&new);
    let log = d.join("journal.json");
    seed(&log, &[("batch-001", vec![(old.clone(), new.clone())])]);
    let r = undo_file(&log, UndoTarget::Last);
    assert_eq!((r.restored, r.failed, r.files.len()), (1, 0, 1));
    assert_eq!(r.files[0].status, UndoStatus::Restored);
    // old_path = original (restored), new_path = renamed.
    assert_eq!(
        (
            PathBuf::from(&r.files[0].old_path),
            PathBuf::from(&r.files[0].new_path)
        ),
        (old.clone(), new.clone())
    );
    assert!(old.exists() && !new.exists() && log.exists());
    assert!(journal::read_journal(&log).unwrap().batches[0].undone);
}

#[test]
fn undo_missing_and_occupied_files_fail() {
    let d = tempdir();
    let log = d.join("journal.json");
    let (occupied_old, occupied_new) = (d.join("o.pdf"), d.join("n.pdf"));
    touch(&occupied_old);
    touch(&occupied_new);
    seed(
        &log,
        &[(
            "b",
            vec![
                (d.join("x.pdf"), d.join("y.pdf")),
                (occupied_old.clone(), occupied_new.clone()),
            ],
        )],
    );
    let r = undo_file(&log, UndoTarget::Last);
    assert_eq!((r.restored, r.failed), (0, 2));
    assert!(
        r.files
            .iter()
            .all(|f| f.status == UndoStatus::Failed && f.error.is_some())
    );
    assert!(
        occupied_old.exists() && occupied_new.exists(),
        "never overwrites"
    );
    assert!(r.has_failures());
}

#[test]
fn undo_target_selection() {
    let d = tempdir();
    let log = d.join("journal.json");
    let mk = |i: u32| {
        (
            d.join(format!("orig{i}.pdf")),
            d.join(format!("renamed{i}.pdf")),
        )
    };
    let ((o1, n1), (o2, n2)) = (mk(1), mk(2));
    let reset = || {
        for p in [&o1, &o2, &n1, &n2] {
            let _ = std::fs::remove_file(p);
        }
        touch(&n1);
        touch(&n2);
        seed(
            &log,
            &[
                ("b1", vec![(o1.clone(), n1.clone())]),
                ("b2", vec![(o2.clone(), n2.clone())]),
            ],
        );
    };
    reset();
    let r = undo_file(&log, UndoTarget::Last);
    assert_eq!(
        (r.restored, r.batch_ids.clone()),
        (1, vec!["b2".to_string()])
    );
    assert!(o2.exists() && n1.exists() && !o1.exists());
    reset();
    assert_eq!(undo_file(&log, UndoTarget::Batch("b1".into())).restored, 1);
    assert!(o1.exists() && n2.exists());
    assert_eq!(
        undo_file(&log, UndoTarget::Batch("b1".into())),
        UndoReport::default(),
        "already undone"
    );
    reset();
    assert_eq!(undo_file(&log, UndoTarget::All).restored, 2);
    assert!(o1.exists() && o2.exists());
    assert_eq!(undo_file(&log, UndoTarget::All), UndoReport::default());
    let mut empty = journal::read_journal(&d.join("none.json")).unwrap();
    assert_eq!(
        journal::undo(&mut empty, &UndoTarget::Last),
        UndoReport::default()
    );
}

#[test]
fn corrupt_journal_is_an_error_not_overwritten() {
    let d = tempdir();
    let log = d.join("journal.json");
    std::fs::write(&log, "{corrupt").unwrap();
    assert!(journal::read_journal(&log).is_err());
    assert!(journal::update(&log, Utc::now(), |_| ()).is_err());
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "{corrupt");
}

#[test]
fn runs_without_changes_leave_no_batch() {
    let d = tempdir();
    let (old, new) = (d.join("original.pdf"), d.join("20240315 ACME ER.pdf"));
    touch(&new);
    let log = d.join("journal.json");
    let now = Utc::now();
    journal::update(&log, now, |j| {
        journal::record_rename(j, "real-batch", &old, &new, now)
    })
    .unwrap();
    // A later run that changed nothing records no batch …
    journal::update(&log, now, |j| {
        journal::batch_mut(j, "skip-batch", "cli", now);
    })
    .unwrap();
    let j = journal::read_journal(&log).unwrap();
    assert_eq!(journal::list_batches(&j).len(), 1);
    // … so plain `undo` reverts the real batch.
    let r = undo_file(&log, UndoTarget::Last);
    assert_eq!(r.restored, 1);
    assert!(old.exists() && !new.exists());
}

#[test]
fn same_batch_appends_and_prunes_on_write() {
    let d = tempdir();
    let log = d.join("journal.json");
    let now = Utc::now();
    for i in 0..2 {
        journal::update(&log, now, |j| {
            journal::record_rename(
                j,
                "batch-x",
                &d.join(format!("o{i}")),
                &d.join(format!("n{i}")),
                now,
            )
        })
        .unwrap();
    }
    let mut j = journal::read_journal(&log).unwrap();
    assert_eq!((j.batches.len(), j.batches[0].renames.len()), (1, 2));
    journal::record_rename(&mut j, "newer", &d.join("x"), &d.join("y"), now);
    journal::batch_mut(&mut j, "batch-x", "cli", now).timestamp =
        journal::timestamp(now - TimeDelta::days(31));
    journal::write_journal(&log, &mut j, now).unwrap();
    let ids: Vec<String> = journal::read_journal(&log)
        .unwrap()
        .batches
        .into_iter()
        .map(|b| b.batch_id)
        .collect();
    assert_eq!(ids, ["newer"]);
}

#[test]
fn full_batch_undo_reverts_enrich_rename_and_aliases() {
    let d = tempdir();
    let names = d.join("name-lists/company_name.yaml");
    std::fs::create_dir_all(d.join("name-lists")).unwrap();
    std::fs::write(
        &names,
        "# hand-written\nACME:\n  aliases: [ACME Corporation]\n",
    )
    .unwrap();
    let (orig, renamed) = (d.join("scan.pdf"), d.join("20260906 ACME AP 12,13.pdf"));
    let original = b"%PDF-1.7\noriginal".to_vec();
    let mut post = original.clone();
    post.extend_from_slice(b"\nappended update");
    std::fs::write(&renamed, &post).unwrap();

    // Simulate a run: learn two names, enrich, rename.
    let today = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
    let mut db = harmonize::load_db(&names, F).unwrap();
    let c1 = harmonize::learn(
        &mut db,
        &Choice::Alias {
            canonical: "ACME".into(),
            alias: "ACME Holding".into(),
        },
        today,
    )
    .unwrap();
    let c2 = harmonize::learn(
        &mut db,
        &Choice::New {
            name: "Initech".into(),
            review: true,
        },
        today,
    )
    .unwrap();
    harmonize::save_db(&db, &names).unwrap();
    let log = d.join("journal.json");
    let now = Utc::now();
    let mut j = Journal::default();
    journal::record_alias(&mut j, "run", &names, F, c1, now);
    journal::record_alias(&mut j, "run", &names, F, c2, now);
    journal::record_enrich(
        &mut j,
        "run",
        journal::enrich_entry(&renamed, &original, &post),
        now,
    );
    journal::record_rename(&mut j, "run", &orig, &renamed, now);
    journal::write_journal(&log, &mut j, now).unwrap();

    let r = undo_file(&log, UndoTarget::Last);
    assert!(!r.has_failures(), "{r:?}");
    assert_eq!(
        std::fs::read(&orig).unwrap(),
        original,
        "byte-identical after undo"
    );
    assert!(!renamed.exists());
    assert!(r.aliases.iter().all(|a| a.reverted));
    let db = harmonize::load_db(&names, F).unwrap();
    assert_eq!(db.entries.len(), 1);
    assert!(db.entries[0].learned.is_empty());
    assert_eq!(db.entries[0].aliases, vec!["ACME Corporation"]);
}

#[test]
fn enrich_conflict_is_reported_not_forced() {
    let d = tempdir();
    let p = d.join("a.pdf");
    std::fs::write(&p, b"orig+post").unwrap();
    let entry = journal::enrich_entry(&p, b"orig", b"orig+post");
    std::fs::write(&p, b"orig+post+user-edit").unwrap();
    let mut j = Journal::default();
    journal::record_enrich(&mut j, "b", entry, Utc::now());
    let r = journal::undo(&mut j, &UndoTarget::Last);
    assert_eq!(r.enrich[0].code, Some("conflict"));
    assert!(r.has_failures());
    assert_eq!(std::fs::read(&p).unwrap(), b"orig+post+user-edit");
    assert!(!j.batches[0].undone, "conflicts stay retryable");
}
