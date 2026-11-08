//! Output contract: stdout = data only (text listing, one JSON document, or
//! NDJSON events + final document); stderr = diagnostics and progress.

use std::io::{IsTerminal, Write};
use std::sync::Mutex;

use autorename_core::pipeline::{ErrorDocument, ErrorInfo, Event};
use autorename_core::{Error, ExitCode};
use serde::Serialize;

use crate::cli::{Global, OutputFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Ndjson,
}

/// Auto: json when stdout is not a terminal.
pub fn resolve_format(explicit: Option<OutputFormat>) -> Format {
    match explicit {
        Some(OutputFormat::Text) => Format::Text,
        Some(OutputFormat::Json) => Format::Json,
        Some(OutputFormat::Ndjson) => Format::Ndjson,
        None if std::io::stdout().is_terminal() => Format::Text,
        None => Format::Json,
    }
}

pub fn color_allowed(no_color: bool) -> bool {
    !no_color && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

#[derive(Debug, Clone, Copy)]
pub enum Style {
    Bold,
    Dim,
    Green,
    Yellow,
    Red,
    Cyan,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Style::Bold => "1",
            Style::Dim => "2",
            Style::Green => "32",
            Style::Yellow => "33",
            Style::Red => "31",
            Style::Cyan => "36",
        }
    }
}

pub struct Out {
    pub format: Format,
    pub quiet: bool,
    pub verbose: bool,
    color_out: bool,
    color_err: bool,
    err_tty: bool,
    /// A `\r` progress line is on stderr and must be cleared first.
    progress_line: Mutex<bool>,
    /// A document could not be serialized (exit 1 instead of success).
    output_failed: std::sync::atomic::AtomicBool,
    /// Global flags every suggested command must repeat (`--config`, `--offline`).
    suggest_prefix: Vec<String>,
}

impl Out {
    pub fn new(g: &Global) -> Out {
        let color = color_allowed(g.no_color);
        Out {
            format: resolve_format(g.output),
            quiet: g.quiet,
            verbose: g.verbose && !g.quiet,
            color_out: color && std::io::stdout().is_terminal(),
            color_err: color && std::io::stderr().is_terminal(),
            err_tty: std::io::stderr().is_terminal(),
            progress_line: Mutex::new(false),
            output_failed: std::sync::atomic::AtomicBool::new(false),
            suggest_prefix: suggest_prefix(g),
        }
    }

    /// A command to suggest to the user/agent: `autorename-pdf` + the
    /// effective non-default `--config` / `--offline` + `args`, each quoted
    /// for the platform shell (PowerShell on Windows, POSIX sh elsewhere).
    pub fn command(&self, args: &[&str]) -> String {
        render_command(&self.suggest_prefix, args, cfg!(windows))
    }

    /// Hint text with every `` `autorename-pdf …` `` command carrying the
    /// same global flags as [`Out::command`].
    pub fn fix_hint(&self, hint: &str) -> String {
        if self.suggest_prefix.is_empty() {
            return hint.to_string();
        }
        let head = format!(
            "`{} ",
            render_command(&self.suggest_prefix, &[], cfg!(windows))
        );
        // Idempotent: commands already carrying the flags stay as they are.
        hint.replace(&head, "\u{0}")
            .replace("`autorename-pdf ", &head)
            .replace('\u{0}', &head)
    }

