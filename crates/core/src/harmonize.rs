//! Name matching ("name lists"): harmonization and auto-learning of one
//! identifier field per list, `<config-dir>/name-lists/<field>.yaml`
//! (e.g. `name-lists/company_name.yaml`). The field decides the
//! [`NameKind`]: `company_name` matches company names (legal suffixes are
//! ignored), every other field uses plain [`NameKind::Generic`] matching.
//!
//! File format:
//!
//! ```yaml
//! "ACME":
//!   aliases: ["ACME Corporation"]   # hand-written, never rewritten
//!   learned: ["Acme Corp."]         # auto-added aliases
//!   source: user                    # user | auto
//!   added: "2026-11-07"             # auto entries
//!   review: true                    # auto entry worth a human look
//! ```
//!
//! [`decide`] matches a name: normalized exact index, then ranked candidates
//! with the auto-alias / decider / new-entry bands (spec §Auto-harmonization).
//! [`lookup`] is the read-only variant (auto-learning off).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

use chrono::NaiveDate;
use indexmap::IndexMap;
use rapidfuzz::distance::{indel, jaro_winkler};
use serde::{Deserialize, Serialize};

use crate::fsx;
use crate::textutil::{nfc, nfkc};
use crate::{Error, Result};

/// Score at/above which a name is added as alias without asking.
pub const AUTO_ALIAS_THRESHOLD: f64 = 0.92;
/// Score at/above which (and below auto-alias) the decider is asked.
pub const DECIDER_THRESHOLD: f64 = 0.75;
/// Token-subset matches ("Bank" vs "Bank Austria") never auto-alias.
pub const SUBSET_CAP: f64 = 0.91;
/// Extraction confidence below which nothing is learned.
pub const MIN_LEARN_CONFIDENCE: f64 = 0.5;
pub const TOP_K: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    User,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub canonical: String,
    /// Hand-written aliases (never modified by auto-learning).
    pub aliases: Vec<String>,
    /// Auto-learned aliases.
    pub learned: Vec<String>,
    pub source: Source,
    pub added: Option<String>,
    pub review: bool,
    /// Why the entry needs review (written by auto-learning).
    pub evidence: Option<ReviewEvidence>,
}

/// Persisted next to `review: true`: what the app saw when it could not
/// decide (`review: {reason, candidates, source, batch}` in the file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewEvidence {
    pub reason: String,
    #[serde(default)]
    pub candidates: Vec<ReviewCandidate>,
    /// PDF file name (never a path).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Undo batch that created the entry.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub batch: String,
}

impl Eq for ReviewEvidence {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewCandidate {
    pub name: String,
    pub score: f64,
}

impl Entry {
    fn synonyms(&self) -> impl Iterator<Item = &String> {
        self.aliases.iter().chain(&self.learned)
    }
}

/// The field whose list matches company names ([`NameKind::Company`]).
pub const COMPANY_FIELD: &str = "company_name";

/// How names of a list are normalized and compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NameKind {
    /// `company_name`: legal-form suffixes (`GmbH`, `Inc.`) and most
    /// punctuation are ignored.
    Company,
    /// Any other field: case, spacing and surrounding punctuation are
    /// ignored; inner punctuation and every word count.
    Generic,
}

impl NameKind {
    pub fn for_field(field: &str) -> NameKind {
        if field == COMPANY_FIELD {
            NameKind::Company
        } else {
            NameKind::Generic
        }
    }
}

/// A field that can have a name list: lowercase ASCII identifier
/// (`a-z`, `0-9`, `_`, starting with a letter) and not a Windows device
/// name. The id is the file stem verbatim, so distinct fields never share a
/// file on any filesystem (case-insensitive ones included).
pub fn is_field_id(s: &str) -> bool {
    let mut chars = s.chars();
    let shape = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    let device = matches!(s, "con" | "prn" | "aux" | "nul")
        || (s.len() == 4
            && (s.starts_with("com") || s.starts_with("lpt"))
            && matches!(s.as_bytes()[3], b'1'..=b'9'));
    shape && !device
}

/// UI/help labels of a field's list: `(singular, plural)`, capitalized,
/// e.g. `("Company", "Companies")`. Unmapped ids are humanized
/// (`issuer_name` → `("Issuer name", "Issuer names")`,
/// `vendor` → `("Vendor", "Vendor names")`).
pub fn field_label(field: &str) -> (String, String) {
    let (s, p) = match field {
        COMPANY_FIELD => ("Company", "Companies"),
        "journal_name" => ("Venue", "Venues"),
        "first_author_surname" => ("Author", "Authors"),
        _ => {
            let words = field.replace('_', " ");
            let words = words.trim();
            let mut chars = words.chars();
            let singular: String = match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => "Name".into(),
            };
            let plural = if singular.ends_with("name") || singular == "Name" {
                format!("{singular}s")
            } else {
                format!("{singular} names")
            };
            return (singular, plural);
        }
    };
    (s.into(), p.into())
}

/// One name list. `field` is the profile field it harmonizes; it decides
/// the [`NameKind`] every operation on this DB uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameDb {
    pub field: String,
    pub entries: Vec<Entry>,
}

impl NameDb {
    pub fn empty(field: &str) -> Self {
        NameDb {
            field: field.to_string(),
            entries: Vec::new(),
        }
    }
    pub fn kind(&self) -> NameKind {
        NameKind::for_field(&self.field)
    }
    /// Index of the entry with exactly this canonical name.
    pub fn find(&self, canonical: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.canonical == canonical)
    }
    /// Lowercase singular noun for messages (`company`, `venue`).
    fn noun(&self) -> String {
        field_label(&self.field).0.to_lowercase()
    }
    /// `autorename-pdf names --field <field> <args>` for hints.
    fn cmd(&self, args: &str) -> String {
        format!("autorename-pdf names --field {} {args}", self.field)
    }
}

// ---------------------------------------------------------------------------
// Load / save
// ---------------------------------------------------------------------------

/// On-disk entry shape.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryFile {
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    learned: Vec<String>,
    #[serde(default)]
    source: Source,
    #[serde(default)]
    added: Option<String>,
    #[serde(default)]
    review: ReviewField,
}

/// `review: true | false | {reason, candidates, source, batch}`.
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum ReviewField {
    #[default]
    No,
    Flag(bool),
    Evidence(ReviewEvidence),
}

/// Parse the list of `field` (duplicate keys and unknown entry keys are errors).
pub fn parse_db(text: &str, field: &str) -> Result<NameDb> {
    let options = serde_saphyr::options! { strict_booleans: true };
    let map: Option<IndexMap<String, EntryFile>> =
        serde_saphyr::from_str_with_options(text, options).map_err(|e| {
            Error::config_hint(
                format!("name list {field:?} is invalid: {e}"),
                "Each entry is `Name: {aliases: [...]}`; see docs/migration-3-to-4.md.",
            )
        })?;
    let entries = map
        .unwrap_or_default()
        .into_iter()
        .map(|(canonical, e)| Entry {
            canonical,
            aliases: e.aliases,
            learned: e.learned,
            source: e.source,
            added: e.added,
            review: !matches!(e.review, ReviewField::No | ReviewField::Flag(false)),
            evidence: match e.review {
                ReviewField::Evidence(ev) => Some(ev),
                _ => None,
            },
        })
        .collect();
    Ok(NameDb {
        field: field.to_string(),
        entries,
    })
}

