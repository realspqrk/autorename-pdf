//! Comment-preserving edits of `config.yaml` (GUI Settings screen, `setup`):
//! only the value of a known `section.key` changes; everything else
//! (comments, order, profiles, line endings, the final newline) stays
//! byte-identical. Plus `.env` helpers that round-trip through dotenvy.

/// Double-quoted YAML string, always a string (`"1234"`, `"true"`) and
/// always one line: `\`, `"`, newlines, tabs and other control characters
/// are escaped, so a multi-line value never splits the config line. Every
/// free-text value (names, models, ids, codes, formats) goes through here.
pub fn quoted(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One line of the text: content without its terminator, plus the
/// terminator (`"\n"`, `"\r\n"` or `""` on a last line without one).
#[derive(Clone, Copy)]
struct Line<'a> {
    body: &'a str,
    eol: &'a str,
}

fn split(text: &str) -> Vec<Line<'_>> {
    text.split_inclusive('\n')
        .map(|l| {
            let (body, eol) = if let Some(b) = l.strip_suffix("\r\n") {
                (b, "\r\n")
            } else if let Some(b) = l.strip_suffix('\n') {
                (b, "\n")
            } else {
                (l, "")
            };
            Line { body, eol }
        })
        .collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// Mapping key at the start of `t`: plain, `"…"` or `'…'` (decoded, so
/// `"model":` and `model:` are the same key). Returns (key, its original
/// spelling, text after the colon).
fn parse_key(t: &str) -> Option<(String, &str, &str)> {
    let mut key = String::new();
    let raw = match t.chars().next()? {
        '#' => return None,
        '"' => {
            let mut it = t.char_indices().skip(1);
            let end = loop {
                match it.next()? {
                    (i, '"') => break i + 1,
                    (_, '\\') => key.push(match it.next()?.1 {
                        'n' => '\n',
                        't' => '\t',
                        '0' => '\0',
                        e => e,
                    }),
                    (_, c) => key.push(c),
                }
            };
            // Full YAML escapes (`\u006d`, `\x41`, …) via the real parser.
            if let Ok(k) = serde_saphyr::from_str::<String>(&t[..end]) {
                key = k;
            }
            &t[..end]
        }
        '\'' => {
            let mut it = t.char_indices().skip(1).peekable();
            let end = loop {
                match it.next()? {
                    (_, '\'') if it.next_if(|n| n.1 == '\'').is_some() => key.push('\''),
                    (i, '\'') => break i + 1,
                    (_, c) => key.push(c),
                }
            };
            &t[..end]
        }
        _ => {
            if t == "-" || t.starts_with(['-', '?']) && t[1..].starts_with([' ', '\t']) {
                return None; // sequence item / complex key
            }
            let colon = t.char_indices().find(|&(i, c)| {
                c == ':' && (t[i + 1..].is_empty() || t[i + 1..].starts_with([' ', '\t']))
            })?;
            let raw = t[..colon.0].trim_end();
            if !comment_from(raw).is_empty() {
                return None; // `a # b: c` is a scalar with a comment
            }
            key.push_str(raw);
            raw
        }
    };
    let after = t[raw.len()..].trim_start_matches([' ', '\t']);
    let rest = after.strip_prefix(':')?;
    (rest.is_empty() || rest.starts_with([' ', '\t'])).then_some((key, raw, rest))
}

/// Mapping key of a line at exactly `indent` (see [`parse_key`]).
fn key_at(body: &str, indent: usize) -> Option<(String, &str, &str)> {
    if indent_of(body) != indent || blank_or_comment(body) {
        return None;
    }
    parse_key(&body[indent..])
}

/// Comment that starts at the first ` #` (whitespace, then `#`) in `s`.
fn comment_from(s: &str) -> &str {
    let mut prev_ws = true;
    for (i, c) in s.char_indices() {
        if c == '#' && prev_ws {
            return &s[i..];
        }
        prev_ws = c == ' ' || c == '\t';
    }
    ""
}

/// What follows a key line.
#[derive(Clone, Copy, PartialEq)]
enum Body {
    /// Nested mapping/sequence (or null): standalone comments at any
    /// column never end it (deeper ones belong to it).
    Nested,
    /// `|`/`>` scalar: deeper lines are content (`#` too); a shallower
    /// comment ends it.
    Literal,
    /// Multi-line plain scalar: deeper non-comment lines continue it.
    Plain,
}

