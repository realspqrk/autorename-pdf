//! Extraction pipeline over tests/fixtures. Needs the pinned PDFium
//! (`cargo xtask fetch-natives` or `AUTORENAME_PDFIUM_DIR`).
//! OCR accuracy test runs only when `AUTORENAME_MODELS_DIR` holds the latin pack.

#[allow(dead_code)]
#[path = "common/pdf.rs"]
mod testpdf;

use std::path::{Path, PathBuf};

use autorename_core::config::Toggle;
use autorename_core::extract::visual::VISUAL_CAP;
use autorename_core::extract::{ExtractConfig, Tier, extract_with, pdfium};
use testpdf::{Extra, INVOICE, page_pdf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.pdf"))
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn text_only() -> ExtractConfig {
    ExtractConfig {
        ocr: Toggle::Off,
        vision: Toggle::Off,
        ..ExtractConfig::default()
    }
}

/// One PDF from fixture pages (PDFium page import), written to a temp dir.
fn combined(dir: &Path, parts: &[&str]) -> PathBuf {
    let bytes = pdfium::with_pdfium(|p| {
        let mut doc = p.create_new_pdf().unwrap();
        for name in parts {
            let src = p.load_pdf_from_file(&fixture(name), None).unwrap();
            doc.pages_mut().append(&src).unwrap();
        }
        doc.save_to_bytes().unwrap()
    })
    .unwrap();
    let path = dir.join("combined.pdf");
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Golden = pdfplumber 0.11 `extract_text()` dumps (contract §4.1) + the
/// 4.0 per-page readability (§4.2): clean text layers read ≥ 0.95, blank
/// pages 0. Pages with pictures score at most 0.75 (Initech's logo, the
/// Springfield scan); the document score is the worst page.
#[test]
fn text_and_quality_match_pdfplumber_golden() {
    let q: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(golden_dir().join("quality.json")).unwrap())
            .unwrap();
    let q = q["quality"].as_object().unwrap();
    assert_eq!(q.len(), 9);
    for (stem, expected_q) in q {
        let golden = std::fs::read_to_string(golden_dir().join(format!("{stem}.txt"))).unwrap();
        let ex = extract_with(&fixture(stem), &text_only()).unwrap();
        assert_eq!(ex.text, golden, "{stem}: text differs from pdfplumber");
        let expected_q = expected_q.as_f64().unwrap();
        let readability = ex.pages.iter().map(|p| p.text_quality).fold(1.0, f64::min);
        assert!(
            (readability - expected_q).abs() < 1e-6,
            "{stem}: readability {readability} vs {expected_q}",
        );
        assert_eq!(ex.tier, Tier::Text, "{stem}");
        assert!(ex.page_images.is_empty());
        let visual = matches!(
            stem.as_str(),
            "mixed_invoice_initech" | "image_invoice_springfield"
        );
        assert!(
            ex.pages.iter().all(|p| p.visual == visual),
            "{stem}: visual flags {:?}",
            ex.pages.iter().map(|p| p.visual).collect::<Vec<_>>()
        );
        let worst = ex.pages.iter().map(|p| p.text_score()).fold(1.0, f64::min);
        assert_eq!(ex.quality, if ex.pages.is_empty() { 0.0 } else { worst });
        let expected = match stem.as_str() {
            "empty" | "image_invoice_springfield" => 0.0,
            "mixed_invoice_initech" => VISUAL_CAP,
            _ => 1.0,
        };
        assert_eq!(ex.quality, expected, "{stem}");
    }
}

/// Mixed document (text, scan, text): only the scan page goes to vision;
/// page numbers stay the original ones.
#[test]
fn pages_are_routed_independently() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = combined(
        dir.path(),
        &[
            "text_invoice_acme",
            "image_invoice_springfield",
            "text_letter_globex",
        ],
    );
    let cfg = ExtractConfig {
        vision: Toggle::Auto,
        ..text_only()
    };
    let ex = extract_with(&pdf, &cfg).unwrap();
    let scores: Vec<(usize, f64, bool)> = ex
        .pages
        .iter()
        .map(|p| (p.number, p.text_quality, p.image))
        .collect();
    assert_eq!(
        scores,
        vec![(1, 1.0, false), (2, 0.0, true), (3, 1.0, false)]
    );
    assert_eq!(ex.page_images.len(), 1);
    assert_eq!(ex.quality, 0.0);
    assert_eq!(ex.tier, Tier::VisionNeeded);
    assert!(ex.text.starts_with("Page 1:\n") && ex.text.contains("\n\nPage 3:\n"));
    assert!(!ex.text.contains("Page 2:"));
    assert!(!ex.pages[0].image);
    // `on` sends every selected page, `off` none.
    let on = extract_with(
        &pdf,
        &ExtractConfig {
            vision: Toggle::On,
            ..cfg.clone()
        },
    )
    .unwrap();
    assert_eq!(on.page_images.len(), 3);
    assert!(on.pages.iter().all(|p| p.image));
    assert!(
        extract_with(&pdf, &text_only())
            .unwrap()
            .page_images
            .is_empty()
    );
}

