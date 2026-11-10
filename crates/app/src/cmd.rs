//! Command implementations. Each returns the exit code; whole-command
//! failures are `Err` and rendered by `main`.

use std::io::Read;
use std::path::{Path, PathBuf};

use autorename_core::config::{
    self, ConfigLocation, ConfigSource, Level, Overrides, Settings, Toggle,
};
use autorename_core::extract::ExtractConfig;
use autorename_core::harmonize::{self, Entry, NameDb, Source};
use autorename_core::journal::{self, UndoStatus, UndoTarget};
use autorename_core::pipeline::{
    self, ApplyOptions, ApplyReport, ErrorDocument, Event, FileReport, InspectReport, LearnStatus,
    Plan, PlanItem, PlanOptions, PlanStatus, RenameStatus, SCHEMA_VERSION,
};
use autorename_core::{Error, ExitCode, Result, fsx, ocr};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::*;
use crate::out::{Out, Style, ellipsize, file_name, pad};

pub struct Ctx<'a> {
    pub g: &'a Global,
    pub out: &'a Out,
}

/// `{"schema_version": 4, ...body}`.
#[derive(Serialize)]
struct Doc<'a, T: Serialize> {
    schema_version: u32,
    #[serde(flatten)]
    body: &'a T,
}

fn doc<T: Serialize>(body: &T) -> Doc<'_, T> {
    Doc {
        schema_version: SCHEMA_VERSION,
        body,
    }
}

impl Ctx<'_> {
    pub(crate) fn location(&self) -> ConfigLocation {
        config::discover_config(self.g.config.as_deref())
    }

    /// Load + override + validate. Warnings go to stderr (text mode).
    fn settings(&self, ov: Overrides, profile: Option<&str>) -> Result<(Settings, ConfigLocation)> {
        let loc = self.location();
        let loaded = config::load_with_overrides(&loc.path, &ov)?;
        let tree = &loaded.tree;
        self.out.debug(&format!("config: {}", loc.path.display()));
        if self.out.is_text() {
            let v = config::validate_base(tree, profile);
            for i in loaded.notices.iter().chain(&v.issues) {
                if i.level == Level::Warning {
                    self.out.warn(&format!("{} ({})", i.message, i.field));
                }
            }
        }
        Ok((config::settings(tree, profile)?, loc))
    }

    fn overrides(&self) -> Overrides {
        Overrides {
            offline: self.g.offline,
            ..Default::default()
        }
    }
}

fn journal_path() -> Result<PathBuf> {
    journal::default_journal_path()
        .ok_or_else(|| Error::General("No OS data directory for the undo journal".into()))
}

pub(crate) fn models_root() -> Result<PathBuf> {
    ocr::models::default_root()
        .ok_or_else(|| Error::General("No OS data directory for OCR models".into()))
}

/// How to install the OCR models of `pack`: scoped like every suggested
/// command (`--config`, `--offline`) and privacy-aware (offline machines
/// import a copy; `models fetch` would be refused).
pub(crate) fn models_hint(out: &Out, offline: bool, pack: &ocr::Pack) -> String {
    if offline {
        format!(
            "Copy the `models` folder (or its `{}` subfolder) from a machine that ran `models fetch`, then run `{} <dir>`.",
            pack.id,
            out.command(&["models", "import"])
        )
    } else {
        format!(
            "Run `{}` (~{} MB) to read scans and logos on this computer; until then scans use AI image reading when it is on.",
            out.command(&["models", "fetch", pack.id]),
            pack.total_size().div_ceil(1_000_000)
        )
    }
}

/// The install hint for the run's settings ([`models_hint`]).
fn settings_models_hint(out: &Out, g: &Global, s: &Settings) -> Option<String> {
    let pack = ocr::manifest::pack_for_language(&s.ocr.language)?;
    let offline = g.offline || s.privacy == config::Privacy::Offline;
    Some(models_hint(out, offline, pack))
}

/// Give `ocr_models_missing` warnings and errors the scoped install hint.
fn scope_models_hint<'a>(
    hint: &str,
    warnings: impl IntoIterator<Item = &'a mut String>,
    error: Option<&mut pipeline::ErrorInfo>,
) {
    for w in warnings {
        if w.starts_with("ocr_models_missing:") && !w.ends_with(hint) {
            w.push_str(if w.ends_with('.') { " " } else { ". " });
            w.push_str(hint);
        }
    }
    if let Some(e) = error.filter(|e| e.code == "ocr_models_missing") {
        e.hint = Some(hint.to_string());
    }
}

fn toggle(m: Option<Mode>) -> Option<Toggle> {
    m.map(|m| match m {
        Mode::Auto => Toggle::Auto,
        Mode::On => Toggle::On,
        Mode::Off => Toggle::Off,
    })
}

pub fn run(cmd: &Command, ctx: &Ctx<'_>) -> Result<ExitCode> {
    match cmd {
        Command::Rename(a) => rename(a, ctx),
        Command::Apply(a) => apply(a, ctx),
        Command::Undo(a) => undo(a, ctx),
        Command::Inspect(a) => inspect(a, ctx),
        Command::Names(a) => names(a, ctx),
        Command::Config(c) => config_cmd(c, ctx),
        Command::Models(c) => models(c, ctx),
        Command::Schema(a) => schema(a, ctx),
        Command::Integrate(c) => integrate(c, ctx),
        Command::Setup(a) => crate::setup::run(a, ctx),
        Command::Gui(_) => Err(Error::Usage {
            message: if cfg!(feature = "gui") {
                "The GUI has no JSON output and needs a display".into()
            } else {
                "This build has no GUI (feature `gui` off)".into()
            },
            hint: Some("Use `rename --output json` for machine output.".into()),
        }),
    }
}

// ---------------------------------------------------------------------------
// rename / apply
// ---------------------------------------------------------------------------

/// `*`/`?` match of one file name, case-insensitive (Windows semantics).
pub(crate) fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    // Classic two-pointer glob with backtracking on the last `*`.
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

/// Expand `*`/`?` in the last component of each argument against
/// `list(dir)` (sorted). No match keeps the argument (so the error names it).
/// Windows shells pass wildcards through unexpanded; POSIX shells expand.
pub(crate) fn expand_wildcards(
    args: &[PathBuf],
    list: &dyn Fn(&Path) -> Vec<String>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for a in args {
        let Some(name) = a.file_name().and_then(|n| n.to_str()) else {
            out.push(a.clone());
            continue;
        };
        if !name.contains(['*', '?']) {
            out.push(a.clone());
            continue;
        }
        let dir = a.parent().filter(|d| !d.as_os_str().is_empty());
        let mut hits: Vec<String> = list(dir.unwrap_or(Path::new(".")))
            .into_iter()
            .filter(|n| wildcard_match(name, n))
            .collect();
        hits.sort();
        if hits.is_empty() {
            out.push(a.clone());
        }
        out.extend(hits.into_iter().map(|h| match dir {
            Some(d) => d.join(h),
            None => PathBuf::from(h),
        }));
    }
    out
}

