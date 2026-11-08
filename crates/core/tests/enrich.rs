//! Enrichment end to end over tests/fixtures plus generated edge cases:
//! apply → prefix → read back → idempotent re-run → re-enrich → undo to the
//! exact original bytes; signed/certified/encrypted/e-invoice/PDF-A skipped;
//! foreign attachments, XMP and Info keys survive.

use std::path::{Path, PathBuf};
use std::process::Command;

use autorename_core::enrich::{
    self, ATTACHMENT_NAME, Assessment, EnrichData, EnrichStatus, NS_ARP, SkipReason,
};
use autorename_core::journal::{EnrichOutcome as UndoOutcome, sha256_hex, undo_enrich};
use lopdf::{Dictionary, Document, Object, Stream, StringFormat, dictionary};
use serde_json::json;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn fixture_paths() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        .collect();
    v.sort();
    assert!(v.len() >= 5);
    v
}

fn data(company: &str) -> EnrichData {
    EnrichData {
        title: format!("20261107 {company} AP 12,13"),
        subject: format!("AP {company}"),
        keywords: vec![company.into(), "AP".into(), "Grüße & <Co>".into()],
        profile: "business".into(),
        fields:
            json!({"company": company, "date": "2026-11-07", "total": 12.13, "invoice_id": null})
                .as_object()
                .unwrap()
                .clone(),
        tier: "text".into(),
        provider: "openai".into(),
        model: "gpt-test".into(),
        pages: vec!["Page one text".into(), "Seite zwei – Ümlaut".into()],
        partial: true,
        timestamp: "2026-11-07T10:11:12Z".parse().unwrap(),
    }
}

fn copy_to(dir: &Path, src: &Path) -> PathBuf {
    let dst = dir.join(src.file_name().unwrap());
    std::fs::copy(src, &dst).unwrap();
    dst
}

fn write_doc(dir: &Path, name: &str, doc: &mut Document) -> PathBuf {
    let path = dir.join(name);
    let mut buf = Vec::new();
    doc.save_to(&mut buf).unwrap();
    std::fs::write(&path, buf).unwrap();
    path
}

fn base_doc() -> Document {
    Document::load(fixtures().join("text_invoice_acme.pdf")).unwrap()
}

fn catalog_mut(doc: &mut Document) -> &mut Dictionary {
    doc.catalog_mut().unwrap()
}

fn qpdf_check(path: &Path) {
    let Ok(out) = Command::new("qpdf").arg("--check").arg(path).output() else {
        return; // qpdf not installed
    };
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("No syntax or stream encoding errors"),
        "qpdf --check {}:\n{text}{}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn enriched(s: EnrichStatus) -> enrich::EnrichOutcome {
    match s {
        EnrichStatus::Enriched(o) => o,
        other => panic!("expected enriched, got {other:?}"),
    }
}

/// Embedded-file tree keys of the final revision.
fn tree_keys(path: &Path) -> Vec<String> {
    let doc = Document::load(path).unwrap();
    let names = doc
        .catalog()
        .unwrap()
        .get_deref(b"Names", &doc)
        .unwrap()
        .as_dict()
        .unwrap();
    let tree = names
        .get_deref(b"EmbeddedFiles", &doc)
        .unwrap()
        .as_dict()
        .unwrap();
    tree.get(b"Names")
        .unwrap()
        .as_array()
        .unwrap()
        .chunks(2)
        .map(|p| String::from_utf8_lossy(p[0].as_str().unwrap()).into_owned())
        .collect()
}

