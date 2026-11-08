//! `autorename-pdf`: agent-first CLI over `autorename_core`, plus the egui
//! GUI (feature `gui`): `gui [paths]`, or no arguments with a display.

mod cli;
mod cmd;
#[cfg(feature = "gui")]
mod gui;
mod integrate;
mod out;
mod setup;

use std::ffi::OsString;
use std::io::IsTerminal;

use autorename_core::ExitCode;
use autorename_core::pipeline::{ErrorDocument, ErrorInfo};
use clap::error::ErrorKind;
use clap::{ColorChoice, CommandFactory, FromArgMatches};

use crate::cli::Cli;

/// Explicit `-o/--output` value: `Some(true)` for json/ndjson.
fn output_flag(args: &[OsString]) -> Option<bool> {
    output_value(args).map(|v| v != "text")
}

/// Explicit `-o/--output` value as written (`text`, `json`, `ndjson`, …).
fn output_value(args: &[OsString]) -> Option<String> {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    for (i, a) in args.iter().enumerate() {
        let value = match a.as_str() {
            "-o" | "--output" => args.get(i + 1).map(String::as_str),
            s => s
                .strip_prefix("--output=")
                .or_else(|| s.strip_prefix("-o=").or_else(|| s.strip_prefix("-o"))),
        };
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            return Some(v.to_string());
        }
    }
    None
}

/// Would this invocation produce JSON? (Used to render usage errors before
/// clap has parsed `--output`.)
fn wants_json(args: &[OsString]) -> bool {
    output_flag(args).unwrap_or_else(|| !std::io::stdout().is_terminal())
}

fn usage_error(e: &clap::Error, args: &[OsString]) -> ExitCode {
    let _ = e.print();
    if wants_json(args) {
        let rendered = e.render().to_string();
        let first = rendered
            .lines()
            .next()
            .unwrap_or("invalid arguments")
            .trim_start_matches("error: ")
            .to_string();
        let doc = ErrorDocument::new(ErrorInfo::new(
            "usage_error",
            first,
            Some("Run `autorename-pdf <command> --help`."),
        ));
        // NDJSON consumers read one object per line.
        let line = if output_value(args).as_deref() == Some("ndjson") {
            out::to_json(&doc, false, out::ASCII_JSON)
        } else {
            out::to_json(&doc, true, out::ASCII_JSON)
        };
        if let Ok(s) = line {
            println!("{s}");
        }
    }
    ExitCode::Usage
}

/// Installed layouts keep PDFium outside the executable's folder (core only
/// searches `$AUTORENAME_PDFIUM_DIR`, the exe dir and the dev vendor dir):
/// deb/pacman/AppImage `<prefix>/lib/autorename-pdf/`, macOS
/// `Contents/Resources/`. Point core at it before any thread starts.
fn locate_packaged_pdfium() {
    const ENV: &str = autorename_core::extract::pdfium::PDFIUM_DIR_ENV;
    if std::env::var_os(ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
    else {
        return;
    };
    let lib = if cfg!(windows) {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    };
    if dir.join(lib).is_file() {
        return;
    }
    for cand in [dir.join("../lib/autorename-pdf"), dir.join("../Resources")] {
        if cand.join(lib).is_file() {
            // SAFETY: called first thing in `main`, before any other thread
            // exists, so no concurrent environment access is possible.
            #[allow(unsafe_code)]
            unsafe {
                std::env::set_var(ENV, &cand);
            }
            return;
        }
    }
}

/// `gui --request-file`: the Windows launcher passes large selections
/// through a file instead of the command line (32,767-char limit). Reading
/// and deleting it is the acknowledgement the launcher waits for.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
fn read_request(path: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !(name.starts_with("req-") && name.ends_with(".txt")) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a launcher request file (req-*.txt)",
        ));
    }
    let text = std::fs::read_to_string(path)?;
    std::fs::remove_file(path)?;
    Ok(text
        .lines()
        .filter(|l| !l.is_empty())
        .map(std::path::PathBuf::from)
        .collect())
}

fn real_main() -> ExitCode {
    locate_packaged_pdfium();
    let args: Vec<OsString> = std::env::args_os().collect();
    // No arguments + a display → GUI (spec §Topology). Checked before clap,
    // whose `arg_required_else_help` would print help for a bare call.
    #[cfg(feature = "gui")]
    if args.len() <= 1 && gui::has_display() {
        return gui::run(Vec::new(), None, false, gui::Launch::Preview);
    }
    let no_color = args.iter().any(|a| a == "--no-color") || !out::color_allowed(false);
    let mut command = Cli::command();
    if no_color {
        command = command.color(ColorChoice::Never);
    }
    let matches = match command.try_get_matches_from(&args) {
        Ok(m) => m,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = e.print();
                    ExitCode::Success
                }
                // Machine output asked for: a structured usage error, not help text.
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                    if output_flag(&args) == Some(true) =>
                {
                    let missing = Cli::command().error(
                        ErrorKind::MissingRequiredArgument,
                        "a required command or argument is missing; see --help",
                    );
                    usage_error(&missing, &args)
                }
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    let _ = e.print();
                    ExitCode::Usage
                }
                _ => usage_error(&e, &args),
            };
        }
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => return usage_error(&e, &args),
    };
    let out = out::Out::new(&cli.global);
    let Some(command) = &cli.command else {
        if output_flag(&args) == Some(true) {
            let e = Cli::command().error(
                ErrorKind::MissingSubcommand,
                "a command is required (e.g. `rename`)",
            );
            return usage_error(&e, &args);
        }
        let _ = Cli::command().print_help();
        return ExitCode::Usage;
    };
    #[cfg(feature = "gui")]
    // GUI launches often have no TTY (launcher, desktop file): only an
    // explicit `--output json` makes `gui` a usage error.
    if let cli::Command::Gui(a) = command
        && output_flag(&args) != Some(true)
    {
        let launch = if a.rename_now {
            gui::Launch::RenameNow
        } else {
            gui::Launch::Preview
        };
        let mut paths = cmd::input_paths(&a.paths);
        if let Some(f) = &a.request_file {
            match read_request(f) {
                Ok(more) => paths.extend(more),
                Err(e) => {
                    eprintln!("error: request file {}: {e}", f.display());
                    return ExitCode::Usage;
                }
            }
        }
        return gui::run(paths, cli.global.config.clone(), cli.global.offline, launch);
    }
    let ctx = cmd::Ctx {
        g: &cli.global,
        out: &out,
    };
    match cmd::run(command, &ctx) {
        Ok(_) if out.output_failed() => ExitCode::General,
        Ok(code) => code,
        Err(e) => out.fail(&e),
    }
}

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(real_main().code() as u8)
}

#[cfg(test)]
mod tests {
    use super::read_request;

    #[test]
    fn request_file_is_read_and_deleted() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("req-1-2.txt");
        std::fs::write(&f, "C:\\a b.pdf\nC:\\c.pdf\n").unwrap();
        let got = read_request(&f).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], std::path::PathBuf::from("C:\\a b.pdf"));
        assert!(!f.exists(), "deleting the file is the acknowledgement");
        let other = t.path().join("notes.txt");
        std::fs::write(&other, "x").unwrap();
        assert!(read_request(&other).is_err());
        assert!(other.exists(), "foreign files are never deleted");
    }
}
