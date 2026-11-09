//! Windows: per-user classic context menu under `HKCU\Software\Classes`
//! (no admin rights): `SystemFileAssociations\.pdf` and `Directory`, each a
//! cascading "AutoRename-PDF" verb with "Rename now" and "Preview in
//! AutoRename-PDF". Commands launch the WINDOWS-subsystem launcher
//! `autorename-pdf-gui.exe` (no console flash), which coalesces the
//! one-process-per-file launches Explorer makes for a multi-selection.
//! On Windows 11 the entries are under "Show more options" (the modern
//! menu needs a packaged IExplorerCommand: stretch goal).
//!
//! The plan is pure data ([`registry_plan`]) so it is tested on any OS; the
//! registry writes are `cfg(windows)`.

// Pure plan is built (and tested) everywhere; only Windows applies it.
#![cfg_attr(not(windows), allow(dead_code))]

use super::Verb;

/// Our verb key under each `shell` root (the ownership unit).
pub const VERB_KEY: &str = "AutoRenamePDF";
/// Value present on our verb keys; `remove` deletes only keys that have it.
pub const OWNER_VALUE: &str = "AutoRenamePdfIntegration";
pub const ROOTS: [&str; 2] = [
    r"Software\Classes\SystemFileAssociations\.pdf\shell",
    r"Software\Classes\Directory\shell",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegKey {
    /// Relative to HKCU.
    pub path: String,
    /// `(name, REG_SZ data)`; `""` = the default value.
    pub values: Vec<(String, String)>,
}

/// `HKCU\…\shell\AutoRenamePDF` for each root.
pub fn owned_keys() -> Vec<String> {
    ROOTS.iter().map(|r| format!(r"{r}\{VERB_KEY}")).collect()
}

/// Ancestors of an owned verb key, deepest first, below `Software\Classes`.
pub fn empty_parent_candidates(root: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = root;
    while let Some((parent, _)) = cur.rsplit_once('\\') {
        if parent.eq_ignore_ascii_case(r"Software\Classes") || !parent.contains('\\') {
            break;
        }
        out.push(parent.to_string());
        cur = parent;
    }
    out
}

/// Quote one command-line argument (CommandLineToArgvW rules for paths:
/// paths cannot contain `"`, so wrapping in quotes is enough).
fn q(s: &str) -> String {
    format!("\"{s}\"")
}

/// Launch command for a verb. `launcher` = `autorename-pdf-gui.exe` (adds
/// `gui` itself); `None` = fall back to the console exe with `gui`.
pub fn verb_command(launcher: Option<&str>, exe: &str, verb: Verb) -> String {
    let mut parts = match launcher {
        Some(l) => vec![q(l)],
        None => vec![q(exe), "gui".into()],
    };
    parts.extend(verb.args().iter().skip(1).map(|a| a.to_string()));
    parts.push("\"%1\"".into());
    parts.join(" ")
}

/// Every key to create, parents first.
pub fn registry_plan(launcher: Option<&str>, exe: &str) -> Vec<RegKey> {
    let icon = format!("{exe},0");
    let kv = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let mut keys = Vec::new();
    for root in owned_keys() {
        keys.push(RegKey {
            path: root.clone(),
            values: kv(&[
                ("MUIVerb", super::PRODUCT),
                ("Icon", &icon),
                // Empty SubCommands = submenu items come from the `shell` subkey.
                ("SubCommands", ""),
                // Show for any number of selected items (default limit: 15).
                ("MultiSelectModel", "Player"),
                (OWNER_VALUE, "1"),
            ]),
        });
        for (i, verb) in Verb::ALL.iter().enumerate() {
            // Subkeys sort by name: the prefix fixes the menu order.
            let key = format!(r"{root}\shell\{}-{}", i + 1, verb.id());
            let label = match verb {
                Verb::RenameNow => "Rename now".to_string(),
                Verb::Preview => format!("Preview in {}…", super::PRODUCT),
            };
            keys.push(RegKey {
                path: key.clone(),
                values: kv(&[
                    ("MUIVerb", &label),
                    ("Icon", &icon),
                    ("MultiSelectModel", "Player"),
                ]),
            });
            keys.push(RegKey {
                path: format!(r"{key}\command"),
                values: kv(&[("", &verb_command(launcher, exe, *verb))]),
            });
        }
    }
    keys
}

/// Our record of the per-user PATH entry we added (only that entry is ever
/// removed again): `HKCU\Software\AutoRename-PDF\Integration`, `UserPath`.
pub const STATE_KEY: &str = r"Software\AutoRename-PDF\Integration";
pub const USER_PATH_VALUE: &str = "UserPath";

fn same_dir(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().trim_end_matches('\\').to_lowercase();
    !a.trim().is_empty() && n(a) == n(b)
}

/// `HKCU\Environment` `Path` with `dir` appended, or `None` if it is
/// already there (case-insensitive, trailing `\` ignored).
pub fn path_add(current: &str, dir: &str) -> Option<String> {
    if current.split(';').any(|e| same_dir(e, dir)) {
        return None;
    }
    let base = current.trim_end_matches(';');
    Some(if base.is_empty() {
        dir.to_string()
    } else {
        format!("{base};{dir}")
    })
}

/// `Path` without `dir` (every occurrence), or `None` if it is not there.
/// Other entries keep their exact spelling and order.
pub fn path_remove(current: &str, dir: &str) -> Option<String> {
    if !current.split(';').any(|e| same_dir(e, dir)) {
        return None;
    }
    Some(
        current
            .split(';')
            .filter(|e| !same_dir(e, dir))
            .collect::<Vec<_>>()
            .join(";"),
    )
}

/// Program files of an install in the old default location
/// (`%LOCALAPPDATA%\AutoRename-PDF`, which is also the app data dir). Data
/// files there (journal, models, locks) are never in this list.
pub const STALE_FILES: [&str; 10] = [
    "autorename-pdf.exe",
    "autorename-pdf-gui.exe",
    "pdfium.dll",
    "DirectML.dll",
    "uninstall.exe",
    r"licenses\AutoRename-PDF-LICENSE.txt",
    r"licenses\Inter-OFL.txt",
    r"licenses\JetBrainsMono-OFL.txt",
    r"licenses\Lucide-ISC.txt",
    r"licenses\PDFium-LICENSE.txt",
];

/// The old install dir to clean, if it is not where we run from and still
/// holds our exe.
pub fn stale_install_dir(
    local_app_data: &str,
    exe_dir: &str,
    has_exe: impl Fn(&str) -> bool,
) -> Option<String> {
    let old = format!(r"{}\AutoRename-PDF", local_app_data.trim_end_matches('\\'));
    (!same_dir(&old, exe_dir) && has_exe(&old)).then_some(old)
}

#[cfg(windows)]
mod user_path {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
    use winreg::{RegKey, RegValue};

    fn decode(v: &RegValue) -> String {
        let w: Vec<u16> = v
            .bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&w)
            .trim_end_matches('\0')
            .to_string()
    }

    /// The raw `Path` (unexpanded `%VAR%`s preserved) and its type.
    pub fn read() -> std::io::Result<(String, winreg::enums::RegType)> {
        let env =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ)?;
        match env.get_raw_value("Path") {
            Ok(v) => Ok((decode(&v), v.vtype)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok((String::new(), REG_EXPAND_SZ))
            }
            Err(e) => Err(e),
        }
    }

    pub fn write(value: &str, vtype: winreg::enums::RegType) -> std::io::Result<()> {
        let (env, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
        let vtype = if vtype == REG_SZ {
            REG_SZ
        } else {
            REG_EXPAND_SZ
        };
        let bytes: Vec<u8> = value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        env.set_raw_value(
            "Path",
            &RegValue {
                bytes: bytes.into(),
                vtype,
            },
        )?;
        broadcast();
        Ok(())
    }

    /// Tell Explorer and new consoles that the environment changed.
    fn broadcast() {
        // SAFETY: the declaration matches user32's SendMessageTimeoutW.
        #[allow(unsafe_code)]
        #[link(name = "user32")]
        unsafe extern "system" {
            fn SendMessageTimeoutW(
                hwnd: isize,
                msg: u32,
                wparam: usize,
                lparam: *const u16,
                flags: u32,
                timeout: u32,
                result: *mut usize,
            ) -> isize;
        }
        const HWND_BROADCAST: isize = 0xffff;
        const WM_SETTINGCHANGE: u32 = 0x001A;
        const SMTO_ABORTIFHUNG: u32 = 0x0002;
        let area: Vec<u16> = "Environment".encode_utf16().chain(Some(0)).collect();
        let mut result = 0usize;
        // SAFETY: `area` is NUL-terminated and outlives the call; `result`
        // is a valid out pointer.
        #[allow(unsafe_code)]
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                area.as_ptr(),
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            );
        }
    }
}