/// Input paths as given; on Windows with wildcards expanded.
pub(crate) fn input_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    if !cfg!(windows) {
        return paths.to_vec();
    }
    expand_wildcards(paths, &|dir| {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn rename(a: &RenameArgs, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let paths = input_paths(&a.paths);
    let ov = Overrides {
        provider: a.provider.map(|p| p.id().to_string()),
        model: a.model.clone(),
        ocr: toggle(a.ocr),
        vision: toggle(a.vision),
        no_enrich: a.no_enrich,
        no_learn: a.no_learn,
        ..ctx.overrides()
    };
    let (settings, loc) = ctx.settings(ov, a.profile.as_deref())?;
    let name_lists = config::name_lists_dir(&loc.path);
    let opts = PlanOptions {
        recursive: a.recursive,
        jobs: a.jobs.map_or_else(pipeline::default_jobs, usize::from),
        name_lists_dir: Some(name_lists.clone()),
        models_dir: None,
        env: config::read_dotenv(&loc.path),
        cancel: None,
    };
    if let Some(path) = &a.plan {
        check_plan_output(path, &paths, &[], a.force)?;
    }
    let out = ctx.out;
    ctx.out.debug(&format!(
        "provider {} / {}, profile {}, jobs {}",
        settings.ai.provider.id(),
        settings.ai.model,
        settings.profile_id,
        opts.jobs
    ));
    let progress = |e: &Event| out.event(e);
    let mut plan = pipeline::plan_files(&paths, &settings, &opts, &progress)?;
    out.finish_progress();
    if let Some(hint) = settings_models_hint(out, ctx.g, &settings) {
        scope_models_hint(&hint, &mut plan.warnings, None);
        for item in &mut plan.items {
            scope_models_hint(&hint, &mut item.warnings, item.error.as_mut());
        }
    }
    for w in &plan.warnings {
        out.warn(w);
    }
    if a.dry_run || a.plan.is_some() {
        if let Some(path) = &a.plan {
            let planned: Vec<PathBuf> = plan
                .items
                .iter()
                .flat_map(|i| std::iter::once(i.source.clone()).chain(i.target.clone()))
                .collect();
            check_plan_output(path, &paths, &planned, a.force)?;
            let text =
                // Same encoding as stdout: ASCII-escaped JSON on Windows.
                crate::out::to_json(&plan, true, crate::out::ASCII_JSON)
                    .map_err(|e| Error::General(e.to_string()))?;
            let bytes = format!("{text}\n");
            let written = if a.force {
                fsx::write_atomic(path, bytes.as_bytes())
            } else {
                fsx::write_new(path, bytes.as_bytes())
            };
            written.map_err(|e| Error::io(path, e))?;
            let p = path.to_string_lossy();
            out.note(&if plan.summary.rename == 0 && plan.summary.failed > 0 {
                format!(
                    "Plan written to {} with nothing to rename; fix the errors above, then run `{}` again.",
                    path.display(),
                    out.command(&["rename", "--force", "--plan", &p, "..."])
                )
            } else {
                format!(
                    "Plan written to {} (run `{}`).",
                    path.display(),
                    out.command(&["apply", &p])
                )
            });
        }
        if out.is_text() {
            print_plan(&plan, out);
        } else {
            out.document(&plan);
        }
        return Ok(plan.exit_code());
    }
    let report = pipeline::apply_plan(
        &plan,
        &settings,
        &ApplyOptions {
            journal_path: journal_path()?,
            name_lists_dir: Some(name_lists),
            source: "cli".into(),
            cancel: None,
        },
        &progress,
    )?;
    finish_report(&report, out);
    Ok(report.exit_code())
}

/// Canonical form of a path that may not exist yet (canonical parent + name).
fn canonical_lenient(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    if let Ok(c) = std::fs::canonicalize(&abs) {
        return c;
    }
    match (abs.parent(), abs.file_name()) {
        (Some(dir), Some(name)) => std::fs::canonicalize(dir)
            .map(|d| d.join(name))
            .unwrap_or(abs.clone()),
        _ => abs,
    }
}

/// `--plan FILE` must not alias any input, planned source or target, and is
/// never overwritten without `--force`.
fn check_plan_output(
    path: &Path,
    inputs: &[PathBuf],
    planned: &[PathBuf],
    force: bool,
) -> Result<()> {
    let out = canonical_lenient(path);
    let usage = |message: String, hint: &str| Error::Usage {
        message,
        hint: Some(hint.into()),
    };
    if inputs
        .iter()
        .chain(planned)
        .any(|p| canonical_lenient(p) == out)
    {
        return Err(usage(
            format!(
                "--plan {} is one of the files being renamed",
                path.display()
            ),
            "Write the plan to a separate file, e.g. `--plan plan.json`.",
        ));
    }
    if !force && std::fs::symlink_metadata(path).is_ok() {
        return Err(usage(
            format!("--plan {} already exists", path.display()),
            "Choose a new file name or pass --force to overwrite it.",
        ));
    }
    Ok(())
}

/// Plan bytes → text: UTF-8 (with or without BOM) or UTF-16 LE/BE with BOM
/// (Windows PowerShell 5 `Out-File`/`Set-Content` default).
pub(crate) fn decode_text(bytes: &[u8]) -> std::result::Result<String, String> {
    let utf16 = |rest: &[u8], le: bool| {
        let (pairs, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return Err("UTF-16 text with an odd number of bytes".to_string());
        }
        let units: Vec<u16> = pairs
            .iter()
            .map(|c| {
                if le {
                    u16::from_le_bytes(*c)
                } else {
                    u16::from_be_bytes(*c)
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|e| e.to_string())
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            String::from_utf8(rest.to_vec()).map_err(|e| e.to_string())
        }
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        _ => String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string()),
    }
}

fn read_plan(path: &Path) -> Result<Plan> {
    let bytes = if path == Path::new("-") {
        let mut b = Vec::new();
        std::io::stdin()
            .read_to_end(&mut b)
            .map_err(|e| Error::io("<stdin>", e))?;
        b
    } else {
        std::fs::read(path).map_err(|e| Error::Usage {
            message: format!("cannot read plan {}: {e}", path.display()),
            hint: Some("Create one with `autorename-pdf rename --plan plan.json <paths>`.".into()),
        })?
    };
    let text = decode_text(&bytes).map_err(|e| Error::Usage {
        message: format!("plan is not UTF-8 or UTF-16 text: {e}"),
        hint: Some("Save it as UTF-8 (PowerShell: `Set-Content -Encoding utf8`), or let `rename --plan` write it.".into()),
    })?;
    Plan::from_json(&text)
}

fn apply(a: &ApplyArgs, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let plan = read_plan(&a.plan)?;
    // Name the plan file in the repair hint (no new analysis needed).
    pipeline::validate_plan(&plan).map_err(|e| match e {
        Error::Usage { message, hint } => Error::Usage {
            message,
            hint: hint.map(|h| {
                let shown = if a.plan == Path::new("-") {
                    "the plan".to_string()
                } else {
                    a.plan.display().to_string()
                };
                h.replace("the plan file", &shown)
                    .replace(
                        "`autorename-pdf apply <plan>`",
                        &format!(
                            "`{}`",
                            ctx.out.command(&["apply", &a.plan.to_string_lossy()])
                        ),
                    )
                    .replace(
                        "`autorename-pdf rename --plan <plan> <paths>`",
                        &format!(
                            "`{} <paths>`",
                            ctx.out
                                .command(&["rename", "--plan", &a.plan.to_string_lossy()])
                        ),
                    )
            }),
        },
        other => other,
    })?;
    let ov = Overrides {
        no_enrich: a.no_enrich,
        no_learn: a.no_learn,
        ..ctx.overrides()
    };
    let (settings, loc) = ctx.settings(ov, Some(&plan.profile))?;
    let out = ctx.out;
    let progress = |e: &Event| out.event(e);
    let report = pipeline::apply_plan(
        &plan,
        &settings,
        &ApplyOptions {
            journal_path: journal_path()?,
            name_lists_dir: Some(config::name_lists_dir(&loc.path)),
            source: "cli".into(),
            cancel: None,
        },
        &progress,
    )?;
    finish_report(&report, out);
    Ok(report.exit_code())
}

fn finish_report(report: &ApplyReport, out: &Out) {
    out.finish_progress();
    if out.is_text() {
        print_report(report, out);
    } else {
        out.document(report);
    }
}

fn name_width<'a>(names: impl Iterator<Item = &'a Path>) -> usize {
    names
        .map(|p| file_name(p).chars().count())
        .max()
        .unwrap_or(0)
        .min(48)
}

fn print_error_lines(out: &Out, indent: usize, e: &pipeline::ErrorInfo) {
    let pad_s = " ".repeat(indent);
    if let Some(h) = &e.hint {
        out.line(&format!("{pad_s}{}", out.paint(h, Style::Dim)));
    }
}

fn print_warnings(out: &Out, indent: usize, warnings: &[String]) {
    let pad_s = " ".repeat(indent);
    for w in warnings {
        out.line(&format!("{pad_s}{} {w}", out.paint("!", Style::Yellow)));
    }
}

fn print_plan(plan: &Plan, out: &Out) {
    let w = name_width(plan.items.iter().map(|i| i.source.as_path()));
    for item in &plan.items {
        let src = pad(&ellipsize(&file_name(&item.source), 48), w);
        match item.status {
            PlanStatus::Rename => out.line(&format!(
                "  {src}  {}  {}",
                out.paint("→", Style::Cyan),
                out.paint(item.proposed_name.as_deref().unwrap_or(""), Style::Bold),
            )),
            PlanStatus::Unchanged => out.line(&format!(
                "  {src}  {}",
                out.paint("· already named", Style::Dim)
            )),
            PlanStatus::Cancelled => out.line(&format!(
                "  {src}  {}",
                out.paint("· cancelled", Style::Dim)
            )),
            PlanStatus::Failed => {
                let e = item.error.as_ref();
                out.line(&format!(
                    "  {src}  {} {}",
                    out.paint("✗", Style::Red),
                    e.map_or("failed", |e| e.message.as_str())
                ));
                if let Some(e) = e {
                    print_error_lines(out, w + 7, e);
                }
            }
        }
        print_images_sent(out, w + 7, &item.page_quality);
        print_warnings(out, w + 7, &item.warnings);
        print_plan_notes(item, w, out);
    }
    let s = &plan.summary;
    let mut parts = vec![format!("{} to rename", s.rename)];
    if s.unchanged > 0 {
        parts.push(format!("{} unchanged", s.unchanged));
    }
    if s.failed > 0 {
        parts.push(out.paint(&format!("{} failed", s.failed), Style::Red));
    }
    out.line(&format!(
        "{} {} {}",
        out.paint("Dry run:", Style::Bold),
        parts.join(", "),
        out.paint("(nothing changed)", Style::Dim)
    ));
}

/// Privacy note: which pages went to the AI as pictures.
fn print_images_sent(out: &Out, indent: usize, pages: &[pipeline::PageQuality]) {
    let sent: Vec<String> = pages
        .iter()
        .filter(|p| p.image)
        .map(|p| p.page.to_string())
        .collect();
    let note = match sent.as_slice() {
        [] => return,
        [one] => format!("page {one} sent to AI as an image"),
        many => format!("pages {} sent to AI as images", many.join(", ")),
    };
    out.line(&format!(
        "{}{}",
        " ".repeat(indent),
        out.paint(&note, Style::Dim)
    ));
}

fn print_plan_notes(item: &PlanItem, w: usize, out: &Out) {
    if out.verbose {
        let mut notes = Vec::new();
        if let pipeline::EnrichPlan::Skip { reason } = &item.enrich {
            notes.push(format!("no enrichment ({reason})"));
        }
        if let Some(l) = &item.learn {
            notes.push(match l {
                pipeline::LearnAction::AddAlias { canonical, alias } => {
                    format!("learn alias {alias:?} → {canonical:?}")
                }
                pipeline::LearnAction::NewEntry { name, .. } => format!("new entry {name:?}"),
            });
        }
        if !notes.is_empty() {
            out.line(&format!(
                "{}{}",
                " ".repeat(w + 7),
                out.paint(&notes.join(" · "), Style::Dim)
            ));
        }
    }
}

fn file_tags(f: &FileReport) -> Vec<String> {
    let mut tags = Vec::new();
    match f.enrich_status.as_str() {
        "enriched" => tags.push("enriched".to_string()),
        "skipped:disabled" | "not_run" | "unchanged" => {}
        other => tags.push(format!(
            "not enriched ({})",
            other.strip_prefix("skipped:").unwrap_or(other)
        )),
    }
    match f.learn_status {
        LearnStatus::AliasAdded => tags.push("learned alias".into()),
        LearnStatus::EntryAdded => tags.push("new entry".into()),
        LearnStatus::Failed => tags.push("learning failed".into()),
        _ => {}
    }
    tags
}

fn print_report(r: &ApplyReport, out: &Out) {
    let w = name_width(r.files.iter().map(|f| f.source.as_path()));
    for f in &r.files {
        let src = pad(&ellipsize(&file_name(&f.source), 48), w);
        let tags = file_tags(f);
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!("  {}", out.paint(&tags.join(" · "), Style::Dim))
        };
        match (f.rename_status, &f.error) {
            (_, Some(e)) => {
                out.line(&format!(
                    "  {src}  {} {}",
                    out.paint("✗", Style::Red),
                    e.message
                ));
                print_error_lines(out, w + 7, e);
            }
            (RenameStatus::Renamed, None) => out.line(&format!(
                "  {src}  {}  {}{tags}",
                out.paint("→", Style::Green),
                out.paint(f.name.as_deref().unwrap_or(""), Style::Bold),
            )),
            (_, None) => out.line(&format!(
                "  {src}  {}{tags}",
                out.paint("· already named", Style::Dim)
            )),
        }
        print_warnings(out, w + 7, &f.warnings);
    }
    let s = &r.summary;
    let mut parts = vec![format!("{} renamed", s.renamed)];
    if s.unchanged > 0 {
        parts.push(format!("{} unchanged", s.unchanged));
    }
    if s.enriched > 0 {
        parts.push(format!("{} enriched", s.enriched));
    }
    if s.learned > 0 {
        parts.push(format!(
            "{} learned",
            if s.learned == 1 {
                "1 name".to_string()
            } else {
                format!("{} names", s.learned)
            }
        ));
    }
    if s.failed > 0 {
        parts.push(out.paint(&format!("{} failed", s.failed), Style::Red));
    }
    let undo = match &r.batch_id {
        Some(_) if s.renamed + s.enriched + s.learned > 0 => {
            format!(
                "  {}",
                out.paint(&format!("(revert: {})", out.command(&["undo"])), Style::Dim)
            )
        }
        _ => String::new(),
    };
    out.line(&format!(
        "{} {}{undo}",
        out.paint("Done:", Style::Bold),
        parts.join(", ")
    ));
}