/// Load the list of `field` from `path`; a missing file is an empty DB.
/// Invalid content is an error so callers never overwrite a file they could
/// not read.
pub fn load_db(path: &Path, field: &str) -> Result<NameDb> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_db(&text, field),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(NameDb::empty(field)),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn q(s: &str) -> String {
    // JSON strings are valid YAML double-quoted scalars.
    serde_json::to_string(s).expect("string serializes")
}

fn list(items: &[String]) -> String {
    format!(
        "[{}]",
        items.iter().map(|s| q(s)).collect::<Vec<_>>().join(", ")
    )
}

/// Comment only: the list's field (and kind) come from the file name.
const HEADER: &str = "\
# AutoRename-PDF name list. Maps a canonical name to its variants.
#   aliases: hand-written variants (never changed by the app)
#   learned: variants added automatically (harmonization.auto_learn)
#   source:  user | auto      review: true = auto entry worth a human look
# Edit freely; comments are not preserved when the app rewrites this file.
";

/// Render the file.
pub fn render_db(db: &NameDb) -> String {
    let mut out = String::from(HEADER);
    for e in &db.entries {
        out.push('\n');
        out.push_str(&format!(
            "{}:\n  aliases: {}\n",
            q(&e.canonical),
            list(&e.aliases)
        ));
        if !e.learned.is_empty() {
            out.push_str(&format!("  learned: {}\n", list(&e.learned)));
        }
        out.push_str(match e.source {
            Source::User => "  source: user\n",
            Source::Auto => "  source: auto\n",
        });
        if let Some(added) = &e.added {
            out.push_str(&format!("  added: {}\n", q(added)));
        }
        match (&e.review, &e.evidence) {
            (true, Some(ev)) => {
                // Block style, like the rest of the entry.
                out.push_str(&format!("  review:\n    reason: {}\n", q(&ev.reason)));
                if ev.candidates.is_empty() {
                    out.push_str("    candidates: []\n");
                } else {
                    out.push_str("    candidates:\n");
                    for c in &ev.candidates {
                        out.push_str(&format!(
                            "      - name: {}\n        score: {:.2}\n",
                            q(&c.name),
                            c.score
                        ));
                    }
                }
                if !ev.source.is_empty() {
                    out.push_str(&format!("    source: {}\n", q(&ev.source)));
                }
                if !ev.batch.is_empty() {
                    out.push_str(&format!("    batch: {}\n", q(&ev.batch)));
                }
            }
            (true, None) => out.push_str("  review: true\n"),
            _ => {}
        }
    }
    out
}

/// Atomic write. Callers only save after a change; concurrent writers use
/// [`update_db`] instead.
pub fn save_db(db: &NameDb, path: &Path) -> Result<()> {
    fsx::write_atomic(path, render_db(db).as_bytes()).map_err(|e| Error::io(path, e))
}

/// Locked reload-modify-save of the list of `field` at `path`: holds
/// `<file>.lock` while `f` edits the current file contents, and saves only
/// when the DB changed. Every writer (learning, undo, `names
/// add|merge|rename|review`) goes through here, so concurrent runs never lose
/// names. `f`'s error aborts without saving.
pub fn update_db<T>(
    path: &Path,
    field: &str,
    f: impl FnOnce(&mut NameDb) -> Result<T>,
) -> Result<T> {
    let _lock = fsx::lock_sibling(path).map_err(|e| Error::io(path, e))?;
    let mut db = load_db(path, field)?;
    let before = db.clone();
    let out = f(&mut db)?;
    if db != before {
        save_db(&db, path)?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

const LEGAL_SUFFIXES: &[&str] = &[
    "gmbh",
    "mbh",
    "ag",
    "kg",
    "kgaa",
    "ohg",
    "gbr",
    "ug",
    "eu",
    "og",
    "se",
    "inc",
    "incorporated",
    "corp",
    "corporation",
    "co",
    "ltd",
    "limited",
    "llc",
    "llp",
    "lp",
    "plc",
    "pty",
    "sa",
    "sas",
    "sasu",
    "sarl",
    "eurl",
    "sca",
    "bv",
    "nv",
    "vof",
    "srl",
    "spa",
    "sl",
    "slu",
    "ab",
    "as",
    "asa",
    "oy",
    "oyj",
    "aps",
    "sro",
    "kft",
    "zrt",
    "doo",
];

/// Unicode default case folding after NFKC (`ß` → `ss`, `Σ`/`ς` → `σ`, …).
pub fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(&nfkc(s))
}

/// Matching key of a name for `kind` (see [`normalize_company`],
/// [`normalize_generic`]).
pub fn normalize(kind: NameKind, name: &str) -> String {
    match kind {
        NameKind::Company => normalize_company(name),
        NameKind::Generic => normalize_generic(name),
    }
}

/// Punctuation trimmed from both ends of a generic name.
fn is_edge_punct(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '.' | ','
                | ';'
                | ':'
                | '"'
                | '\''
                | '`'
                | '´'
                | '‘'
                | '’'
                | '‚'
                | '“'
                | '”'
                | '„'
                | '«'
                | '»'
                | '‹'
                | '›'
        )
}