#[test]
fn page_limit_and_counts() {
    let cfg = ExtractConfig {
        max_pages: 1,
        ..text_only()
    };
    let ex = extract_with(&fixture("multipage_invoice_stark"), &cfg).unwrap();
    assert!(ex.page_count >= 2);
    assert_eq!(ex.pages_used, 1);
    assert!(ex.text.starts_with("Page 1:\n") && !ex.text.contains("Page 2:"));
}

/// Auto vision: low-quality scan → images prepared at scale 2.0; good text → none.
#[test]
fn auto_vision_routing_and_render_size() {
    let cfg = ExtractConfig {
        vision: Toggle::Auto,
        ..text_only()
    };
    let scan = extract_with(&fixture("image_invoice_springfield"), &cfg).unwrap();
    assert_eq!(scan.tier, Tier::VisionNeeded);
    assert_eq!(scan.sources, vec!["text", "vision"]);
    assert_eq!(scan.page_images.len(), 1);
    let img = image::load_from_memory(&scan.page_images[0]).unwrap();
    assert_eq!((img.width(), img.height()), (1191, 1684)); // A4 @ 2.0
    assert!(scan.has_content());

    let text = extract_with(&fixture("text_invoice_acme"), &cfg).unwrap();
    assert_eq!(text.tier, Tier::Text);
    assert!(text.page_images.is_empty());
}

/// OCR never downloads. `on` + missing models is a hard, actionable error;
/// `auto` warns (`ocr_models_missing`; the CLI adds the install hint) and
/// continues to vision `auto`.
#[test]
fn ocr_without_models_on_errors_auto_falls_through_to_vision() {
    let empty = tempfile::tempdir().unwrap();
    let expected = if autorename_core::ocr::ENGINE_AVAILABLE {
        "ocr_models_missing"
    } else {
        "ocr_unavailable"
    };
    let on = ExtractConfig {
        ocr: Toggle::On,
        models_dir: Some(empty.path().to_path_buf()),
        ..text_only()
    };
    let err = extract_with(&fixture("image_invoice_springfield"), &on).unwrap_err();
    assert_eq!(err.code(), expected);
    assert!(err.hint().is_some());

    let auto = ExtractConfig {
        ocr: Toggle::Auto,
        vision: Toggle::Auto,
        models_dir: Some(empty.path().to_path_buf()),
        ..text_only()
    };
    let scan = extract_with(&fixture("image_invoice_springfield"), &auto).unwrap();
    assert_eq!(scan.tier, Tier::VisionNeeded);
    assert!(!scan.page_images.is_empty());
    assert!(
        scan.warnings.iter().any(|w| w.starts_with(expected)),
        "{:?}",
        scan.warnings
    );
    // Good text: neither OCR nor vision needed, no warning.
    let ok = extract_with(&fixture("text_invoice_acme"), &auto).unwrap();
    assert_eq!(ok.tier, Tier::Text);
    assert!(ok.warnings.is_empty(), "{:?}", ok.warnings);
    assert!(ok.pages[0].ocr_text.is_none() && ok.page_images.is_empty());
}