/// Lines from `start` that belong to the value of a key at `indent`:
/// deeper lines (and `- ` items at `indent` for a nested sequence), with
/// blank and comment lines only between such lines.
fn block_end(lines: &[Line<'_>], start: usize, indent: usize, body: Body) -> usize {
    let mut last = start;
    for (j, l) in lines.iter().enumerate().skip(start) {
        let b = l.body;
        if b.trim().is_empty() {
            continue;
        }
        let ind = indent_of(b);
        if b.trim_start().starts_with('#') {
            match body {
                Body::Nested if ind > indent => last = j + 1,
                Body::Nested | Body::Plain => {}
                Body::Literal if ind > indent => last = j + 1,
                Body::Literal => break,
            }
            continue;
        }
        let item = body == Body::Nested
            && ind == indent
            && (b[ind..] == *"-" || b[ind..].starts_with("- "));
        if ind > indent || item {
            last = j + 1;
        } else {
            break;
        }
    }
    last
}

/// End of a quoted (`"`/`'`) or flow (`[`/`{`) value that starts at byte
/// `at` of line `i`: (line index of the end, byte after the end on it).
fn scan_value(lines: &[Line<'_>], i: usize, at: usize) -> Option<(usize, usize)> {
    // Every char from the opener on, with its line and end byte.
    let mut it = lines
        .iter()
        .enumerate()
        .skip(i)
        .flat_map(|(li, l)| {
            let start = if li == i { at } else { 0 };
            l.body[start..]
                .char_indices()
                .map(move |(k, c)| (li, start + k + c.len_utf8(), c))
                .chain(std::iter::once((li, usize::MAX, '\n')))
        })
        .peekable();
    let (_, _, open) = it.next()?;
    let mut depth = i32::from(matches!(open, '[' | '{'));
    let mut quote = matches!(open, '"' | '\'').then_some(open);
    let mut prev_ws = false;
    let mut prev = open;
    while let Some((li, pos, c)) = it.next() {
        if let Some(q) = quote {
            if q == '"' && c == '\\' {
                it.next(); // escaped char (or line continuation)
            } else if c == q {
                if q == '\'' && it.peek().is_some_and(|n| n.2 == '\'') {
                    it.next(); // `''` is an escaped quote
                } else {
                    quote = None;
                    if depth == 0 {
                        return Some((li, pos));
                    }
                }
            }
            continue;
        }
        match c {
            // Only at the start of a flow item (`Bob's` is plain).
            '"' | '\'' if prev_ws || matches!(prev, '[' | '{' | ',' | ':') => quote = Some(c),
            '[' | '{' => depth += 1,
            ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((li, pos));
                }
            }
            // A comment inside a multi-line flow value runs to the line end.
            '#' if prev_ws => while it.next_if(|n| n.0 == li && n.1 != usize::MAX).is_some() {},
            _ => {}
        }
        prev_ws = matches!(c, ' ' | '\t' | '\n');
        prev = c;
    }
    None
}

/// Value of the key on line `i` whose text after the colon is `rest`:
/// (first line after the value, trailing comment to keep).
fn value_span(lines: &[Line<'_>], i: usize, indent: usize, rest: &str) -> (usize, String) {
    let t = rest.trim_start();
    let at = lines[i].body.len() - t.len();
    match t.chars().next() {
        // Nested block, or null: deeper lines (and `- ` items) belong to it.
        None | Some('#') => (block_end(lines, i + 1, indent, Body::Nested), t.to_string()),
        Some('|' | '>') => {
            let comment = comment_from(t).to_string();
            (block_end(lines, i + 1, indent, Body::Literal), comment)
        }
        Some('"' | '\'' | '[' | '{') => match scan_value(lines, i, at) {
            Some((li, pos)) => {
                let tail = &lines[li].body[pos..];
                (li + 1, comment_from(tail).to_string())
            }
            None => (i + 1, String::new()),
        },
        // Plain scalar: a `#` after whitespace starts the comment; deeper
        // non-comment lines continue it (multi-line plain scalar).
        _ => (
            block_end(lines, i + 1, indent, Body::Plain),
            comment_from(t).to_string(),
        ),
    }
}

/// `{ind}key: value` with the old trailing comment (aligned like the
/// starter config's comments).
fn key_line(indent: usize, key: &str, value: &str, comment: &str) -> String {
    let base = format!("{}{key}: {value}", " ".repeat(indent));
    if comment.is_empty() {
        base
    } else {
        let pad = 30usize.saturating_sub(base.chars().count()).max(2);
        format!("{base}{}{comment}", " ".repeat(pad))
    }
}

/// A mapping entry found by [`find_key`].
struct Entry<'a> {
    /// Key line.
    line: usize,
    /// First line after its value.
    end: usize,
    /// The key as written (`model`, `"model"`): kept on replace.
    raw: &'a str,
    /// Text after the colon on the key line.
    rest: &'a str,
    /// Trailing comment of the value, kept on replace.
    comment: String,
}