#[test]
fn fixtures_round_trip_rerun_and_undo_to_exact_original() {
    let dir = tempfile::tempdir().unwrap();
    for src in fixture_paths() {
        let path = copy_to(dir.path(), &src);
        let original = std::fs::read(&path).unwrap();
        let name = src.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(enrich::plan(&path).unwrap(), Assessment::Eligible, "{name}");
        assert!(enrich::read(&path).unwrap().is_none(), "{name}");

        // First enrichment.
        let first = enriched(enrich::apply(&path, &data("ACME")).unwrap());
        let after1 = std::fs::read(&path).unwrap();
        assert!(after1.starts_with(&original), "{name}: prefix");
        assert_eq!(first.orig_len, original.len() as u64);
        assert_eq!(first.orig_sha256, sha256_hex(&original));
        assert_eq!(first.post_sha256, sha256_hex(&after1));
        qpdf_check(&path);

        let got = enrich::read(&path).unwrap().expect("attachment");
        let a = &got.attachment;
        assert_eq!(a.schema_version, 1);
        assert_eq!(a.content.fields["company"], "ACME");
        assert_eq!(a.content.extraction.pages_used, 2);
        assert!(a.content.extraction.partial);
        assert_eq!(a.content.pages[1].text, "Seite zwei – Ümlaut");
        assert_eq!(a.original.sha256, sha256_hex(&original));
        assert_eq!(a.original.length, original.len() as u64);
        assert_eq!(got.xmp["profile"], "business");
        assert_eq!(got.xmp["fields.company"], "ACME");
        assert_eq!(got.xmp["fields.total"], "12.13");
        assert!(!got.xmp.contains_key("fields.invoice_id"));

        let doc = Document::load(&path).unwrap();
        let info = doc
            .trailer
            .get_deref(b"Info", &doc)
            .unwrap()
            .as_dict()
            .unwrap();
        let text = |k: &[u8]| lopdf::decode_text_string(info.get(k).unwrap()).unwrap();
        assert_eq!(text(b"Title"), "20261107 ACME AP 12,13");
        assert_eq!(text(b"Keywords"), "ACME; AP; Grüße & <Co>");
        assert!(text(b"ModDate").starts_with("D:20261107101112"));

        // Same content → unchanged, no bytes written.
        assert_eq!(
            enrich::apply(&path, &data("ACME")).unwrap(),
            EnrichStatus::Unchanged
        );
        assert_eq!(std::fs::read(&path).unwrap(), after1);

        // Different content → second update replaces our attachment.
        let second = enriched(enrich::apply(&path, &data("Globex")).unwrap());
        assert_eq!(second.orig_sha256, sha256_hex(&after1));
        let got2 = enrich::read(&path).unwrap().unwrap();
        assert_eq!(got2.attachment.content.fields["company"], "Globex");
        assert_eq!(got2.xmp["fields.company"], "Globex");
        // Pre-AutoRename identity is carried across re-runs.
        assert_eq!(got2.attachment.original.sha256, sha256_hex(&original));
        assert_eq!(tree_keys(&path), [ATTACHMENT_NAME]);
        qpdf_check(&path);

        // Undo newest first → byte-identical original.
        let e2 = second.journal_entry(&path);
        let e1 = first.journal_entry(&path);
        assert_eq!(undo_enrich(&path, &e2).unwrap(), UndoOutcome::Restored);
        assert_eq!(std::fs::read(&path).unwrap(), after1);
        assert_eq!(undo_enrich(&path, &e1).unwrap(), UndoOutcome::Restored);
        assert_eq!(std::fs::read(&path).unwrap(), original, "{name}: undo");
    }
}

#[test]
fn undo_refuses_when_file_changed_after_enrichment() {
    let dir = tempfile::tempdir().unwrap();
    let path = copy_to(dir.path(), &fixtures().join("minimal_text.pdf"));
    let out = enriched(enrich::apply(&path, &data("ACME")).unwrap());
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"\n% edited by someone else\n");
    std::fs::write(&path, &bytes).unwrap();
    let err = undo_enrich(&path, &out.journal_entry(&path)).unwrap_err();
    assert_eq!(err.code(), "conflict");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn xref_stream_documents_are_updated_with_a_xref_stream_section() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = base_doc();
    doc.version = "1.5".into();
    let path = dir.path().join("modern.pdf");
    let mut buf = Vec::new();
    doc.save_modern(&mut buf).unwrap();
    std::fs::write(&path, &buf).unwrap();
    assert!(String::from_utf8_lossy(&buf).contains("/XRef"));
    enriched(enrich::apply(&path, &data("ACME")).unwrap());
    qpdf_check(&path);
    assert_eq!(
        enrich::read(&path)
            .unwrap()
            .unwrap()
            .attachment
            .content
            .fields["company"],
        "ACME"
    );
}

fn assert_skipped(path: &Path, reason: SkipReason) {
    let before = std::fs::read(path).unwrap();
    assert_eq!(
        enrich::plan(path).unwrap(),
        Assessment::Skip(reason),
        "{}",
        path.display()
    );
    let status = enrich::apply(path, &data("ACME")).unwrap();
    assert_eq!(status, EnrichStatus::Skipped(reason));
    assert_eq!(status.code(), format!("skipped:{}", reason.code()));
    assert_eq!(std::fs::read(path).unwrap(), before, "skip must not write");
}