    /// Some document failed to serialize; the command must not report success.
    pub fn output_failed(&self) -> bool {
        self.output_failed.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_text(&self) -> bool {
        self.format == Format::Text
    }

    fn paint_with(on: bool, s: &str, style: Style) -> String {
        if on {
            format!("\x1b[{}m{s}\x1b[0m", style.code())
        } else {
            s.to_string()
        }
    }

    /// Style for stdout text.
    pub fn paint(&self, s: &str, style: Style) -> String {
        Self::paint_with(self.color_out, s, style)
    }

    /// Style for stderr text.
    pub fn epaint(&self, s: &str, style: Style) -> String {
        Self::paint_with(self.color_err, s, style)
    }

    fn clear_progress(&self) {
        let mut on = self.progress_line.lock().unwrap_or_else(|p| p.into_inner());
        if *on {
            eprint!("\r\x1b[2K");
            *on = false;
        }
    }

    /// One stdout line (text mode data).
    pub fn line(&self, s: &str) {
        self.clear_progress();
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{s}");
    }

    /// Diagnostic on stderr (suppressed by -q).
    pub fn note(&self, s: &str) {
        if self.quiet {
            return;
        }
        self.clear_progress();
        eprintln!("{s}");
    }

    pub fn warn(&self, s: &str) {
        if self.quiet {
            return;
        }
        self.clear_progress();
        eprintln!("{} {s}", self.epaint("warning:", Style::Yellow));
    }

    pub fn debug(&self, s: &str) {
        if self.verbose {
            self.clear_progress();
            eprintln!("{}", self.epaint(s, Style::Dim));
        }
    }

    /// Transient progress: an overwritten line on a terminal, else one line each.
    pub fn progress(&self, s: &str) {
        if self.quiet || self.format != Format::Text {
            return;
        }
        let mut on = self.progress_line.lock().unwrap_or_else(|p| p.into_inner());
        if self.err_tty {
            let width = 100usize;
            let s: String = s.chars().take(width).collect();
            eprint!("\r\x1b[2K{}", self.epaint(&s, Style::Dim));
            let _ = std::io::stderr().flush();
            *on = true;
        } else {
            eprintln!("{s}");
        }
    }

    pub fn finish_progress(&self) {
        self.clear_progress();
    }

    /// Final document: pretty JSON (json) or one compact line (ndjson). A
    /// serialization failure prints a structured `serialize_error` document
    /// instead and marks the run failed ([`Out::output_failed`]).
    pub fn document<T: Serialize>(&self, v: &T) {
        self.clear_progress();
        let text = render(self.format, v).or_else(|e| {
            self.output_failed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let doc = ErrorDocument::new(ErrorInfo::new(
                "serialize_error",
                format!("result could not be written as JSON: {e}"),
                None,
            ));
            render(self.format, &doc)
        });
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{}", text.unwrap_or_default());
    }

    /// NDJSON event line (stdout), or a text progress line.
    pub fn event(&self, e: &Event) {
        match self.format {
            Format::Ndjson => {
                if let Ok(line) = to_json(e, false, ASCII_JSON) {
                    let mut o = std::io::stdout().lock();
                    let _ = writeln!(o, "{line}");
                    let _ = o.flush();
                }
            }
            Format::Json => {
                if let Event::FileStarted {
                    index,
                    total,
                    source,
                } = e
                    && !self.quiet
                {
                    eprintln!("Processing [{index}/{total}] {}", file_name(source));
                }
            }
            Format::Text => match e {
                Event::FileStarted {
                    index,
                    total,
                    source,
                } => self.progress(&format!("[{index}/{total}] {}", file_name(source))),
                Event::FileApplied {
                    index,
                    total,
                    source,
                    ..
                } if self.err_tty => {
                    self.progress(&format!("[{index}/{total}] applying {}", file_name(source)))
                }
                _ => {}
            },
        }
    }

    /// Report a whole-command failure; returns the exit code.
    pub fn fail(&self, e: &Error) -> ExitCode {
        self.fail_info(&ErrorInfo::from_error(e), e.exit_code())
    }

    pub fn fail_info(&self, info: &ErrorInfo, code: ExitCode) -> ExitCode {
        self.clear_progress();
        match self.format {
            Format::Text => {
                eprintln!("{} {}", self.epaint("error:", Style::Red), info.message);
                if let Some(h) = &info.hint {
                    eprintln!(
                        "  {} {}",
                        self.epaint("hint:", Style::Dim),
                        self.fix_hint(h)
                    );
                }
            }
            _ => {
                let mut info = info.clone();
                info.hint = info.hint.map(|h| self.fix_hint(&h));
                self.document(&ErrorDocument::new(info))
            }
        }
        code
    }
}

/// `--config <abs path>` when given (flag or `$AUTORENAME_PDF_CONFIG`),
/// `--offline` when given on the command line.
fn suggest_prefix(g: &Global) -> Vec<String> {
    let mut v = Vec::new();
    let config = g.config.clone().or_else(|| {
        std::env::var_os("AUTORENAME_PDF_CONFIG")
            .filter(|c| !c.is_empty())
            .map(Into::into)
    });
    if let Some(c) = config {
        let abs = std::path::absolute(&c).unwrap_or(c);
        v.push("--config".into());
        v.push(abs.to_string_lossy().into_owned());
    }
    if g.offline {
        v.push("--offline".into());
    }
    v
}

/// One shell word: bare when it only has safe characters, else quoted
/// (`'…'` with `'\''` for POSIX sh; `'…'` with `''` for PowerShell).
pub fn shell_quote(s: &str, powershell: bool) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=,@+%".contains(c));
    if safe {
        s.to_string()
    } else if powershell {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn render_command(prefix: &[String], args: &[&str], powershell: bool) -> String {
    std::iter::once("autorename-pdf".to_string())
        .chain(prefix.iter().map(|a| shell_quote(a, powershell)))
        .chain(args.iter().map(|a| shell_quote(a, powershell)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn render<T: Serialize + ?Sized>(f: Format, v: &T) -> serde_json::Result<String> {
    to_json(v, f != Format::Ndjson, ASCII_JSON)
}

/// Windows consoles mangle UTF-8 depending on the code page, so machine
/// output there escapes every non-ASCII character (`\u00e4`, surrogate
/// pairs above U+FFFF): lossless JSON in any code page. Raw UTF-8 elsewhere.
pub const ASCII_JSON: bool = cfg!(windows);

/// JSON text; `ascii` escapes all non-ASCII characters as `\uXXXX`.
pub fn to_json<T: Serialize + ?Sized>(
    v: &T,
    pretty: bool,
    ascii: bool,
) -> serde_json::Result<String> {
    use serde_json::ser::{CompactFormatter, PrettyFormatter, Serializer};
    let mut buf = Vec::new();
    match (pretty, ascii) {
        (false, false) => {
            v.serialize(&mut Serializer::with_formatter(&mut buf, CompactFormatter))?
        }
        (true, false) => v.serialize(&mut Serializer::with_formatter(
            &mut buf,
            PrettyFormatter::new(),
        ))?,
        (false, true) => v.serialize(&mut Serializer::with_formatter(
            &mut buf,
            Ascii(CompactFormatter),
        ))?,
        (true, true) => v.serialize(&mut Serializer::with_formatter(
            &mut buf,
            Ascii(PrettyFormatter::new()),
        ))?,
    }
    Ok(String::from_utf8(buf).expect("serde_json writes UTF-8"))
}

/// Wraps a formatter; string fragments are written with `\uXXXX` escapes
/// for every non-ASCII character.
struct Ascii<F>(F);

impl<F: serde_json::ser::Formatter> serde_json::ser::Formatter for Ascii<F> {
    fn write_string_fragment<W: ?Sized + Write>(
        &mut self,
        w: &mut W,
        fragment: &str,
    ) -> std::io::Result<()> {
        for c in fragment.chars() {
            if c.is_ascii() {
                w.write_all(&[c as u8])?;
            } else {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    write!(w, "\\u{:04x}", u)?;
                }
            }
        }
        Ok(())
    }
    fn begin_array<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.begin_array(w)
    }
    fn end_array<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_array(w)
    }
    fn begin_array_value<W: ?Sized + Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.0.begin_array_value(w, first)
    }
    fn end_array_value<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_array_value(w)
    }
    fn begin_object<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.begin_object(w)
    }
    fn end_object<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_object(w)
    }
    fn begin_object_key<W: ?Sized + Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.0.begin_object_key(w, first)
    }
    fn end_object_key<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_object_key(w)
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.begin_object_value(w)
    }
    fn end_object_value<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_object_value(w)
    }
}