/// Visual cap on constructed pages: pictures and drawings cap the clean
/// text layer at 0.75; tables, rules, backgrounds, stroke-only curves and
/// tiny images do not.
#[test]
fn pictures_and_drawings_cap_the_page_score() {
    let dir = tempfile::tempdir().unwrap();
    let cases: &[(&str, &[Extra], bool)] = &[
        ("clean", &[], false),
        ("raster_logo", &[Extra::RasterLogo("Zenith Robotics")], true),
        ("vector_logo", &[Extra::VectorLogo], true),
        ("table", &[Extra::Background, Extra::Table], false),
        ("stroked_curve", &[Extra::StrokedCurve], false),
        ("tiny_image", &[Extra::TinyImage], false),
    ];
    for (name, extras, visual) in cases {
        let pdf = page_pdf(dir.path(), name, INVOICE, extras);
        let ex = extract_with(&pdf, &text_only()).unwrap();
        let p = &ex.pages[0];
        assert_eq!(p.text_quality, 1.0, "{name}: {:?}", p.text);
        assert_eq!(p.visual, *visual, "{name}");
        let expected = if *visual { VISUAL_CAP } else { 1.0 };
        assert_eq!(
            (p.text_score(), p.quality, ex.quality),
            (expected, expected, expected),
            "{name}"
        );
        assert!(ex.warnings.is_empty(), "{name}: {:?}", ex.warnings);
    }
    // Images inside (nested) form XObjects: matrices apply exactly once,
    // so the 40 pt image shown at 28 pt counts and at 20 pt does not.
    for (scale, depth, visual) in [
        (0.7, 1, true),
        (0.5, 1, false),
        (0.7, 3, true),
        (0.5, 3, false),
    ] {
        let pdf = testpdf::form_image_pdf(dir.path(), "form", scale, depth);
        let ex = extract_with(&pdf, &text_only()).unwrap();
        assert_eq!(ex.pages[0].text_quality, 1.0, "{:?}", ex.pages[0].text);
        assert_eq!(ex.pages[0].visual, visual, "scale {scale}, depth {depth}");
        assert!(ex.warnings.is_empty(), "{:?}", ex.warnings);
    }
    // Forms nested deeper than the limit count as visual, with a warning.
    let deep = testpdf::form_image_pdf(dir.path(), "deep", 0.1, 40);
    let ex = extract_with(&deep, &text_only()).unwrap();
    assert!(ex.pages[0].visual);
    assert!(
        ex.warnings
            .iter()
            .any(|w| w.starts_with("visual_scan_incomplete: page 1")),
        "{:?}",
        ex.warnings
    );
    // A scan page: no text layer, one full-page image.
    let scan = extract_with(&fixture("image_invoice_springfield"), &text_only()).unwrap();
    assert!(scan.pages[0].visual);
    assert_eq!(scan.quality, 0.0);
}

/// The limit decides: a logo page (0.75) is read again at 0.8, not at
/// 0.75. Without OCR models, `auto` warns and the capped page goes to
/// vision; `vision: off` sends nothing.
#[test]
fn capped_page_goes_to_vision_without_models() {
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let pdf = page_pdf(
        dir.path(),
        "logo",
        INVOICE,
        &[Extra::RasterLogo("Zenith Robotics")],
    );
    let cfg = ExtractConfig {
        ocr: Toggle::Auto,
        vision: Toggle::Auto,
        models_dir: Some(empty.path().to_path_buf()),
        ..ExtractConfig::default()
    };
    let ex = extract_with(&pdf, &cfg).unwrap();
    assert_eq!(ex.page_images.len(), 1);
    assert!(ex.pages[0].image);
    assert_eq!(ex.pages[0].quality, VISUAL_CAP);
    assert_eq!(ex.tier, Tier::VisionNeeded);
    if autorename_core::ocr::ENGINE_AVAILABLE {
        assert!(
            ex.warnings
                .iter()
                .any(|w| w.starts_with("ocr_models_missing")),
            "{:?}",
            ex.warnings
        );
    }
    let at_cap = extract_with(
        &pdf,
        &ExtractConfig {
            text_quality_threshold: VISUAL_CAP,
            ..cfg.clone()
        },
    )
    .unwrap();
    assert!(at_cap.page_images.is_empty() && at_cap.warnings.is_empty());
    assert_eq!(at_cap.tier, Tier::Text);
    let no_vision = extract_with(
        &pdf,
        &ExtractConfig {
            vision: Toggle::Off,
            ..cfg
        },
    )
    .unwrap();
    assert!(no_vision.page_images.is_empty());
    assert_eq!(no_vision.quality, VISUAL_CAP);
}

#[test]
fn empty_pdf_has_no_content() {
    let ex = extract_with(&fixture("empty"), &text_only()).unwrap();
    assert_eq!(ex.quality, 0.0);
    assert!(!ex.has_content());
}