// ---------------------------------------------------------------------------
// undo
// ---------------------------------------------------------------------------

fn undo(a: &UndoArgs, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let out = ctx.out;
    let path = journal_path()?;
    if a.list {
        let batches = journal::list_batches(&journal::read_journal(&path)?);
        if out.is_text() {
            if batches.is_empty() {
                out.line("No undo batches.");
            }
            for b in batches.iter().rev() {
                let state = if b.undone {
                    out.paint("undone", Style::Dim)
                } else {
                    out.paint("active", Style::Green)
                };
                let mut what = vec![format!(
                    "{} file{}",
                    b.file_count,
                    if b.file_count == 1 { "" } else { "s" }
                )];
                if b.metadata_count > 0 {
                    what.push(format!("{} metadata", b.metadata_count));
                }
                if b.name_count > 0 {
                    what.push(format!(
                        "{} name{}",
                        b.name_count,
                        if b.name_count == 1 { "" } else { "s" }
                    ));
                }
                out.line(&format!(
                    "  {}  {}  {}  {state}",
                    b.batch_id,
                    b.timestamp.get(..19).unwrap_or(&b.timestamp),
                    what.join(" · ")
                ));
            }
        } else {
            out.document(&json!({"schema_version": SCHEMA_VERSION, "batches": batches}));
        }
        return Ok(ExitCode::Success);
    }
    let target = match (&a.batch, a.all) {
        (Some(id), _) => UndoTarget::Batch(id.clone()),
        (None, true) => UndoTarget::All,
        (None, false) => UndoTarget::Last,
    };
    // Locked read-undo-write: concurrent runs cannot lose or double-undo batches.
    let report = journal::undo_at(&path, &target, chrono::Utc::now())?;
    if let UndoTarget::Batch(id) = &target
        && report.batch_ids.is_empty()
    {
        return Err(Error::Usage {
            message: format!("no open undo batch {id}"),
            hint: Some("List batches with `autorename-pdf undo --list`.".into()),
        });
    }
    if out.is_text() {
        if report.batch_ids.is_empty() {
            out.line("Nothing to undo.");
        }
        let w = report
            .files
            .iter()
            .map(|f| file_name(Path::new(&f.new_path)).chars().count())
            .max()
            .unwrap_or(0)
            .min(48);
        for f in &report.files {
            let from = pad(&file_name(Path::new(&f.new_path)), w);
            match f.status {
                UndoStatus::Restored => out.line(&format!(
                    "  {from}  {}  {}",
                    out.paint("→", Style::Green),
                    file_name(Path::new(&f.old_path))
                )),
                UndoStatus::Failed => out.line(&format!(
                    "  {from}  {} {}",
                    out.paint("✗", Style::Red),
                    f.error.as_deref().unwrap_or("failed")
                )),
            }
            for w in &f.warnings {
                out.line(&format!("  {} {w}", out.paint("!", Style::Yellow)));
            }
        }
        for e in &report.enrich {
            if let Some(err) = &e.error {
                out.line(&format!("  {} {err}", out.paint("✗", Style::Red)));
            }
            for w in &e.warnings {
                out.line(&format!("  {} {w}", out.paint("!", Style::Yellow)));
            }
        }
        for al in &report.aliases {
            if let Some(err) = &al.error {
                out.line(&format!("  {} {err}", out.paint("✗", Style::Red)));
            }
            if let (Some(note), false) = (&al.note, al.reverted) {
                let name = match &al.change {
                    harmonize::Change::AliasAdded { alias, .. } => alias,
                    harmonize::Change::EntryAdded { canonical } => canonical,
                };
                out.line(&out.paint(&format!("  {name}: {note}"), Style::Dim));
            }
        }
        if !report.batch_ids.is_empty() {
            let restored_meta = report.enrich.iter().filter(|e| e.error.is_none()).count();
            let names = report.aliases.iter().filter(|a| a.reverted).count();
            out.line(&format!(
                "{} {} restored, {} failed, {} metadata reverted, {} names forgotten  {}",
                out.paint("Undo:", Style::Bold),
                report.restored,
                report.failed,
                restored_meta,
                names,
                out.paint(
                    &format!("(batch {})", report.batch_ids.join(", ")),
                    Style::Dim
                )
            ));
        }
    } else {
        out.document(&doc(&report));
    }
    Ok(if report.has_failures() {
        ExitCode::Partial
    } else {
        ExitCode::Success
    })
}

