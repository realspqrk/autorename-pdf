//! lopdf side: load + safety assessment, our attachment lookup, and the
//! incremental update.

use lopdf::{
    Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, StringFormat,
    decode_text_string, dictionary, text_string,
};
use md5::{Digest, Md5};

use super::payload::{ATTACHMENT_NAME, Attachment};
use super::{SkipReason, xmp};

/// Name-tree / AF nesting bound (cycles and hostile files).
const MAX_DEPTH: usize = 32;
const DESC: &str = "AutoRename-PDF extraction data (JSON)";

/// Where an indirect-or-direct dictionary lives.
#[derive(Debug, Clone)]
pub enum Loc {
    Absent,
    Direct(Dictionary),
    Indirect(ObjectId, Dictionary),
}

impl Loc {
    fn dict(&self) -> Option<&Dictionary> {
        match self {
            Loc::Absent => None,
            Loc::Direct(d) | Loc::Indirect(_, d) => Some(d),
        }
    }
}

/// Our previous attachment.
#[derive(Debug, Clone)]
pub struct Ours {
    pub key: Vec<u8>,
    pub filespec: Object,
    pub attachment: Attachment,
}

/// Loaded document plus everything the assessment and the update need.
pub struct Parsed {
    pub doc: Document,
    pub catalog_id: ObjectId,
    pub catalog: Dictionary,
    pub info: Loc,
    pub names: Loc,
    /// Indirect EmbeddedFiles tree root (reused for the new tree).
    pub ef_tree_id: Option<ObjectId>,
    /// Flattened `EmbeddedFiles` leaves (key, value).
    pub leaves: Vec<(Vec<u8>, Object)>,
    pub af: Vec<Object>,
    pub metadata_id: Option<ObjectId>,
    pub xmp: Option<String>,
    pub ours: Option<Ours>,
    pub page_count: usize,
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    doc.dereference(o).ok().map(|(_, o)| o)
}

fn resolve_dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match resolve(doc, o)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn dict_loc(doc: &Document, parent: &Dictionary, key: &[u8]) -> Result<Loc, SkipReason> {
    match parent.get(key) {
        Err(_) | Ok(Object::Null) => Ok(Loc::Absent),
        Ok(Object::Reference(id)) => match doc.get_object(*id) {
            Ok(Object::Dictionary(d)) => Ok(Loc::Indirect(*id, d.clone())),
            Ok(Object::Null) | Err(_) => Ok(Loc::Absent),
            Ok(_) => Err(SkipReason::Unparseable),
        },
        Ok(Object::Dictionary(d)) => Ok(Loc::Direct(d.clone())),
        Ok(_) => Err(SkipReason::Unparseable),
    }
}

pub fn stream_bytes(s: &Stream) -> Vec<u8> {
    if s.dict.has(b"Filter") {
        s.decompressed_content().unwrap_or_default()
    } else {
        s.content.clone()
    }
}

fn collect_leaves(
    doc: &Document,
    node: &Object,
    depth: usize,
    out: &mut Vec<(Vec<u8>, Object)>,
) -> Result<(), SkipReason> {
    if depth > MAX_DEPTH {
        return Err(SkipReason::Unparseable);
    }
    let d = resolve_dict(doc, node).ok_or(SkipReason::Unparseable)?;
    if let Ok(names) = d.get(b"Names") {
        let arr = resolve(doc, names)
            .and_then(|a| a.as_array().ok())
            .ok_or(SkipReason::Unparseable)?;
        for pair in arr.chunks(2) {
            if let [k, v] = pair {
                let key = resolve(doc, k)
                    .and_then(|k| k.as_str().ok())
                    .ok_or(SkipReason::Unparseable)?;
                out.push((key.to_vec(), v.clone()));
            }
        }
    }
    if let Ok(kids) = d.get(b"Kids") {
        let arr = resolve(doc, kids)
            .and_then(|a| a.as_array().ok())
            .ok_or(SkipReason::Unparseable)?;
        for kid in arr {
            collect_leaves(doc, kid, depth + 1, out)?;
        }
    }
    Ok(())
}

