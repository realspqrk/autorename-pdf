//! Port of tests/test_profiles.py and the pure parts of test_profile_ai.py.

mod common;

use autorename_core::profiles::{
    interpolate_profile_text, metadata_schema, resolve_profiles, select_profile, system_prompt,
    validate_metadata,
};
use common::sample_config;
use serde_json::{Value, json};

fn keys(p: &autorename_core::profiles::Profile) -> Vec<&str> {
    p.fields.keys().map(String::as_str).collect()
}

#[test]
fn builtin_ids_fields_and_templates() {
    let profiles = resolve_profiles(&sample_config()).unwrap();
    assert_eq!(
        profiles.keys().collect::<Vec<_>>(),
        ["business", "academic"]
    );
    let b = &profiles["business"];
    let a = &profiles["academic"];
    assert_eq!(
        keys(b),
        [
            "document_date",
            "company_name",
            "issuer",
            "recipient",
            "document_type",
            "total_amount",
            "currency",
            "description"
        ]
    );
    assert_eq!(
        keys(a),
        [
            "document_date",
            "first_author_surname",
            "journal_name",
            "title"
        ]
    );
    assert_eq!(
        b.template,
        "{document_date} {company_name} {document_type} {description}"
    );
    assert_eq!(
        a.template,
        "{document_date} {first_author_surname} {journal_name} {title}"
    );
    assert_eq!(
        (b.truncate_field.as_str(), b.harmonize_field.as_deref()),
        ("description", Some("company_name"))
    );
    assert_eq!(
        (a.truncate_field.as_str(), a.harmonize_field.as_deref()),
        ("title", None)
    );
    let dt = &b.fields["document_type"].description;
    assert!(dt.contains("use only \"AP\"") && dt.contains("use only \"AR\""));
    assert!(
        b.fields["company_name"]
            .description
            .contains("My company name is \"Test Company\"")
    );
    let mut c = sample_config();
    c["company"]["name"] = json!("");
    let b = select_profile(&c, None).unwrap().1;
    assert!(
        b.fields["company_name"]
            .description
            .starts_with("Find the main company")
    );
}

#[test]
fn overlay_before_inheritance_replaces_in_place_and_deletes() {
    let mut c = sample_config();
    c["profiles"] = json!({
        "receipts": {
            "extends": "business",
            "fields": {"document_type": null, "description": null, "short_title": {"description": "Printed receipt title."}},
            "template": "{document_date}_{company_name}_{city}_{short_title}",
            "truncate_field": "short_title",
            "harmonize_field": null
        },
        "business": {"fields": {
            "company_name": {"description": "Issuer exactly as printed."},
            "city": {"description": "Printed city."}
        }},
        "child": {"extends": "receipts", "intro": "Child intro."}
    });
    let before = c.clone();
    let p = resolve_profiles(&c).unwrap();
    assert_eq!(
        keys(&p["receipts"]),
        [
            "document_date",
            "company_name",
            "issuer",
            "recipient",
            "total_amount",
            "currency",
            "city",
            "short_title"
        ]
    );
    assert_eq!(
        p["receipts"].fields["company_name"].description,
        "Issuer exactly as printed."
    );
    assert_eq!(p["receipts"].harmonize_field, None);
    assert_eq!(p["child"].fields, p["receipts"].fields);
    assert_eq!(p["child"].intro, "Child intro.");
    assert_eq!(c, before);
    assert_eq!(
        p.keys().collect::<Vec<_>>(),
        ["business", "academic", "receipts", "child"]
    );
}

#[test]
fn standalone_nine_fields_and_selection_no_leak() {
    let mut c = sample_config();
    let mut fields = serde_json::Map::new();
    fields.insert(
        "document_date".into(),
        json!({"description": "Printed date."}),
    );
    for n in 0..8 {
        fields.insert(
            format!("part_{n}"),
            json!({"description": format!("Printed part {n}.")}),
        );
    }
    c["profile"] = json!("nine");
    c["profiles"] =
        json!({"nine": {"fields": fields, "template": "{part_0}", "truncate_field": "part_0"}});
    let (id, p) = select_profile(&c, None).unwrap();
    assert_eq!(id, "nine");
    assert_eq!(p.fields.len(), 9);
    assert_eq!((p.intro.as_str(), p.harmonize_field.as_deref()), ("", None));
    let (id, a) = select_profile(&c, Some("academic")).unwrap();
    assert_eq!(id, "academic");
    assert_eq!(
        a.template,
        "{document_date} {first_author_surname} {journal_name} {title}"
    );
    assert_eq!(c["profile"], "nine");
}