/// `key` at `indent` in `lines[from..to]`. Every entry's value is skipped
/// whole (the same scanner for every value shape), so a `key:` inside
/// another value's body never matches.
fn find_key<'a>(
    lines: &[Line<'a>],
    from: usize,
    to: usize,
    indent: usize,
    key: &str,
) -> Option<Entry<'a>> {
    let mut j = from;
    while j < to {
        let Some((k, raw, rest)) = key_at(lines[j].body, indent) else {
            j += 1;
            continue;
        };
        let (end, comment) = value_span(lines, j, indent, rest);
        if k == key {
            return Some(Entry {
                line: j,
                end: end.min(to),
                raw,
                rest,
                comment,
            });
        }
        j = end.max(j + 1);
    }
    None
}

/// Text editor over the original lines: replaced ranges keep the
/// terminator of their last line, inserted lines use the file's style.
struct Doc<'a> {
    lines: Vec<Line<'a>>,
    nl: &'static str,
}

impl<'a> Doc<'a> {
    fn new(text: &'a str) -> Self {
        let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
        Doc {
            lines: split(text),
            nl,
        }
    }

    /// Lines `[from, to)` become `new` (one or more lines).
    fn render(&self, from: usize, to: usize, new: &[String]) -> String {
        let mut out = String::new();
        for l in &self.lines[..from] {
            out.push_str(l.body);
            out.push_str(l.eol);
        }
        // Inserting after a last line without terminator: give it one.
        if from == to && from == self.lines.len() && from > 0 && self.lines[from - 1].eol.is_empty()
        {
            out.push_str(self.nl);
        }
        let end_eol = if to > from {
            self.lines[to - 1].eol
        } else if from == self.lines.len() {
            // Appending keeps the file's final-newline state.
            if self.lines.last().is_none_or(|l| !l.eol.is_empty()) {
                self.nl
            } else {
                ""
            }
        } else {
            self.nl
        };
        for (k, n) in new.iter().enumerate() {
            out.push_str(n);
            out.push_str(if k + 1 == new.len() { end_eol } else { self.nl });
        }
        for l in &self.lines[to..] {
            out.push_str(l.body);
            out.push_str(l.eol);
        }
        out
    }
}

/// YAML for a parsed value when a flow section is rewritten as a block.
fn yaml_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => quoted(s),
        other => other.to_string(),
    }
}

/// A key of a rewritten flow section: plain when safe, else quoted.
fn yaml_key(k: &str) -> String {
    let plain = k
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if plain { k.to_string() } else { quoted(k) }
}