/// NFKC, casefold, trim quotes/periods/commas at both ends, collapse spaces.
/// Never strips words (`Nature Physics Ltd` keeps `ltd`) and keeps inner
/// punctuation (`C++`, `J. Chem. Phys`).
pub fn normalize_generic(name: &str) -> String {
    let folded = casefold(name);
    folded
        .trim_matches(is_edge_punct)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Casefold, drop periods/apostrophes, every other non-alphanumeric → space.
fn company_fold(name: &str) -> String {
    casefold(name)
        .chars()
        .filter(|c| !matches!(c, '.' | '\'' | '’' | '`' | '´'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect()
}

/// NFKC, casefold, drop punctuation, strip trailing legal-form tokens, collapse spaces.
pub fn normalize_company(name: &str) -> String {
    let mapped = company_fold(name);
    let mut tokens: Vec<&str> = mapped.split_whitespace().collect();
    while tokens.len() > 1 && tokens.last().is_some_and(|t| LEGAL_SUFFIXES.contains(t)) {
        tokens.pop();
    }
    tokens.join(" ")
}

/// `name` as written, minus the trailing legal-form words that
/// [`normalize_company`] ignores, split at the same punctuation
/// (`ACME Corp. GmbH` → `ACME`, `Müller GmbH & Co. KG` → `Müller`,
/// `ACME,Inc.` → `ACME`). Never empties a name; `normalize_company` of the
/// result equals that of `name` (spaces collapsed).
pub fn strip_legal_form(name: &str) -> String {
    // Byte ranges of the tokens `company_fold` keeps (periods and
    // apostrophes join, every other non-alphanumeric splits). Combining
    // marks belong to their letter (`Cafe\u{301}` is `Café` after NFKC).
    let joins = |c: char| {
        c.is_alphanumeric()
            || matches!(c, '.' | '\'' | '’' | '`' | '´')
            || c.general_category_group() == GeneralCategoryGroup::Mark
    };
    let mut tokens: Vec<(usize, String)> = Vec::new();
    let mut at: Option<usize> = None;
    for (i, c) in name.char_indices().chain([(name.len(), ' ')]) {
        match (at, joins(c)) {
            (None, true) => at = Some(i),
            (Some(st), false) => {
                let folded = company_fold(&name[st..i]);
                if !folded.trim().is_empty() {
                    tokens.push((st, folded));
                }
                at = None;
            }
            _ => {}
        }
    }
    let legal = |f: &str| f.split_whitespace().all(|t| LEGAL_SUFFIXES.contains(&t));
    let mut cut = name.len();
    while tokens.len() > 1 && tokens.last().is_some_and(|(_, f)| legal(f)) {
        cut = tokens.pop().map_or(cut, |(st, _)| st);
    }
    let mut out = name[..cut].split_whitespace().collect::<Vec<_>>().join(" ");
    if cut < name.len() {
        // Drop the removed suffix's separators and wrappers (`, `, `-`,
        // `(`, `，`), but keep a bracket that closes one in the name.
        while let Some(c) = out.chars().last() {
            if joins(c) || closes_open_bracket(&out[..out.len() - c.len_utf8()], c) {
                break;
            }
            out.pop();
        }
    }
    out
}

/// `c` closes a bracket left open in `before` (`Foo (Europe` + `)`).
fn closes_open_bracket(before: &str, c: char) -> bool {
    const PAIRS: [(char, char); 7] = [
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('（', '）'),
        ('［', '］'),
        ('【', '】'),
        ('「', '」'),
    ];
    PAIRS.iter().any(|&(open, close)| {
        c == close
            && before.chars().filter(|&x| x == open).count()
                > before.chars().filter(|&x| x == close).count()
    })
}

fn indel_ratio(a: &str, b: &str) -> f64 {
    indel::normalized_similarity(a.chars(), b.chars())
}

/// rapidfuzz `token_set_ratio` (0..1).
pub fn token_set_ratio(a: &str, b: &str) -> f64 {
    let sa: BTreeSet<&str> = a.split_whitespace().collect();
    let sb: BTreeSet<&str> = b.split_whitespace().collect();
    let inter: Vec<&str> = sa.intersection(&sb).copied().collect();
    let ab: Vec<&str> = sa.difference(&sb).copied().collect();
    let ba: Vec<&str> = sb.difference(&sa).copied().collect();
    if !inter.is_empty() && (ab.is_empty() || ba.is_empty()) {
        return 1.0;
    }
    let t0 = inter.join(" ");
    let join = |diff: &[&str]| {
        if t0.is_empty() {
            diff.join(" ")
        } else {
            format!("{t0} {}", diff.join(" "))
        }
    };
    let (t1, t2) = (join(&ab), join(&ba));
    let mut best = indel_ratio(&t1, &t2);
    if !t0.is_empty() {
        best = best.max(indel_ratio(&t0, &t1)).max(indel_ratio(&t0, &t2));
    }
    best
}

/// Similarity of two normalized names: max(Jaro-Winkler, token-set), capped
/// below the auto-alias band when one token set strictly contains the other.
pub fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let jw = jaro_winkler::similarity(a.chars(), b.chars());
    let score = jw.max(token_set_ratio(a, b));
    let sa: BTreeSet<&str> = a.split_whitespace().collect();
    let sb: BTreeSet<&str> = b.split_whitespace().collect();
    let strict_subset = sa != sb && (sa.is_subset(&sb) || sb.is_subset(&sa));
    // Prefix extensions ("Mueller" vs "Mueller Bau") are different firms often enough.
    let prefix_ext = a.starts_with(b) || b.starts_with(a);
    if strict_subset || prefix_ext {
        score.min(SUBSET_CAP)
    } else {
        score
    }
}

/// Read-only lookup (auto-learning off): canonical for exact hits and
/// auto-alias matches (≥ [`AUTO_ALIAS_THRESHOLD`]); ambiguous matches are
/// never mapped without a decider.
pub fn lookup(db: &NameDb, name: &str) -> Option<String> {
    let r = decide(db, name, None).resolve(name, None);
    r.mapped.then_some(r.name)
}

/// Normalized form → entry index over canonical names, aliases and learned aliases.
pub fn index(db: &NameDb) -> HashMap<String, usize> {
    let mut idx = HashMap::new();
    for (i, e) in db.entries.iter().enumerate() {
        for name in std::iter::once(&e.canonical).chain(e.synonyms()) {
            let n = normalize(db.kind(), name);
            if !n.is_empty() {
                idx.entry(n).or_insert(i);
            }
        }
    }
    idx
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub canonical: String,
    /// The canonical/alias spelling that scored best.
    pub matched: String,
    pub score: f64,
}

/// Top-`k` entries by best similarity over canonical + aliases (stable on ties).
pub fn rank(db: &NameDb, name: &str, k: usize) -> Vec<Candidate> {
    let target = normalize(db.kind(), name);
    let mut out: Vec<Candidate> = db
        .entries
        .iter()
        .filter_map(|e| {
            std::iter::once(&e.canonical)
                .chain(e.synonyms())
                .map(|s| (s, similarity(&target, &normalize(db.kind(), s))))
                .fold(None, |best: Option<(&String, f64)>, (s, sc)| match best {
                    Some((_, b)) if b >= sc => best,
                    _ => Some((s, sc)),
                })
                .map(|(s, score)| Candidate {
                    canonical: e.canonical.clone(),
                    matched: s.clone(),
                    score,
                })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(k);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Empty,
    Unknown,
    LowConfidence,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Nothing to match or learn; keep the raw name.
    Skip { reason: SkipReason },
    /// Exact normalized hit; no write.
    Known { canonical: String },
    /// High-confidence variant of an existing entry.
    AddAlias {
        canonical: String,
        alias: String,
        score: f64,
    },
    /// Ambiguous band: ask the decider (choice over candidates + "new").
    AskDecider {
        name: String,
        candidates: Vec<Candidate>,
    },
    /// No plausible match.
    NewEntry { name: String },
}

fn is_unknown(name: &str) -> bool {
    name.eq_ignore_ascii_case("unknown")
}

/// Classify an extracted name against the list.
pub fn decide(db: &NameDb, raw: &str, confidence: Option<f64>) -> Decision {
    let name = nfc(raw.trim());
    let norm = normalize(db.kind(), &name);
    if norm.is_empty() {
        return Decision::Skip {
            reason: SkipReason::Empty,
        };
    }
    if is_unknown(&name) {
        return Decision::Skip {
            reason: SkipReason::Unknown,
        };
    }
    if let Some(i) = index(db).get(&norm) {
        return Decision::Known {
            canonical: nfc(&db.entries[*i].canonical),
        };
    }
    if confidence.is_some_and(|c| c < MIN_LEARN_CONFIDENCE) {
        return Decision::Skip {
            reason: SkipReason::LowConfidence,
        };
    }
    let candidates = rank(db, &name, TOP_K);
    match candidates.first() {
        Some(top) if top.score >= AUTO_ALIAS_THRESHOLD => Decision::AddAlias {
            canonical: top.canonical.clone(),
            alias: name,
            score: top.score,
        },
        Some(top) if top.score >= DECIDER_THRESHOLD => {
            let candidates = candidates
                .into_iter()
                .filter(|c| c.score >= DECIDER_THRESHOLD)
                .collect();
            Decision::AskDecider { name, candidates }
        }
        _ => Decision::NewEntry { name },
    }
}

/// Decider verdict for [`Decision::AskDecider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Existing(String),
    New,
}

/// What to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Alias { canonical: String, alias: String },
    New { name: String, review: bool },
}

/// The single harmonization outcome: the name for the filename and the
/// optional DB write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub name: String,
    /// `name` is an existing canonical entry.
    pub mapped: bool,
    pub learn: Option<Choice>,
}

impl Decision {
    /// Resolve with an optional decider verdict (only meaningful for
    /// [`Decision::AskDecider`]; `raw` is the fallback name for skips).
    ///
    /// `verdict = None` means decider off/unavailable: the whole ambiguous
    /// band (below [`AUTO_ALIAS_THRESHOLD`]) keeps the extracted name as a new
    /// entry flagged `review: true` — never a silent mapping. A verdict naming
    /// a non-candidate is treated the same way.
    pub fn resolve(&self, raw: &str, verdict: Option<&Verdict>) -> Resolution {
        let map = |c: &str, learn| Resolution {
            name: c.to_string(),
            mapped: true,
            learn,
        };
        let new = |name: &str, review| Resolution {
            name: name.to_string(),
            mapped: false,
            learn: Some(Choice::New {
                name: name.to_string(),
                review,
            }),
        };
        match self {
            Decision::Skip { .. } => Resolution {
                name: raw.trim().to_string(),
                mapped: false,
                learn: None,
            },
            Decision::Known { canonical } => map(canonical, None),
            Decision::AddAlias {
                canonical, alias, ..
            } => map(
                canonical,
                Some(Choice::Alias {
                    canonical: canonical.clone(),
                    alias: alias.clone(),
                }),
            ),
            Decision::NewEntry { name } => new(name, false),
            Decision::AskDecider { name, candidates } => match verdict {
                Some(Verdict::Existing(c)) if candidates.iter().any(|x| &x.canonical == c) => map(
                    c,
                    Some(Choice::Alias {
                        canonical: c.clone(),
                        alias: name.clone(),
                    }),
                ),
                Some(Verdict::New) => new(name, false),
                None | Some(Verdict::Existing(_)) => new(name, true),
            },
        }
    }
}

/// A journaled DB change (undoable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    AliasAdded { canonical: String, alias: String },
    EntryAdded { canonical: String },
}