#[test]
fn direction_is_configurable_and_follows_field_deletion() {
    let (_, b) = select_profile(&sample_config(), None).unwrap();
    let d = b.direction.unwrap();
    assert_eq!(
        (d.field.as_str(), d.incoming.as_str(), d.outgoing.as_str()),
        ("document_type", "AP", "AR")
    );
    // Disable explicitly; deleting a referenced field drops it on inheritance.
    let mut c = sample_config();
    c["profiles"] = json!({"business": {"direction": null}, "plain": {"extends": "business", "fields": {"issuer": null}}});
    assert!(select_profile(&c, None).unwrap().1.direction.is_none());
    assert!(
        select_profile(&c, Some("plain"))
            .unwrap()
            .1
            .direction
            .is_none()
    );
    // An explicit direction must reference real fields.
    let mut c = sample_config();
    c["profiles"] = json!({"business": {"direction": {"field": "nope", "issuer": "issuer", "recipient": "recipient", "incoming": "AP", "outgoing": "AR"}}});
    assert!(select_profile(&c, None).is_err());
}

#[test]
fn deleting_company_with_repaired_references() {
    let mut c = sample_config();
    c["profiles"] = json!({"business": {"fields": {"company_name": null}, "harmonize_field": null, "template": "{document_date} {document_type} {description}"}});
    assert_eq!(
        keys(&select_profile(&c, None).unwrap().1),
        [
            "document_date",
            "issuer",
            "recipient",
            "document_type",
            "total_amount",
            "currency",
            "description"
        ]
    );
}

#[test]
fn invalid_profile_definitions() {
    let both = json!({"document_date": {"description": "Date"}, "title": {"description": "Title"}});
    let cases: Vec<(&str, Value)> = vec![
        ("null-profile", json!({"business": null})),
        (
            "unknown-profile-key",
            json!({"business": {"extra_fields": {}}}),
        ),
        (
            "mixed-unknown-keys",
            json!({"business": {"1": "bad", "typo": "bad"}}),
        ),
        (
            "builtin-extends",
            json!({"business": {"extends": "academic"}}),
        ),
        ("unknown-parent", json!({"x": {"extends": "missing"}})),
        (
            "forward-parent",
            json!({"x": {"extends": "y"}, "y": {"extends": "business"}}),
        ),
        ("self-parent", json!({"x": {"extends": "x"}})),
        (
            "cycle",
            json!({"x": {"extends": "y"}, "y": {"extends": "x"}}),
        ),
        ("multiple-parents", json!({"x": {"extends": ["business"]}})),
        ("standalone-missing-settings", json!({"x": {"fields": {}}})),
        ("field-list", json!({"business": {"fields": []}})),
        (
            "scalar-field",
            json!({"business": {"fields": {"description": "text"}}}),
        ),
        (
            "empty-description",
            json!({"business": {"fields": {"description": {"description": ""}}}}),
        ),
        (
            "blank-description",
            json!({"business": {"fields": {"description": {"description": "  "}}}}),
        ),
        (
            "nonstring-description",
            json!({"business": {"fields": {"description": {"description": 3}}}}),
        ),
        (
            "unknown-field-key",
            json!({"business": {"fields": {"description": {"description": "x", "type": "str"}}}}),
        ),
        (
            "delete-missing",
            json!({"business": {"fields": {"absent": null}}}),
        ),
        (
            "delete-date",
            json!({"business": {"fields": {"document_date": null}}}),
        ),
        (
            "missing-date",
            json!({"x": {"fields": {"title": {"description": "Title."}}, "template": "{title}", "truncate_field": "title"}}),
        ),
        (
            "dangling-company",
            json!({"business": {"fields": {"company_name": null}}}),
        ),
        (
            "truncate-date",
            json!({"business": {"truncate_field": "document_date"}}),
        ),
        (
            "truncate-missing",
            json!({"business": {"truncate_field": "absent"}}),
        ),
        (
            "truncate-not-in-template",
            json!({"business": {"template": "{document_date}"}}),
        ),
        (
            "truncate-escaped-only",
            json!({"business": {"template": "{document_date} {{description}}"}}),
        ),
        (
            "harmonize-date",
            json!({"business": {"harmonize_field": "document_date"}}),
        ),
        (
            "harmonize-missing",
            json!({"business": {"harmonize_field": "absent"}}),
        ),
        ("null-intro", json!({"business": {"intro": null}})),
        ("empty-template", json!({"business": {"template": ""}})),
        (
            "extension-in-template",
            json!({"business": {"template": "{description}.pdf"}}),
        ),
        (
            "literal-slash",
            json!({"business": {"template": "folder/{description}"}}),
        ),
        (
            "literal-backslash",
            json!({"business": {"template": "folder\\{description}"}}),
        ),
        (
            "wrong-namespace",
            json!({"business": {"template": "{description} {company}"}}),
        ),
        ("truncate-list", json!({"business": {"truncate_field": []}})),
        (
            "harmonize-dict",
            json!({"business": {"harmonize_field": {}}}),
        ),
        (
            "standalone-null-date",
            json!({"x": {"fields": {"document_date": null, "title": {"description": "Title"}}, "template": "{title}", "truncate_field": "title"}}),
        ),
        (
            "standalone-delete-missing",
            json!({"x": {"fields": {"document_date": {"description": "Date"}, "title": {"description": "Title"}, "absent": null}, "template": "{title}", "truncate_field": "title"}}),
        ),
        (
            "empty-parent",
            json!({"notes": {"extends": "", "fields": both, "template": "{title}", "truncate_field": "title"}}),
        ),
        (
            "blank-parent",
            json!({"notes": {"extends": "  ", "fields": both, "template": "{title}", "truncate_field": "title"}}),
        ),
        ("profiles-not-mapping", json!(["business"])),
        ("declared-empty-id", json!({"": {"extends": "business"}})),
        (
            "declared-leading-space",
            json!({" x": {"extends": "business"}}),
        ),
        (
            "declared-trailing-space",
            json!({"x ": {"extends": "business"}}),
        ),
    ];
    for (id, defs) in cases {
        let mut c = sample_config();
        c["profiles"] = defs;
        let err = resolve_profiles(&c).expect_err(id);
        assert_eq!(err.code(), "config_error", "{id}");
    }
}