/// Set `section.key` (or top-level `key` when `section` is `None`) to the
/// YAML `value` (one line). The key's complete old value is replaced
/// (block scalars, multi-line quoted and flow values included); missing
/// keys/sections are added. Keys match in any YAML spelling (`"model":`)
/// and keep it. Comments, untouched bytes, CRLF line endings and the
/// final-newline state are kept.
pub fn set(text: &str, section: Option<&str>, key: &str, value: &str) -> String {
    let doc = Doc::new(text);
    let n = doc.lines.len();
    let Some(sec) = section else {
        return match find_key(&doc.lines, 0, n, 0, key) {
            Some(e) => doc.render(e.line, e.end, &[key_line(0, e.raw, value, &e.comment)]),
            None => doc.render(0, 0, &[format!("{key}: {value}")]),
        };
    };
    let Some(sec_entry) = find_key(&doc.lines, 0, n, 0, sec) else {
        return doc.render(n, n, &[format!("{sec}:"), format!("  {key}: {value}")]);
    };
    let (s, end) = (sec_entry.line, sec_entry.end);
    let rest = sec_entry.rest.trim_start();
    if !rest.is_empty() && !rest.starts_with('#') {
        // Flow (`ai: {provider: openai}`) or scalar (`ai: null`) section:
        // rewrite it as a block mapping with the edited key.
        let span: Vec<&str> = doc.lines[s..end].iter().map(|l| l.body).collect();
        let parsed = crate::config::parse_yaml(&span.join("\n")).ok();
        let map = parsed
            .as_ref()
            .and_then(|v| v.get(sec))
            .and_then(|v| v.as_object());
        let head = key_line(0, sec_entry.raw, "", &sec_entry.comment);
        let mut out = vec![head.trim_end().to_string()];
        if let Some(m) = map {
            for (k, v) in m.iter().filter(|(k, _)| *k != key) {
                out.push(format!("  {}: {}", yaml_key(k), yaml_value(v)));
            }
        } else if !matches!(rest.split_whitespace().next(), Some("null" | "~")) {
            // Unknown shape: leave it; validation reports the problem.
            return doc.render(s + 1, s + 1, &[format!("  {key}: {value}")]);
        }
        out.push(format!("  {key}: {value}"));
        return doc.render(s, end, &out);
    }
    let child = doc.lines[s + 1..end]
        .iter()
        .find(|l| !blank_or_comment(l.body))
        .map_or(2, |l| indent_of(l.body));
    match find_key(&doc.lines, s + 1, end, child, key) {
        Some(e) => doc.render(e.line, e.end, &[key_line(child, e.raw, value, &e.comment)]),
        None => doc.render(end, end, &[format!("{}{key}: {value}", " ".repeat(child))]),
    }
}

/// Write `.env` (holds API keys): atomic and owner-only (0600 on Unix) from
/// creation — the key is never in a file others can read.
pub fn write_env(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let dir = crate::fsx::parent_dir(path);
    std::fs::create_dir_all(dir)?;
    // `temp_in` creates 0600 on Unix; no permissions are copied from an
    // older (possibly world-readable) file.
    let tmp = crate::fsx::write_temp(dir, text.as_bytes(), None)?;
    crate::fsx::persist_replace(tmp, path)
}

/// Value of `NAME` in a `.env` text, parsed exactly like `config::read_dotenv`
/// (dotenvy: quotes, escapes, `${VAR}`; the last definition wins).
pub fn get_env(text: &str, name: &str) -> Option<String> {
    dotenvy::from_read_iter(text.as_bytes())
        .flatten()
        .filter(|(k, _)| k == name)
        .last()
        .map(|(_, v)| v)
}

/// `.env` literal that dotenvy reads back as exactly `value`: double
/// quotes with `\\`, `\"`, `\$` and `\n` escaped (dotenvy's line reader
/// treats `\` as an escape even inside single quotes, so those can't hold
/// every value).
pub fn env_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' | '"' | '$' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Key defined by a `.env` line (`NAME=`, `NAME =`, `export NAME=`).
fn env_line_key(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let t = t.strip_prefix("export ").map_or(t, str::trim_start);
    let end = t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))?;
    t[end..].trim_start().starts_with('=').then(|| &t[..end])
}