fn filespec_name(doc: &Document, fs: &Dictionary) -> String {
    [b"UF".as_slice(), b"F"]
        .iter()
        .find_map(|k| fs.get(k).ok().and_then(|o| resolve(doc, o)))
        .and_then(|o| decode_text_string(o).ok())
        .unwrap_or_default()
}

fn embedded_bytes(doc: &Document, fs: &Dictionary) -> Option<Vec<u8>> {
    let ef = resolve_dict(doc, fs.get(b"EF").ok()?)?;
    let f = ef.get(b"UF").or_else(|_| ef.get(b"F")).ok()?;
    match resolve(doc, f)? {
        Object::Stream(s) => Some(stream_bytes(s)),
        _ => None,
    }
}

/// E-invoice payloads (ZUGFeRD / Factur-X / XRechnung) or source/alternative
/// representations whose consumers we must not confuse.
fn is_invoice_filespec(doc: &Document, fs: &Dictionary) -> bool {
    let name = filespec_name(doc, fs).to_lowercase();
    let rel = fs
        .get(b"AFRelationship")
        .ok()
        .and_then(|o| o.as_name().ok())
        .unwrap_or_default();
    matches!(
        name.as_str(),
        "factur-x.xml" | "zugferd-invoice.xml" | "zugferd.xml" | "xrechnung.xml"
    ) || rel == b"Alternative"
        || rel == b"Source"
}

fn signature_evidence(doc: &Document) -> bool {
    doc.objects.values().any(|o| {
        let d = match o {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => return false,
        };
        let name_is = |k: &[u8], v: &[u8]| d.get(k).and_then(Object::as_name).is_ok_and(|n| n == v);
        d.has(b"ByteRange")
            || name_is(b"Type", b"Sig")
            || name_is(b"Type", b"DocTimeStamp")
            || name_is(b"FT", b"Sig")
    })
}

fn sig_flags(doc: &Document, catalog: &Dictionary) -> i64 {
    catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|o| resolve_dict(doc, o))
        .and_then(|f| f.get(b"SigFlags").ok())
        .and_then(|o| resolve(doc, o))
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
}

fn load(bytes: &[u8]) -> Result<Document, SkipReason> {
    let doc = match Document::load_mem(bytes) {
        Ok(d) => d,
        Err(_) if contains(bytes, b"/Encrypt") => return Err(SkipReason::Encrypted),
        Err(_) => return Err(SkipReason::Unparseable),
    };
    if doc.encryption_state.is_some() || doc.trailer.has(b"Encrypt") {
        return Err(SkipReason::Encrypted);
    }
    Ok(doc)
}

/// Load and assess. `Err` is a skip; `Ok` documents are safe to update.
pub fn parse(bytes: &[u8]) -> Result<Parsed, SkipReason> {
    let p = structure(load(bytes)?)?;
    let (doc, catalog) = (&p.doc, &p.catalog);
    if catalog
        .get(b"Perms")
        .ok()
        .and_then(|o| resolve_dict(doc, o))
        .is_some_and(|d| d.has(b"DocMDP"))
    {
        return Err(SkipReason::Certified);
    }
    if catalog.has(b"Perms")
        || sig_flags(doc, catalog) & 3 != 0
        || contains(bytes, b"/ByteRange")
        || signature_evidence(doc)
    {
        return Err(SkipReason::Signed);
    }
    if let Some(x) = &p.xmp {
        if x.contains(xmp::NS_PDFAID) {
            return Err(SkipReason::PdfA);
        }
        if xmp::E_INVOICE_NS.iter().any(|n| x.contains(n)) {
            return Err(SkipReason::EInvoice);
        }
    }
    if p.leaves
        .iter()
        .map(|(_, v)| v)
        .chain(&p.af)
        .filter_map(|o| resolve_dict(doc, o))
        .any(|fs| is_invoice_filespec(doc, fs))
    {
        return Err(SkipReason::EInvoice);
    }
    if doc.xref_recovery.is_some() {
        return Err(SkipReason::XrefRepaired);
    }
    if let Some(x) = &p.xmp {
        xmp::merge(x, "").map_err(|_| SkipReason::InvalidXmp)?;
    }
    Ok(p)
}

