//! Linux/freedesktop integration. Generators are pure (tested everywhere);
//! [`run`] applies them under `$XDG_DATA_HOME` / `$XDG_CONFIG_HOME`.
//!
//! | Host | Entry | Icon |
//! |---|---|---|
//! | menus/launchers | `applications/autorename-pdf.desktop` (MIME + 2 actions) | hicolor `autorename-pdf` |
//! | Dolphin | `kio/servicemenus/autorename-pdf.desktop` (+x, submenu) | yes |
//! | Nemo | `nemo/actions/autorename-pdf-<verb>.nemo_action` | yes |
//! | Thunar | `<action>`s merged into `~/.config/Thunar/uca.xml` (submenu) | yes |
//! | Nautilus | `nautilus/scripts/AutoRename-PDF: <verb>` (Scripts submenu) | no: the host ignores icons for scripts and extensions (`MenuItem.icon` has no effect since API 4.1) |
//!
//! A native libnautilus-extension is deliberately not built (YAGNI): it
//! needs a distro-ABI cdylib outside the AppImage and still shows no icon.

#![cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};

use super::{
    APP_ID, Action, FileEntry, Item, MARKER, OwnedLog, PRODUCT, Report, State, Verb, io_err,
    shell_quote,
};

/// Rendered by `cargo xtask icons`.
const HICOLOR: &[(u32, &[u8])] = &[
    (
        16,
        include_bytes!("../../../../assets/icons/generated/hicolor/16x16/apps/autorename-pdf.png"),
    ),
    (
        22,
        include_bytes!("../../../../assets/icons/generated/hicolor/22x22/apps/autorename-pdf.png"),
    ),
    (
        24,
        include_bytes!("../../../../assets/icons/generated/hicolor/24x24/apps/autorename-pdf.png"),
    ),
    (
        32,
        include_bytes!("../../../../assets/icons/generated/hicolor/32x32/apps/autorename-pdf.png"),
    ),
    (
        48,
        include_bytes!("../../../../assets/icons/generated/hicolor/48x48/apps/autorename-pdf.png"),
    ),
    (
        64,
        include_bytes!("../../../../assets/icons/generated/hicolor/64x64/apps/autorename-pdf.png"),
    ),
    (
        128,
        include_bytes!(
            "../../../../assets/icons/generated/hicolor/128x128/apps/autorename-pdf.png"
        ),
    ),
    (
        256,
        include_bytes!(
            "../../../../assets/icons/generated/hicolor/256x256/apps/autorename-pdf.png"
        ),
    ),
    (
        512,
        include_bytes!(
            "../../../../assets/icons/generated/hicolor/512x512/apps/autorename-pdf.png"
        ),
    ),
];
const SCALABLE: &[u8] =
    include_bytes!("../../../../assets/icons/generated/hicolor/scalable/apps/autorename-pdf.svg");

const COMMENT: &str = "Rename PDFs from their content (AI + OCR)";
const MIME: &str = "application/pdf;inode/directory;";
const THUNAR_ID: &str = "autorename-pdf-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Dolphin,
    Nemo,
    Thunar,
    Nautilus,
}

impl Host {
    pub fn binary(self) -> &'static str {
        match self {
            Host::Dolphin => "dolphin",
            Host::Nemo => "nemo",
            Host::Thunar => "thunar",
            Host::Nautilus => "nautilus",
        }
    }
}

/// Base directories (XDG); injectable for tests.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub data: PathBuf,
    pub config: PathBuf,
    /// `~/.local/bin` (systemd file-hierarchy user executables).
    pub bin: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Option<Dirs> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let xdg = |var: &str, fallback: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| home.as_ref().map(|h| h.join(fallback)))
        };
        Some(Dirs {
            data: xdg("XDG_DATA_HOME", ".local/share")?,
            config: xdg("XDG_CONFIG_HOME", ".config")?,
            bin: home.as_ref()?.join(".local/bin"),
        })
    }

    pub fn thunar_uca(&self) -> PathBuf {
        self.config.join("Thunar/uca.xml")
    }
}

// ---------------------------------------------------------------------------
// Quoting
// ---------------------------------------------------------------------------

/// One `Exec=` argument: Desktop Entry spec quoting (reserved characters →
/// double quotes with `" ` $ \` escaped), `%` doubled, then the general
/// string-value escape for backslashes.
pub fn exec_arg(s: &str) -> String {
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    let quoted = if s.chars().any(reserved) {
        let mut o = String::from("\"");
        for c in s.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                o.push('\\');
            }
            o.push(c);
        }
        o.push('"');
        o
    } else {
        s.to_string()
    };
    quoted.replace('%', "%%").replace('\\', "\\\\")
}

fn exec_line(exe: &str, verb: Verb, field: &str) -> String {
    let mut parts = vec![exec_arg(exe)];
    parts.extend(verb.args().iter().map(|a| a.to_string()));
    parts.push(field.into());
    parts.join(" ")
}

fn shell_line(exe: &str, verb: Verb, field: &str) -> String {
    let mut parts = vec![shell_quote(exe)];
    parts.extend(verb.args().iter().map(|a| a.to_string()));
    parts.push(field.into());
    parts.join(" ")
}

/// GKeyFile string-value escaping (Nemo reads `Exec` with
/// `g_key_file_get_string`): `\\`, line breaks, tabs, leading space.
pub fn keyfile_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            ' ' if i == 0 => o.push_str("\\s"),
            c => o.push(c),
        }
    }
    o
}

/// Nemo and Thunar expand `%x` tokens across the whole command before
/// shell parsing, so quoting cannot protect a `%` in the executable path.
/// Such paths are launched through an owned wrapper script instead.
pub fn needs_wrapper(exe: &str) -> bool {
    exe.contains('%')
}