pub fn file_name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Pad to a display width in chars.
pub fn pad(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

/// Shorten to `max` chars with a middle ellipsis.
pub fn ellipsize(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max || max < 5 {
        return s.to_string();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let chars: Vec<char> = s.chars().collect();
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[n - tail..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_commands_quote_for_the_shell_and_carry_globals() {
        let prefix = vec![
            "--config".to_string(),
            "/home/me/My Configs/c.yaml".to_string(),
            "--offline".to_string(),
        ];
        assert_eq!(
            render_command(&prefix, &["names", "merge", "O'Brien & Co", "ACME"], false),
            "autorename-pdf --config '/home/me/My Configs/c.yaml' --offline names merge 'O'\\''Brien & Co' ACME"
        );
        assert_eq!(
            render_command(&prefix, &["names", "merge", "O'Brien & Co", "ACME"], true),
            "autorename-pdf --config '/home/me/My Configs/c.yaml' --offline names merge 'O''Brien & Co' ACME"
        );
        assert_eq!(render_command(&[], &["undo"], false), "autorename-pdf undo");
        assert_eq!(shell_quote("", false), "''");
    }

    #[test]
    fn json_ascii_mode_escapes_losslessly() {
        let v = serde_json::json!({"name": "Müller & Söhne 😀", "n": 1});
        let raw = to_json(&v, false, false).unwrap();
        assert_eq!(raw, r#"{"name":"Müller & Söhne 😀","n":1}"#);
        let ascii = to_json(&v, false, true).unwrap();
        assert!(ascii.is_ascii(), "{ascii}");
        assert_eq!(
            ascii,
            r#"{"name":"M\u00fcller & S\u00f6hne \ud83d\ude00","n":1}"#
        );
        // Same value back; pretty mode as well.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&ascii).unwrap(),
            v
        );
        let pretty = to_json(&v, true, true).unwrap();
        assert!(pretty.is_ascii() && pretty.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&pretty).unwrap(),
            v
        );
    }
}