/// Read-only lookup for documents we would not update (signed after
/// enrichment, …): our attachment + raw XMP packet.
pub fn read_lenient(bytes: &[u8]) -> Result<Option<(Attachment, Option<String>)>, &'static str> {
    let p = structure(load(bytes).map_err(SkipReason::code)?).map_err(SkipReason::code)?;
    Ok(p.ours.map(|o| (o.attachment, p.xmp)))
}

/// Locate catalog, Info, Names/EmbeddedFiles, AF, XMP and our attachment.
fn structure(doc: Document) -> Result<Parsed, SkipReason> {
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| SkipReason::Unparseable)?;
    let catalog = doc
        .get_dictionary(catalog_id)
        .map_err(|_| SkipReason::Unparseable)?
        .clone();
    let page_count = doc.get_pages().len();
    let (metadata_id, xmp) = match catalog.get(b"Metadata") {
        Ok(o) => match resolve(&doc, o) {
            Some(Object::Stream(s)) => {
                let text =
                    String::from_utf8(stream_bytes(s)).map_err(|_| SkipReason::InvalidXmp)?;
                (
                    o.as_reference().ok(),
                    Some(text.trim_start_matches('\u{FEFF}').to_string()),
                )
            }
            _ => (None, None),
        },
        Err(_) => (None, None),
    };
    let info = dict_loc(&doc, &doc.trailer, b"Info")?;
    let names = dict_loc(&doc, &catalog, b"Names")?;
    let mut leaves = Vec::new();
    let mut ef_tree_id = None;
    if let Some(ef) = names.dict().and_then(|n| n.get(b"EmbeddedFiles").ok()) {
        ef_tree_id = ef.as_reference().ok();
        collect_leaves(&doc, ef, 0, &mut leaves)?;
    }
    let af: Vec<Object> = match catalog.get(b"AF").ok().and_then(|o| resolve(&doc, o)) {
        Some(Object::Array(a)) => a.clone(),
        Some(_) => return Err(SkipReason::Unparseable),
        None => Vec::new(),
    };
    let ours = leaves.iter().find_map(|(key, value)| {
        let fs = resolve_dict(&doc, value)?;
        if filespec_name(&doc, fs) != ATTACHMENT_NAME {
            return None;
        }
        let attachment = Attachment::parse(&embedded_bytes(&doc, fs)?)?;
        Some(Ours {
            key: key.clone(),
            filespec: value.clone(),
            attachment,
        })
    });
    Ok(Parsed {
        doc,
        catalog_id,
        catalog,
        info,
        names,
        ef_tree_id,
        leaves,
        af,
        metadata_id,
        xmp,
        ours,
        page_count,
    })
}

/// `D:YYYYMMDDHHmmSSZ`
pub fn pdf_date(ts: &chrono::DateTime<chrono::Utc>) -> Object {
    Object::string_literal(ts.format("D:%Y%m%d%H%M%SZ").to_string())
}

pub struct Update<'a> {
    pub title: &'a str,
    pub subject: &'a str,
    pub keywords: &'a str,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub json: &'a [u8],
    pub xmp_desc: &'a str,
}