/// Apply a choice. Never inserts empty/`Unknown` names or anything whose
/// normalized form already exists; never edits hand-written aliases.
pub fn learn(db: &mut NameDb, choice: &Choice, today: NaiveDate) -> Option<Change> {
    learn_checked(db, choice, today).ok().flatten()
}

/// [`learn`] for replayed (saved) choices: `Ok(None)` = nothing to do
/// (already present, empty, `Unknown`); an alias whose canonical entry no
/// longer exists is an [`Error::Conflict`] instead of a silent no-op.
pub fn learn_checked(db: &mut NameDb, choice: &Choice, today: NaiveDate) -> Result<Option<Change>> {
    let (name, target) = match choice {
        Choice::Alias { canonical, alias } => (nfc(alias.trim()), Some(canonical)),
        Choice::New { name, .. } => (nfc(name.trim()), None),
    };
    let norm = normalize(db.kind(), &name);
    if norm.is_empty() || is_unknown(&name) || index(db).contains_key(&norm) {
        return Ok(None);
    }
    Ok(match (choice, target) {
        (Choice::Alias { .. }, Some(canonical)) => {
            let i = db.find(canonical).ok_or_else(|| Error::Conflict {
                message: format!(
                    "{} {canonical:?} is no longer in the {} name list; alias {name:?} not learned",
                    db.noun(),
                    db.field
                ),
            })?;
            db.entries[i].learned.push(name.clone());
            Some(Change::AliasAdded {
                canonical: canonical.clone(),
                alias: name,
            })
        }
        (Choice::New { review, .. }, _) => {
            db.entries.push(Entry {
                canonical: name.clone(),
                aliases: Vec::new(),
                learned: Vec::new(),
                source: Source::Auto,
                added: Some(today.format("%Y-%m-%d").to_string()),
                review: *review,
                evidence: None,
            });
            Some(Change::EntryAdded { canonical: name })
        }
        _ => None,
    })
}

/// Revert a change. Only learned aliases and `source: auto` entries are
/// removed. `Ok(false)` = already gone (nothing to do). An auto entry that
/// gained aliases since (later learning or manual edits) is a
/// [`Error::Conflict`]: undo never deletes subsequent changes.
pub fn unlearn(db: &mut NameDb, change: &Change) -> Result<Unlearned> {
    Ok(match change {
        // The alias string is its identity (normalized names are unique in
        // the DB): remove it wherever it lives now, e.g. after a merge or
        // rename moved it to another entry.
        Change::AliasAdded { alias, .. } => {
            match db.entries.iter_mut().find_map(|e| {
                let p = e.learned.iter().position(|a| a == alias)?;
                Some((e, p))
            }) {
                Some((e, p)) => {
                    e.learned.remove(p);
                    Unlearned::Reverted
                }
                None => match db.entries.iter().find(|e| e.aliases.contains(alias)) {
                    Some(e) => Unlearned::Kept(format!(
                        "made a hand-written alias of {:?} by user",
                        e.canonical
                    )),
                    None => Unlearned::Gone,
                },
            }
        }
        Change::EntryAdded { canonical } => match db.find(canonical) {
            Some(i) if db.entries[i].source == Source::User => {
                Unlearned::Kept("accepted by user".into())
            }
            None => {
                let kind = db.kind();
                let norm = normalize(kind, canonical);
                if let Some(e) = db.entries.iter().find(|e| e.aliases.contains(canonical)) {
                    Unlearned::Kept(format!("merged into {:?} by user", e.canonical))
                } else if let Some(e) = db.entries.iter().find(|e| {
                    e.source == Source::User
                        && !norm.is_empty()
                        && normalize(kind, &e.canonical) == norm
                }) {
                    // `names rename` to a spelling with the same normalized form.
                    Unlearned::Kept(format!("renamed by user (now {:?})", e.canonical))
                } else {
                    Unlearned::Gone
                }
            }
            Some(i) => {
                let e = &db.entries[i];
                if !e.aliases.is_empty() || !e.learned.is_empty() {
                    return Err(Error::Conflict {
                        message: format!(
                            "{} entry {canonical:?} has aliases added after it was learned; not removed",
                            db.noun()
                        ),
                    });
                }
                db.entries.remove(i);
                Unlearned::Reverted
            }
        },
    })
}

/// Outcome of [`unlearn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unlearned {
    Reverted,
    /// Already removed (e.g. by hand); nothing to do.
    Gone,
    /// A human decision since made it permanent (accepted, merged, renamed).
    Kept(String),
}

// ---------------------------------------------------------------------------
// Manual maintenance (`names add|merge|rename|review`)
// ---------------------------------------------------------------------------

fn missing(db: &NameDb, name: &str) -> Error {
    Error::Usage {
        message: format!("no {} entry named {name:?}", db.noun()),
        hint: Some(format!("List entries with `{}`.", db.cmd("list"))),
    }
}

fn owner_of(db: &NameDb, name: &str) -> Option<usize> {
    index(db).get(&normalize(db.kind(), name)).copied()
}