#[cfg(feature = "ocr")]
#[test]
fn ocr_reads_image_fixtures_when_models_present() {
    let Some(root) = std::env::var_os("AUTORENAME_MODELS_DIR") else {
        eprintln!("skipped: set AUTORENAME_MODELS_DIR to a models root with the latin pack");
        return;
    };
    let cfg = ExtractConfig {
        ocr: Toggle::Auto,
        models_dir: Some(PathBuf::from(root)),
        ..text_only()
    };
    let cases = [
        (
            "image_invoice_springfield",
            &[
                "SC-2024-0099",
                "03.11.2024",
                "Springfield Power",
                "4,300.00",
            ][..],
        ),
        (
            "mixed_invoice_initech",
            &["INIT-2024-1577", "28.06.2024", "237.50"][..],
        ),
    ];
    for (stem, keys) in cases {
        // Both pages have pictures (0.75 < 0.8): `auto` reads them.
        let ex = extract_with(&fixture(stem), &cfg).unwrap();
        let p = &ex.pages[0];
        assert!(p.visual && p.ocr_text.is_some(), "{stem}");
        // The result is the OCR text's own score (no cap, no max).
        assert!(p.ocr_quality.is_some_and(|q| q >= 0.9), "{stem}");
        assert_eq!(Some(p.quality), p.ocr_quality, "{stem}");
        let full = p.ocr_text.as_deref().unwrap();
        let flat: String = full.split_whitespace().collect();
        for k in keys {
            let k: String = k.split_whitespace().collect();
            assert!(flat.contains(&k), "{stem}: missing {k} in {full}");
        }
    }
    // The scan has no text layer: every OCR line is evidence.
    let scan = extract_with(&fixture("image_invoice_springfield"), &cfg).unwrap();
    assert_eq!(scan.tier, Tier::Ocr);
    assert!(scan.ocr_text.contains("SC-2024-0099"), "{}", scan.ocr_text);
    // Initech's clean text layer already has the OCR'd words: little or
    // nothing is added, never a second copy of the page.
    let initech = extract_with(&fixture("mixed_invoice_initech"), &cfg).unwrap();
    let added = initech.pages[0].ocr_evidence.lines().count();
    let read = initech.pages[0]
        .ocr_text
        .as_deref()
        .unwrap()
        .lines()
        .count();
    assert!(added * 4 < read, "{added} of {read} lines added");
}