/// Build the incremental update; returns the complete new file bytes.
pub fn build(bytes: &[u8], p: Parsed, u: &Update<'_>) -> lopdf::Result<Vec<u8>> {
    let packet = match &p.xmp {
        Some(x) => xmp::merge(x, u.xmp_desc)
            .map_err(|e| lopdf::Error::IO(std::io::Error::other(e.to_string())))?,
        None => xmp::fresh_packet(u.xmp_desc),
    };
    let mut inc = IncrementalDocument::create_from(bytes.to_vec(), p.doc);
    let nd = &mut inc.new_document;
    // Old xref-stream / hybrid entries must not leak into our section.
    for k in [b"XRefStm".as_slice(), b"DecodeParms"] {
        nd.trailer.remove(k);
    }
    let date = pdf_date(&u.timestamp);

    // Embedded file + filespec.
    let mut ef = Stream::new(
        dictionary! {
            "Type" => "EmbeddedFile",
            "Subtype" => Object::Name(b"application/json".to_vec()),
            "Params" => dictionary! {
                "Size" => u.json.len() as i64,
                "CheckSum" => Object::String(Md5::digest(u.json).to_vec(), StringFormat::Hexadecimal),
                "CreationDate" => date.clone(),
                "ModDate" => date.clone(),
            },
        },
        u.json.to_vec(),
    );
    ef.compress()?;
    let ef_id = nd.add_object(ef);
    let fs_id = nd.add_object(dictionary! {
        "Type" => "Filespec",
        "F" => text_string(ATTACHMENT_NAME),
        "UF" => text_string(ATTACHMENT_NAME),
        "Desc" => text_string(DESC),
        "AFRelationship" => "Data",
        "EF" => dictionary! { "F" => ef_id, "UF" => ef_id },
    });

    // EmbeddedFiles name tree: flattened, our previous entry replaced,
    // foreign entries kept; the key is made unique against foreign leaves.
    let our_key = p.ours.as_ref().map(|o| o.key.clone());
    let mut leaves: Vec<(Vec<u8>, Object)> = p
        .leaves
        .into_iter()
        .filter(|(k, _)| Some(k) != our_key.as_ref())
        .collect();
    let mut key = ATTACHMENT_NAME.as_bytes().to_vec();
    let mut n = 2;
    while leaves.iter().any(|(k, _)| *k == key) {
        key = format!("{ATTACHMENT_NAME} ({n})").into_bytes();
        n += 1;
    }
    leaves.push((key, Object::Reference(fs_id)));
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let tree = dictionary! {
        "Names" => leaves
            .into_iter()
            .flat_map(|(k, v)| [Object::String(k, StringFormat::Literal), v])
            .collect::<Vec<_>>(),
    };
    let tree_id = match p.ef_tree_id {
        Some(id) => {
            nd.set_object(id, tree);
            id
        }
        None => nd.add_object(tree),
    };
    let mut catalog = p.catalog;
    match p.names {
        Loc::Indirect(id, mut d) => {
            d.set("EmbeddedFiles", tree_id);
            nd.set_object(id, d);
        }
        Loc::Direct(mut d) => {
            d.set("EmbeddedFiles", tree_id);
            catalog.set("Names", d);
        }
        Loc::Absent => catalog.set("Names", dictionary! { "EmbeddedFiles" => tree_id }),
    }

    // Associated files: drop our previous filespec, append the new one.
    let old_fs = p.ours.as_ref().map(|o| o.filespec.clone());
    let mut af: Vec<Object> =
        p.af.into_iter()
            .filter(|o| Some(o) != old_fs.as_ref())
            .collect();
    af.push(Object::Reference(fs_id));
    catalog.set("AF", af);

    // XMP metadata (uncompressed, as XMP readers expect).
    let mut meta = Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        packet.into_bytes(),
    );
    meta.allows_compression = false;
    let meta_id = match p.metadata_id {
        Some(id) => {
            nd.set_object(id, meta);
            id
        }
        None => nd.add_object(meta),
    };
    catalog.set("Metadata", meta_id);
    nd.set_object(p.catalog_id, catalog);

    // Info: only Title/Subject/Keywords/ModDate; everything else untouched.
    let set_info = |d: &mut Dictionary| {
        d.set("Title", text_string(u.title));
        d.set("Subject", text_string(u.subject));
        if u.keywords.is_empty() {
            d.remove(b"Keywords");
        } else {
            d.set("Keywords", text_string(u.keywords));
        }
        d.set("ModDate", date.clone());
    };
    match p.info {
        Loc::Indirect(id, mut d) => {
            set_info(&mut d);
            nd.set_object(id, d);
        }
        Loc::Direct(mut d) => {
            set_info(&mut d);
            nd.trailer.set("Info", d);
        }
        Loc::Absent => {
            let mut d = Dictionary::new();
            set_info(&mut d);
            let id = nd.add_object(d);
            nd.trailer.set("Info", id);
        }
    }

    let mut out = Vec::with_capacity(bytes.len() + u.json.len() + 8192);
    inc.save_to(&mut out)?;
    Ok(out)
}