#[test]
fn field_id_rules() {
    for fid in ["_private", "bad-name", "9field", "", "7", "städt", "a b"] {
        let mut c = sample_config();
        c["profiles"] = json!({"business": {"fields": {fid: {"description": "Value."}}}});
        assert!(resolve_profiles(&c).is_err(), "{fid:?}");
    }
    for ok in [
        "fields",
        "title",
        "date",
        "class",
        "schema",
        "model_dump",
        "invoice_id2",
    ] {
        let mut c = sample_config();
        c["profiles"] = json!({"business": {"fields": {ok: {"description": "Value."}}}});
        assert!(resolve_profiles(&c).is_ok(), "{ok:?}");
    }
}

#[test]
fn invalid_selected_ids() {
    for v in [
        json!(""),
        json!(" business"),
        json!("business "),
        json!("Business"),
        json!("missing"),
        json!(null),
        json!(7),
        json!(false),
    ] {
        let mut c = sample_config();
        c["profile"] = v.clone();
        assert!(select_profile(&c, None).is_err(), "{v}");
    }
    assert_eq!(
        select_profile(&sample_config(), Some("nope"))
            .unwrap_err()
            .to_string(),
        "unknown profile \"nope\""
    );
}

#[test]
fn reject_placeholder_syntax() {
    let c = sample_config();
    for t in [
        "{unknown}",
        "{company.name}",
        "{company[0]}",
        "{company!r}",
        "{company:>10}",
        "{company:}",
        "{",
        "}",
        "{}",
    ] {
        assert!(interpolate_profile_text(t, &c).is_err(), "prompt {t}");
    }
    for t in [
        "{description} {absent}",
        "{description.upper}",
        "{description[0]}",
        "{description!r}",
        "{description:>10}",
        "{description:}",
        "{",
        "}",
        "{}",
    ] {
        let mut c = sample_config();
        c["profiles"] = json!({"business": {"template": t}});
        assert!(resolve_profiles(&c).is_err(), "template {t}");
    }
}