/// `<data>/autorename-pdf/autorename-pdf-launch`: `exec <exe> "$@"`.
pub fn wrapper_script(exe: &str) -> String {
    format!(
        "#!/bin/sh
# {MARKER}: launcher for file-manager actions (the real path contains `%`).
exec {} \"$@\"
",
        shell_quote(exe)
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Generators (pure)
// ---------------------------------------------------------------------------

/// `applications/autorename-pdf.desktop`.
pub fn desktop_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]
Type=Application
Version=1.5
Name={PRODUCT}
GenericName=PDF Renamer
Comment={COMMENT}
Exec={main}
Icon={APP_ID}
Terminal=false
Categories=Office;
MimeType={MIME}
Keywords=PDF;rename;invoice;OCR;
StartupNotify=true
StartupWMClass={APP_ID}
Actions=rename-now;preview;
{MARKER}=1

[Desktop Action rename-now]
Name={rename}
Exec={rename_exec}

[Desktop Action preview]
Name={preview}
Exec={preview_exec}
",
        main = exec_line(exe, Verb::Preview, "%F"),
        rename = Verb::RenameNow.label(),
        rename_exec = exec_line(exe, Verb::RenameNow, "%F"),
        preview = Verb::Preview.label(),
        preview_exec = exec_line(exe, Verb::Preview, "%F"),
    )
}

/// Dolphin (KF5/KF6) service menu with an "AutoRename-PDF" submenu.
pub fn dolphin_menu(exe: &str) -> String {
    format!(
        "[Desktop Entry]
Type=Service
X-KDE-ServiceTypes=KonqPopupMenu/Plugin
MimeType={MIME}
Actions=renameNow;preview;
X-KDE-Submenu={PRODUCT}
Icon={APP_ID}
{MARKER}=1

[Desktop Action renameNow]
Name={rename}
Icon={APP_ID}
Exec={rename_exec}

[Desktop Action preview]
Name={preview}
Icon={APP_ID}
Exec={preview_exec}
",
        rename = Verb::RenameNow.label(),
        rename_exec = exec_line(exe, Verb::RenameNow, "%F"),
        preview = Verb::Preview.label(),
        preview_exec = exec_line(exe, Verb::Preview, "%F"),
    )
}

/// One Nemo action file per verb (Nemo has no submenus for actions).
pub fn nemo_action(exe: &str, verb: Verb) -> String {
    format!(
        "[Nemo Action]
Name={PRODUCT}: {label}
Comment={COMMENT}
Exec={exec}
Icon-Name={APP_ID}
Selection=notnone
Extensions=pdf;dir;
Quote=double
{MARKER}=1
",
        label = verb.label(),
        exec = keyfile_escape(&shell_line(exe, verb, "%F")),
    )
}