// ---------------------------------------------------------------------------
// inspect
// ---------------------------------------------------------------------------

/// `p1 0.75 pictures ocr 0.97, p2 0.00 ocr 0.95, p3 0.40 → vision`.
fn page_scores(pages: &[pipeline::PageReport]) -> String {
    if pages.is_empty() {
        return "none".into();
    }
    pages
        .iter()
        .map(|p| {
            let mut s = format!("p{} {:.2}", p.page, p.quality);
            if p.visual {
                s.push_str(" pictures");
            }
            if let Some(q) = p.ocr_quality {
                s.push_str(&format!(" ocr {q:.2}"));
            }
            if p.would_vision {
                s.push_str(" → vision");
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn inspect(a: &InspectArgs, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let out = ctx.out;
    let (cfg, hint) = match ctx.settings(ctx.overrides(), None) {
        Ok((s, _)) => (
            ExtractConfig::from_settings(&s),
            settings_models_hint(out, ctx.g, &s),
        ),
        Err(e) => {
            out.debug(&format!("config not used ({e}); default thresholds"));
            let pack = ocr::manifest::pack_for_language(ocr::AUTO_LANGUAGE);
            let hint = pack.map(|p| models_hint(out, ctx.g.offline, p));
            (ExtractConfig::default(), hint)
        }
    };
    let mut files: Vec<_> = input_paths(&a.paths)
        .iter()
        .map(|p| pipeline::inspect(p, &cfg))
        .collect();
    if let Some(hint) = &hint {
        for f in &mut files {
            scope_models_hint(hint, &mut f.warnings, f.error.as_mut());
        }
    }
    let code =
        pipeline::batch_exit_code(files.len(), files.iter().filter_map(|f| f.error.as_ref()));
    if out.is_text() {
        for f in &files {
            out.line(&out.paint(&f.path.display().to_string(), Style::Bold));
            let kv = |k: &str, v: String| out.line(&format!("  {}  {v}", pad(k, 10)));
            if let Some(e) = &f.error
                && f.size.is_none()
            {
                kv("error", out.paint(&e.message, Style::Red));
                continue;
            }
            if let Some(e) = &f.error {
                kv("error", out.paint(&e.message, Style::Red));
            }
            if let Some(size) = f.size {
                kv("size", format!("{size} bytes"));
            }
            let mut sec = Vec::new();
            if f.encrypted {
                sec.push("encrypted");
            }
            if f.signed {
                sec.push("signed");
            }
            kv(
                "security",
                if sec.is_empty() {
                    "not signed, not encrypted".into()
                } else {
                    sec.join(", ")
                },
            );
            kv("enrich", f.enrich.clone());
            if let Some(t) = &f.text {
                kv(
                    "text",
                    format!(
                        "quality {:.2} (threshold {:.2}), {} chars, {}/{} pages",
                        t.quality, t.threshold, t.chars, t.pages_used, t.page_count
                    ),
                );
                kv("pages", page_scores(&t.pages));
                if let Some(q) = t.ocr_quality {
                    kv("ocr", format!("quality {q:.2} with local OCR text"));
                }
                let route = match (t.would_ocr, t.would_vision) {
                    (false, false) => "text layer only",
                    (true, false) => "OCR",
                    (false, true) => "vision",
                    (true, true) => "OCR + vision",
                };
                kv("tier", route.into());
            }
            match &f.embedded {
                Some(e) => {
                    let fields = e
                        .pointer("/attachment/fields")
                        .and_then(Value::as_object)
                        .map(|m| {
                            m.iter()
                                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let ts = e
                        .pointer("/attachment/timestamp")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    kv("embedded", format!("yes ({ts})"));
                    kv("fields", fields);
                    if let Some(sent) = f.images_sent {
                        kv(
                            "images",
                            if sent {
                                "sent to the model"
                            } else {
                                "not sent"
                            }
                            .into(),
                        );
                    }
                }
                None => kv("embedded", "no".into()),
            }
            for w in &f.warnings {
                kv("warning", out.paint(w, Style::Yellow));
            }
        }
    } else {
        out.document(&InspectReport {
            schema_version: SCHEMA_VERSION,
            files,
        });
    }
    Ok(code)
}

// ---------------------------------------------------------------------------
// names
// ---------------------------------------------------------------------------

/// The list `names` works on: `--field`, else the active profile's
/// `harmonize_field` (none → usage error naming `--field`).
struct NameList {
    field: String,
    file: PathBuf,
    /// `--field` was given: printed commands repeat it.
    explicit: bool,
}

fn name_list(a: &NamesArgs, ctx: &Ctx<'_>) -> Result<NameList> {
    let loc = ctx.location();
    let (field, explicit) = match &a.field {
        Some(f) if harmonize::is_field_id(f) => (f.clone(), true),
        Some(f) => {
            return Err(Error::Usage {
                message: format!(
                    "invalid --field {f:?}: not a name-list field id (lowercase a-z, 0-9, _)"
                ),
                hint: Some("Use a profile field id such as company_name or journal_name.".into()),
            });
        }
        None => {
            let (settings, _) = ctx.settings(ctx.overrides(), None)?;
            match settings.profile.harmonize_field.clone() {
                Some(f) => (f, false),
                None => {
                    return Err(Error::Usage {
                        message: format!(
                            "profile {:?} has no harmonize_field (Name matching is off), so there is no default name list",
                            settings.profile_id
                        ),
                        hint: Some(
                            "Pass --field <id>, e.g. `autorename-pdf names --field journal_name list`."
                                .into(),
                        ),
                    });
                }
            }
        }
    };
    Ok(NameList {
        file: config::name_list_path(&loc.path, &field),
        field,
        explicit,
    })
}

impl NameList {
    /// `names [--field F] <args>` as a ready-to-run command.
    fn command(&self, out: &Out, args: &[&str]) -> String {
        let mut all = vec!["names"];
        if self.explicit {
            all.extend(["--field", self.field.as_str()]);
        }
        all.extend_from_slice(args);
        out.command(&all)
    }

    /// Lowercase plural label (`companies`, `venues`).
    fn plural(&self) -> String {
        harmonize::field_label(&self.field).1.to_lowercase()
    }
}

#[derive(Serialize)]
struct NamesDoc<'a> {
    schema_version: u32,
    field: &'a str,
    #[serde(serialize_with = "fsx::path_serde::serialize")]
    file: &'a Path,
    entries: Vec<&'a Entry>,
}

fn print_entries(out: &Out, list: &NameList, entries: &[&Entry]) {
    let file = list.file.as_path();
    if !out.is_text() {
        out.document(&NamesDoc {
            schema_version: SCHEMA_VERSION,
            field: &list.field,
            file,
            entries: entries.to_vec(),
        });
        return;
    }
    if entries.is_empty() {
        out.line(&out.paint(&format!("No entries in {}", file.display()), Style::Dim));
        return;
    }
    let w = entries
        .iter()
        .map(|e| e.canonical.chars().count())
        .max()
        .unwrap_or(0)
        .min(40);
    for e in entries {
        let mut tags = Vec::new();
        if e.source == Source::Auto {
            tags.push(
                format!("auto {}", e.added.as_deref().unwrap_or(""))
                    .trim()
                    .to_string(),
            );
        }
        if e.review {
            tags.push(out.paint("review", Style::Yellow));
        }
        let mut names: Vec<String> = e.aliases.clone();
        names.extend(e.learned.iter().map(|l| format!("{l}*")));
        out.line(&format!(
            "  {}  {}  {}",
            out.paint(&pad(&e.canonical, w), Style::Bold),
            names.join(", "),
            out.paint(&tags.join(" · "), Style::Dim)
        ));
    }
    out.line(&out.paint(
        &format!(
            "{} entries · * = learned · {}",
            entries.len(),
            file.display()
        ),
        Style::Dim,
    ));
}

/// Candidates close enough to suggest a merge (the decider band).
fn close_candidates(e: &Entry) -> impl Iterator<Item = &harmonize::ReviewCandidate> {
    e.evidence
        .iter()
        .flat_map(|ev| &ev.candidates)
        .filter(|c| c.score >= harmonize::DECIDER_THRESHOLD)
}

/// Ready-to-run decisions for a review entry.
fn review_commands(out: &Out, list: &NameList, e: &Entry) -> Vec<String> {
    let mut cmds: Vec<String> = close_candidates(e)
        .map(|c| list.command(out, &["merge", &e.canonical, &c.name]))
        .collect();
    cmds.push(list.command(out, &["review", "--accept", &e.canonical]));
    cmds
}

#[derive(Serialize)]
struct ReviewItem<'a> {
    #[serde(flatten)]
    entry: &'a Entry,
    commands: Vec<String>,
}

#[derive(Serialize)]
struct ReviewDoc<'a> {
    schema_version: u32,
    field: &'a str,
    #[serde(serialize_with = "fsx::path_serde::serialize")]
    file: &'a Path,
    /// All entries in the file.
    total: usize,
    accepted: &'a [String],
    entries: Vec<ReviewItem<'a>>,
}

fn print_review(out: &Out, list: &NameList, db: &NameDb, accepted: &[String]) {
    let pending: Vec<&Entry> = db.entries.iter().filter(|e| e.review).collect();
    if !out.is_text() {
        out.document(&ReviewDoc {
            schema_version: SCHEMA_VERSION,
            field: &list.field,
            file: &list.file,
            total: db.entries.len(),
            accepted,
            entries: pending
                .iter()
                .map(|e| ReviewItem {
                    entry: e,
                    commands: review_commands(out, list, e),
                })
                .collect(),
        });
        return;
    }
    if pending.is_empty() {
        out.line(&out.paint(
            &format!(
                "No {} need review ({} total).",
                list.plural(),
                db.entries.len()
            ),
            Style::Dim,
        ));
        return;
    }
    for e in &pending {
        out.line(&out.paint(&e.canonical, Style::Bold));
        if let Some(ev) = &e.evidence {
            out.line(&format!("  {}", ev.reason));
            let mut close = close_candidates(e).peekable();
            if close.peek().is_none() {
                out.line(&format!("  no close match among known {}", list.plural()));
            }
            for c in close {
                out.line(&format!("  candidate  {}  ({:.2})", c.name, c.score));
            }
            if !ev.source.is_empty() {
                out.line(&out.paint(
                    &format!("  from {} (batch {})", ev.source, ev.batch),
                    Style::Dim,
                ));
            }
        }
        for cmd in review_commands(out, list, e) {
            out.line(&format!("  $ {cmd}"));
        }
    }
    out.line(&out.paint(
        &format!("{} of {} need review", pending.len(), db.entries.len()),
        Style::Dim,
    ));
}

/// Index of the entry `name` (any spelling core normalizes to it) belongs to.
fn entry_of(db: &harmonize::NameDb, name: &str) -> Result<usize> {
    harmonize::index(db)
        .get(&harmonize::normalize(db.kind(), name))
        .copied()
        .ok_or_else(|| Error::General(format!("entry {name:?} not found after saving")))
}

fn names(a: &NamesArgs, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let list = name_list(a, ctx)?;
    let (file, field) = (&list.file, list.field.as_str());
    let out = ctx.out;
    // Writers use the locked reload-modify-save (`update_db`).
    match &a.cmd {
        NamesCmd::List => {
            let db = harmonize::load_db(file, field)?;
            print_entries(out, &list, &db.entries.iter().collect::<Vec<_>>());
        }
        NamesCmd::Rename { old, new } => {
            let db = harmonize::update_db(file, field, |db| {
                harmonize::rename(db, old, new)?;
                Ok(db.clone())
            })?;
            // Core stores NFC; find the entry by its normalized spelling.
            let i = entry_of(&db, new)?;
            print_entries(out, &list, &[&db.entries[i]]);
        }
        NamesCmd::Review { accept, accept_all } => {
            let (names, db) = harmonize::update_db(file, field, |db| {
                let names: Vec<String> = if *accept_all {
                    db.entries
                        .iter()
                        .filter(|e| e.review)
                        .map(|e| e.canonical.clone())
                        .collect()
                } else {
                    accept.clone()
                };
                for n in &names {
                    harmonize::set_review(db, n, false)?;
                }
                Ok((names, db.clone()))
            })?;
            if !names.is_empty() {
                out.note(&format!("Accepted {} entr(y/ies).", names.len()));
            }
            print_review(out, &list, &db, &names);
        }
        NamesCmd::Merge { from, into } => {
            let db = harmonize::update_db(file, field, |db| {
                harmonize::merge(db, from, into)?;
                Ok(db.clone())
            })?;
            let i = db.find(into).expect("merge target exists");
            print_entries(out, &list, &[&db.entries[i]]);
        }
        NamesCmd::Add { name, alias } => {
            let db = harmonize::update_db(file, field, |db| {
                harmonize::add_user(db, name, alias)?;
                Ok(db.clone())
            })?;
            let i = entry_of(&db, name)?;
            print_entries(out, &list, &[&db.entries[i]]);
        }
    }
    Ok(ExitCode::Success)
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

pub(crate) const STARTER_CONFIG: &str = include_str!("starter-config.yaml");

fn source_label(s: ConfigSource) -> &'static str {
    match s {
        ConfigSource::Cli => "--config",
        ConfigSource::Env => "$AUTORENAME_PDF_CONFIG",
        ConfigSource::User => "OS config dir",
    }
}

fn config_cmd(c: &ConfigCmd, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let out = ctx.out;
    let loc = ctx.location();
    match c {
        ConfigCmd::Show => {
            let loaded = config::load_with_overrides(&loc.path, &ctx.overrides())?;
            let redacted = config::redact(&loaded.tree);
            if out.is_text() {
                out.line(&out.paint(&format!("# {}", loc.path.display()), Style::Dim));
                let yaml = serde_saphyr::to_string(&redacted)
                    .map_err(|e| Error::General(e.to_string()))?;
                out.line(yaml.trim_end());
            } else {
                out.document(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "path": fsx::path_serde::to_value(&loc.path),
                    "config": redacted,
                }));
            }
            Ok(ExitCode::Success)
        }
        ConfigCmd::Validate => {
            let loaded = config::load_with_overrides(&loc.path, &ctx.overrides())?;
            let v = config::validate_loaded(&loaded, None);
            if out.is_text() {
                for i in &v.issues {
                    let tag = match i.level {
                        Level::Error => out.paint("error  ", Style::Red),
                        Level::Warning => out.paint("warning", Style::Yellow),
                    };
                    out.line(&format!("  {tag}  {}", i.message));
                    if let Some(h) = &i.hint {
                        out.line(&format!("           {}", out.paint(h, Style::Dim)));
                    }
                }
                let verdict = if v.valid {
                    out.paint("valid", Style::Green)
                } else {
                    out.paint("invalid", Style::Red)
                };
                out.line(&format!(
                    "{} is {verdict} (profile {})",
                    loc.path.display(),
                    v.profile.as_deref().unwrap_or("-")
                ));
            } else {
                out.document(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "path": fsx::path_serde::to_value(&loc.path),
                    "valid": v.valid,
                    "issues": v.issues,
                    "profile": v.profile,
                    "resolved_profile": v.resolved_profile,
                }));
            }
            Ok(if v.valid {
                ExitCode::Success
            } else {
                ExitCode::Config
            })
        }
        ConfigCmd::Path => {
            let dir = std::path::absolute(&loc.path)
                .unwrap_or_else(|_| loc.path.clone())
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let entries: Vec<(&str, Option<PathBuf>)> = vec![
                (
                    "config",
                    Some(std::path::absolute(&loc.path).unwrap_or(loc.path.clone())),
                ),
                ("env_file", Some(dir.join(".env"))),
                ("name_lists", Some(config::name_lists_dir(&loc.path))),
                ("data", config::app_data_dir()),
                ("journal", journal::default_journal_path()),
                ("models", ocr::models::default_root()),
            ];
            if out.is_text() {
                for (k, p) in &entries {
                    let (shown, state) = match p {
                        Some(p) => (
                            p.display().to_string(),
                            if p.exists() { "" } else { "  (missing)" },
                        ),
                        None => ("-".into(), ""),
                    };
                    let extra = if *k == "config" {
                        format!("  [{}]", source_label(loc.source))
                    } else {
                        String::new()
                    };
                    out.line(&format!(
                        "  {}  {shown}{}{}",
                        pad(k, 10),
                        out.paint(state, Style::Yellow),
                        out.paint(&extra, Style::Dim)
                    ));
                }
            } else {
                let mut m = serde_json::Map::new();
                m.insert("schema_version".into(), json!(SCHEMA_VERSION));
                for (k, p) in &entries {
                    m.insert(
                        (*k).into(),
                        json!({
                            "path": p.as_deref().map(fsx::path_serde::to_value),
                            "exists": p.as_ref().is_some_and(|p| p.exists()),
                        }),
                    );
                }
                m.insert("config_source".into(), json!(loc.source));
                out.document(&Value::Object(m));
            }
            Ok(ExitCode::Success)
        }
        ConfigCmd::Init { force } => {
            let path = &loc.path;
            if path.exists() && !force {
                return Err(Error::Conflict {
                    message: format!("{} already exists", path.display()),
                });
            }
            fsx::write_atomic(path, STARTER_CONFIG.as_bytes()).map_err(|e| Error::io(path, e))?;
            let env_file = path.with_file_name(".env");
            if out.is_text() {
                out.line(&format!("Wrote {}", path.display()));
                out.note(&format!(
                    "Next: put OPENAI_API_KEY=... into {}, set company.names, then `{}`.",
                    env_file.display(),
                    out.command(&["config", "validate"])
                ));
            } else {
                out.document(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "written": fsx::path_serde::to_value(path),
                    "env_file": fsx::path_serde::to_value(&env_file),
                }));
            }
            Ok(ExitCode::Success)
        }
    }
}