/// The `.env` text split into dotenvy's logical lines (an assignment whose
/// quoted value spans several physical lines is one entry), each with its
/// terminator. Mirrors dotenvy 0.15's `QuotedLines` state machine.
fn env_entries(text: &str) -> Vec<&str> {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        Complete,
        Escape,
        Space,
        Weak,
        WeakEsc,
        Strong,
        StrongEsc,
    }
    let mut out = Vec::new();
    let (mut start, mut pos, mut state) = (0, 0, S::Complete);
    for line in text.split_inclusive('\n') {
        let first = pos == start;
        pos += line.len();
        if first && line.trim_start().starts_with('#') {
            out.push(&text[start..pos]);
            start = pos;
            continue;
        }
        for c in line.chars() {
            state = match (state, c) {
                (S::Space, '#') => break, // comment: the line ends here
                (S::Space | S::Complete, '\\') => S::Escape,
                (S::Space | S::Complete, '"') => S::Weak,
                (S::Space | S::Complete, '\'') => S::Strong,
                (S::Complete, c) if c.is_whitespace() && c != '\n' && c != '\r' => S::Space,
                (S::Space | S::Complete | S::Escape, _) => S::Complete,
                (S::Weak, '\\') => S::WeakEsc,
                (S::Weak, '"') => S::Complete,
                (S::Weak | S::WeakEsc, _) => S::Weak,
                (S::Strong, '\\') => S::StrongEsc,
                (S::Strong, '\'') => S::Complete,
                (S::Strong | S::StrongEsc, _) => S::Strong,
            };
        }
        if matches!(state, S::Complete | S::Space) {
            state = S::Complete;
            out.push(&text[start..pos]);
            start = pos;
        }
    }
    if start < text.len() {
        out.push(&text[start..]); // unterminated quote: the rest
    }
    out
}

