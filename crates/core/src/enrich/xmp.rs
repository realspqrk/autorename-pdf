//! XMP packet merge and read-back.
//!
//! Merge is span-based: the existing packet is kept byte-for-byte except for
//! the properties we own (removed wherever they appear, element or attribute
//! form, resolved by namespace URI not prefix) and one `rdf:Description` we
//! insert before `</rdf:RDF>`. A description left with nothing foreign (our
//! previous one) is dropped as a whole, so re-runs never accumulate; foreign
//! properties sharing a description with ours are kept.

use std::collections::BTreeMap;

use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{QName, ResolveResult};
use serde_json::Value;

pub const NS_ARP: &str = "https://autorename-pdf.dev/ns/1.0/";
const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
const NS_PDF: &str = "http://ns.adobe.com/pdf/1.3/";
pub const NS_PDFAID: &str = "http://www.aiim.org/pdfa/ns/id/";
/// Factur-X / ZUGFeRD 1.x / ZUGFeRD 2.x extension schema URIs (prefix match).
pub const E_INVOICE_NS: &[&str] = &["urn:factur-x", "urn:ferd:", "urn:zugferd"];

/// Properties written by us (namespace, local name). `arp` is owned entirely.
const OWNED: &[(&str, &str)] = &[
    (NS_DC, "title"),
    (NS_DC, "description"),
    (NS_DC, "subject"),
    (NS_PDF, "Keywords"),
    (NS_XMP, "ModifyDate"),
    (NS_XMP, "MetadataDate"),
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid XMP packet: {0}")]
pub struct XmpError(String);

fn err(e: impl std::fmt::Display) -> XmpError {
    XmpError(e.to_string())
}

/// Values for our description. `arp` holds `(name, value)` scalars;
/// `fields` becomes an `arp:fields` struct.
#[derive(Debug, Clone, Default)]
pub struct Props {
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
    /// ISO 8601 (`2026-11-07T10:11:12Z`).
    pub modify_date: String,
    pub arp: Vec<(&'static str, String)>,
    pub fields: Vec<(String, String)>,
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Chars XML 1.0 cannot carry at all.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => out.push(' '),
            '\u{FFFE}' | '\u{FFFF}' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// XML NCName subset safe as an element local name.
pub fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Scalar rendering of a JSON field for XMP (`None` = omit).
pub fn field_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(_) | Value::Number(_) => Some(v.to_string()),
        Value::Array(_) | Value::Object(_) => Some(v.to_string()),
    }
}

pub fn description(p: &Props) -> String {
    let mut s = format!(
        "<rdf:Description rdf:about=\"\" xmlns:dc=\"{NS_DC}\" xmlns:xmp=\"{NS_XMP}\" \
         xmlns:pdf=\"{NS_PDF}\" xmlns:arp=\"{NS_ARP}\">\n"
    );
    let alt = |name: &str, v: &str| {
        format!(
            "   <{name}><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></{name}>\n",
            escape(v)
        )
    };
    s.push_str(&alt("dc:title", &p.title));
    s.push_str(&alt("dc:description", &p.description));
    if !p.keywords.is_empty() {
        s.push_str("   <dc:subject><rdf:Bag>");
        for k in &p.keywords {
            s.push_str(&format!("<rdf:li>{}</rdf:li>", escape(k)));
        }
        s.push_str("</rdf:Bag></dc:subject>\n");
        s.push_str(&format!(
            "   <pdf:Keywords>{}</pdf:Keywords>\n",
            escape(&p.keywords.join("; "))
        ));
    }
    s.push_str(&format!(
        "   <xmp:ModifyDate>{0}</xmp:ModifyDate>\n   <xmp:MetadataDate>{0}</xmp:MetadataDate>\n",
        escape(&p.modify_date)
    ));
    for (k, v) in &p.arp {
        s.push_str(&format!("   <arp:{k}>{}</arp:{k}>\n", escape(v)));
    }
    if !p.fields.is_empty() {
        s.push_str("   <arp:fields rdf:parseType=\"Resource\">\n");
        for (k, v) in &p.fields {
            s.push_str(&format!("    <arp:{k}>{}</arp:{k}>\n", escape(v)));
        }
        s.push_str("   </arp:fields>\n");
    }
    s.push_str("  </rdf:Description>\n");
    s
}

pub fn fresh_packet(desc: &str) -> String {
    let pad = format!("{}\n", " ".repeat(99)).repeat(20);
    format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n \
         <rdf:RDF xmlns:rdf=\"{NS_RDF}\">\n  {desc} </rdf:RDF>\n</x:xmpmeta>\n{pad}<?xpacket end=\"w\"?>"
    )
}

fn ns_of(r: &ResolveResult<'_>) -> Option<String> {
    match r {
        ResolveResult::Bound(ns) => Some(ns.as_ref().to_string()),
        _ => None,
    }
}