/// The logo case: the issuer exists only in a raster logo, the clean text
/// layer has everything else. At limit 0.8 the page (0.75) is OCR'd and
/// only the logo's line is added to the evidence (prompt and enrichment),
/// not a second copy of the page. At 0.75 it is not read again.
#[cfg(feature = "ocr")]
#[test]
fn logo_name_reaches_the_evidence_once() {
    let Some(root) = std::env::var_os("AUTORENAME_MODELS_DIR") else {
        eprintln!("skipped: set AUTORENAME_MODELS_DIR to a models root with the latin pack");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let pdf = page_pdf(
        dir.path(),
        "logo",
        INVOICE,
        &[Extra::RasterLogo("Zenith Robotics")],
    );
    let cfg = ExtractConfig {
        ocr: Toggle::Auto,
        vision: Toggle::Auto,
        models_dir: Some(PathBuf::from(root)),
        ..ExtractConfig::default()
    };
    let ex = extract_with(&pdf, &cfg).unwrap();
    let p = &ex.pages[0];
    assert!(p.ocr_text.is_some(), "{:?}", ex.warnings);
    assert!(p.ocr_evidence.contains("Zenith"), "{:?}", p.ocr_text);
    // Text-layer lines are not repeated.
    for line in INVOICE {
        assert!(!p.ocr_evidence.contains(line), "{}", p.ocr_evidence);
    }
    assert!(p.ocr_evidence.lines().count() <= 2, "{}", p.ocr_evidence);
    assert!(!p.ocr_evidence.contains("Spqrk"), "{}", p.ocr_evidence);
    // One merged evidence string: prompt text and per-page enrichment text.
    let combined = ex.combined_text();
    assert_eq!(combined.matches("Zenith").count(), 1, "{combined}");
    assert_eq!(combined.matches("Spqrk").count(), 1, "{combined}");
    assert!(ex.page_texts()[0].contains("Zenith"));
    assert_eq!(ex.tier, Tier::Ocr);
    assert!(ex.page_images.is_empty(), "OCR read the page: no vision");
    let at_cap = extract_with(
        &pdf,
        &ExtractConfig {
            text_quality_threshold: VISUAL_CAP,
            ..cfg
        },
    )
    .unwrap();
    assert!(at_cap.pages[0].ocr_text.is_none());
    assert!(!at_cap.combined_text().contains("Zenith"));
}

/// A logo that shares a word with the text layer (`GmbH` of the recipient
/// `Spqrk GmbH`) is still new: OCR lines are matched against text lines and
/// word windows, not against a page-wide word set.
#[cfg(feature = "ocr")]
#[test]
fn logo_sharing_a_legal_suffix_still_reaches_the_evidence() {
    let Some(root) = std::env::var_os("AUTORENAME_MODELS_DIR") else {
        eprintln!("skipped: set AUTORENAME_MODELS_DIR to a models root with the latin pack");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let cfg = ExtractConfig {
        ocr: Toggle::Auto,
        vision: Toggle::Auto,
        models_dir: Some(PathBuf::from(root)),
        ..ExtractConfig::default()
    };
    // Short and long shared legal forms (`GmbH`, `GmbH & Co. KG`).
    let long: Vec<String> = INVOICE
        .iter()
        .map(|l| l.replace("Spqrk GmbH", "Spqrk GmbH & Co. KG"))
        .collect();
    let long: Vec<&str> = long.iter().map(String::as_str).collect();
    for (name, logo, lines) in [
        ("acme", "ACME GmbH", INVOICE),
        ("acme-kg", "ACME GmbH & Co. KG", &long[..]),
    ] {
        let pdf = page_pdf(dir.path(), name, lines, &[Extra::RasterLogo(logo)]);
        let ex = extract_with(&pdf, &cfg).unwrap();
        let p = &ex.pages[0];
        assert!(p.ocr_text.is_some(), "{:?}", ex.warnings);
        assert!(p.ocr_evidence.contains("ACME"), "{name}: {:?}", p.ocr_text);
        let combined = ex.combined_text();
        assert_eq!(combined.matches("ACME").count(), 1, "{combined}");
        assert_eq!(combined.matches("Spqrk").count(), 1, "{combined}");
        assert_eq!(ex.tier, Tier::Ocr);
        assert!(ex.page_images.is_empty());
    }
}

/// `ocr: on` reads every page, and its result replaces the text score even
/// when lower: a text layer hidden under a white box (perfect readability,
/// nothing visible) OCRs to nothing, scores 0 and goes to vision (`auto`).
#[cfg(feature = "ocr")]
#[test]
fn forced_ocr_can_lower_a_perfect_page() {
    let Some(root) = std::env::var_os("AUTORENAME_MODELS_DIR") else {
        eprintln!("skipped: set AUTORENAME_MODELS_DIR to a models root with the latin pack");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let pdf = page_pdf(dir.path(), "hidden", INVOICE, &[Extra::HiddenText]);
    let cfg = ExtractConfig {
        ocr: Toggle::On,
        vision: Toggle::Auto,
        models_dir: Some(PathBuf::from(root)),
        ..ExtractConfig::default()
    };
    let ex = extract_with(&pdf, &cfg).unwrap();
    let p = &ex.pages[0];
    assert_eq!((p.text_quality, p.visual), (1.0, false));
    assert_eq!(p.ocr_quality, Some(0.0));
    assert_eq!((p.quality, ex.quality), (0.0, 0.0));
    assert!(p.image && ex.tier == Tier::VisionNeeded);
    assert!(ex.warnings.iter().any(|w| w == "OCR returned no text"));
    // A clean page under `on`: OCR'd, nothing new added, still Text.
    let clean = page_pdf(dir.path(), "clean", INVOICE, &[]);
    let ex = extract_with(&clean, &cfg).unwrap();
    assert!(ex.pages[0].ocr_text.is_some());
    assert!(
        ex.pages[0].ocr_evidence.is_empty(),
        "{}",
        ex.pages[0].ocr_evidence
    );
    assert_eq!(ex.tier, Tier::Text);
}

/// Live download of the pinned latin pack (network, ~39 MB): `cargo test -- --ignored`.
#[test]
#[ignore = "network"]
fn fetch_latin_pack_live() {
    use autorename_core::ocr::{manifest::LATIN, models};
    let root = tempfile::tempdir().unwrap();
    let mut last = 0u64;
    models::fetch(root.path(), &LATIN, &mut |p| last = p.downloaded, None).unwrap();
    assert!(last > 0);
    assert!(models::verify(root.path(), &LATIN).unwrap().is_empty());
    assert!(models::require(root.path(), &LATIN).is_ok());
    models::remove(root.path(), &LATIN).unwrap();
    assert!(!models::list(root.path())[0].installed);
}
