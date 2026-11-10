//! Port of tests/test_profile_filenames.py + rename parts of test_document_processing.py.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use autorename_core::journal::{self, Journal, UndoTarget};
use autorename_core::naming::{Target, plan_target, rename_file};
use chrono::Utc;
use common::{rendered, sample_config, tempdir};
use serde_json::{Value, json};

/// (profile, model values, overlay, date_format, expected)
type Case = (
    &'static str,
    Value,
    Option<Value>,
    Option<&'static str>,
    &'static str,
);

const DATE: &str = "06.09.2026";
const MIXED: &str = "{document_date} {first_author_surname} {journal_name} - {title}";

#[test]
fn spec_filename_table() {
    let c = sample_config();
    let cases: Vec<Case> = vec![
        (
            "business",
            json!({"document_date": DATE, "company_name": "ACME", "document_type": "AP", "description": "12,13"}),
            None,
            None,
            "20260906 ACME AP 12,13.pdf",
        ),
        (
            "business",
            json!({"document_date": DATE, "company_name": "ACME", "document_type": "AR", "description": "1.234,56"}),
            None,
            None,
            "20260906 ACME AR 1.234,56.pdf",
        ),
        (
            "business",
            json!({"document_date": DATE, "company_name": "ACME", "document_type": "AP", "description": "EUR 12,13"}),
            None,
            None,
            "20260906 ACME AP EUR 12,13.pdf",
        ),
        (
            "business",
            json!({"document_date": DATE, "company_name": "ACME", "document_type": "AP", "description": "", "invoice_id": "12345"}),
            Some(json!({"fields": {"invoice_id": {"description": "Printed invoice id."}}})),
            None,
            "20260906 ACME AP.pdf",
        ),
        (
            "business",
            json!({"document_date": DATE, "company_name": "ACME", "document_type": "Letter", "description": "Terminbestätigung"}),
            None,
            None,
            "20260906 ACME Letter Terminbestätigung.pdf",
        ),
        (
            "business",
            json!({"document_date": DATE, "company_name": "<>:", "document_type": "?*", "description": ""}),
            None,
            None,
            "20260906.pdf",
        ),
        (
            "academic",
            json!({"document_date": DATE, "first_author_surname": "Smith", "title": "A Study"}),
            None,
            None,
            "20260906 Smith A Study.pdf",
        ),
        (
            "academic",
            json!({"document_date": DATE, "first_author_surname": "Smith"}),
            Some(
                json!({"template": "{document_date}_{first_author_surname}_{journal_name}_{title}"}),
            ),
            None,
            "20260906_Smith.pdf",
        ),
        (
            "academic",
            json!({"document_date": DATE, "first_author_surname": "Smith", "title": "A Study"}),
            Some(json!({"template": MIXED})),
            None,
            "20260906 Smith - A Study.pdf",
        ),
        (
            "academic",
            json!({"document_date": DATE, "first_author_surname": "Smith"}),
            Some(json!({"template": MIXED})),
            None,
            "20260906 Smith.pdf",
        ),
        (
            "academic",
            json!({"document_date": DATE, "first_author_surname": "Smith-Jones", "title": "A Study"}),
            None,
            Some("%Y-%m-%d"),
            "2026-09-06 Smith-Jones A Study.pdf",
        ),
        (
            "empty",
            json!({}),
            Some(
                json!({"fields": {"document_date": {"description": "Date."}, "city": {"description": "City."}, "description": {"description": "Text."}}, "template": "{city} {description}", "truncate_field": "description"}),
            ),
            None,
            "Unknown.pdf",
        ),
        (
            "custom",
            json!({}),
            Some(json!({"extends": "business"})),
            None,
            "00000000.pdf",
        ),
        ("academic", json!({}), None, None, "00000000.pdf"),
        ("business", json!({}), None, None, "00000000.pdf"),
        (
            "business",
            json!({"document_date": "not a date", "company_name": "ACME", "document_type": "AP", "description": "12,13"}),
            None,
            None,
            "00000000 ACME AP 12,13.pdf",
        ),
    ];
    for (selected, values, overlay, fmt, expected) in cases {
        assert_eq!(
            rendered(&c, selected, values.clone(), overlay, fmt),
            expected,
            "{selected} {values}"
        );
    }
}