#[test]
fn signed_and_certified_documents_are_skipped() {
    let dir = tempfile::tempdir().unwrap();

    // Signature field with a signature value carrying /ByteRange.
    let mut doc = base_doc();
    let sig = doc.add_object(dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => "adbe.pkcs7.detached",
        "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
        "Contents" => Object::String(vec![0; 16], StringFormat::Hexadecimal),
    });
    let field = doc.add_object(
        dictionary! { "FT" => "Sig", "T" => Object::string_literal("Signature1"), "V" => sig },
    );
    catalog_mut(&mut doc).set(
        "AcroForm",
        dictionary! { "Fields" => vec![field.into()], "SigFlags" => 3 },
    );
    assert_skipped(
        &write_doc(dir.path(), "signed.pdf", &mut doc),
        SkipReason::Signed,
    );

    // Empty signature field only (no value yet) is still treated as signed.
    let mut doc = base_doc();
    let field = doc.add_object(dictionary! { "FT" => "Sig", "T" => Object::string_literal("Sig") });
    catalog_mut(&mut doc).set("AcroForm", dictionary! { "Fields" => vec![field.into()] });
    assert_skipped(
        &write_doc(dir.path(), "sigfield.pdf", &mut doc),
        SkipReason::Signed,
    );

    // Certification (DocMDP).
    let mut doc = base_doc();
    let sig = doc.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()] });
    catalog_mut(&mut doc).set("Perms", dictionary! { "DocMDP" => sig });
    assert_skipped(
        &write_doc(dir.path(), "certified.pdf", &mut doc),
        SkipReason::Certified,
    );
}

#[test]
fn encrypted_documents_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    for (name, user_pw) in [("enc-user.pdf", "secret"), ("enc-empty.pdf", "")] {
        let mut doc = base_doc();
        let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner",
            user_password: user_pw,
            key_length: 128,
            permissions: lopdf::Permissions::all(),
        })
        .unwrap();
        doc.encrypt(&state).unwrap();
        assert_skipped(
            &write_doc(dir.path(), name, &mut doc),
            SkipReason::Encrypted,
        );
    }
    let junk = dir.path().join("junk.pdf");
    std::fs::write(&junk, b"%PDF-1.7\nnot really a pdf\n").unwrap();
    assert_skipped(&junk, SkipReason::Unparseable);
    assert!(enrich::read(&junk).is_err());
}

fn embedded(doc: &mut Document, name: &str, content: &[u8], rel: Option<&str>) -> Object {
    let ef = doc.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        content.to_vec(),
    ));
    let mut fs = dictionary! {
        "Type" => "Filespec",
        "F" => Object::string_literal(name),
        "UF" => Object::string_literal(name),
        "EF" => dictionary! { "F" => ef },
    };
    if let Some(r) = rel {
        fs.set("AFRelationship", Object::Name(r.as_bytes().to_vec()));
    }
    doc.add_object(fs).into()
}

fn set_xmp(doc: &mut Document, packet: &str) {
    let id = doc.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        packet.as_bytes().to_vec(),
    ));
    catalog_mut(doc).set("Metadata", id);
}

#[test]
fn e_invoice_and_pdfa_documents_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = base_doc();
    let fs = embedded(
        &mut doc,
        "factur-x.xml",
        b"<rsm:CrossIndustryInvoice/>",
        Some("Alternative"),
    );
    catalog_mut(&mut doc).set(
        "Names",
        dictionary! { "EmbeddedFiles" => dictionary! { "Names" => vec![Object::string_literal("factur-x.xml"), fs.clone()] } },
    );
    catalog_mut(&mut doc).set("AF", vec![fs]);
    assert_skipped(
        &write_doc(dir.path(), "facturx.pdf", &mut doc),
        SkipReason::EInvoice,
    );

    let mut doc = base_doc();
    set_xmp(
        &mut doc,
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" pdfaid:part="3" pdfaid:conformance="B"/></rdf:RDF></x:xmpmeta>"#,
    );
    assert_skipped(
        &write_doc(dir.path(), "pdfa.pdf", &mut doc),
        SkipReason::PdfA,
    );

    let mut doc = base_doc();
    set_xmp(&mut doc, "<x:xmpmeta><rdf:RDF></x:xmpmeta>");
    assert_skipped(
        &write_doc(dir.path(), "badxmp.pdf", &mut doc),
        SkipReason::InvalidXmp,
    );
}

