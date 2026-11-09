//! Dev tasks (`cargo xtask <task>`):
//!
//! - `fetch-natives [--target <triple> | --all]`: pinned PDFium into `vendor/`.
//! - `icons [--check]`: render `assets/icons/generated/` from the SVG masters.
//! - `dist [--target <triple>] [--formats a,b] [--no-package] [--sign]`: release build,
//!   natives, staging and cargo-packager (`playbooks/release.md`).
//! - `notices [--check]`: `THIRD_PARTY_NOTICES.md` from cargo-about + `assets/licenses/`.
//! - `sign <files…>`: Azure Trusted Signing via signtool (Windows only).
//! - `check-tag <tag>`: the release tag must be `v<workspace version>`.

mod dist;
mod icons;
mod natives;
mod notices;
mod sign;

use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: cargo xtask <task>");
    eprintln!("  fetch-natives [--target <triple> | --all]");
    eprintln!("  icons [--check]");
    eprintln!("  dist [--target <triple>] [--formats <list>] [--no-package] [--sign]");
    eprintln!("  notices [--check]");
    eprintln!("  sign <file>...");
    eprintln!("  check-tag <tag>   (release tag must be v<workspace version>)");
    eprintln!(
        "targets: {}",
        natives::PDFIUM
            .iter()
            .map(|n| n.triple)
            .collect::<Vec<_>>()
            .join(", ")
    );
    ExitCode::from(2)
}

fn fetch_natives(args: &[String]) -> ExitCode {
    let selected: Vec<&natives::Native> = match args.first().map(String::as_str) {
        None => match natives::host_triple().and_then(natives::find) {
            Some(n) => vec![n],
            None => {
                eprintln!("no pinned PDFium for this host; pass --target");
                return usage();
            }
        },
        Some("--all") => natives::PDFIUM.iter().collect(),
        Some("--target") => match args.get(1).and_then(|t| natives::find(t)) {
            Some(n) => vec![n],
            None => return usage(),
        },
        Some(_) => return usage(),
    };
    for n in selected {
        if let Err(e) = natives::fetch(n) {
            eprintln!("{}: {e}", n.triple);
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn report(r: Result<(), String>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("fetch-natives") => fetch_natives(rest),
        Some("icons") => match rest {
            [] => report(icons::run(&natives::workspace_root(), false)),
            [c] if c == "--check" => report(icons::run(&natives::workspace_root(), true)),
            _ => usage(),
        },
        Some("dist") => match dist::Opts::parse(rest) {
            Some(o) => report(dist::run(&o)),
            None => usage(),
        },
        Some("notices") => match rest {
            [] => report(notices::run(&natives::workspace_root(), false)),
            [c] if c == "--check" => report(notices::run(&natives::workspace_root(), true)),
            _ => usage(),
        },
        Some("sign") if !rest.is_empty() => report(sign::run(rest)),
        Some("check-tag") if rest.len() == 1 => report(dist::check_tag_cmd(&rest[0])),
        _ => usage(),
    }
}