#[test]
fn literal_and_value_safety() {
    let c = sample_config();
    let cases = [
        (
            "{{Paper}} {title}",
            json!({"title": "A Study"}),
            "{Paper} A Study.pdf",
        ),
        (
            "Prefix {journal_name} - {title}",
            json!({"title": "Smith-Jones 1.234,56"}),
            "Prefix - Smith-Jones 1.234,56.pdf",
        ),
        (
            "{title}",
            json!({"title": "Cafe\u{301} <>:/\\?*\u{0}."}),
            "Café.pdf",
        ),
        ("{title}", json!({"title": "A   Study"}), "A Study.pdf"),
        (
            "{title}",
            json!({"title": "--Smith_Jones--"}),
            "--Smith_Jones--.pdf",
        ),
        (
            "{first_author_surname}_ -_{journal_name}__{title}",
            json!({"first_author_surname": "Smith", "title": "A Study"}),
            "Smith - A Study.pdf",
        ),
        (
            "{first_author_surname} _ {journal_name} {title}",
            json!({"first_author_surname": "Smith", "title": "A Study"}),
            "Smith_A Study.pdf",
        ),
        (
            "{first_author_surname}--{journal_name}--{title}",
            json!({"first_author_surname": "Smith", "title": "A Study"}),
            "Smith-A Study.pdf",
        ),
        ("_ -{title}- _", json!({"title": ""}), "Unknown.pdf"),
        (
            "{first_author_surname}_\u{a0}{journal_name}-{title}",
            json!({"first_author_surname": "Smith", "title": "Study"}),
            "Smith - Study.pdf",
        ),
        ("_\u{a0}{title}_\u{a0}", json!({"title": ""}), "Unknown.pdf"),
    ];
    for (template, values, expected) in cases {
        assert_eq!(
            rendered(
                &c,
                "academic",
                values,
                Some(json!({"template": template})),
                None
            ),
            expected,
            "{template}"
        );
    }
}

#[test]
fn truncation_algorithm() {
    let c = sample_config();
    // Only the explicit truncate field shrinks.
    assert_eq!(
        rendered(
            &c,
            "academic",
            json!({"document_date": DATE, "first_author_surname": "Smith", "title": "A".repeat(400)}),
            None,
            None
        ),
        format!("20260906 Smith {}.pdf", "A".repeat(229))
    );
    // Repeated truncate field shares one shortened value.
    assert_eq!(
        rendered(
            &c,
            "academic",
            json!({"title": "A".repeat(200)}),
            Some(json!({"template": "{title} - {title}"})),
            None
        ),
        format!("{} - {}.pdf", "A".repeat(120), "A".repeat(120))
    );
    // Exhausted Unknown fallback is not reinserted.
    assert_eq!(
        rendered(
            &c,
            "business",
            json!({}),
            Some(
                json!({"template": format!("{} {{company_name}}", "L".repeat(250)), "truncate_field": "company_name"})
            ),
            None
        ),
        format!("{}.pdf", "L".repeat(244))
    );
    // Hard cut strips a trailing dot.
    assert_eq!(
        rendered(
            &c,
            "academic",
            json!({"title": "abc"}),
            Some(json!({"template": format!("{}.tail {{title}}", "x".repeat(243))})),
            None
        ),
        format!("{}.pdf", "x".repeat(243))
    );
    // No second truncation target.
    let expected: String = format!("20260906 {} AP", "C".repeat(400))
        .chars()
        .take(244)
        .collect();
    assert_eq!(
        rendered(
            &c,
            "business",
            json!({"document_date": DATE, "company_name": "C".repeat(400), "document_type": "AP", "description": "12,13"}),
            None,
            None
        ),
        format!("{expected}.pdf")
    );
    // UTF-8 bytes (and UTF-16 units), not code points: 2-byte `ä`.
    let name = rendered(
        &c,
        "academic",
        json!({"document_date": DATE, "title": "ä".repeat(300)}),
        None,
        None,
    );
    assert!((243 + 4..=244 + 4).contains(&name.len()), "{}", name.len());
    assert!(name.ends_with("ä.pdf"));
}