#[cfg(windows)]
pub fn run(action: super::Action, exe: &std::path::Path) -> autorename_core::Result<super::Report> {
    use super::{Action, Item, Report, State};
    use winreg::RegKey as WinKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let exe_s = exe.to_str().ok_or_else(|| autorename_core::Error::Usage {
        message: format!("executable path is not valid Unicode: {}", exe.display()),
        hint: None,
    })?;
    let launcher = exe
        .parent()
        .map(|d| d.join("autorename-pdf-gui.exe"))
        .filter(|p| p.is_file());
    let launcher_s = launcher.as_deref().and_then(|p| p.to_str());
    let plan = registry_plan(launcher_s, exe_s);
    let hkcu = WinKey::predef(HKEY_CURRENT_USER);
    let reg_err = |path: &str, e: std::io::Error| {
        super::io_err(std::path::Path::new(&format!(r"HKCU\{path}")), e)
    };

    let owned = |path: &str| -> Option<bool> {
        let k = hkcu.open_subkey(path).ok()?;
        Some(k.get_value::<String, _>(OWNER_VALUE).is_ok())
    };
    let matches_plan = |root: &str| -> bool {
        plan.iter().filter(|k| k.path.starts_with(root)).all(|k| {
            hkcu.open_subkey(&k.path).is_ok_and(|key| {
                k.values
                    .iter()
                    .all(|(n, v)| key.get_value::<String, _>(n).is_ok_and(|got| &got == v))
            })
        })
    };
    let mut items = Vec::new();
    for root in owned_keys() {
        let state = match owned(&root) {
            None => State::Missing,
            Some(false) => State::Foreign,
            Some(true) if matches_plan(&root) => State::Installed,
            Some(true) => State::Outdated,
        };
        let mut it = Item {
            kind: "registry".into(),
            path: format!(r"HKCU\{root}"),
            state,
            action: None,
            note: None,
        };
        match (action, state) {
            (Action::Status, _) => {}
            (_, State::Foreign) => it.action = Some("kept"),
            (Action::Install, State::Installed) => it.action = Some("unchanged"),
            (Action::Install, _) => {
                if state == State::Outdated {
                    hkcu.delete_subkey_all(&root)
                        .map_err(|e| reg_err(&root, e))?;
                }
                for k in plan.iter().filter(|k| k.path.starts_with(&root)) {
                    let (key, _) = hkcu
                        .create_subkey(&k.path)
                        .map_err(|e| reg_err(&k.path, e))?;
                    for (n, v) in &k.values {
                        key.set_value(n, v).map_err(|e| reg_err(&k.path, e))?;
                    }
                }
                it.state = State::Installed;
                it.action = Some("written");
            }
            (Action::Remove, State::Installed | State::Outdated) => {
                hkcu.delete_subkey_all(&root)
                    .map_err(|e| reg_err(&root, e))?;
                // Parents `install` may have created (`SystemFileAssociations\.pdf\shell`, …):
                // delete them only while completely empty (no subkeys, no values).
                for parent in empty_parent_candidates(&root) {
                    let empty = hkcu.open_subkey(&parent).is_ok_and(|k| {
                        k.enum_keys().next().is_none() && k.enum_values().next().is_none()
                    });
                    if !empty || hkcu.delete_subkey(&parent).is_err() {
                        break;
                    }
                }
                it.state = State::Missing;
                it.action = Some("removed");
            }
            (Action::Remove, _) => {}
        }
        items.push(it);
    }
    items.push(user_path_item(action, exe)?);
    if action == Action::Install
        && let Some(it) = remove_stale_install(exe)
    {
        items.push(it);
    }
    let mut notes = Vec::new();
    if launcher.is_none() {
        notes.push(
            "autorename-pdf-gui.exe not found beside this exe: entries run the console exe (a console window flashes)."
                .into(),
        );
    }
    notes.push("Windows 11: the entries are under \"Show more options\" (Shift+F10).".into());
    notes.push(
        "The install folder is on your user PATH (`autorename-pdf` in new terminals); `integrate remove` takes it off again."
            .into(),
    );
    Ok(Report {
        platform: "windows",
        scope: "user",
        command: launcher_s.unwrap_or(exe_s).into(),
        items,
        notes,
    })
}