// ---------------------------------------------------------------------------
// models
// ---------------------------------------------------------------------------

fn resolve_pack(ctx: &Ctx<'_>, id: Option<&str>) -> Result<&'static ocr::Pack> {
    if let Some(id) = id {
        return ocr::pack(id).ok_or_else(|| Error::Usage {
            message: format!("unknown model pack {id:?}"),
            hint: Some(format!(
                "Available: {}",
                ocr::PACKS
                    .iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        });
    }
    let lang = config::load(&ctx.location().path)
        .ok()
        .and_then(|l| {
            l.tree
                .pointer("/ocr/language")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| ocr::AUTO_LANGUAGE.into());
    ocr::pack_for_language_or_err(&lang)
}

fn models(c: &ModelsCmd, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let out = ctx.out;
    let root = models_root()?;
    let status_doc = |pack: &ocr::Pack| {
        ocr::models::list(&root)
            .into_iter()
            .find(|s| s.pack == pack.id)
    };
    match c {
        ModelsCmd::List => {
            let packs = ocr::models::list(&root);
            if out.is_text() {
                for p in &packs {
                    let state = if p.installed {
                        out.paint("installed", Style::Green)
                    } else {
                        out.paint("not installed", Style::Yellow)
                    };
                    out.line(&format!(
                        "  {}  {state}  {:.1} MB  {}",
                        out.paint(&pad(p.pack, 8), Style::Bold),
                        p.total_size as f64 / 1e6,
                        p.description
                    ));
                }
                out.line(&out.paint(
                    &format!(
                        "{} · OCR engine {}",
                        root.display(),
                        if ocr::ENGINE_AVAILABLE {
                            "built in"
                        } else {
                            "not in this build"
                        }
                    ),
                    Style::Dim,
                ));
            } else {
                out.document(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "root": fsx::path_serde::to_value(&root),
                    "engine_available": ocr::ENGINE_AVAILABLE,
                    "packs": packs,
                }));
            }
        }
        ModelsCmd::Fetch { pack } => {
            let offline = ctx.g.offline
                || config::load(&ctx.location().path).is_ok_and(|l| {
                    l.tree.get("privacy").and_then(|v| v.as_str()) == Some("offline")
                });
            let pack = resolve_pack(ctx, pack.as_deref())?;
            if offline {
                return Err(Error::config_hint(
                    "privacy: offline forbids downloading OCR models",
                    models_hint(ctx.out, true, pack),
                ));
            }
            ocr::models::fetch(
                &root,
                pack,
                &mut |p| {
                    out.progress_bytes(&format!("Downloading {}", p.file), p.downloaded, p.total)
                },
                None,
            )?;
            out.finish_progress();
            emit_pack(out, status_doc(pack), "installed");
        }
        ModelsCmd::Import { dir, pack } => {
            let pack = resolve_pack(ctx, pack.as_deref())?;
            ocr::models::import(&root, pack, dir)?;
            emit_pack(out, status_doc(pack), "imported");
        }
        ModelsCmd::Remove { pack } => {
            let pack = resolve_pack(ctx, Some(pack))?;
            ocr::models::remove(&root, pack)?;
            emit_pack(out, status_doc(pack), "removed");
        }
    }
    Ok(ExitCode::Success)
}

fn emit_pack(out: &Out, status: Option<ocr::PackStatus>, verb: &str) {
    match status {
        Some(s) if out.is_text() => out.line(&format!(
            "{} {verb} ({})",
            out.paint(s.pack, Style::Bold),
            s.dir.display()
        )),
        Some(s) => {
            out.document(&json!({"schema_version": SCHEMA_VERSION, "action": verb, "pack": s}))
        }
        None => {}
    }
}

// ---------------------------------------------------------------------------
// schema
// ---------------------------------------------------------------------------

fn schema_value(kind: SchemaKind) -> Value {
    let v = match kind {
        SchemaKind::Result => serde_json::to_value(schemars::schema_for!(ApplyReport)),
        SchemaKind::Plan => serde_json::to_value(schemars::schema_for!(Plan)),
        SchemaKind::Error => serde_json::to_value(schemars::schema_for!(ErrorDocument)),
        SchemaKind::Event => serde_json::to_value(schemars::schema_for!(Event)),
        SchemaKind::Inspect => serde_json::to_value(schemars::schema_for!(InspectReport)),
        SchemaKind::Config => Ok(config::json_schema()),
        SchemaKind::Setup => serde_json::to_value(schemars::schema_for!(crate::setup::Report)),
        SchemaKind::Integrate => {
            serde_json::to_value(schemars::schema_for!(crate::integrate::Report))
                .map(with_schema_version)
        }
        SchemaKind::Undo => serde_json::to_value(schemars::schema_for!(journal::UndoReport))
            .map(with_schema_version),
    };
    v.unwrap_or(Value::Null)
}

/// Documents wrapped by `doc()` carry a top-level `schema_version`.
fn with_schema_version(mut v: Value) -> Value {
    if let Some(props) = v.get_mut("properties").and_then(Value::as_object_mut) {
        props.insert(
            "schema_version".into(),
            json!({"type": "integer", "const": SCHEMA_VERSION}),
        );
    }
    if let Some(req) = v.get_mut("required").and_then(Value::as_array_mut) {
        req.push(json!("schema_version"));
    }
    v
}

fn schema(a: &SchemaArgs, ctx: &Ctx<'_>) -> Result<ExitCode> {
    let v = match a.kind {
        Some(k) => schema_value(k),
        None => {
            let mut m = serde_json::Map::new();
            for (name, k) in [
                ("result", SchemaKind::Result),
                ("plan", SchemaKind::Plan),
                ("config", SchemaKind::Config),
                ("error", SchemaKind::Error),
                ("event", SchemaKind::Event),
                ("inspect", SchemaKind::Inspect),
                ("setup", SchemaKind::Setup),
                ("integrate", SchemaKind::Integrate),
                ("undo", SchemaKind::Undo),
            ] {
                m.insert(name.into(), schema_value(k));
            }
            Value::Object(m)
        }
    };
    // Schemas are JSON in every output mode (one compact line for ndjson).
    if ctx.out.is_text() {
        let text = serde_json::to_string_pretty(&v).map_err(|e| Error::General(e.to_string()))?;
        ctx.out.line(&text);
    } else {
        ctx.out.document(&v);
    }
    Ok(ExitCode::Success)
}

// ---------------------------------------------------------------------------
// integrate
// ---------------------------------------------------------------------------

fn integrate(c: &IntegrateCmd, ctx: &Ctx<'_>) -> Result<ExitCode> {
    use crate::integrate::{self, Action, State};
    let (action, a) = match c {
        IntegrateCmd::Install(a) => (Action::Install, a),
        IntegrateCmd::Remove(a) => (Action::Remove, a),
        IntegrateCmd::Status(a) => (Action::Status, a),
    };
    let Scope::User = a.scope;
    let report = integrate::run(action, a.exe.as_deref())?;
    if !ctx.out.is_text() {
        ctx.out.document(&doc(&report));
    } else {
        ctx.out.line(&format!(
            "{} ({} scope) -> {}",
            report.platform, report.scope, report.command
        ));
        for i in &report.items {
            let (word, style) = match i.state {
                State::Installed => ("installed", Style::Green),
                State::Outdated => ("outdated", Style::Yellow),
                State::Missing => ("missing", Style::Dim),
                State::Foreign => ("foreign", Style::Yellow),
                State::Skipped => ("skipped", Style::Dim),
            };
            let state = ctx.out.paint(&pad(word, 9), style);
            let action = i.action.map(|a| format!(" ({a})")).unwrap_or_default();
            ctx.out.line(&format!(
                "  {state} {}  {}{action}",
                pad(&i.kind, 20),
                i.path
            ));
            if let Some(n) = &i.note {
                ctx.out.note(&format!("{}: {n}", i.path));
            }
        }
        for n in &report.notes {
            ctx.out.note(n);
        }
    }
    let kept = report.items.iter().any(|i| i.action == Some("kept"));
    Ok(if kept && action != Action::Status {
        ExitCode::Partial
    } else {
        ExitCode::Success
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_style_wildcards_expand_case_insensitively() {
        assert!(wildcard_match("*.pdf", "Scan.PDF"));
        assert!(wildcard_match("inv?ice*.pdf", "INVOICE-2026.pdf"));
        assert!(!wildcard_match("*.pdf", "notes.txt"));
        assert!(wildcard_match("a*b*c", "aXXbYYc") && !wildcard_match("a*b*c", "aXXbYY"));
        let list = |d: &Path| -> Vec<String> {
            assert_eq!(d, Path::new("docs"));
            vec!["b.PDF".into(), "a.pdf".into(), "c.txt".into()]
        };
        let got = expand_wildcards(
            &[
                PathBuf::from("docs/*.pdf"),
                PathBuf::from("plain.pdf"),
                PathBuf::from("docs/*.xls"),
            ],
            &list,
        );
        assert_eq!(
            got,
            [
                PathBuf::from("docs/a.pdf"),
                PathBuf::from("docs/b.PDF"),
                PathBuf::from("plain.pdf"),
                PathBuf::from("docs/*.xls")
            ]
        );
    }

    #[test]
    fn plans_decode_from_utf8_bom_and_utf16() {
        let text = "{\"a\": \"Müller\"}";
        assert_eq!(decode_text(text.as_bytes()).unwrap(), text);
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(text.as_bytes());
        assert_eq!(decode_text(&bom).unwrap(), text);
        let mut le = vec![0xFF, 0xFE];
        le.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_text(&le).unwrap(), text);
        let mut be = vec![0xFE, 0xFF];
        be.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(decode_text(&be).unwrap(), text);
        assert!(decode_text(&[0xFF, 0xFE, 0x41]).is_err());
        assert!(decode_text(&[0xC3]).is_err());
    }

    #[test]
    fn starter_config_is_valid_except_for_the_key() {
        let raw = config::parse_yaml(STARTER_CONFIG).unwrap();
        let env = |n: &str| (n == "OPENAI_API_KEY").then(|| "sk-test".to_string());
        let (tree, notices) = config::from_raw(raw, &env).unwrap();
        assert!(notices.is_empty());
        let v = config::validate(&tree, None);
        assert!(v.valid, "{:?}", v.issues);
    }
}