/// Add a hand-written entry, or hand-written aliases to an existing entry.
/// Fails when an alias already belongs to a different entry.
pub fn add_user(db: &mut NameDb, canonical: &str, aliases: &[String]) -> Result<()> {
    let kind = db.kind();
    let canonical = nfc(canonical.trim());
    if normalize(kind, &canonical).is_empty() || is_unknown(&canonical) {
        return Err(Error::Usage {
            message: format!("invalid {} name {canonical:?}", db.noun()),
            hint: None,
        });
    }
    let i = match db.find(&canonical) {
        Some(i) => i,
        None => {
            if let Some(j) = owner_of(db, &canonical) {
                return Err(Error::Conflict {
                    message: format!(
                        "{canonical:?} already matches entry {:?}",
                        db.entries[j].canonical
                    ),
                });
            }
            db.entries.push(Entry {
                canonical: canonical.clone(),
                aliases: Vec::new(),
                learned: Vec::new(),
                source: Source::User,
                added: None,
                review: false,
                evidence: None,
            });
            db.entries.len() - 1
        }
    };
    for alias in aliases {
        let alias = nfc(alias.trim());
        if normalize(kind, &alias).is_empty() {
            continue;
        }
        match owner_of(db, &alias) {
            // A learned alias the user now writes by hand: it becomes theirs.
            Some(j) if j == i => {
                let e = &mut db.entries[i];
                if let Some(p) = e
                    .learned
                    .iter()
                    .position(|l| normalize(kind, l) == normalize(kind, &alias))
                {
                    let learned = e.learned.remove(p);
                    if !e.aliases.contains(&learned) {
                        e.aliases.push(learned);
                    }
                }
            }
            Some(j) => {
                return Err(Error::Conflict {
                    message: format!(
                        "alias {alias:?} already belongs to {:?}",
                        db.entries[j].canonical
                    ),
                });
            }
            None => db.entries[i].aliases.push(alias),
        }
    }
    // Manually maintained entries are user-owned: undo never deletes them.
    db.entries[i].source = Source::User;
    Ok(())
}

/// Fold entry `from` into `into`: its canonical name and hand-written aliases
/// become aliases, its learned aliases stay learned. `into` becomes a user entry.
pub fn merge(db: &mut NameDb, from: &str, into: &str) -> Result<()> {
    let fi = db.find(from).ok_or_else(|| missing(db, from))?;
    let Some(ti) = db.find(into) else {
        return Err(Error::Usage {
            message: format!("no {} entry named {into:?}", db.noun()),
            hint: Some(format!(
                "To change the spelling of {from:?} instead, run `{}`.",
                db.cmd(&format!("rename {from:?} {into:?}"))
            )),
        });
    };
    if ti == fi {
        return Err(Error::Usage {
            message: "cannot merge an entry into itself".into(),
            hint: None,
        });
    }
    let src = db.entries.remove(fi);
    let ti = db.find(into).expect("checked above");
    let target = &mut db.entries[ti];
    for a in std::iter::once(src.canonical).chain(src.aliases) {
        if !target.aliases.contains(&a) {
            target.aliases.push(a);
        }
    }
    for a in src.learned {
        if !target.learned.contains(&a) {
            target.learned.push(a);
        }
    }
    target.source = Source::User;
    target.review = false;
    target.evidence = None;
    Ok(())
}

/// Change an entry's canonical spelling (`names rename`). The old
/// spelling stays as a hand-written alias; the entry becomes user-owned.
/// Refuses when NEW already belongs to another entry (use [`merge`]).
pub fn rename(db: &mut NameDb, old: &str, new: &str) -> Result<()> {
    let kind = db.kind();
    let new = nfc(new.trim());
    let i = db.find(old).ok_or_else(|| missing(db, old))?;
    if normalize(kind, &new).is_empty() || is_unknown(&new) {
        return Err(Error::Usage {
            message: format!("invalid {} name {new:?}", db.noun()),
            hint: None,
        });
    }
    if let Some(j) = owner_of(db, &new).filter(|&j| j != i) {
        return Err(Error::Usage {
            message: format!(
                "{new:?} already belongs to entry {:?}",
                db.entries[j].canonical
            ),
            hint: Some(format!(
                "Merge instead: `{}`.",
                db.cmd(&format!("merge {old:?} {:?}", db.entries[j].canonical))
            )),
        });
    }
    let e = &mut db.entries[i];
    if e.canonical != new {
        let old_name = std::mem::replace(&mut e.canonical, new);
        if normalize(kind, &old_name) != normalize(kind, &e.canonical)
            && !e.aliases.contains(&old_name)
        {
            e.aliases.push(old_name);
        }
    }
    e.source = Source::User;
    Ok(())
}