#[test]
fn interpolation_is_single_pass() {
    let mut c = sample_config();
    c["company"]["name"] = json!("Own {literal}");
    c["output"]["language"] = json!("German");
    c["pdf"]["incoming_invoice"] = json!("IN");
    c["pdf"]["outgoing_invoice"] = json!("OUT");
    assert_eq!(
        interpolate_profile_text(
            "{{tag}} {company}|{language}|{incoming_invoice}|{outgoing_invoice}",
            &c
        )
        .unwrap(),
        "{tag} Own {literal}|German|IN|OUT"
    );
    c["profiles"] =
        json!({"business": {"fields": {"company_name": {"description": "CUSTOM {company}"}}}});
    assert_eq!(
        select_profile(&c, None).unwrap().1.fields["company_name"].description,
        "CUSTOM Own {literal}"
    );
    c["company"]["name"] = json!("");
    assert_eq!(
        select_profile(&c, None).unwrap().1.fields["company_name"].description,
        "CUSTOM "
    );
}

#[test]
fn inherited_interpolation_happens_once() {
    let mut c = sample_config();
    c["company"]["name"] = json!("Own {literal}");
    c["profiles"] = json!({
        "business": {"fields": {"company_name": {"description": "Issuer {company} {{tag}}"}}},
        "child": {"extends": "business"},
        "grandchild": {"extends": "child"}
    });
    let p = resolve_profiles(&c).unwrap();
    assert_eq!(
        p["grandchild"].fields["company_name"].description,
        "Issuer Own {literal} {tag}"
    );
}

#[test]
fn strict_schema_shape() {
    let (_, p) = select_profile(&sample_config(), None).unwrap();
    let s = metadata_schema(&p);
    assert_eq!(s["additionalProperties"], false);
    assert_eq!(s["title"], "DocumentMetadata");
    assert_eq!(
        s["required"],
        json!([
            "document_date",
            "company_name",
            "issuer",
            "recipient",
            "document_type",
            "total_amount",
            "currency",
            "description"
        ])
    );
    // No per-property titles (Python `str.title()` emulation dropped).
    assert!(s["properties"]["document_date"].get("title").is_none());
    assert_eq!(s["properties"]["company_name"]["type"], "string");
    let keys: Vec<&String> = s["properties"].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "document_date",
            "company_name",
            "issuer",
            "recipient",
            "document_type",
            "total_amount",
            "currency",
            "description"
        ]
    );
    let (_, a) = select_profile(&sample_config(), Some("academic")).unwrap();
    assert_eq!(
        metadata_schema(&a)["properties"]["first_author_surname"]["type"],
        "string"
    );

    // Local validation of model output.
    assert!(validate_metadata(&a, &json!({"document_date": "", "first_author_surname": "", "journal_name": "", "title": ""})).is_ok());
    assert!(validate_metadata(&a, &json!({"document_date": ""})).is_err());
    assert!(validate_metadata(&a, &json!({"document_date": "", "first_author_surname": "", "journal_name": "", "title": "", "x": ""})).is_err());
    assert!(validate_metadata(&a, &json!({"document_date": 1, "first_author_surname": "", "journal_name": "", "title": ""})).is_err());
}

#[test]
fn prompt_assembly_uses_resolved_order() {
    let mut c = sample_config();
    c["profiles"] = json!({"academic": {"fields": {"doi": {"description": "DOI as printed."}}}});
    let (_, a) = select_profile(&c, Some("academic")).unwrap();
    let prompt = system_prompt(&a, "Extra rule.");
    let parts: Vec<&str> = prompt.split("\n\n").collect();
    assert!(parts[0].starts_with("Extract the requested fields"));
    assert_eq!(
        parts[1],
        "You are naming an academic paper, preprint, or thesis."
    );
    assert!(parts[2].starts_with("document_date: Publication date"));
    assert!(parts[6].starts_with("doi: DOI as printed."));
    assert!(parts[7].starts_with("Return every requested field as a string."));
    assert_eq!(*parts.last().unwrap(), "Extra rule.");
    let mut c = sample_config();
    c["profiles"] = json!({"bare": {"fields": {"document_date": {"description": "D"}, "t": {"description": "T"}}, "template": "{t}", "truncate_field": "t"}});
    let (_, bare) = select_profile(&c, Some("bare")).unwrap();
    assert_eq!(
        system_prompt(&bare, "").split("\n\n").count(),
        4,
        "no intro part when intro empty"
    );
}
