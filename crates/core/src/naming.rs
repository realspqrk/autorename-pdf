//! Filename rendering (contract §3.5) and collision-safe rename planning (§7.1).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::dates::{format_date, parse_date};
use crate::fsx;
use crate::profiles::{Profile, Token, template_tokens};
use crate::textutil::nfc;
use crate::{Error, Result};

pub const UNKNOWN_VALUE: &str = "Unknown";
pub const DEFAULT_DATE: &str = "00000000";
/// Maximum basename length (before `.pdf` and `_(n)`), in UTF-8 bytes and
/// in UTF-16 units alike (Linux/macOS count bytes, Windows UTF-16 units).
pub const MAX_BASE: usize = 244;
/// Per-component limit of every supported filesystem (bytes / UTF-16 units).
pub const MAX_COMPONENT: usize = 255;

/// `s` fits `max` UTF-8 bytes and `max` UTF-16 units.
pub fn fits(s: &str, max: usize) -> bool {
    s.len() <= max && s.encode_utf16().count() <= max
}

/// Longest prefix of `s` (at a char boundary) that [`fits`] `max`.
pub fn truncate_units(s: &str, max: usize) -> &str {
    let (mut bytes, mut units) = (0, 0);
    for (i, c) in s.char_indices() {
        bytes += c.len_utf8();
        units += c.len_utf16();
        if bytes > max || units > max {
            return &s[..i];
        }
    }
    s
}

const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// Windows reserved device name (`CON`, `NUL`, `COM1`, `LPT¹`, …) as the part
/// before the first dot, case-insensitive; such names are unusable on Windows
/// even with an extension.
pub fn is_reserved_device(base: &str) -> bool {
    let head = base.split('.').next().unwrap_or("").trim_end_matches(' ');
    let upper = head.to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let rest: String = chars.collect();
    (prefix == "COM" || prefix == "LPT")
        && rest.chars().count() == 1
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '\u{b9}' | '\u{b2}' | '\u{b3}'))
}

/// Prefix reserved device names with `_` (`CON` → `_CON`).
fn avoid_reserved(base: String) -> String {
    if is_reserved_device(&base) {
        format!("_{base}")
    } else {
        base
    }
}

const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Drop control chars and Windows-forbidden chars; strip `.`/space at both ends.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !(('\u{0}'..='\u{1f}').contains(c) || *c == '\u{7f}' || FORBIDDEN.contains(c)))
        .collect();
    cleaned.trim_matches(['.', ' ']).to_string()
}

fn is_separator_run(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_whitespace() || c == '-' || c == '_')
}

fn collapse_separator_run(text: &str) -> &'static str {
    if text.contains('-') {
        if text.chars().any(char::is_whitespace) {
            " - "
        } else {
            "-"
        }
    } else if text.contains('_') {
        "_"
    } else {
        " "
    }
}

fn raw_str<'a>(fields: &'a Map<String, Value>, name: &str) -> &'a str {
    fields.get(name).and_then(Value::as_str).unwrap_or("")
}

/// Per-field display values: formatted date or `00000000`, sanitized text
/// (empty stays empty: the field and its separator are left out).
fn prepare_values(
    profile: &Profile,
    fields: &Map<String, Value>,
    date_format: &str,
) -> Vec<(String, String)> {
    profile
        .fields
        .keys()
        .map(|name| {
            let raw = raw_str(fields, name);
            let value = if name == "document_date" {
                match parse_date(raw) {
                    Some(d) => format_date(d, date_format),
                    None => DEFAULT_DATE.to_string(),
                }
            } else {
                let cleaned = sanitize_filename(&nfc(raw));
                if cleaned.trim().is_empty() {
                    String::new()
                } else {
                    cleaned
                }
            };
            (name.clone(), value)
        })
        .collect()
}

fn lookup<'a>(values: &'a [(String, String)], name: &str) -> &'a str {
    values
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