#[test]
fn foreign_attachments_xmp_and_info_survive() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = base_doc();
    let notes = embedded(&mut doc, "notes.txt", b"keep me", None);
    // A foreign file that happens to use our name must not be mistaken for ours.
    let impostor = embedded(&mut doc, ATTACHMENT_NAME, b"{\"not\":\"ours\"}", None);
    // Two-level name tree (Kids) to exercise flattening.
    let kid = doc.add_object(dictionary! {
        "Names" => vec![
            Object::string_literal(ATTACHMENT_NAME), impostor,
            Object::string_literal("notes.txt"), notes.clone(),
        ],
        "Limits" => vec![Object::string_literal(ATTACHMENT_NAME), Object::string_literal("notes.txt")],
    });
    let root = doc.add_object(dictionary! { "Kids" => vec![kid.into()] });
    let names = doc.add_object(dictionary! { "EmbeddedFiles" => root });
    catalog_mut(&mut doc).set("Names", names);
    catalog_mut(&mut doc).set("AF", vec![notes]);
    set_xmp(
        &mut doc,
        r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:foo="urn:foo" xmp:CreatorTool="Word" xmp:ModifyDate="2020-01-01T00:00:00Z"><foo:bar>kept</foo:bar></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#,
    );
    let info = doc.trailer.get(b"Info").unwrap().as_reference().unwrap();
    let info = doc.get_dictionary_mut(info).unwrap();
    info.set("Author", Object::string_literal("Jane Doe"));
    info.set("CreationDate", Object::string_literal("D:20200101000000Z"));
    info.set("Custom", Object::string_literal("x"));
    let path = write_doc(dir.path(), "foreign.pdf", &mut doc);

    enriched(enrich::apply(&path, &data("ACME")).unwrap());
    qpdf_check(&path);
    let keys = tree_keys(&path);
    assert_eq!(
        keys,
        [
            ATTACHMENT_NAME.to_string(),
            format!("{ATTACHMENT_NAME} (2)"),
            "notes.txt".into()
        ]
    );
    enriched(enrich::apply(&path, &data("Globex")).unwrap());
    assert_eq!(
        tree_keys(&path),
        keys,
        "re-run replaces ours, keeps foreign"
    );

    let doc = Document::load(&path).unwrap();
    let cat = doc.catalog().unwrap();
    assert_eq!(
        cat.get(b"AF").unwrap().as_array().unwrap().len(),
        2,
        "notes + ours"
    );
    let info = doc
        .trailer
        .get_deref(b"Info", &doc)
        .unwrap()
        .as_dict()
        .unwrap();
    for (k, v) in [
        (b"Author".as_slice(), "Jane Doe"),
        (b"CreationDate", "D:20200101000000Z"),
        (b"Custom", "x"),
    ] {
        assert_eq!(lopdf::decode_text_string(info.get(k).unwrap()).unwrap(), v);
    }
    let meta = cat
        .get_deref(b"Metadata", &doc)
        .unwrap()
        .as_stream()
        .unwrap();
    let xmp = String::from_utf8(meta.content.clone()).unwrap();
    assert!(xmp.contains(r#"xmp:CreatorTool="Word""#) && xmp.contains("<foo:bar>kept</foo:bar>"));
    assert!(!xmp.contains("2020-01-01T00:00:00Z"));
    assert_eq!(xmp.matches(NS_ARP).count(), 1);
    assert_eq!(
        enrich::read(&path)
            .unwrap()
            .unwrap()
            .attachment
            .content
            .fields["company"],
        "Globex"
    );
}

/// Independent reader (pdfminer from the Python venv) over an enriched file.
/// `cargo test -p autorename-core --test enrich -- --ignored`
#[test]
#[ignore = "needs the repo Python venv"]
fn python_pdfminer_reads_enriched_output() {
    let py = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../venv/bin/python");
    let dir = tempfile::tempdir().unwrap();
    let path = copy_to(dir.path(), &fixtures().join("multipage_invoice_stark.pdf"));
    enriched(enrich::apply(&path, &data("ACME")).unwrap());
    let script = r#"
import sys, json
from pdfminer.pdfparser import PDFParser
from pdfminer.pdfdocument import PDFDocument
from pdfminer.pdftypes import resolve1
doc = PDFDocument(PDFParser(open(sys.argv[1], 'rb')))
cat = resolve1(doc.catalog)
names = resolve1(resolve1(cat['Names'])['EmbeddedFiles'])['Names']
fs = resolve1(names[1])
data = json.loads(resolve1(resolve1(fs['EF'])['F']).get_data())
print(data['content_sha256'] and data['fields']['company'], resolve1(doc.info[-1]['Title']).decode())
"#;
    let out = Command::new(py)
        .arg("-I")
        .arg("-c")
        .arg(script)
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout.trim(), "ACME 20261107 ACME AP 12,13");
}