/// After an upgrade from the old location, delete the old program files
/// (never data files) so no stale copy stays behind.
#[cfg(windows)]
fn remove_stale_install(exe: &std::path::Path) -> Option<super::Item> {
    use std::path::Path;
    let lad = std::env::var("LOCALAPPDATA").ok()?;
    let exe_dir = exe.parent()?.to_str()?;
    let old = stale_install_dir(&lad, exe_dir, |d| {
        Path::new(d).join("autorename-pdf.exe").is_file()
    })?;
    let old_p = Path::new(&old);
    let mut kept = Vec::new();
    for f in STALE_FILES {
        let p = old_p.join(f);
        if p.exists() && std::fs::remove_file(&p).is_err() {
            kept.push(f);
        }
    }
    let _ = std::fs::remove_dir(old_p.join("licenses"));
    let _ = std::fs::remove_dir(old_p);
    Some(super::Item {
        kind: "old-install".into(),
        path: old.clone(),
        state: super::State::Missing,
        action: Some("removed"),
        note: (!kept.is_empty()).then(|| format!("could not delete: {}", kept.join(", "))),
    })
}

/// Per-user PATH entry for the folder holding `autorename-pdf.exe`.
#[cfg(windows)]
fn user_path_item(
    action: super::Action,
    exe: &std::path::Path,
) -> autorename_core::Result<super::Item> {
    use super::{Action, Item, State};
    use winreg::RegKey as WinKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let dir = exe
        .parent()
        .and_then(|d| d.to_str())
        .unwrap_or_default()
        .to_string();
    let err = |e: std::io::Error| super::io_err(std::path::Path::new(r"HKCU\Environment\Path"), e);
    let hkcu = WinKey::predef(HKEY_CURRENT_USER);
    let recorded: Option<String> = hkcu
        .open_subkey(STATE_KEY)
        .ok()
        .and_then(|k| k.get_value(USER_PATH_VALUE).ok());
    let (current, vtype) = user_path::read().map_err(err)?;
    let present = |d: &str| path_add(&current, d).is_none();
    let mut it = Item {
        kind: "user-path".into(),
        path: format!(r"HKCU\Environment\Path += {dir}"),
        state: if present(&dir) {
            State::Installed
        } else {
            State::Missing
        },
        action: None,
        note: None,
    };
    match action {
        Action::Status => {}
        Action::Install => {
            // Our older entry for another folder goes first.
            if let Some(old) = recorded.as_deref().filter(|o| !same_dir(o, &dir))
                && let Some(v) = path_remove(&current, old)
            {
                user_path::write(&v, vtype).map_err(err)?;
            }
            let (current, vtype) = user_path::read().map_err(err)?;
            match path_add(&current, &dir) {
                Some(v) => {
                    user_path::write(&v, vtype).map_err(err)?;
                    let (k, _) = hkcu.create_subkey(STATE_KEY).map_err(err)?;
                    k.set_value(USER_PATH_VALUE, &dir).map_err(err)?;
                    it.action = Some("written");
                }
                // Already on PATH: ours only if we recorded it.
                None => it.action = Some("unchanged"),
            }
            it.state = State::Installed;
        }
        Action::Remove => {
            if let Some(old) = recorded {
                if let Some(v) = path_remove(&current, &old) {
                    user_path::write(&v, vtype).map_err(err)?;
                }
                let _ = hkcu.delete_subkey(STATE_KEY);
                let _ = hkcu.delete_subkey(r"Software\AutoRename-PDF");
                it.state = State::Missing;
                it.action = Some("removed");
            } else if it.state == State::Installed {
                it.action = Some("kept");
                it.note = Some("on PATH, but not added by autorename-pdf; left alone".into());
            }
        }
    }
    Ok(it)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\Me\AppData\Local\AutoRename-PDF\autorename-pdf.exe";
    const GUI: &str = r"C:\Users\Me\AppData\Local\AutoRename-PDF\autorename-pdf-gui.exe";

    #[test]
    fn registry_plan_golden() {
        let plan = registry_plan(Some(GUI), EXE);
        let text: String = plan
            .iter()
            .map(|k| {
                let mut s = format!("[HKCU\\{}]\n", k.path);
                for (n, v) in &k.values {
                    let n = if n.is_empty() {
                        "@".to_string()
                    } else {
                        format!("\"{n}\"")
                    };
                    s.push_str(&format!("{n}={v}\n"));
                }
                s
            })
            .collect();
        let want = r#"[HKCU\Software\Classes\SystemFileAssociations\.pdf\shell\AutoRenamePDF]
"MUIVerb"=AutoRename-PDF
"Icon"=C:\Users\Me\AppData\Local\AutoRename-PDF\autorename-pdf.exe,0
"SubCommands"=
"MultiSelectModel"=Player
"AutoRenamePdfIntegration"=1
[HKCU\Software\Classes\SystemFileAssociations\.pdf\shell\AutoRenamePDF\shell\1-rename-now]
"MUIVerb"=Rename now
"Icon"=C:\Users\Me\AppData\Local\AutoRename-PDF\autorename-pdf.exe,0
"MultiSelectModel"=Player
[HKCU\Software\Classes\SystemFileAssociations\.pdf\shell\AutoRenamePDF\shell\1-rename-now\command]
@="C:\Users\Me\AppData\Local\AutoRename-PDF\autorename-pdf-gui.exe" --rename-now "%1"
[HKCU\Software\Classes\SystemFileAssociations\.pdf\shell\AutoRenamePDF\shell\2-preview]
"MUIVerb"=Preview in AutoRename-PDF…
"Icon"=C:\Users\Me\AppData\Local\AutoRename-PDF\autorename-pdf.exe,0
"MultiSelectModel"=Player
[HKCU\Software\Classes\SystemFileAssociations\.pdf\shell\AutoRenamePDF\shell\2-preview\command]
@="C:\Users\Me\AppData\Local\AutoRename-PDF\autorename-pdf-gui.exe" "%1"
"#;
        assert!(text.starts_with(want), "{text}");
        // Same tree for folders.
        assert_eq!(plan.len(), 10);
        assert!(text.contains(
            r"[HKCU\Software\Classes\Directory\shell\AutoRenamePDF\shell\2-preview\command]"
        ));
    }

    #[test]
    fn user_path_add_and_remove_touch_only_our_entry() {
        let dir = r"C:\Users\Me\AppData\Local\AutoRename-PDF";
        let cur = r"%USERPROFILE%\AppData\Local\Microsoft\WindowsApps;C:\Tools;";
        let added = path_add(cur, dir).unwrap();
        assert_eq!(
            added,
            format!(r"%USERPROFILE%\AppData\Local\Microsoft\WindowsApps;C:\Tools;{dir}")
        );
        // No duplicates (case/trailing backslash insensitive).
        assert_eq!(path_add(&added, &format!(r"{}\", dir.to_uppercase())), None);
        assert_eq!(path_add("", dir).unwrap(), dir);
        // Removal keeps every other entry byte for byte.
        assert_eq!(
            path_remove(&added, dir).unwrap(),
            r"%USERPROFILE%\AppData\Local\Microsoft\WindowsApps;C:\Tools"
        );
        assert_eq!(path_remove(cur, dir), None);
        assert_eq!(
            path_remove(&format!("{dir};C:\\X;{dir}"), dir).unwrap(),
            r"C:\X"
        );
    }

    #[test]
    fn stale_install_is_only_the_old_location_with_our_exe() {
        let lad = r"C:\Users\Me\AppData\Local";
        let new = r"C:\Users\Me\AppData\Local\Programs\AutoRename-PDF";
        assert_eq!(
            stale_install_dir(lad, new, |_| true).as_deref(),
            Some(r"C:\Users\Me\AppData\Local\AutoRename-PDF")
        );
        assert_eq!(stale_install_dir(lad, new, |_| false), None);
        // Running from the old location itself: nothing to clean.
        assert_eq!(
            stale_install_dir(lad, r"c:\users\me\appdata\local\autorename-pdf", |_| true),
            None
        );
        assert!(
            STALE_FILES
                .iter()
                .all(|f| !f.contains("journal") && !f.contains("models"))
        );
    }

    #[test]
    fn only_parents_below_classes_are_pruned() {
        assert_eq!(
            empty_parent_candidates(
                r"Software\Classes\SystemFileAssociations\.pdf\shell\AutoRenamePDF"
            ),
            vec![
                r"Software\Classes\SystemFileAssociations\.pdf\shell".to_string(),
                r"Software\Classes\SystemFileAssociations\.pdf".to_string(),
                r"Software\Classes\SystemFileAssociations".to_string(),
            ]
        );
        assert_eq!(
            empty_parent_candidates(r"Software\Classes\Directory\shell\AutoRenamePDF"),
            vec![
                r"Software\Classes\Directory\shell".to_string(),
                r"Software\Classes\Directory".to_string(),
            ]
        );
    }

    #[test]
    fn falls_back_to_console_exe_without_launcher() {
        assert_eq!(
            verb_command(None, EXE, Verb::RenameNow),
            format!("\"{EXE}\" gui --rename-now \"%1\"")
        );
    }

    #[test]
    fn every_root_is_owned_and_marked() {
        let plan = registry_plan(Some(GUI), EXE);
        for root in owned_keys() {
            let k = plan.iter().find(|k| k.path == root).unwrap();
            assert!(k.values.iter().any(|(n, v)| n == OWNER_VALUE && v == "1"));
            assert!(plan.iter().filter(|p| p.path.starts_with(&root)).count() == 5);
        }
    }
}