fn flush_pending(pieces: &mut String, pending: &mut String) {
    if !pending.is_empty() {
        if is_separator_run(pending) {
            pieces.push_str(collapse_separator_run(pending));
        } else {
            pieces.push_str(pending);
        }
        pending.clear();
    }
}

fn join_tokens(tokens: &[Token], values: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut pending = String::new();
    let mut started = false;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Field(name) => {
                let value = lookup(values, name);
                if value.is_empty() || repeated_next(tokens, values, i, value) {
                    continue;
                }
                if started {
                    flush_pending(&mut out, &mut pending);
                }
                out.push_str(value);
                started = true;
            }
            Token::Literal(lit) => {
                if !started {
                    if is_separator_run(lit) {
                        continue;
                    }
                    out.push_str(lit);
                    started = true;
                    pending.clear();
                } else if is_separator_run(lit) {
                    pending.push_str(lit);
                } else {
                    flush_pending(&mut out, &mut pending);
                    out.push_str(lit);
                }
            }
        }
    }
    let base = sanitize_filename(&nfc(&out));
    base.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The field value at token `i` is said again at the start of the next
/// non-empty field (`Invoice` + `Invoice 2024-11`, case-insensitive, whole
/// words) with only separators between them: that occurrence is left out
/// (never for the same field written twice).
fn repeated_next(tokens: &[Token], values: &[(String, String)], i: usize, value: &str) -> bool {
    let cur = value.to_lowercase();
    let Some(Token::Field(own)) = tokens.get(i) else {
        return false;
    };
    for t in &tokens[i + 1..] {
        match t {
            Token::Literal(l) if is_separator_run(l) => {}
            Token::Literal(_) => return false,
            // A field written twice on purpose (`{title} - {title}`) stays.
            Token::Field(n) if n == own => return false,
            Token::Field(n) => {
                let next = lookup(values, n);
                if next.is_empty() {
                    continue;
                }
                return next
                    .to_lowercase()
                    .strip_prefix(&cur)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '-', '_', ',']));
            }
        }
    }
    false
}

/// Render `<basename>.pdf` for raw model `fields`. Pure; does not mutate input.
pub fn render_filename(
    profile: &Profile,
    fields: &Map<String, Value>,
    date_format: &str,
) -> String {
    let tokens = template_tokens(&profile.template);
    let mut values = prepare_values(profile, fields, date_format);
    let truncate = profile.truncate_field.as_str();

    let mut base = join_tokens(&tokens, &values);
    while !fits(&base, MAX_BASE) {
        let Some(slot) = values.iter_mut().find(|(k, _)| k == truncate) else {
            break;
        };
        if slot.1.is_empty() {
            break;
        }
        slot.1.pop();
        base = join_tokens(&tokens, &values);
    }
    if !fits(&base, MAX_BASE) {
        base = truncate_units(&base, MAX_BASE)
            .trim_end_matches(['.', ' '])
            .to_string();
    }
    if base.is_empty() {
        base = UNKNOWN_VALUE.to_string();
    }
    nfc(&format!("{}.pdf", avoid_reserved(base)))
}

/// Check a single target file name (e.g. from a saved plan): one path
/// component, `.pdf` extension, no forbidden/control characters, no leading
/// or trailing dots/spaces, no reserved device name, within
/// [`MAX_COMPONENT`]. `Err` carries the reason.
pub fn validate_filename(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("empty file name".into());
    }
    if name.contains(['/', '\\']) {
        return Err("file name must not contain path separators".into());
    }
    if !name.to_lowercase().ends_with(".pdf") || name.len() <= 4 {
        return Err("file name must end in .pdf".into());
    }
    if sanitize_filename(name) != name {
        return Err("file name contains characters that are not allowed".into());
    }
    if is_reserved_device(name) {
        return Err("file name is a reserved device name".into());
    }
    if !fits(name, MAX_COMPONENT) {
        return Err(format!("file name is longer than {MAX_COMPONENT} bytes"));
    }
    Ok(())
}