#[test]
fn date_behavior_and_category_overlap() {
    let c = sample_config();
    assert_eq!(
        rendered(
            &c,
            "academic",
            json!({"document_date": "invalid", "title": "06.09.2026"}),
            None,
            Some("%Y-%m-%d")
        ),
        "00000000 06.09.2026.pdf"
    );
    assert_eq!(
        rendered(
            &c,
            "academic",
            json!({"document_date": "01.01.2026", "title": "Full title: subtitle"}),
            None,
            None
        ),
        "20260101 Full title subtitle.pdf"
    );
    assert_eq!(
        rendered(
            &c,
            "business",
            json!({"document_date": DATE, "company_name": "ACME", "document_type": "Letter", "description": "Letter"}),
            None,
            None
        ),
        "20260906 ACME Letter.pdf"
    );
    // A type the subject starts with is not said twice.
    assert_eq!(
        rendered(
            &c,
            "business",
            json!({"document_date": DATE, "company_name": "", "document_type": "Hausübung", "description": "Hausübung Wiederholung"}),
            None,
            None
        ),
        "20260906 Hausübung Wiederholung.pdf"
    );
    // Custom date format and NFC of composed values (test_document_processing).
    assert_eq!(
        rendered(
            &c,
            "business",
            json!({"document_date": "15.03.2024", "company_name": "RO\u{308}HRS", "document_type": "ER"}),
            None,
            Some("%d-%m-%Y")
        ),
        "15-03-2024 R\u{d6}HRS ER.pdf"
    );
    assert_eq!(
        rendered(
            &c,
            "business",
            json!({"document_date": "15.03.2024", "company_name": "Invalid<>Name", "document_type": "ER"}),
            None,
            None
        ),
        "20240315 InvalidName ER.pdf"
    );
    // Non-string model values render as empty.
    assert_eq!(
        rendered(
            &c,
            "business",
            json!({"document_date": 20240315, "company_name": null, "document_type": 5}),
            None,
            None
        ),
        "00000000.pdf"
    );
}

#[test]
fn invoice_id_overlay_and_opt_out() {
    let c = sample_config();
    let values = json!({"document_date": DATE, "company_name": "ACME", "document_type": "AP", "description": "12,13", "invoice_id": "12345"});
    assert_eq!(
        rendered(
            &c,
            "business",
            values.clone(),
            Some(json!({
                "fields": {"invoice_id": {"description": "Printed invoice number."}},
                "template": "{document_date} {company_name} {document_type} {description} {invoice_id}"
            })),
            None
        ),
        "20260906 ACME AP 12,13 12345.pdf"
    );
    assert_eq!(
        rendered(
            &c,
            "business",
            values,
            Some(
                json!({"template": "{document_date} {company_name} {document_type}", "truncate_field": "company_name"})
            ),
            None
        ),
        "20260906 ACME AP.pdf"
    );
}

#[test]
fn collision_dry_run_skip_and_batch_undo() {
    let dir = tempdir();
    let source = dir.join("original.pdf");
    std::fs::write(&source, b"original bytes").unwrap();
    let target_name = rendered(
        &sample_config(),
        "academic",
        json!({"document_date": DATE, "first_author_surname": "Smith", "title": "A".repeat(400)}),
        None,
        None,
    );
    let existing = dir.join(&target_name);
    std::fs::write(&existing, b"existing bytes").unwrap();
    let expected = dir.join(format!("{}_(1).pdf", &target_name[..target_name.len() - 4]));

    // Dry-run: plan only.
    let plan = plan_target(&source, &target_name, &[], Path::exists);
    assert_eq!(plan, Target::Path(expected.clone()));
    assert!(!expected.exists());

    // Real rename + journal.
    let log = dir.join("journal.json");
    let Target::Path(dst) = plan else {
        unreachable!()
    };
    rename_file(&source, &dst, 3, Duration::from_millis(1)).unwrap();
    journal::update(&log, Utc::now(), |j| {
        journal::record_rename(j, "academic-batch", &source, &dst, Utc::now())
    })
    .unwrap();
    assert_eq!(std::fs::read(&expected).unwrap(), b"original bytes");
    assert_eq!(std::fs::read(&existing).unwrap(), b"existing bytes");
    assert_eq!(
        expected
            .file_name()
            .unwrap()
            .to_string_lossy()
            .chars()
            .count(),
        252
    );

    // Already named → skipped.
    assert_eq!(
        plan_target(&existing, &target_name, &[], Path::exists),
        Target::Unchanged
    );

    let mut j: Journal = journal::read_journal(&log).unwrap();
    let report = journal::undo(&mut j, &UndoTarget::Batch("academic-batch".into()));
    assert_eq!(
        (report.restored, report.failed, report.files.len()),
        (1, 0, 1)
    );
    // Not swapped: old_path = original, new_path = renamed.
    assert_eq!(PathBuf::from(&report.files[0].old_path), source);
    assert_eq!(PathBuf::from(&report.files[0].new_path), expected);
    assert_eq!(std::fs::read(&source).unwrap(), b"original bytes");
    assert!(!expected.exists());
    assert_eq!(std::fs::read(&existing).unwrap(), b"existing bytes");
}

#[test]
fn rename_never_overwrites() {
    let dir = tempdir();
    let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();
    assert_eq!(
        rename_file(&a, &b, 3, Duration::from_millis(1))
            .unwrap_err()
            .code(),
        "conflict"
    );
    assert_eq!(std::fs::read(&b).unwrap(), b"b");
}
