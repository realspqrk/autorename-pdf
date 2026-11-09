//! `autorename-pdf-gui.exe`: Windows-subsystem launcher (no console flash).
//! Spawns `autorename-pdf.exe gui <args…>` from the same directory with
//! CREATE_NO_WINDOW and exits. Explorer context-menu launches (one per
//! selected file: `[--rename-now] <path>`) are coalesced into one GUI
//! process (`coalesce.rs`); the collected paths are handed over in a private
//! request file (no command-line length limit) that the app deletes after
//! reading it, which is the acknowledgement this launcher waits for. On
//! other platforms the main binary is used directly, so this prints an
//! error.

#![windows_subsystem = "windows"]

#[cfg_attr(not(windows), allow(dead_code))]
mod coalesce;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the launcher waits for the app to take the request.
#[cfg_attr(not(windows), allow(dead_code))]
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

/// Explorer verb shape: `[--rename-now] <one path>` → (spool key, path).
#[cfg_attr(not(windows), allow(dead_code))]
fn verb_invocation(args: &[std::ffi::OsString]) -> Option<(&'static str, String)> {
    match args {
        [p] if p != "--rename-now" => Some(("preview", p.to_str()?.to_string())),
        [f, p] if f == "--rename-now" => Some(("rename-now", p.to_str()?.to_string())),
        _ => None,
    }
}

/// Write the batch to `<dir>/req-<pid>-<nanos>.txt` (one path per line).
#[cfg_attr(not(windows), allow(dead_code))]
fn write_request(dir: &Path, paths: &[String]) -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let f = dir.join(format!("req-{}-{nanos}.txt", std::process::id()));
    let mut body = paths.join("\n");
    body.push('\n');
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&f)
        .and_then(|mut h| std::io::Write::write_all(&mut h, body.as_bytes()))?;
    Ok(f)
}

/// Wait until the app deleted the request (ack) or exited without it.
#[cfg_attr(not(windows), allow(dead_code))]
fn wait_ack(req: &Path, child: &mut std::process::Child) -> bool {
    let start = Instant::now();
    while start.elapsed() < ACK_TIMEOUT {
        if !req.exists() {
            return true;
        }
        if let Ok(Some(_)) = child.try_wait() {
            return !req.exists();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !req.exists()
}

#[cfg(windows)]
fn error_box(text: &str) {
    // SAFETY: the declaration matches user32's MessageBoxW signature.
    #[allow(unsafe_code)]
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (t, c) = (w(text), w("AutoRename-PDF"));
    // SAFETY: both buffers are NUL-terminated UTF-16 and outlive the call;
    // a null owner window is allowed. MB_ICONERROR = 0x10.
    #[allow(unsafe_code)]
    unsafe {
        MessageBoxW(0, t.as_ptr(), c.as_ptr(), 0x10);
    }
}

#[cfg(windows)]
fn main() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("autorename-pdf.exe")))
        .unwrap_or_else(|| "autorename-pdf.exe".into());
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut request = None;
    if let Some((key, path)) = verb_invocation(&args) {
        let dir = std::env::temp_dir().join("autorename-pdf-launch");
        match coalesce::collect(&dir, key, &path) {
            Ok(None) => std::process::exit(0),
            Ok(Some(paths)) if paths.is_empty() => std::process::exit(0),
            Ok(Some(paths)) => match write_request(&dir, &paths) {
                Ok(req) => {
                    args = Vec::new();
                    if key == "rename-now" {
                        args.push("--rename-now".into());
                    }
                    args.push("--request-file".into());
                    args.push(req.clone().into());
                    request = Some(req);
                }
                Err(e) => {
                    error_box(&format!("Could not hand over the selected files:\n{e}"));
                    std::process::exit(1);
                }
            },
            // Spool trouble: launch for this file alone.
            Err(_) => {}
        }
    }
    let child = std::process::Command::new(&exe)
        .arg("gui")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let ok = match (child, &request) {
        (Ok(mut c), Some(req)) => wait_ack(req, &mut c),
        (Ok(_), None) => true,
        (Err(e), _) => {
            error_box(&format!("Could not start {}:\n{e}", exe.display()));
            false
        }
    };
    if !ok
        && let Some(req) = &request
        && req.exists()
    {
        error_box("AutoRename-PDF did not open the selected files. Try again.");
        let _ = std::fs::remove_file(req);
    }
    std::process::exit(if ok { 0 } else { 1 });
}

#[cfg(not(windows))]
fn main() {
    eprintln!("autorename-pdf-gui is a Windows-only launcher; run `autorename-pdf gui` instead.");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_explorer_verbs_only() {
        let v = |a: &[&str]| a.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert_eq!(
            verb_invocation(&v(&["--rename-now", r"C:\a b.pdf"])),
            Some(("rename-now", r"C:\a b.pdf".into()))
        );
        assert_eq!(
            verb_invocation(&v(&[r"C:\x"])),
            Some(("preview", r"C:\x".into()))
        );
        assert_eq!(verb_invocation(&v(&[])), None);
        assert_eq!(verb_invocation(&v(&["a", "b"])), None);
    }

    /// Review finding: 200 long paths exceed CreateProcessW's 32,767-char
    /// limit. The hand-off file has no limit and the command line stays short.
    #[test]
    fn large_selections_go_through_a_request_file() {
        let dir = std::env::temp_dir().join(format!("arp-req-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<String> = (0..200)
            .map(|i| format!(r"C:\Users\someone\Documents\{}\{i}.pdf", "x".repeat(170)))
            .collect();
        assert!(paths.iter().map(|p| p.len() + 3).sum::<usize>() > 32_767);
        let req = write_request(&dir, &paths).unwrap();
        assert!(
            req.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("req-")
        );
        assert!(
            req.as_os_str().len() < 300,
            "the command line only carries the file name"
        );
        let back: Vec<String> = std::fs::read_to_string(&req)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(back, paths);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