/// Outcome of planning a rename target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Source already has the rendered name.
    Unchanged,
    /// Free target path (may carry a `_(n)` suffix).
    Path(PathBuf),
}

/// Choose the target path next to `source`, appending `_(n)` while `exists`.
/// The whole component (stem, `_(n)`, `.pdf`) is kept within
/// [`MAX_COMPONENT`] bytes and UTF-16 units; reserved device names get a `_`.
///
/// `reserved` lets a batch avoid handing out the same target twice
/// (dry-run/plan).
pub fn plan_target(
    source: &Path,
    new_name: &str,
    reserved: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Target {
    let mut name = nfc(new_name);
    if !name.to_lowercase().ends_with(".pdf") {
        name.push_str(".pdf");
    }
    let stem = name[..name.len() - 4]
        .trim_end_matches(['.', ' '])
        .to_string();
    let stem = avoid_reserved(if stem.is_empty() {
        UNKNOWN_VALUE.to_string()
    } else {
        stem
    });
    let file = |suffix: &str| {
        let room = MAX_COMPONENT - suffix.len() - ".pdf".len();
        let s = truncate_units(&stem, room).trim_end_matches(['.', ' ']);
        format!("{s}{suffix}.pdf")
    };
    let dir = source.parent().unwrap_or(Path::new(""));
    let source_nfc = PathBuf::from(nfc(&source.to_string_lossy()));
    let mut candidate = dir.join(file(""));
    if candidate == source_nfc {
        return Target::Unchanged;
    }
    // Re-runs are stable: a file already carrying this name plus a
    // collision suffix (`_(n)`) keeps it.
    let src_name = source
        .file_name()
        .map(|n| nfc(&n.to_string_lossy()))
        .unwrap_or_default();
    if let Some(n) = collision_number(src_name.strip_suffix(".pdf").unwrap_or(""))
        && src_name == file(&format!("_({n})"))
    {
        return Target::Unchanged;
    }
    let mut counter = 0u32;
    while exists(&candidate) || reserved.contains(&candidate) {
        counter += 1;
        candidate = dir.join(file(&format!("_({counter})")));
    }
    Target::Path(candidate)
}

/// `n` of a trailing `_(n)` collision suffix in a stem.
fn collision_number(stem: &str) -> Option<u32> {
    let inner = stem.strip_suffix(')')?;
    let (_, digits) = inner.rsplit_once("_(")?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// Stem without a trailing `_(n)` collision suffix (`ACME_(2)` → `ACME`).
pub fn strip_collision_suffix(stem: &str) -> &str {
    match collision_number(stem) {
        Some(_) => stem.rsplit_once("_(").map_or(stem, |(s, _)| s),
        None => stem,
    }
}

/// Rename `src` → `dst`; the OS refuses to replace any existing entry at
/// `dst` (see [`fsx::rename_noreplace`]). Retries on permission/busy errors
/// (Windows: file open in a viewer). `retries` attempts, `delay` apart. The
/// moved object must be the one present when the attempt started.
pub fn rename_file(
    src: &Path,
    dst: &Path,
    retries: u32,
    delay: Duration,
) -> Result<fsx::Committed> {
    rename_file_checked(src, dst, retries, delay, &|| {
        fsx::path_id(src).map_err(|e| Error::io(src, e))
    })
}

/// [`rename_file`] with a verification step: `check` runs before *every*
/// attempt and returns the identity of the verified source. If a different
/// object ends up at `dst` (swapped between check and rename), it is moved
/// back and the rename is a [`Error::Conflict`].
pub fn rename_file_checked(
    src: &Path,
    dst: &Path,
    retries: u32,
    delay: Duration,
    check: &dyn Fn() -> Result<fsx::FileId>,
) -> Result<fsx::Committed> {
    use std::io::ErrorKind;
    let busy = |k: ErrorKind| matches!(k, ErrorKind::PermissionDenied | ErrorKind::ResourceBusy);
    let retries = retries.max(1);
    for attempt in 1..=retries {
        let id = check()?;
        match fsx::rename_noreplace(src, dst) {
            Ok(done) => {
                if fsx::path_id(dst).ok() != Some(id) {
                    let _ = fsx::rename_noreplace(dst, src);
                    return Err(Error::Conflict {
                        message: format!(
                            "{} changed while it was being renamed; left in place",
                            src.display()
                        ),
                    });
                }
                return Ok(done);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                return Err(Error::Conflict {
                    message: format!("Target already exists: {}", dst.display()),
                });
            }
            Err(e) if busy(e.kind()) && attempt < retries => {
                std::thread::sleep(delay);
            }
            Err(e) if busy(e.kind()) => {
                return Err(Error::General(format!(
                    "Cannot rename: file is in use after {retries} attempts. Close the file and try again: {}",
                    src.display()
                )));
            }
            Err(e) => return Err(Error::io(src, e)),
        }
    }
    unreachable!("loop returns on last attempt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_table() {
        let table = [
            ("Invalid<>Name", "InvalidName"),
            ("<>:\"/\\|?*", ""),
            ("  .name. ", "name"),
            ("a\u{0}b\u{1f}c\u{7f}", "abc"),
            ("Café", "Café"),
            ("Full title: subtitle", "Full title subtitle"),
        ];
        for (input, expected) in table {
            assert_eq!(sanitize_filename(input), expected, "{input:?}");
        }
    }

    #[test]
    fn reserved_device_names() {
        for name in [
            "CON",
            "con",
            "Nul",
            "COM1",
            "lpt9",
            "COM0",
            "LPT\u{b9}",
            "COM\u{b3}",
            "CON.anything",
            "AUX .x",
            "CONIN$",
        ] {
            assert!(is_reserved_device(name), "{name}");
        }
        for name in ["CONSOLE", "COM10", "LPT", "ACON", "_CON", "COMX"] {
            assert!(!is_reserved_device(name), "{name}");
        }
        let p = |n: &str| plan_target(Path::new("/in/a.pdf"), n, &[], |_| false);
        assert_eq!(p("CON.pdf"), Target::Path("/in/_CON.pdf".into()));
        assert_eq!(
            p("lpt1.report.pdf"),
            Target::Path("/in/_lpt1.report.pdf".into())
        );
        assert_eq!(p("name. .pdf"), Target::Path("/in/name.pdf".into()));
    }

    #[test]
    fn component_budget_counts_bytes_utf16_and_suffix() {
        assert!(fits(&"a".repeat(255), 255) && !fits(&"é".repeat(128), 255));
        assert!(!fits(&"😀".repeat(64), 255), "4 bytes each");
        assert_eq!(truncate_units("aé😀", 3), "aé");
        assert_eq!(truncate_units("aé😀", 2), "a");
        // A long multibyte name plus a collision suffix stays within 255 bytes.
        let long = format!("{}.pdf", "é".repeat(200));
        let taken = |p: &Path| p.file_name().unwrap().to_str().unwrap().ends_with("é.pdf");
        let Target::Path(t) = plan_target(Path::new("/in/a.pdf"), &long, &[], taken) else {
            panic!()
        };
        let name = t.file_name().unwrap().to_str().unwrap();
        assert!(
            name.ends_with("_(1).pdf") && name.len() <= 255,
            "{}",
            name.len()
        );
    }

    #[test]
    fn rerun_keeps_collision_suffixed_names() {
        let taken = |_: &Path| true;
        assert_eq!(
            plan_target(Path::new("/in/ACME AP_(1).pdf"), "ACME AP.pdf", &[], taken),
            Target::Unchanged
        );
        assert_eq!(
            plan_target(Path::new("/in/ACME AP_(12).pdf"), "ACME AP", &[], taken),
            Target::Unchanged
        );
        // A different stem with a suffix is still renamed.
        assert_ne!(
            plan_target(Path::new("/in/Other_(1).pdf"), "ACME AP.pdf", &[], |_| {
                false
            }),
            Target::Unchanged
        );
        assert_eq!(strip_collision_suffix("ACME AP_(3)"), "ACME AP");
        assert_eq!(strip_collision_suffix("Report (v2)"), "Report (v2)");
        assert_eq!(strip_collision_suffix("x_()"), "x_()");
    }

    #[test]
    fn filename_validation() {
        for ok in ["a.pdf", "20260315 ACME AP 12,13.pdf", "x.PDF"] {
            assert!(validate_filename(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "..",
            ".pdf",
            "a.conf",
            "../a.pdf",
            "/abs/a.pdf",
            "a\\b.pdf",
            "CON.pdf",
            "a:b.pdf",
            " a.pdf",
            "a\u{0}.pdf",
        ] {
            assert!(validate_filename(bad).is_err(), "{bad:?}");
        }
        assert!(validate_filename(&format!("{}.pdf", "a".repeat(252))).is_err());
    }

    #[test]
    fn every_retry_reverifies_the_source() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a.pdf"), d.path().join("b.pdf"));
        std::fs::write(&src, b"x").unwrap();
        let calls = std::cell::Cell::new(0);
        // First check passes, then the "file" is found changed on the retry.
        let check = || -> Result<fsx::FileId> {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Err(Error::Conflict {
                    message: "changed".into(),
                })
            } else {
                fsx::path_id(&src).map_err(|e| Error::io(&src, e))
            }
        };
        assert_eq!(
            rename_file_checked(&src, &dst, 3, Duration::ZERO, &check)
                .unwrap_err()
                .code(),
            "conflict"
        );
        assert!(src.exists() && !dst.exists());
        // A stale identity (object changed after the check) is moved back.
        let stale = fsx::path_id(&src).unwrap();
        std::fs::write(&src, b"newer bytes").unwrap();
        let e = rename_file_checked(&src, &dst, 1, Duration::ZERO, &|| Ok(stale)).unwrap_err();
        assert_eq!(e.code(), "conflict");
        assert_eq!(std::fs::read(&src).unwrap(), b"newer bytes");
        assert!(!dst.exists());
    }

    #[cfg(unix)]
    #[test]
    fn rename_never_replaces_a_dangling_symlink() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a.pdf"), d.path().join("b.pdf"));
        std::fs::write(&src, b"x").unwrap();
        std::os::unix::fs::symlink(d.path().join("gone"), &dst).unwrap();
        let e = rename_file(&src, &dst, 1, Duration::ZERO).unwrap_err();
        assert_eq!(e.code(), "conflict");
        assert!(src.exists());
        assert!(
            std::fs::symlink_metadata(&dst)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn plan_target_collisions_and_skip() {
        let dir = Path::new("/in");
        let taken = [PathBuf::from("/in/x.pdf"), PathBuf::from("/in/x_(1).pdf")];
        let exists = |p: &Path| taken.iter().any(|t| t == p);
        assert_eq!(
            plan_target(&dir.join("a.pdf"), "x.pdf", &[], exists),
            Target::Path("/in/x_(2).pdf".into())
        );
        assert_eq!(
            plan_target(&dir.join("a.pdf"), "y", &[], exists),
            Target::Path("/in/y.pdf".into())
        );
        assert_eq!(
            plan_target(&dir.join("x.pdf"), "x.pdf", &[], exists),
            Target::Unchanged
        );
        let reserved = [PathBuf::from("/in/y.pdf")];
        assert_eq!(
            plan_target(&dir.join("a.pdf"), "y.pdf", &reserved, exists),
            Target::Path("/in/y_(1).pdf".into())
        );
        // NFC: decomposed source equals composed target.
        assert_eq!(
            plan_target(Path::new("/in/Cafe\u{301}.pdf"), "Café.pdf", &[], |_| {
                false
            }),
            Target::Unchanged
        );
    }
}