fn owned(ns: Option<&str>, local: &str) -> bool {
    match ns {
        Some(NS_ARP) => true,
        Some(ns) => OWNED.iter().any(|(n, l)| *n == ns && *l == local),
        None => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rdf,
    /// Top-level `rdf:Description` (direct child of `rdf:RDF`).
    Description,
    Other,
}

/// Open top-level description: where it starts, the edit count when it
/// opened, whether it holds anything we do not own, and whether we removed
/// something from it.
struct Desc {
    start: usize,
    mark: usize,
    foreign: bool,
    removed: bool,
}

/// Merge `desc` into `existing`. Errors when the packet is not well-formed
/// XML or has no `rdf:RDF` element.
///
/// Owned properties (element or attribute form, by namespace URI) are
/// removed from every top-level description; a description is dropped as a
/// whole only when nothing foreign remains in it (so our previous
/// description never accumulates, and consolidated foreign ones survive).
pub fn merge(existing: &str, desc: &str) -> Result<String, XmpError> {
    let mut r = NsReader::from_str(existing);
    r.config_mut().check_end_names = true;
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut stack: Vec<Kind> = Vec::new();
    let mut descs: Vec<Desc> = Vec::new();
    let mut insert_at = None;
    loop {
        let start = usize::try_from(r.buffer_position()).map_err(err)?;
        let (res, ev) = r.read_resolved_event().map_err(err)?;
        let ns = ns_of(&res);
        let pos = |r: &NsReader<&[u8]>| usize::try_from(r.buffer_position()).map_err(err);
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                let local = e.local_name().as_ref().to_string();
                let parent = stack.last().copied();
                let is_rdf = |l: &str| ns.as_deref() == Some(NS_RDF) && local == l;
                let kind = if is_rdf("RDF") {
                    Kind::Rdf
                } else if is_rdf("Description") && parent == Some(Kind::Rdf) {
                    Kind::Description
                } else {
                    Kind::Other
                };
                if parent == Some(Kind::Description) {
                    let d = descs.last_mut().expect("description frame");
                    if owned(ns.as_deref(), &local) {
                        if !empty {
                            let name = e.name().as_ref().to_string();
                            r.read_to_end(QName(&name)).map_err(err)?;
                        }
                        edits.push((start, pos(&r)?, String::new()));
                        d.removed = true;
                        continue;
                    }
                    d.foreign = true;
                }
                if kind == Kind::Description {
                    let mut d = Desc {
                        start,
                        mark: edits.len(),
                        foreign: false,
                        removed: false,
                    };
                    let mut kept = Vec::new();
                    for a in e.attributes() {
                        let a = a.map_err(err)?;
                        if a.key.as_namespace_binding().is_some() {
                            kept.push(a);
                            continue;
                        }
                        let (ares, alocal) = r.resolver().resolve_attribute(a.key);
                        let ans = ns_of(&ares);
                        if owned(ans.as_deref(), alocal.as_ref()) {
                            d.removed = true;
                            continue;
                        }
                        // rdf:about etc. are structure, not content.
                        d.foreign |= ans.as_deref() != Some(NS_RDF);
                        kept.push(a);
                    }
                    let end = pos(&r)?;
                    if empty && d.removed && !d.foreign {
                        edits.push((start, end, String::new()));
                        continue;
                    }
                    if d.removed {
                        let mut tag = format!("<{}", e.name().as_ref());
                        for a in kept {
                            let v = a.value.as_ref();
                            let q = if v.contains('"') { '\'' } else { '"' };
                            tag.push_str(&format!(" {}={q}{v}{q}", a.key.as_ref()));
                        }
                        tag.push_str(if empty { "/>" } else { ">" });
                        edits.push((start, end, tag));
                    }
                    if !empty {
                        descs.push(d);
                    }
                }
                if !empty {
                    stack.push(kind);
                }
            }
            Event::Text(ref t) if stack.last() == Some(&Kind::Description) => {
                if !t.xml10_content().trim().is_empty() {
                    descs.last_mut().expect("description frame").foreign = true;
                }
            }
            Event::CData(_) if stack.last() == Some(&Kind::Description) => {
                descs.last_mut().expect("description frame").foreign = true;
            }
            Event::End(_) => match stack.pop() {
                Some(Kind::Rdf) if insert_at.is_none() => insert_at = Some(start),
                Some(Kind::Description) => {
                    let d = descs.pop().expect("description frame");
                    if d.removed && !d.foreign {
                        edits.truncate(d.mark);
                        edits.push((d.start, pos(&r)?, String::new()));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let at = insert_at.ok_or_else(|| XmpError("no rdf:RDF element".into()))?;
    edits.push((at, at, format!("  {desc} ")));
    edits.sort_by_key(|(s, e, _)| (*s, *e));
    let mut out = String::with_capacity(existing.len() + desc.len() + 16);
    let mut pos = 0;
    for (s, e, rep) in edits {
        out.push_str(&existing[pos..s]);
        out.push_str(&rep);
        pos = e;
    }
    out.push_str(&existing[pos..]);
    Ok(out)
}

/// `arp:` leaf values, nested structs as dotted paths (`fields.company`).
pub fn read_arp(packet: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut r = NsReader::from_str(packet);
    // Stack of arp local names (None for non-arp elements).
    let mut path: Vec<Option<String>> = Vec::new();
    let mut text = String::new();
    while let Ok((res, ev)) = r.read_resolved_event() {
        let is_arp = ns_of(&res).as_deref() == Some(NS_ARP);
        match ev {
            Event::Start(e) => {
                path.push(is_arp.then(|| e.local_name().as_ref().to_string()));
                text.clear();
            }
            Event::Text(t) => {
                text.push_str(&t.xml10_content());
            }
            Event::GeneralRef(g) => {
                let name = g.as_ref().to_string();
                text.push_str(match name.as_str() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
            }
            Event::End(_) => {
                if let Some(Some(_)) = path.last() {
                    let key: Vec<&str> = path.iter().flatten().map(String::as_str).collect();
                    let v = text.trim();
                    if !v.is_empty() {
                        out.insert(key.join("."), v.to_string());
                    }
                }
                path.pop();
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> Props {
        Props {
            title: "20261107 ACME AP 12,13".into(),
            description: "AP ACME & Co".into(),
            keywords: vec!["ACME".into(), "AP".into()],
            modify_date: "2026-11-07T10:11:12Z".into(),
            arp: vec![("profile", "business".into())],
            fields: vec![("company".into(), "ACME <x>".into())],
        }
    }

    const ADOBE: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:xap="http://ns.adobe.com/xap/1.0/" xmlns:pdf="http://ns.adobe.com/pdf/1.3/" xap:ModifyDate="2020-01-01" xap:CreatorTool="Word" pdf:Producer="Acrobat"/>
<rdf:Description rdf:about="" xmlns:d="http://purl.org/dc/elements/1.1/" xmlns:foo="urn:foo"><d:title><rdf:Alt><rdf:li xml:lang="x-default">Old</rdf:li></rdf:Alt></d:title><d:creator><rdf:Seq><rdf:li>Jane</rdf:li></rdf:Seq></d:creator><foo:bar>keep</foo:bar></rdf:Description>
</rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn merge_replaces_owned_props_by_namespace_and_keeps_rest() {
        let merged = merge(ADOBE, &description(&props())).unwrap();
        // Foreign props survive (any prefix), owned ones are replaced.
        assert!(merged.contains(r#"xap:CreatorTool="Word""#));
        assert!(merged.contains(r#"pdf:Producer="Acrobat""#));
        assert!(merged.contains("<d:creator>"));
        assert!(merged.contains("<foo:bar>keep</foo:bar>"));
        assert!(!merged.contains("2020-01-01"));
        assert!(!merged.contains(">Old<"));
        assert!(merged.contains("AP ACME &amp; Co"));
        assert!(merged.starts_with("<?xpacket begin"));
        assert!(merged.ends_with("<?xpacket end=\"w\"?>"));
        // Re-merge: our previous description is dropped, never duplicated.
        let again = merge(&merged, &description(&props())).unwrap();
        assert_eq!(again.matches(NS_ARP).count(), 1);
        assert_eq!(again.matches("<dc:title>").count(), 1);
        let arp = read_arp(&again);
        assert_eq!(arp["profile"], "business");
        assert_eq!(arp["fields.company"], "ACME <x>");
    }

    #[test]
    fn consolidated_description_keeps_foreign_properties() {
        let ours = merge(ADOBE, &description(&props())).unwrap();
        // Another tool consolidates everything into one description with ours.
        let consolidated = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="{NS_RDF}">
<rdf:Description rdf:about="" xmlns:dc="{NS_DC}" xmlns:arp="{NS_ARP}" xmlns:ret="urn:retention" ret:until="2030">
<arp:profile>business</arp:profile><dc:title><rdf:Alt><rdf:li>T</rdf:li></rdf:Alt></dc:title>
<dc:creator><rdf:Seq><rdf:li>Jane Doe</rdf:li></rdf:Seq></dc:creator><ret:class>tax</ret:class>
</rdf:Description></rdf:RDF></x:xmpmeta>"#
        );
        let merged = merge(&consolidated, &description(&props())).unwrap();
        assert!(merged.contains("Jane Doe"), "{merged}");
        assert!(merged.contains("<ret:class>tax</ret:class>"));
        assert!(merged.contains(r#"ret:until="2030""#));
        assert_eq!(merged.matches("<arp:profile>").count(), 1);
        assert_eq!(merged.matches("<dc:title>").count(), 1);
        // A description holding only owned props (ours) is dropped whole.
        let again = merge(&ours, &description(&props())).unwrap();
        assert_eq!(
            again.matches("<rdf:Description").count(),
            ours.matches("<rdf:Description").count()
        );
    }

    #[test]
    fn fresh_packet_round_trips_and_garbage_is_rejected() {
        let p = fresh_packet(&description(&props()));
        assert_eq!(read_arp(&p)["profile"], "business");
        assert!(merge(&p, &description(&props())).is_ok());
        assert!(merge("<a><b></a>", "").is_err());
        assert!(merge("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>", "").is_err());
    }
}