/// Nautilus script (Scripts submenu). Nautilus passes the selected local
/// files as arguments (names relative to the folder it runs the script in).
/// `NAUTILUS_SCRIPT_SELECTED_FILE_PATHS` is newline-separated and therefore
/// ambiguous for names containing a newline: never used. No arguments
/// (non-local selection) → do nothing rather than guess.
pub fn nautilus_script(exe: &str, verb: Verb) -> String {
    let mut cmd = vec![shell_quote(exe)];
    cmd.extend(verb.args().iter().map(|a| a.to_string()));
    format!(
        "#!/bin/sh
# {MARKER}: written by `autorename-pdf integrate install`,
# deleted by `autorename-pdf integrate remove`.
[ \"$#\" -gt 0 ] || exit 0
exec {} -- \"$@\"
",
        cmd.join(" ")
    )
}

pub fn nautilus_script_name(verb: Verb) -> String {
    match verb {
        Verb::RenameNow => format!("{PRODUCT}: Rename now"),
        Verb::Preview => format!("{PRODUCT}: Preview"),
    }
}

/// Our Thunar `<action>` blocks (PDF files and folders per verb), tab
/// indented like Thunar writes them.
pub fn thunar_actions(exe: &str) -> String {
    let mut out = String::new();
    for verb in Verb::ALL {
        for (target, patterns, kind) in [
            ("pdf", "*.pdf;*.PDF", "<other-files/>"),
            ("dir", "*", "<directories/>"),
        ] {
            out.push_str(&format!(
                "\t<action>
\t\t<icon>{APP_ID}</icon>
\t\t<name>{label}</name>
\t\t<submenu>{PRODUCT}</submenu>
\t\t<unique-id>{THUNAR_ID}{id}-{target}</unique-id>
\t\t<command>{cmd}</command>
\t\t<description>{COMMENT}</description>
\t\t<range>*</range>
\t\t<patterns>{patterns}</patterns>
\t\t{kind}
\t</action>
",
                label = xml_escape(verb.label()),
                id = verb.id(),
                cmd = xml_escape(&shell_line(exe, verb, "%F")),
            ));
        }
    }
    out
}

const THUNAR_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n";
const THUNAR_TAIL: &str = "</actions>\n";

/// Remove our `<action>` blocks (identified by the unique-id prefix) from a
/// uca.xml document. Returns the new text and the number removed.
pub fn thunar_strip(doc: &str) -> (String, usize) {
    let mut out = String::with_capacity(doc.len());
    let mut rest = doc;
    let mut removed = 0;
    while let Some(start) = rest.find("<action>") {
        let Some(len) = rest[start..].find("</action>") else {
            break;
        };
        let end = start + len + "</action>".len();
        let block = &rest[start..end];
        if block.contains(&format!("<unique-id>{THUNAR_ID}")) {
            // Drop the block with its own line's indentation and newline.
            let line_start = rest[..start].trim_end_matches([' ', '\t']).len();
            out.push_str(&rest[..line_start]);
            let after = &rest[end..];
            rest = after.strip_prefix('\n').unwrap_or(after);
            removed += 1;
        } else {
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    (out, removed)
}

/// Merge our actions into an existing uca.xml (or create one). Foreign
/// actions are kept byte for byte; a document without `</actions>` is
/// refused rather than rewritten.
pub fn thunar_merge(existing: Option<&str>, exe: &str) -> Result<String, String> {
    let ours = thunar_actions(exe);
    let Some(doc) = existing.filter(|d| !d.trim().is_empty()) else {
        return Ok(format!("{THUNAR_HEAD}{ours}{THUNAR_TAIL}"));
    };
    let (doc, _) = thunar_strip(doc);
    let Some(at) = doc.rfind("</actions>") else {
        return Err("uca.xml has no </actions> element; not modifying it".into());
    };
    let line_start = doc[..at].rfind('\n').map_or(0, |i| i + 1);
    let (head, tail) = if doc[line_start..at].trim().is_empty() {
        doc.split_at(line_start)
    } else {
        doc.split_at(at)
    };
    let sep = if head.ends_with('\n') { "" } else { "\n" };
    Ok(format!("{head}{sep}{ours}{tail}"))
}

/// Our entry would be the only content: `remove` deletes the file.
fn thunar_is_empty_skeleton(doc: &str) -> bool {
    doc == format!("{THUNAR_HEAD}{THUNAR_TAIL}")
}

pub fn launcher_path(d: &Dirs) -> PathBuf {
    d.data.join(format!("{APP_ID}/{APP_ID}-launch"))
}

/// The executable Nemo/Thunar commands use.
pub fn token_free_exe(d: &Dirs, exe: &str) -> String {
    if needs_wrapper(exe) {
        launcher_path(d).to_string_lossy().into_owned()
    } else {
        exe.to_string()
    }
}

/// Every file entry with the host it belongs to (`None` = always).
pub fn entries(d: &Dirs, exe: &str) -> Vec<(Option<Host>, FileEntry)> {
    let text = |kind, path: PathBuf, body: String, executable| FileEntry {
        kind,
        path,
        bytes: body.into_bytes(),
        executable,
        owned_by_name: false,
    };
    let data = &d.data;
    let mut v = vec![(
        None,
        text(
            "desktop-entry",
            data.join(format!("applications/{APP_ID}.desktop")),
            desktop_entry(exe),
            false,
        ),
    )];
    let icons = data.join("icons");
    for (n, bytes) in HICOLOR {
        v.push((
            None,
            FileEntry {
                kind: "icon",
                path: icons.join(format!("hicolor/{n}x{n}/apps/{APP_ID}.png")),
                bytes: bytes.to_vec(),
                executable: false,
                owned_by_name: true,
            },
        ));
    }
    v.push((
        None,
        FileEntry {
            kind: "icon",
            path: icons.join(format!("hicolor/scalable/apps/{APP_ID}.svg")),
            bytes: SCALABLE.to_vec(),
            executable: false,
            owned_by_name: true,
        },
    ));
    v.push((
        Some(Host::Dolphin),
        text(
            "dolphin-service-menu",
            data.join(format!("kio/servicemenus/{APP_ID}.desktop")),
            dolphin_menu(exe),
            true,
        ),
    ));
    // Token-free executable for Nemo/Thunar (`%` in the real path).
    if needs_wrapper(exe) {
        v.push((
            None,
            FileEntry {
                kind: "launcher",
                path: launcher_path(d),
                bytes: wrapper_script(exe).into_bytes(),
                executable: true,
                owned_by_name: false,
            },
        ));
    }
    let token_free = token_free_exe(d, exe);
    for verb in Verb::ALL {
        v.push((
            Some(Host::Nemo),
            text(
                "nemo-action",
                data.join(format!("nemo/actions/{APP_ID}-{}.nemo_action", verb.id())),
                nemo_action(&token_free, verb),
                false,
            ),
        ));
    }
    for verb in Verb::ALL {
        v.push((
            Some(Host::Nautilus),
            text(
                "nautilus-script",
                data.join("nautilus/scripts")
                    .join(nautilus_script_name(verb)),
                nautilus_script(exe, verb),
                true,
            ),
        ));
    }
    v
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

pub fn in_path(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(bin))
            .find(|f| f.is_file())
    })
}

fn host_present(h: Host) -> bool {
    in_path(h.binary()).is_some()
}

fn thunar_item(path: &Path, state: State, action: Option<&'static str>) -> Item {
    Item {
        kind: "thunar-action".into(),
        path: path.display().to_string(),
        state,
        action,
        note: None,
    }
}

fn thunar_state(doc: Option<&str>, exe: &str) -> State {
    let Some(doc) = doc else {
        return State::Missing;
    };
    match thunar_strip(doc).1 {
        0 => State::Missing,
        _ if doc.contains(&thunar_actions(exe)) => State::Installed,
        _ => State::Outdated,
    }
}

fn thunar(
    action: Action,
    path: &Path,
    exe: &str,
    present: bool,
    log: &mut OwnedLog,
) -> autorename_core::Result<Item> {
    // Only a missing file counts as absent. Unreadable or non-UTF-8 files
    // are reported and never rewritten (a rewrite would drop their actions).
    let doc = match std::fs::read(path) {
        Ok(b) => match String::from_utf8(b) {
            Ok(s) => Some(s),
            Err(_) => {
                let mut i = thunar_item(
                    path,
                    State::Foreign,
                    (action != Action::Status).then_some("kept"),
                );
                i.note = Some("uca.xml is not UTF-8; not modifying it".into());
                return Ok(i);
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_err(path, e)),
    };
    let st = thunar_state(doc.as_deref(), exe);
    let mut write = |s: &str| -> autorename_core::Result<()> {
        if let Some(dir) = path.parent() {
            log.create_dir_all(dir).map_err(|e| io_err(dir, e))?;
        }
        autorename_core::fsx::write_atomic(path, s.as_bytes()).map_err(|e| io_err(path, e))?;
        log.save().map_err(|e| io_err(path, e))
    };
    Ok(match action {
        Action::Status if !present && st == State::Missing => {
            thunar_item(path, State::Skipped, None)
        }
        Action::Status => thunar_item(path, st, None),
        Action::Install if !present => thunar_item(path, State::Skipped, None),
        Action::Install if st == State::Installed => {
            thunar_item(path, State::Installed, Some("unchanged"))
        }
        Action::Install => match thunar_merge(doc.as_deref(), exe) {
            Ok(s) => {
                write(&s)?;
                let mut i = thunar_item(path, State::Installed, Some("written"));
                i.note = Some("restart Thunar (`thunar -q`) to load the actions".into());
                i
            }
            Err(e) => {
                let mut i = thunar_item(path, State::Foreign, Some("kept"));
                i.note = Some(e);
                i
            }
        },
        Action::Remove => match doc {
            Some(d) if st != State::Missing => {
                let (rest, _) = thunar_strip(&d);
                if thunar_is_empty_skeleton(&rest) {
                    std::fs::remove_file(path).map_err(|e| io_err(path, e))?;
                } else {
                    write(&rest)?;
                }
                thunar_item(path, State::Missing, Some("removed"))
            }
            _ => thunar_item(path, State::Missing, None),
        },
    })
}

/// Refresh a cache only where one already exists: without a cache the hosts
/// scan the directories themselves, and creating one would leave a file
/// behind after `remove`.
fn refresh_caches(d: &Dirs) {
    let run = |bin: &str, args: &[&std::ffi::OsStr]| {
        if let Some(b) = in_path(bin) {
            let _ = std::process::Command::new(b)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    };
    let hicolor = d.data.join("icons/hicolor");
    if hicolor.join("icon-theme.cache").is_file() {
        run(
            "gtk-update-icon-cache",
            &[
                "-f".as_ref(),
                "-t".as_ref(),
                "-q".as_ref(),
                hicolor.as_os_str(),
            ],
        );
    }
    let apps = d.data.join("applications");
    if apps.join("mimeinfo.cache").is_file() {
        run(
            "update-desktop-database",
            &["-q".as_ref(), apps.as_os_str()],
        );
    }
}

/// `~/.local/bin/autorename-pdf` → the executable, so the CLI is on PATH
/// for AppImage/portable installs. Skipped when `autorename-pdf` on PATH
/// already is this executable (deb/pacman). Owned only via the log.
fn cli_link(
    action: Action,
    d: &Dirs,
    exe: &str,
    which: &dyn Fn(&str) -> Option<PathBuf>,
    log: &mut OwnedLog,
) -> autorename_core::Result<Item> {
    let link = d.bin.join(APP_ID);
    let exe_p = Path::new(exe);
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    };
    let current = std::fs::symlink_metadata(&link).ok();
    let target = std::fs::read_link(&link).ok();
    let recorded = log.links.get(&link).cloned();
    // Ours: recorded by us, or (record lost) a symlink to exactly this exe.
    let ours = current.as_ref().is_some_and(|m| m.file_type().is_symlink())
        && (recorded.is_some() && target == recorded || target.as_deref() == Some(exe_p));
    let mut it = thunar_item(&link, State::Missing, None);
    it.kind = "cli-link".into();
    let on_path = which(APP_ID).is_some_and(|p| same(&p, exe_p));
    it.state = match (&current, ours) {
        (None, _) => State::Missing,
        (Some(_), true) if target.as_deref() == Some(exe_p) => State::Installed,
        (Some(_), true) => State::Outdated,
        (Some(_), false) => State::Foreign,
    };
    let write = |log: &mut OwnedLog| -> autorename_core::Result<()> {
        log.create_dir_all(&d.bin).map_err(|e| io_err(&d.bin, e))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(exe_p, &link).map_err(|e| io_err(&link, e))?;
        log.links.insert(link.clone(), exe_p.to_path_buf());
        log.save().map_err(|e| io_err(&link, e))
    };
    match action {
        Action::Status if it.state == State::Missing && on_path => {
            it.state = State::Skipped;
            it.note = Some("autorename-pdf on PATH already is this executable".into());
        }
        Action::Status => {}
        Action::Install => match it.state {
            State::Installed => {
                if recorded.is_none() {
                    log.links.insert(link.clone(), exe_p.to_path_buf());
                    log.save().map_err(|e| io_err(&link, e))?;
                }
                it.action = Some("unchanged");
            }
            State::Missing if on_path => {
                it.state = State::Skipped;
                it.note = Some("autorename-pdf on PATH already is this executable".into());
            }
            State::Missing => {
                write(log)?;
                it.state = State::Installed;
                it.action = Some("written");
            }
            State::Outdated => {
                std::fs::remove_file(&link).map_err(|e| io_err(&link, e))?;
                write(log)?;
                it.state = State::Installed;
                it.action = Some("written");
            }
            State::Foreign | State::Skipped => {
                it.action = Some("kept");
                it.note = Some("not created by autorename-pdf; left alone".into());
            }
        },
        Action::Remove => {
            if ours {
                std::fs::remove_file(&link).map_err(|e| io_err(&link, e))?;
                it.state = State::Missing;
                it.action = Some("removed");
            } else if it.state == State::Foreign {
                it.action = Some("kept");
            }
            log.links.remove(&link);
        }
    }
    if it.state == State::Installed
        && action != Action::Remove
        && !std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|x| x == d.bin))
    {
        it.note = Some(format!(
            "{} is not on PATH: add `export PATH=\"{}:$PATH\"` to your shell profile (e.g. ~/.bashrc), then open a new terminal",
            d.bin.display(),
            d.bin.display()
        ));
    }
    Ok(it)
}