/// Clear (or set) the `review` flag of an entry.
pub fn set_review(db: &mut NameDb, canonical: &str, review: bool) -> Result<()> {
    let i = db.find(canonical).ok_or_else(|| missing(db, canonical))?;
    let e = &mut db.entries[i];
    e.review = review;
    if !review {
        // Accepting is a human decision: the entry is now user-owned and
        // undo of the batch that learned it keeps it.
        e.evidence = None;
        e.source = Source::User;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: &str = COMPANY_FIELD;
    const NAMES: &str = "# my comment\nACME:\n  aliases: [ACME Corporation, Acme Corp, ACME Inc]\nGlobex:\n  aliases: [Globex Corporation, Globex Corp]\n";

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()
    }

    #[test]
    fn read_only_lookup() {
        let db = parse_db(NAMES, F).unwrap();
        let table = [
            ("ACME Corporation", Some("ACME")),
            ("Acme Corp.", Some("ACME")),
            ("  ACME Corporation  ", Some("ACME")),
            ("acme corporation", Some("ACME")),
            ("Globex Corp", Some("Globex")),
            // Typo survives legal-suffix stripping → prefix-capped match:
            // never mapped without a decider (review entry instead).
            ("Globex Corporatoin", None),
            ("Completely Different Company", None),
        ];
        for (input, expected) in table {
            assert_eq!(lookup(&db, input).as_deref(), expected, "{input}");
        }
        assert_eq!(lookup(&NameDb::empty(F), "ACME"), None);
    }

    #[test]
    fn normalization_table() {
        let table = [
            ("ACME GmbH", "acme"),
            ("ACME GmbH & Co. KG", "acme"),
            ("Acme Corp.", "acme"),
            ("Müller e.U.", "müller"),
            ("Straße AG", "strasse"),
            ("ＡＣＭＥ Ltd", "acme"),
            ("  O'Reilly   Media, Inc. ", "oreilly media"),
            ("GmbH", "gmbh"),
            ("...", ""),
            ("Société Générale SA", "société générale"),
        ];
        for (input, expected) in table {
            assert_eq!(normalize_company(input), expected, "{input}");
        }
    }

    #[test]
    fn legal_form_stripped_as_written() {
        let table = [
            ("ACME Corporation GmbH", "ACME"),
            ("Müller GmbH & Co. KG", "Müller"),
            ("ACME, Inc.", "ACME"),
            ("Müller e.U.", "Müller"),
            ("Société Générale SA", "Société Générale"),
            ("Wayne Enterprises", "Wayne Enterprises"),
            ("GmbH", "GmbH"),
            ("  O'Reilly   Media, Inc. ", "O'Reilly Media"),
            ("ACME,Inc.", "ACME"),
            ("ACME-GmbH", "ACME"),
            ("A.C.M.E. GmbH", "A.C.M.E."),
            ("Foo (Europe) Ltd", "Foo (Europe)"),
            ("ACME (GmbH)", "ACME"),
            ("ACME [Inc.]", "ACME"),
            ("ACME，Inc.", "ACME"),
            ("Foo (Europe) (Ltd)", "Foo (Europe)"),
            ("Cafe\u{301} GmbH", "Cafe\u{301}"),
        ];
        for (input, expected) in table {
            assert_eq!(strip_legal_form(input), expected, "{input}");
            assert_eq!(
                normalize_company(&strip_legal_form(input)),
                normalize_company(input),
                "{input}"
            );
        }
    }

    #[test]
    fn decision_bands_on_fixtures() {
        let db = parse_db(
            "ACME: {aliases: [ACME Corporation]}\nGlobex: {aliases: [Globex Corp]}\nMustermann Bau: {}\nDeutsche Bank: {}\nBank: {}\nMueller: {}\nMicrosoft: {aliases: [MSFT]}\n",
            F,
        )
        .unwrap();
        let known = |name: &str, c: &str| {
            assert_eq!(
                decide(&db, name, None),
                Decision::Known {
                    canonical: c.into()
                },
                "{name}"
            )
        };
        known("ACME GmbH", "ACME");
        known("acme corporation inc.", "ACME");
        known("GLOBEX CORP.", "Globex");
        known("Deutsche Bank AG", "Deutsche Bank");

        let kind = |name: &str| match decide(&db, name, None) {
            Decision::AddAlias { canonical, .. } => format!("alias:{canonical}"),
            Decision::AskDecider { candidates, .. } => format!("ask:{}", candidates[0].canonical),
            Decision::NewEntry { .. } => "new".into(),
            other => format!("{other:?}"),
        };
        let table = [
            ("Musterman Bau", "alias:Mustermann Bau"),  // typo
            ("Bau Mustermann", "alias:Mustermann Bau"), // reordered tokens
            ("Microsfot", "alias:Microsoft"),           // transposition typo
            ("Globus", "ask:Globex"),
            ("Bank Austria", "ask:Bank"),   // token subset → never auto
            ("Mueller Bau", "ask:Mueller"), // prefix extension → never auto
            ("Deutsche Telekom", "ask:Deutsche Bank"),
            ("Initech", "new"),
            ("Wayne Enterprises", "new"),
        ];
        for (name, expected) in table {
            assert_eq!(kind(name), expected, "{name}");
        }
        assert_eq!(
            decide(&db, "", None),
            Decision::Skip {
                reason: SkipReason::Empty
            }
        );
        assert_eq!(
            decide(&db, "unknown", None),
            Decision::Skip {
                reason: SkipReason::Unknown
            }
        );
        assert_eq!(
            decide(&db, "Initech", Some(0.2)),
            Decision::Skip {
                reason: SkipReason::LowConfidence
            }
        );
    }

    #[test]
    fn learn_rules_and_undo() {
        let mut db = parse_db(NAMES, F).unwrap();
        let before = db.clone();
        // Alias on a hand-written entry goes to `learned`, aliases untouched.
        let c = learn(
            &mut db,
            &Choice::Alias {
                canonical: "ACME".into(),
                alias: "ACME Holding".into(),
            },
            today(),
        )
        .unwrap();
        assert_eq!(
            c,
            Change::AliasAdded {
                canonical: "ACME".into(),
                alias: "ACME Holding".into()
            }
        );
        assert_eq!(db.entries[0].aliases, before.entries[0].aliases);
        assert_eq!(db.entries[0].learned, vec!["ACME Holding"]);
        // Never duplicate (normalized form exists), never clutter.
        assert_eq!(
            learn(
                &mut db,
                &Choice::Alias {
                    canonical: "ACME".into(),
                    alias: "acme holding gmbh".into()
                },
                today()
            ),
            None
        );
        assert_eq!(
            learn(
                &mut db,
                &Choice::New {
                    name: "Globex Corporation Ltd".into(),
                    review: false
                },
                today()
            ),
            None
        );
        assert_eq!(
            learn(
                &mut db,
                &Choice::New {
                    name: "Unknown".into(),
                    review: false
                },
                today()
            ),
            None
        );
        assert_eq!(
            learn(
                &mut db,
                &Choice::New {
                    name: "  ".into(),
                    review: false
                },
                today()
            ),
            None
        );
        assert_eq!(
            learn(
                &mut db,
                &Choice::Alias {
                    canonical: "Nope".into(),
                    alias: "Zed".into()
                },
                today()
            ),
            None
        );
        let e = learn(
            &mut db,
            &Choice::New {
                name: "Initech".into(),
                review: true,
            },
            today(),
        )
        .unwrap();
        let last = db.entries.last().unwrap();
        assert_eq!(
            (last.source, last.added.as_deref(), last.review),
            (Source::Auto, Some("2026-11-07"), true)
        );
        // Undo restores the original DB exactly (modulo format flag).
        assert_eq!(unlearn(&mut db, &e).unwrap(), Unlearned::Reverted);
        assert_eq!(unlearn(&mut db, &c).unwrap(), Unlearned::Reverted);
        assert_ne!(unlearn(&mut db, &c).unwrap(), Unlearned::Reverted);
        assert_eq!(db.entries, before.entries);
        // Hand-written entries are never removed by unlearn.
        assert_ne!(
            unlearn(
                &mut db,
                &Change::EntryAdded {
                    canonical: "ACME".into()
                }
            )
            .unwrap(),
            Unlearned::Reverted
        );
    }

    #[test]
    fn undo_never_deletes_later_manual_or_learned_aliases() {
        let mut db = NameDb::empty(F);
        let added = learn(
            &mut db,
            &Choice::New {
                name: "Initech".into(),
                review: false,
            },
            today(),
        )
        .unwrap();
        // A later learned alias on the auto entry blocks removing it…
        let alias = learn(
            &mut db,
            &Choice::Alias {
                canonical: "Initech".into(),
                alias: "Initech Holding".into(),
            },
            today(),
        )
        .unwrap();
        assert_eq!(unlearn(&mut db, &added).unwrap_err().code(), "conflict");
        // …until that later change is undone first (newest-first undo).
        assert_eq!(unlearn(&mut db, &alias).unwrap(), Unlearned::Reverted);
        assert_eq!(unlearn(&mut db, &added).unwrap(), Unlearned::Reverted);
        // A manual alias promotes the entry to user ownership: undo keeps it.
        let added = learn(
            &mut db,
            &Choice::New {
                name: "ACME".into(),
                review: false,
            },
            today(),
        )
        .unwrap();
        add_user(&mut db, "ACME", &["Handwritten alias".into()]).unwrap();
        assert_eq!(db.entries[0].source, Source::User);
        assert_ne!(unlearn(&mut db, &added).unwrap(), Unlearned::Reverted);
        assert_eq!(db.entries[0].aliases, vec!["Handwritten alias"]);
    }

    #[test]
    fn concurrent_learning_never_loses_names() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("names.yaml");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let file = file.clone();
                std::thread::spawn(move || {
                    for i in 0..5 {
                        let choice = Choice::New {
                            name: format!("Firm{t}x{i} Unique{}", t * 10 + i),
                            review: false,
                        };
                        update_db(&file, F, |db| Ok(learn(db, &choice, today()))).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(load_db(&file, F).unwrap().entries.len(), 40);
    }

    #[test]
    fn decider_off_flags_only_the_ambiguous_band() {
        let db = parse_db("Microsoft: {}\n", F).unwrap();
        for raw in ["Unrelated ZZZ", "Initech"] {
            let d = decide(&db, raw, None);
            assert_eq!(
                choice_for(&d, None),
                Some(Choice::New {
                    name: raw.into(),
                    review: false
                }),
                "{raw}: clearly new → plain entry"
            );
        }
        let none = decide(&NameDb::empty(F), "Initech", None);
        assert_eq!(
            choice_for(&none, None),
            Some(Choice::New {
                name: "Initech".into(),
                review: false
            })
        );
    }

    #[test]
    fn stale_alias_choice_is_a_conflict() {
        let mut db = parse_db("Globex: {}\n", F).unwrap();
        let stale = Choice::Alias {
            canonical: "ACME".into(),
            alias: "ACMEE".into(),
        };
        assert_eq!(
            learn_checked(&mut db, &stale, today()).unwrap_err().code(),
            "conflict"
        );
        assert_eq!(learn(&mut db, &stale, today()), None);
        assert_eq!(db.entries.len(), 1);
    }

    #[test]
    fn learned_aliases_follow_merge_and_rename_for_undo() {
        let mut db = parse_db("ACME Corporation: {aliases: [ACME Corp]}\n", F).unwrap();
        let new = learn(
            &mut db,
            &Choice::New {
                name: "Acme Holding".into(),
                review: true,
            },
            today(),
        )
        .unwrap();
        let alias = learn(
            &mut db,
            &Choice::Alias {
                canonical: "Acme Holding".into(),
                alias: "ACME Hldg".into(),
            },
            today(),
        )
        .unwrap();
        // The user merges the learned entry into the hand-written one …
        merge(&mut db, "Acme Holding", "ACME Corporation").unwrap();
        assert!(db.entries[0].learned.contains(&"ACME Hldg".to_string()));
        // … and renames it: undo still finds the learned alias.
        rename(&mut db, "ACME Corporation", "ACME").unwrap();
        assert_eq!(unlearn(&mut db, &alias).unwrap(), Unlearned::Reverted);
        assert!(db.entries[0].learned.is_empty());
        // The merged entry itself is a human decision: kept.
        assert!(matches!(
            unlearn(&mut db, &new).unwrap(),
            Unlearned::Kept(_)
        ));
        assert_eq!(db.entries[0].canonical, "ACME");
        assert_eq!(lookup(&db, "ACME Corporation").as_deref(), Some("ACME"));
    }

    #[test]
    fn undo_keeps_an_entry_the_user_renamed() {
        let mut db = NameDb::empty(F);
        let added = learn(
            &mut db,
            &Choice::New {
                name: "ACME Corporation GmbH".into(),
                review: false,
            },
            today(),
        )
        .unwrap();
        rename(&mut db, "ACME Corporation GmbH", "ACME").unwrap();
        assert_eq!(db.entries[0].source, Source::User);
        assert_eq!(
            unlearn(&mut db, &added).unwrap(),
            Unlearned::Kept("renamed by user (now \"ACME\")".into())
        );
        assert_eq!(db.entries[0].canonical, "ACME");
    }

    #[test]
    fn rename_refuses_existing_names_and_merge_hints_rename() {
        let mut db = parse_db("ACME Corporation: {}\nGlobex: {}\n", F).unwrap();
        let e = rename(&mut db, "ACME Corporation", "Globex").unwrap_err();
        assert!(
            e.hint()
                .unwrap()
                .contains("names --field company_name merge")
        );
        let e = merge(&mut db, "ACME Corporation", "ACME").unwrap_err();
        assert!(
            e.hint()
                .unwrap()
                .contains("names --field company_name rename")
        );
        rename(&mut db, "ACME Corporation", "ACME").unwrap();
        assert_eq!(lookup(&db, "ACME Corporation").as_deref(), Some("ACME"));
        assert_eq!(db.entries[0].source, Source::User);
    }

    #[test]
    fn review_evidence_round_trips_and_accept_makes_entries_permanent() {
        let mut db = NameDb::empty(F);
        let added = learn(
            &mut db,
            &Choice::New {
                name: "Bank".into(),
                review: true,
            },
            today(),
        )
        .unwrap();
        db.entries[0].evidence = Some(ReviewEvidence {
            reason: "ambiguous match (0.75-0.92) without a decider verdict".into(),
            candidates: vec![ReviewCandidate {
                name: "Bank Austria".into(),
                score: 0.91,
            }],
            source: "scan.pdf".into(),
            batch: "20261107T101112-abcdef".into(),
        });
        let text = render_db(&db);
        assert!(
            text.contains("  review:\n    reason: \"ambiguous match (0.75-0.92) without a decider verdict\"\n    candidates:\n      - name: \"Bank Austria\"\n        score: 0.91\n    source: \"scan.pdf\"\n"),
            "{text}"
        );
        let back = parse_db(&text, F).unwrap();
        assert_eq!(back.entries, db.entries);
        assert!(parse_db("A: {review: true}\n", F).unwrap().entries[0].review);
        // Accepting = human decision: user-owned, evidence cleared, kept by undo.
        set_review(&mut db, "Bank", false).unwrap();
        assert_eq!(db.entries[0].source, Source::User);
        assert!(db.entries[0].evidence.is_none());
        assert_eq!(
            unlearn(&mut db, &added).unwrap(),
            Unlearned::Kept("accepted by user".into())
        );
        assert_eq!(db.entries.len(), 1);
    }

    #[test]
    fn adding_a_learned_alias_by_hand_makes_it_the_users() {
        let mut db = parse_db("ACME: {}\n", F).unwrap();
        let change = learn(
            &mut db,
            &Choice::Alias {
                canonical: "ACME".into(),
                alias: "Acme Hldg".into(),
            },
            today(),
        )
        .unwrap();
        add_user(&mut db, "ACME", &["Acme Hldg".into()]).unwrap();
        assert!(db.entries[0].learned.is_empty());
        assert_eq!(db.entries[0].aliases, vec!["Acme Hldg"]);
        assert_eq!(db.entries[0].source, Source::User);
        assert!(matches!(
            unlearn(&mut db, &change).unwrap(),
            Unlearned::Kept(_)
        ));
        assert_eq!(db.entries[0].aliases, vec!["Acme Hldg"]);
    }

    #[test]
    fn full_unicode_casefold() {
        assert_eq!(normalize_company("ΟΣ"), normalize_company("οσ"));
        assert_eq!(normalize_company("ΟΣ"), normalize_company("ος"));
        assert_eq!(normalize_company("Straße"), normalize_company("STRASSE"));
        assert_eq!(normalize_company("ﬁrma"), "firma");
        let db = parse_db("ΟΣ: {}\n", F).unwrap();
        assert_eq!(
            decide(&db, "οσ", None),
            Decision::Known {
                canonical: "ΟΣ".into()
            }
        );
    }

    #[test]
    fn decider_off_never_maps_ambiguous_matches() {
        let db = parse_db("Bank Austria: {}\n", F).unwrap();
        let d = decide(&db, "Bank", None);
        let Decision::AskDecider { candidates, .. } = &d else {
            panic!("{d:?}");
        };
        assert!(candidates[0].score > 0.85);
        assert_eq!(mapped(&d), None);
        assert_eq!(lookup(&db, "Bank"), None);
        assert_eq!(
            choice_for(&d, None),
            Some(Choice::New {
                name: "Bank".into(),
                review: true
            })
        );
    }

    fn choice_for(d: &Decision, v: Option<&Verdict>) -> Option<Choice> {
        d.resolve("", v).learn
    }

    fn mapped(d: &Decision) -> Option<String> {
        let r = d.resolve("", None);
        r.mapped.then_some(r.name)
    }

    #[test]
    fn choice_mapping() {
        let ask = Decision::AskDecider {
            name: "Microsfot".into(),
            candidates: vec![Candidate {
                canonical: "Microsoft".into(),
                matched: "Microsoft".into(),
                score: 0.9,
            }],
        };
        assert_eq!(
            choice_for(&ask, None),
            Some(Choice::New {
                name: "Microsfot".into(),
                review: true
            }),
            "decider off: ambiguous band never maps silently"
        );
        assert_eq!(mapped(&ask), None);
        let weak = Decision::AskDecider {
            name: "Microsfot".into(),
            candidates: vec![Candidate {
                canonical: "Microsoft".into(),
                matched: "Microsoft".into(),
                score: 0.8,
            }],
        };
        assert_eq!(
            choice_for(&weak, None),
            Some(Choice::New {
                name: "Microsfot".into(),
                review: true
            })
        );
        assert_eq!(mapped(&weak), None);
        assert_eq!(
            choice_for(&Decision::NewEntry { name: "X".into() }, None),
            Some(Choice::New {
                name: "X".into(),
                review: false
            })
        );
        assert_eq!(
            choice_for(&ask, Some(&Verdict::Existing("Microsoft".into()))),
            Some(Choice::Alias {
                canonical: "Microsoft".into(),
                alias: "Microsfot".into()
            })
        );
        assert_eq!(
            choice_for(&ask, Some(&Verdict::Existing("Other".into()))),
            Some(Choice::New {
                name: "Microsfot".into(),
                review: true
            })
        );
        assert_eq!(
            choice_for(&ask, Some(&Verdict::New)),
            Some(Choice::New {
                name: "Microsfot".into(),
                review: false
            })
        );
        assert_eq!(
            choice_for(
                &Decision::Known {
                    canonical: "A".into()
                },
                None
            ),
            None
        );
    }

    #[test]
    fn roundtrip_and_strict_shape() {
        let mut db = parse_db(NAMES, F).unwrap();
        learn(
            &mut db,
            &Choice::New {
                name: "Café \"Zum\" Wien".into(),
                review: true,
            },
            today(),
        )
        .unwrap();
        let text = render_db(&db);
        assert!(text.starts_with("# AutoRename-PDF name list"));
        let back = parse_db(&text, F).unwrap();
        assert_eq!(back.entries, db.entries);
        // Strict shape: flat lists, unknown keys and non-mappings are rejected.
        assert!(parse_db("A: [x]\n", F).is_err());
        assert!(parse_db("A: {aliases: [x], typo: 1}\n", F).is_err());
        assert!(parse_db("- a\n- b\n", F).is_err());
        assert!(parse_db("A: {}\nA: {}\n", F).is_err());
        assert!(parse_db(": invalid: yaml: {{{}}", F).is_err());
        assert!(parse_db("", F).unwrap().entries.is_empty());
        assert!(
            parse_db("A: {}\n", F).unwrap().entries[0]
                .aliases
                .is_empty()
        );
    }

    #[test]
    fn token_set_matches_rapidfuzz() {
        assert_eq!(
            token_set_ratio("fuzzy was a bear", "fuzzy fuzzy was a bear"),
            1.0
        );
        assert_eq!(token_set_ratio("a b", "b a"), 1.0);
        assert!(token_set_ratio("deutsche bank", "deutsche telekom") < 0.8);
        assert_eq!(token_set_ratio("abc", "xyz"), 0.0);
    }

    #[test]
    fn manual_maintenance() {
        let mut db = parse_db(NAMES, F).unwrap();
        add_user(&mut db, "Initech", &["Initech LLC".into()]).unwrap();
        assert_eq!(lookup(&db, "initech llc").as_deref(), Some("Initech"));
        assert!(add_user(&mut db, "Initech", &["Acme Corp".into()]).is_err());
        add_user(&mut db, "ACME", &["Acme Holdings".into()]).unwrap();
        assert!(db.entries[0].aliases.contains(&"Acme Holdings".to_string()));
        merge(&mut db, "Globex", "ACME").unwrap();
        assert!(db.find("Globex").is_none());
        assert_eq!(lookup(&db, "Globex Corp").as_deref(), Some("ACME"));
        assert!(merge(&mut db, "ACME", "ACME").is_err());
        assert!(merge(&mut db, "Nope", "ACME").is_err());
        set_review(&mut db, "ACME", true).unwrap();
        assert!(db.entries[0].review);
        assert!(set_review(&mut db, "Nope", false).is_err());
    }

    #[test]
    fn generic_lists_keep_every_word_and_inner_punctuation() {
        let g = |s: &str| normalize(NameKind::Generic, s);
        assert_eq!(g("  \u{201c}Nature  Physics.\u{201d} "), "nature physics");
        assert_eq!(g("Nature Physics Ltd"), "nature physics ltd");
        assert_eq!(g("J. Chem. Phys."), "j. chem. phys");
        assert_eq!(g("C++ Report"), "c++ report");
        assert_eq!(g("..."), "");
        // A legal-form word is part of a generic name: `Acme Inc` is not
        // `Acme` (it is for companies).
        let venues = parse_db("Acme: {}\n", "journal_name").unwrap();
        assert_eq!(venues.kind(), NameKind::Generic);
        assert!(!matches!(
            decide(&venues, "Acme Inc", None),
            Decision::Known { .. }
        ));
        assert_eq!(lookup(&venues, "acme.").as_deref(), Some("Acme"));
        let companies = parse_db("Acme: {}\n", F).unwrap();
        assert_eq!(lookup(&companies, "Acme Inc").as_deref(), Some("Acme"));
        // Uniqueness follows the kind too.
        let mut v = venues.clone();
        add_user(&mut v, "Acme", &["Acme Inc".into()]).unwrap();
        assert_eq!(v.entries[0].aliases, vec!["Acme Inc"]);
        let mut c = companies.clone();
        add_user(&mut c, "Acme", &["Acme Inc".into()]).unwrap();
        assert!(c.entries[0].aliases.is_empty(), "redundant for companies");
    }

    #[test]
    fn field_labels() {
        let l = |f: &str| {
            let (s, p) = field_label(f);
            format!("{s}/{p}")
        };
        assert_eq!(l("company_name"), "Company/Companies");
        assert_eq!(l("journal_name"), "Venue/Venues");
        assert_eq!(l("first_author_surname"), "Author/Authors");
        assert_eq!(l("issuer_name"), "Issuer name/Issuer names");
        assert_eq!(l("vendor"), "Vendor/Vendor names");
        assert_eq!(NameKind::for_field("company_name"), NameKind::Company);
        assert_eq!(NameKind::for_field("issuer"), NameKind::Generic);
    }
}