/// Add or replace `NAME=value` in a `.env` text; the value is encoded so
/// [`get_env`] / dotenvy return the exact bytes. Whole logical assignments
/// are replaced (multi-line quoted values included; the first definition
/// in place, later ones removed); lines inside another value and every
/// other entry stay byte-identical.
pub fn set_env(text: &str, name: &str, value: &str) -> String {
    let line = format!("{name}={}", env_literal(value));
    let mut out = String::with_capacity(text.len() + line.len() + 1);
    let mut done = false;
    for entry in env_entries(text) {
        if env_line_key(entry.lines().next().unwrap_or("")) != Some(name) {
            out.push_str(entry);
            continue;
        }
        if !done {
            out.push_str(&line);
            out.push_str(if entry.ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            });
            done = true;
        }
    }
    if !done {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// `.env` text that dotenvy parses without error (what Save writes).
pub fn check_env(text: &str) -> Result<(), String> {
    dotenvy::from_read_iter(text.as_bytes())
        .try_for_each(|r| r.map(drop))
        .map_err(|e| format!(".env cannot be read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const Y: &str = "config_version: 4\nprivacy: cloud\nai:\n  provider: openai              # comment\n  model: null\n# between\npdf:\n  ocr: auto\nprofiles: {}\n";

    fn tree(t: &str) -> serde_json::Value {
        crate::config::parse_yaml(t).unwrap_or_else(|e| panic!("invalid YAML ({e}):\n{t}"))
    }

    #[test]
    fn edits_keep_comments_and_order() {
        let t = set(Y, Some("ai"), "provider", "anthropic");
        assert!(t.contains("  provider: anthropic"));
        assert!(t.contains("# comment"));
        let t = set(&t, None, "privacy", "offline");
        assert!(t.contains("\nprivacy: offline\n"));
        let t = set(&t, Some("pdf"), "vision", "true");
        assert!(t.contains("  ocr: auto\n  vision: true\nprofiles"), "{t}");
        let t = set(&t, Some("enrich"), "enabled", "false");
        assert!(t.ends_with("enrich:\n  enabled: false\n"));
        assert!(t.contains("# between"));
    }

    #[test]
    fn quoted_strings_stay_one_line_strings() {
        assert_eq!(quoted("1234"), "\"1234\"");
        assert_eq!(quoted("true"), "\"true\"");
        let v = "Line \"one\"\nC:\\x\tend";
        let t = set(Y, None, "prompt_extension", &quoted(v));
        assert_eq!(t.lines().count(), Y.lines().count() + 1, "{t}");
        assert_eq!(tree(&t)["prompt_extension"], v);
        // Editing it again keeps one line (escaped quotes, `#` inside).
        let t2 = set(&t, None, "prompt_extension", &quoted("a \" # b"));
        assert_eq!(tree(&t2)["prompt_extension"], "a \" # b");
        let t3 = set(&t2, None, "prompt_extension", &quoted("c"));
        assert!(t3.contains("prompt_extension: \"c\"\n"), "{t3}");
        let t = set(Y, Some("company"), "name", &quoted("1234"));
        assert_eq!(tree(&t)["company"]["name"], "1234");
    }

    /// The whole old value is replaced: block scalars with their indented
    /// bodies, multi-line quoted strings and flow values.
    #[test]
    fn replaces_multiline_and_flow_values() {
        let y = "a: 1\nprompt_extension: |  # keep me\n  line one\n\n  line two # not a comment\n# after\nprofile: business\n";
        let t = set(y, None, "prompt_extension", &quoted("new"));
        assert_eq!(
            t,
            "a: 1\nprompt_extension: \"new\"       # keep me\n# after\nprofile: business\n"
        );
        let y = "output:\n  date_format: >-\n    %Y\n    %m\n  language: English\n";
        let t = set(y, Some("output"), "date_format", &quoted("%d"));
        assert_eq!(t, "output:\n  date_format: \"%d\"\n  language: English\n");
        let y = "company:\n  name: \"ACME\n    GmbH\"  # who we are\n  x: 1\n";
        let t = set(y, Some("company"), "name", &quoted("Bob"));
        assert_eq!(
            t,
            "company:\n  name: \"Bob\"                 # who we are\n  x: 1\n"
        );
        let y = "company:\n  name: 'It''s\n    multi'\n  x: 1\n";
        let t = set(y, Some("company"), "name", &quoted("B"));
        assert_eq!(t, "company:\n  name: \"B\"\n  x: 1\n");
        let y = "ai:\n  model: [a,\n    b]  # flow\n  provider: openai\n";
        let t = set(y, Some("ai"), "model", &quoted("m"));
        assert_eq!(
            t,
            "ai:\n  model: \"m\"                  # flow\n  provider: openai\n"
        );
        // Plain multi-line scalar.
        let y = "company:\n  name: Foo\n    Bar\n  x: 1\n";
        assert_eq!(
            set(y, Some("company"), "name", &quoted("Z")),
            "company:\n  name: \"Z\"\n  x: 1\n"
        );
        // A key inside another key's block body is not the target.
        let y = "prompt_extension: |\n  profile: inner\nprofile: business\n";
        let t = set(y, None, "profile", &quoted("academic"));
        assert_eq!(tree(&t)["profile"], "academic");
        assert_eq!(tree(&t)["prompt_extension"], "profile: inner\n");
    }

    /// Flow-style sections become block mappings with the edited key.
    #[test]
    fn edits_inside_flow_sections() {
        let y = "ai: {provider: openai, temperature: 0.5}  # inline\npdf:\n  ocr: auto\n";
        let t = set(y, Some("ai"), "model", &quoted("1234"));
        let v = tree(&t);
        assert_eq!(v["ai"]["model"], "1234");
        assert_eq!(v["ai"]["provider"], "openai");
        assert_eq!(v["ai"]["temperature"], 0.5);
        assert!(
            t.contains("# inline") && t.ends_with("pdf:\n  ocr: auto\n"),
            "{t}"
        );
        let t = set("enrich: null\n", Some("enrich"), "enabled", "false");
        assert_eq!(t, "enrich:\n  enabled: false\n");
    }

    /// Plain scalars with an apostrophe keep their comment; quoted ones
    /// with `#` inside are not cut.
    #[test]
    fn plain_and_quoted_comments() {
        let y = "company:\n  name: Bob's Ltd  # Keep this comment\n";
        let t = set(y, Some("company"), "name", &quoted("Bob"));
        assert!(t.contains("# Keep this comment"), "{t}");
        assert_eq!(tree(&t)["company"]["name"], "Bob");
        let y = "company:\n  name: 'a # b' # c\n";
        let t = set(y, Some("company"), "name", &quoted("x"));
        assert!(t.contains("  name: \"x\"") && t.ends_with("# c\n"), "{t}");
        assert!(!t.contains("# b"), "{t}");
        let y = "company:\n  name: a#b\n";
        assert_eq!(
            set(y, Some("company"), "name", &quoted("x")),
            "company:\n  name: \"x\"\n"
        );
    }

    /// CRLF files stay CRLF (untouched lines byte-identical); a missing
    /// final newline stays missing.
    #[test]
    fn keeps_line_endings_and_final_newline() {
        let y = "# c\r\nai:\r\n  provider: openai  # p\r\n  model: null\r\n";
        let t = set(y, Some("ai"), "model", &quoted("m"));
        assert_eq!(
            t,
            "# c\r\nai:\r\n  provider: openai  # p\r\n  model: \"m\"\r\n"
        );
        let t = set(y, Some("pdf"), "ocr", "auto");
        assert!(
            t.starts_with(y) && t.ends_with("pdf:\r\n  ocr: auto\r\n"),
            "{t:?}"
        );
        let y = "ai:\n  model: null";
        assert_eq!(set(y, Some("ai"), "model", "\"m\""), "ai:\n  model: \"m\"");
        assert_eq!(
            set(y, Some("ai"), "provider", "\"x\""),
            "ai:\n  model: null\n  provider: \"x\""
        );
        assert_eq!(
            set(y, None, "privacy", "cloud"),
            "privacy: cloud\nai:\n  model: null"
        );
        assert_eq!(set("", None, "privacy", "cloud"), "privacy: cloud\n");
    }

    /// Standalone comments at any column never end a mapping: the key
    /// after the comment is replaced, not duplicated.
    #[test]
    fn comments_inside_a_mapping_do_not_end_it() {
        for comment in ["# model note", "    # deeper", " # one space"] {
            let y =
                format!("ai:\n  provider: openai\n{comment}\n  model: old\npdf:\n  ocr: auto\n");
            let t = set(&y, Some("ai"), "model", &quoted("new"));
            assert_eq!(
                t,
                format!(
                    "ai:\n  provider: openai\n{comment}\n  model: \"new\"\npdf:\n  ocr: auto\n"
                )
            );
            assert_eq!(tree(&t)["ai"]["model"], "new");
            // A new key goes to the end of the section, after the comment.
            let t = set(&y, Some("ai"), "base_url", &quoted("u"));
            assert_eq!(tree(&t)["ai"]["base_url"], "u", "{t}");
            assert!(t.contains("  model: old\n  base_url: \"u\"\npdf:"), "{t}");
        }
        // A comment between top-level sections still separates them.
        let t = set(Y, Some("ai"), "base_url", &quoted("u"));
        assert!(t.contains("  base_url: \"u\"\n# between\npdf:"), "{t}");
    }

    /// Quoted keys (`"model":`, `'ai':`) are found, keep their spelling and
    /// are never duplicated.
    #[test]
    fn quoted_keys_are_replaced_in_place() {
        let y = "ai:\n  provider: openai\n  \"model\": old\n";
        let t = set(y, Some("ai"), "model", &quoted("new"));
        assert_eq!(t, "ai:\n  provider: openai\n  \"model\": \"new\"\n");
        assert_eq!(tree(&t)["ai"]["model"], "new");
        let y = "'ai':\n  'model' : old # c\n\"privacy\": cloud\n";
        let t = set(y, Some("ai"), "model", &quoted("m"));
        assert_eq!(tree(&t)["ai"]["model"], "m", "{t}");
        assert!(t.contains("  'model': \"m\"") && t.contains("# c"), "{t}");
        let t = set(&t, None, "privacy", "offline");
        assert_eq!(tree(&t)["privacy"], "offline", "{t}");
        assert!(t.contains("\"privacy\": offline"), "{t}");
        let t = set(
            "'ai': {provider: openai}\n",
            Some("ai"),
            "model",
            &quoted("m"),
        );
        assert_eq!(tree(&t)["ai"]["model"], "m", "{t}");
        assert_eq!(tree(&t)["ai"]["provider"], "openai", "{t}");
        // An escaped quote inside a double-quoted key.
        let y = "\"a\\\"b\": 1\nk: 2\n";
        assert_eq!(tree(&set(y, None, "k", "3"))["k"], 3);
        // YAML escapes (`\u006d` = m) name the same key: replaced, not duplicated.
        let y = "\"\\u0061i\":\n  \"\\u006dodel\": old\n";
        let t = set(y, Some("ai"), "model", &quoted("new"));
        assert_eq!(tree(&t)["ai"]["model"], "new", "{t}");
        assert_eq!(t.matches("odel").count(), 1, "{t}");
    }

    /// `#` inside a flow value's string is not a comment.
    #[test]
    fn flow_strings_keep_hashes() {
        let y = "company: {name: \"a # b\"}\nai:\n  provider: openai\n";
        let t = set(y, Some("company"), "language", &quoted("German"));
        assert_eq!(tree(&t)["company"]["name"], "a # b");
        assert_eq!(tree(&t)["company"]["language"], "German");
        assert!(!t.contains("# b\"}"), "{t}");
        let y = "company: {name: 'x # y'}  # real\n";
        let t = set(y, Some("company"), "name", &quoted("z"));
        assert_eq!(t, "company:                      # real\n  name: \"z\"\n");
    }

    #[test]
    fn env_replace() {
        let e = set_env("A=1\nOPENAI_API_KEY=old\n", "OPENAI_API_KEY", "new");
        assert_eq!(e, "A=1\nOPENAI_API_KEY=\"new\"\n");
        let e = set_env("export OPENAI_API_KEY = old\nB=2\n", "OPENAI_API_KEY", "n");
        assert_eq!(e, "OPENAI_API_KEY=\"n\"\nB=2\n");
        let e = set_env("# c\r\nK=1\r\nK=2\r\nB=2", "K", "n");
        assert_eq!(e, "# c\r\nK=\"n\"\r\nB=2");
        assert_eq!(set_env("B=2", "K", "n"), "B=2\nK=\"n\"\n");
    }

    /// Multi-line quoted values are one assignment: replacing it drops all
    /// its lines; assignment-looking lines inside another value stay.
    #[test]
    fn env_replaces_whole_multiline_assignments() {
        let e = set_env("KEY=\"old\nvalue\"\nOTHER=ok\n", "KEY", "new");
        assert_eq!(e, "KEY=\"new\"\nOTHER=ok\n");
        assert_eq!(get_env(&e, "OTHER").as_deref(), Some("ok"));
        assert_eq!(get_env(&e, "KEY").as_deref(), Some("new"));
        let text = "CERT='line\nKEY=inside\n'\nKEY=old # c\nZ=\"a\\\"\nb\"\n";
        let e = set_env(text, "KEY", "new");
        assert_eq!(
            e,
            "CERT='line\nKEY=inside\n'\nKEY=\"new\"\nZ=\"a\\\"\nb\"\n"
        );
        assert_eq!(get_env(&e, "CERT").as_deref(), Some("line\nKEY=inside\n"));
        assert_eq!(get_env(&e, "Z").as_deref(), Some("a\"\nb"));
        assert_eq!(get_env(&e, "KEY").as_deref(), Some("new"));
        check_env(&e).unwrap();
        assert!(check_env("A=\"open\n").is_err());
    }

    /// `.env` values come back byte for byte through dotenvy (the parser
    /// `config::read_dotenv` uses) and through `get_env`.
    #[test]
    fn env_values_round_trip_exactly() {
        let nasty = [
            "prefix${AUTORENAME_TEST_DEFINITELY_UNSET}suffix",
            "$HOME and $AUTORENAME_X",
            "with 'single' and \"double\" quotes",
            "back\\slash\\n not a newline \\",
            "spaces  inside # and a hash",
            "'",
            "''",
            "trailing space ",
            "sk-proj-AbC_123-xyz=",
            "ünïcödé €",
            "multi\nline",
            "ends with backslash\\",
        ];
        let mut env = "A=1\n".to_string();
        for (i, v) in nasty.iter().enumerate() {
            env = set_env(&env, &format!("K{i}"), v);
        }
        let parsed: std::collections::HashMap<String, String> =
            dotenvy::from_read_iter(env.as_bytes())
                .map(|r| r.unwrap_or_else(|e| panic!("{e}: {env}")))
                .collect();
        for (i, v) in nasty.iter().enumerate() {
            assert_eq!(parsed[&format!("K{i}")], *v, "dotenvy, {env}");
            assert_eq!(
                get_env(&env, &format!("K{i}")).as_deref(),
                Some(*v),
                "get_env"
            );
        }
        assert_eq!(get_env(&env, "A").as_deref(), Some("1"));
        // Hand-written lines keep dotenvy semantics in get_env too.
        assert_eq!(get_env("X=\"a b\"\nX='c'\n", "X").as_deref(), Some("c"));
    }
}