/// Note for Nemo/Thunar entries that cannot be generated safely.
pub const PERCENT_NOTE: &str =
    "skipped: path contains % (Nemo/Thunar would expand it as a field code)";

pub fn run(action: Action, exe: &Path) -> autorename_core::Result<Report> {
    let exe_s = exe.to_str().ok_or_else(|| autorename_core::Error::Usage {
        message: format!("executable path is not UTF-8: {}", exe.display()),
        hint: Some("Pass --exe with a UTF-8 path.".into()),
    })?;
    let d = Dirs::from_env().ok_or_else(|| {
        autorename_core::Error::General(
            "neither HOME nor XDG_DATA_HOME/XDG_CONFIG_HOME is set".into(),
        )
    })?;
    run_in(action, exe_s, &d, &host_present, &in_path)
}

/// [`run`] with injectable base dirs and host detection (tests).
pub fn run_in(
    action: Action,
    exe_s: &str,
    d: &Dirs,
    host_present: &dyn Fn(Host) -> bool,
    which: &dyn Fn(&str) -> Option<PathBuf>,
) -> autorename_core::Result<Report> {
    let mut all = entries(d, exe_s);
    if action == Action::Remove && !needs_wrapper(exe_s) {
        // A launcher from an earlier install (marker-owned) goes as well.
        all.push((
            None,
            FileEntry {
                kind: "launcher",
                path: launcher_path(d),
                bytes: wrapper_script(exe_s).into_bytes(),
                executable: true,
                owned_by_name: false,
            },
        ));
    }
    let thunar_exe = token_free_exe(d, exe_s);
    // Even the wrapper path has a `%` (data dir): no safe Nemo/Thunar entry.
    let tokens_ok = !needs_wrapper(&thunar_exe);
    let token_bound =
        |h: Option<Host>, e: &FileEntry| h == Some(Host::Nemo) || e.kind == "launcher";
    let mut log = OwnedLog::load(d.data.join(format!("{APP_ID}/integration-owned.txt")));
    let present = |h: Option<Host>| h.is_none_or(host_present);
    let skipped = |log: &OwnedLog, e: &FileEntry, note: String| {
        let mut it = super::status_files(std::slice::from_ref(e), log).remove(0);
        if it.state == State::Missing {
            it.state = State::Skipped;
        }
        it.note = Some(note);
        it
    };
    let mut items = Vec::new();
    match action {
        Action::Status => {
            for (h, e) in &all {
                if !tokens_ok && token_bound(*h, e) {
                    items.push(skipped(&log, e, PERCENT_NOTE.into()));
                    continue;
                }
                let mut it = super::status_files(std::slice::from_ref(e), &log).remove(0);
                if !present(*h) && it.state == State::Missing {
                    it.state = State::Skipped;
                }
                items.push(it);
            }
        }
        Action::Install => {
            for (h, e) in &all {
                if !tokens_ok && token_bound(*h, e) {
                    items.push(skipped(&log, e, PERCENT_NOTE.into()));
                } else if present(*h) {
                    items.extend(
                        super::install_files(std::slice::from_ref(e), &mut log)
                            .map_err(|er| io_err(&e.path, er))?,
                    );
                } else {
                    items.push(skipped(
                        &log,
                        e,
                        format!("{} not found", h.map_or("", Host::binary)),
                    ));
                }
            }
        }
        Action::Remove => {
            for (_, e) in &all {
                items.extend(
                    super::remove_files(std::slice::from_ref(e), &mut log)
                        .map_err(|er| io_err(&e.path, er))?,
                );
            }
        }
    }
    if !tokens_ok && action != Action::Remove {
        let mut it = thunar_item(&d.thunar_uca(), State::Skipped, None);
        it.note = Some(PERCENT_NOTE.into());
        items.push(it);
    } else {
        items.push(thunar(
            action,
            &d.thunar_uca(),
            &thunar_exe,
            host_present(Host::Thunar),
            &mut log,
        )?);
    }
    items.push(cli_link(action, d, exe_s, which, &mut log)?);
    match action {
        Action::Install => log.save().map_err(|e| io_err(&d.data, e))?,
        Action::Remove => log.prune().map_err(|e| io_err(&d.data, e))?,
        Action::Status => {}
    }
    if action != Action::Status {
        refresh_caches(d);
    }
    let mut notes = Vec::new();
    if host_present(Host::Nautilus) {
        notes.push(
            "Nautilus: entries are under Scripts in the context menu; Nautilus shows no icons there (host limitation)."
                .into(),
        );
    }
    Ok(Report {
        platform: "linux",
        scope: "user",
        command: exe_s.into(),
        items,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/opt/Auto Rename/autorename-pdf";

    #[test]
    fn exec_quoting_follows_the_desktop_entry_spec() {
        assert_eq!(
            exec_arg("/usr/bin/autorename-pdf"),
            "/usr/bin/autorename-pdf"
        );
        assert_eq!(exec_arg("/opt/A B/x"), r#""/opt/A B/x""#);
        // `$` escaped inside quotes, then `\` doubled by the string escape.
        assert_eq!(exec_arg("/a$b"), r#""/a\\$b""#);
        assert_eq!(exec_arg("/100%/x"), "/100%%/x");
    }

    #[test]
    fn desktop_entry_golden() {
        let want = "[Desktop Entry]
Type=Application
Version=1.5
Name=AutoRename-PDF
GenericName=PDF Renamer
Comment=Rename PDFs from their content (AI + OCR)
Exec=\"/opt/Auto Rename/autorename-pdf\" gui %F
Icon=autorename-pdf
Terminal=false
Categories=Office;
MimeType=application/pdf;inode/directory;
Keywords=PDF;rename;invoice;OCR;
StartupNotify=true
StartupWMClass=autorename-pdf
Actions=rename-now;preview;
X-AutoRename-PDF-Integration=1

[Desktop Action rename-now]
Name=Rename now
Exec=\"/opt/Auto Rename/autorename-pdf\" gui --rename-now %F

[Desktop Action preview]
Name=Preview in AutoRename-PDF
Exec=\"/opt/Auto Rename/autorename-pdf\" gui %F
";
        assert_eq!(desktop_entry(EXE), want);
    }

    #[test]
    fn dolphin_service_menu_golden() {
        let want = "[Desktop Entry]
Type=Service
X-KDE-ServiceTypes=KonqPopupMenu/Plugin
MimeType=application/pdf;inode/directory;
Actions=renameNow;preview;
X-KDE-Submenu=AutoRename-PDF
Icon=autorename-pdf
X-AutoRename-PDF-Integration=1

[Desktop Action renameNow]
Name=Rename now
Icon=autorename-pdf
Exec=/usr/bin/autorename-pdf gui --rename-now %F

[Desktop Action preview]
Name=Preview in AutoRename-PDF
Icon=autorename-pdf
Exec=/usr/bin/autorename-pdf gui %F
";
        assert_eq!(dolphin_menu("/usr/bin/autorename-pdf"), want);
    }

    #[test]
    fn nemo_action_golden() {
        let want = "[Nemo Action]
Name=AutoRename-PDF: Rename now
Comment=Rename PDFs from their content (AI + OCR)
Exec='/opt/Auto Rename/autorename-pdf' gui --rename-now %F
Icon-Name=autorename-pdf
Selection=notnone
Extensions=pdf;dir;
Quote=double
X-AutoRename-PDF-Integration=1
";
        assert_eq!(nemo_action(EXE, Verb::RenameNow), want);
    }

    #[test]
    fn nautilus_script_golden() {
        let want = "#!/bin/sh
# X-AutoRename-PDF-Integration: written by `autorename-pdf integrate install`,
# deleted by `autorename-pdf integrate remove`.
[ \"$#\" -gt 0 ] || exit 0
exec '/opt/Auto Rename/autorename-pdf' gui -- \"$@\"
";
        assert_eq!(nautilus_script(EXE, Verb::Preview), want);
    }

    /// Review finding: a selected name containing a newline must stay one
    /// path; the newline-separated env var must never be split into others.
    #[cfg(unix)]
    #[test]
    fn nautilus_script_keeps_args_intact_and_ignores_the_env_list() {
        // /tmp may be noexec: run the script with `sh`, and let the "exe"
        // be `sh` too, which then reads the script file `gui` from the cwd.
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            t.path().join("gui"),
            "for a in \"$@\"; do printf '[%s]' \"$a\"; done\n",
        )
        .unwrap();
        let script = t.path().join("s");
        std::fs::write(&script, nautilus_script("sh", Verb::RenameNow)).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("sh")
                .arg(&script)
                .args(args)
                .current_dir(t.path())
                .env(
                    "NAUTILUS_SCRIPT_SELECTED_FILE_PATHS",
                    "/a b/x.pdf\n/tmp/victim.pdf\n",
                )
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        if let Ok(s) = run(&["a b/x.pdf\n/tmp/victim.pdf", "c.pdf"]) {
            assert_eq!(s, "[--rename-now][--][a b/x.pdf\n/tmp/victim.pdf][c.pdf]");
        }
        // Only the ambiguous env list: do nothing.
        if let Ok(s) = run(&[]) {
            assert_eq!(s, "");
        }
    }

    #[test]
    fn nemo_exec_is_keyfile_escaped() {
        let exe = "/tmp/it's/My App";
        let a = nemo_action(exe, Verb::Preview);
        assert!(a.contains(r"Exec='/tmp/it'\\''s/My App' gui %F"), "{a}");
        // Decode through GLib when PyGObject is available.
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.nemo_action");
        std::fs::write(&f, &a).unwrap();
        let py = "import sys\nfrom gi.repository import GLib\nk=GLib.KeyFile()\nk.load_from_file(sys.argv[1], GLib.KeyFileFlags.NONE)\nsys.stdout.write(k.get_string('Nemo Action','Exec'))";
        if let Ok(o) = std::process::Command::new("python3")
            .args(["-I", "-c", py])
            .arg(&f)
            .output()
            && o.status.success()
        {
            assert_eq!(
                String::from_utf8_lossy(&o.stdout),
                r"'/tmp/it'\''s/My App' gui %F"
            );
        }
    }

    /// Review finding: `%F` inside the executable path must not reach
    /// Nemo/Thunar token expansion; they launch an owned wrapper instead.
    // Linux paths and file names (':' etc.); Windows cannot hold them.
    #[cfg(unix)]
    #[test]
    fn percent_in_exe_uses_a_token_free_wrapper() {
        let d = Dirs {
            data: "/h/.local/share".into(),
            config: "/h/.config".into(),
            bin: "/h/.local/bin".into(),
        };
        let exe = "/tmp/100%F/My App";
        let e = entries(&d, exe);
        let wrapper = e.iter().find(|(_, f)| f.kind == "launcher").unwrap();
        assert_eq!(
            wrapper.1.path,
            Path::new("/h/.local/share/autorename-pdf/autorename-pdf-launch")
        );
        assert!(wrapper.1.executable);
        let body = String::from_utf8_lossy(&wrapper.1.bytes).into_owned();
        assert!(body.contains("exec '/tmp/100%F/My App' \"$@\""), "{body}");
        for (_, f) in e.iter().filter(|(h, _)| *h == Some(Host::Nemo)) {
            let t = String::from_utf8_lossy(&f.bytes).into_owned();
            assert!(
                t.contains("Exec=/h/.local/share/autorename-pdf/autorename-pdf-launch gui"),
                "{t}"
            );
            assert!(!t.contains("100%F"));
        }
        assert_eq!(
            token_free_exe(&d, exe),
            "/h/.local/share/autorename-pdf/autorename-pdf-launch"
        );
        // Desktop Exec escapes `%` itself (`%%`); no wrapper needed there.
        assert!(desktop_entry(exe).contains(r#"Exec="/tmp/100%%F/My App" gui %F"#));
        // No `%`: no wrapper.
        assert!(
            !entries(&d, "/usr/bin/autorename-pdf")
                .iter()
                .any(|(_, f)| f.kind == "launcher")
        );
    }

    /// Review finding: an unreadable/undecodable uca.xml must never be
    /// replaced (it would drop the user's actions).
    #[test]
    fn thunar_never_overwrites_a_non_utf8_uca() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("Thunar/uca.xml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let latin1: &[u8] = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<actions>\n<action><name>Gr\xfc\xdfe</name><unique-id>1-1</unique-id></action>\n</actions>\n";
        std::fs::write(&path, latin1).unwrap();
        let mut log = OwnedLog::load(t.path().join("log"));
        for action in [Action::Install, Action::Remove, Action::Status] {
            let i = thunar(action, &path, "/usr/bin/autorename-pdf", true, &mut log).unwrap();
            assert_eq!(i.state, State::Foreign, "{action:?}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), latin1);
    }

    #[test]
    fn thunar_block_golden() {
        let s = thunar_actions("/usr/bin/autorename-pdf");
        let first = "\t<action>
\t\t<icon>autorename-pdf</icon>
\t\t<name>Rename now</name>
\t\t<submenu>AutoRename-PDF</submenu>
\t\t<unique-id>autorename-pdf-rename-now-pdf</unique-id>
\t\t<command>/usr/bin/autorename-pdf gui --rename-now %F</command>
\t\t<description>Rename PDFs from their content (AI + OCR)</description>
\t\t<range>*</range>
\t\t<patterns>*.pdf;*.PDF</patterns>
\t\t<other-files/>
\t</action>
";
        assert!(s.starts_with(first), "{s}");
        assert_eq!(s.matches("<action>").count(), 4);
        assert!(s.contains("<unique-id>autorename-pdf-preview-dir</unique-id>"));
        assert!(s.contains("<patterns>*</patterns>\n\t\t<directories/>"));
        // XML escaping of the command.
        assert!(thunar_actions("/a&b/x").contains("<command>'/a&amp;b/x' gui"));
    }

    const USER_UCA: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<actions>
<action>
\t<icon>utilities-terminal</icon>
\t<name>Open Terminal Here</name>
\t<unique-id>1600000000000000-1</unique-id>
\t<command>exo-open --working-directory %f --launch TerminalEmulator</command>
\t<description>Example for a custom action</description>
\t<patterns>*</patterns>
\t<startup-notify/>
\t<directories/>
</action>
</actions>
";

    #[test]
    fn thunar_merge_keeps_user_actions_and_is_idempotent() {
        let exe = "/usr/bin/autorename-pdf";
        let merged = thunar_merge(Some(USER_UCA), exe).unwrap();
        assert!(merged.contains("Open Terminal Here"));
        assert_eq!(merged.matches("<action>").count(), 5);
        assert!(merged.ends_with("\t</action>\n</actions>\n"));
        // Re-merge (other exe) replaces ours, never duplicates.
        let again = thunar_merge(Some(&merged), "/other/autorename-pdf").unwrap();
        assert_eq!(again.matches("<action>").count(), 5);
        assert!(!again.contains("/usr/bin/autorename-pdf"));
        // Strip restores the user's document byte for byte.
        let (stripped, n) = thunar_strip(&merged);
        assert_eq!((stripped.as_str(), n), (USER_UCA, 4));
        assert_eq!(thunar_state(Some(&merged), exe), State::Installed);
        assert_eq!(thunar_state(Some(&merged), "/x"), State::Outdated);
        assert_eq!(thunar_state(Some(USER_UCA), exe), State::Missing);
    }

    #[test]
    fn thunar_merge_creates_and_refuses_broken_files() {
        let fresh = thunar_merge(None, "/x").unwrap();
        assert!(fresh.starts_with(THUNAR_HEAD) && fresh.ends_with(THUNAR_TAIL));
        assert!(thunar_is_empty_skeleton(&thunar_strip(&fresh).0));
        assert!(thunar_merge(Some("<actions><action>"), "/x").is_err());
    }

    // Linux paths and file names (':' etc.); Windows cannot hold them.
    #[cfg(unix)]
    #[test]
    fn entries_cover_every_host() {
        let d = Dirs {
            data: "/h/.local/share".into(),
            config: "/h/.config".into(),
            bin: "/h/.local/bin".into(),
        };
        let e = entries(&d, "/usr/bin/autorename-pdf");
        let paths: Vec<String> = e
            .iter()
            .map(|(_, f)| f.path.display().to_string())
            .collect();
        for want in [
            "/h/.local/share/applications/autorename-pdf.desktop",
            "/h/.local/share/icons/hicolor/16x16/apps/autorename-pdf.png",
            "/h/.local/share/icons/hicolor/scalable/apps/autorename-pdf.svg",
            "/h/.local/share/kio/servicemenus/autorename-pdf.desktop",
            "/h/.local/share/nemo/actions/autorename-pdf-rename-now.nemo_action",
            "/h/.local/share/nautilus/scripts/AutoRename-PDF: Rename now",
        ] {
            assert!(paths.iter().any(|p| p == want), "{want} missing");
        }
        let dolphin = e.iter().find(|(h, _)| *h == Some(Host::Dolphin)).unwrap();
        assert!(dolphin.1.executable);
        assert!(
            e.iter()
                .all(|(_, f)| f.owned_by_name || super::super::has_marker(&f.bytes))
        );
    }

    fn sandbox(root: &Path) -> Dirs {
        Dirs {
            data: root.join("data"),
            config: root.join("config"),
            bin: root.join("bin"),
        }
    }

    fn all_hosts(_: Host) -> bool {
        true
    }

    fn no_path(_: &str) -> Option<PathBuf> {
        None
    }

    /// AppImage/portable: `~/.local/bin/autorename-pdf` → exe, owned via the
    /// log; skipped when PATH already resolves to this exe; foreign kept.
    #[cfg(unix)]
    #[test]
    fn cli_symlink_is_owned_and_removed() {
        let t = tempfile::tempdir().unwrap();
        let d = sandbox(t.path());
        let exe = t.path().join("autorename-pdf.AppImage");
        std::fs::write(&exe, "x").unwrap();
        let exe_s = exe.to_str().unwrap();
        let link = d.bin.join("autorename-pdf");
        let r = run_in(Action::Install, exe_s, &d, &all_hosts, &no_path).unwrap();
        let it = r.items.iter().find(|i| i.kind == "cli-link").unwrap();
        assert_eq!((it.state, it.action), (State::Installed, Some("written")));
        assert_eq!(std::fs::read_link(&link).unwrap(), exe);
        run_in(Action::Remove, exe_s, &d, &all_hosts, &no_path).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(
            !d.bin.exists(),
            "~/.local/bin was created by us and is empty"
        );
        // Already on PATH as this exe: no link.
        let found = exe.clone();
        let on_path = move |_: &str| Some(found.clone());
        let r = run_in(Action::Install, exe_s, &d, &all_hosts, &on_path).unwrap();
        let it = r.items.iter().find(|i| i.kind == "cli-link").unwrap();
        assert_eq!(it.state, State::Skipped);
        run_in(Action::Remove, exe_s, &d, &all_hosts, &on_path).unwrap();
        // A foreign file is never replaced or deleted.
        std::fs::create_dir_all(&d.bin).unwrap();
        std::fs::write(&link, "user script").unwrap();
        run_in(Action::Install, exe_s, &d, &all_hosts, &no_path).unwrap();
        run_in(Action::Remove, exe_s, &d, &all_hosts, &no_path).unwrap();
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "user script");
    }

    /// Codex NEW-2: an install that fails late (unreadable uca.xml, mode
    /// 000) must not orphan the icons it already wrote; remove cleans up.
    #[cfg(unix)]
    #[test]
    fn failed_install_keeps_icon_ownership() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let d = sandbox(t.path());
        let uca = d.thunar_uca();
        std::fs::create_dir_all(uca.parent().unwrap()).unwrap();
        std::fs::write(&uca, USER_UCA).unwrap();
        std::fs::set_permissions(&uca, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&uca).is_ok() {
            return; // running as root: mode 000 is readable
        }
        let exe = "/usr/bin/autorename-pdf";
        assert!(run_in(Action::Install, exe, &d, &all_hosts, &no_path).is_err());
        std::fs::set_permissions(&uca, std::fs::Permissions::from_mode(0o644)).unwrap();
        let r = run_in(Action::Remove, exe, &d, &all_hosts, &no_path).unwrap();
        let kept: Vec<_> = r
            .items
            .iter()
            .filter(|i| i.action == Some("kept"))
            .collect();
        assert!(kept.is_empty(), "{kept:?}");
        // Only the user's Thunar file is left.
        let left: Vec<_> = walk(t.path());
        assert_eq!(left, vec![uca.clone()], "{left:?}");
        assert_eq!(std::fs::read_to_string(&uca).unwrap(), USER_UCA);
    }

    /// A lost ownership record: icons with exactly our bytes are re-adopted.
    // Linux paths and file names (':' etc.); Windows cannot hold them.
    #[cfg(unix)]
    #[test]
    fn identical_icons_are_readopted() {
        let t = tempfile::tempdir().unwrap();
        let d = sandbox(t.path());
        let exe = "/usr/bin/autorename-pdf";
        run_in(Action::Install, exe, &d, &all_hosts, &no_path).unwrap();
        std::fs::remove_file(d.data.join("autorename-pdf/integration-owned.txt")).unwrap();
        run_in(Action::Install, exe, &d, &all_hosts, &no_path).unwrap();
        run_in(Action::Remove, exe, &d, &all_hosts, &no_path).unwrap();
        assert_eq!(walk(t.path()), Vec::<PathBuf>::new());
    }

    /// Finding 9 residue: `%` in the data dir as well → Nemo/Thunar skipped.
    // Linux paths and file names (':' etc.); Windows cannot hold them.
    #[cfg(unix)]
    #[test]
    fn percent_in_data_dir_skips_nemo_and_thunar() {
        let t = tempfile::tempdir().unwrap();
        let d = sandbox(&t.path().join("100%F"));
        // Exe without `%`: no wrapper needed, everything installs.
        let r = run_in(
            Action::Install,
            "/usr/bin/autorename-pdf",
            &d,
            &all_hosts,
            &no_path,
        )
        .unwrap();
        assert!(
            r.items.iter().all(|i| i.state == State::Installed),
            "{:?}",
            r.items
        );
        run_in(
            Action::Remove,
            "/usr/bin/autorename-pdf",
            &d,
            &all_hosts,
            &no_path,
        )
        .unwrap();
        // Exe with `%` and a `%` data dir: the wrapper would carry it too.
        let exe = "/opt/50%U/autorename-pdf";
        let r = run_in(Action::Install, exe, &d, &all_hosts, &no_path).unwrap();
        for i in &r.items {
            let token_bound =
                i.kind == "nemo-action" || i.kind == "thunar-action" || i.kind == "launcher";
            if token_bound {
                assert_eq!(i.state, State::Skipped, "{i:?}");
                assert_eq!(i.note.as_deref(), Some(PERCENT_NOTE));
            } else {
                assert_eq!(i.state, State::Installed, "{i:?}");
            }
        }
        assert!(!d.thunar_uca().exists());
        assert!(!d.data.join("nemo").exists());
        assert!(!launcher_path(&d).exists());
        run_in(Action::Remove, exe, &d, &all_hosts, &no_path).unwrap();
        assert_eq!(walk(t.path()), Vec::<PathBuf>::new());
    }

    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }
}
