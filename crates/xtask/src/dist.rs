//! `cargo xtask dist [--target <triple>] [--formats <list>] [--no-package] [--sign]`
//!
//! 1. Fetch the pinned PDFium for the target (`fetch-natives`).
//! 2. `cargo build --release --locked` (Windows from a non-Windows host:
//!    `cargo xwin build`, see known.md) of `autorename-pdf` (+ the
//!    `autorename-pdf-gui` launcher on Windows).
//! 3. Stage binaries + natives + licenses in `target/dist/<triple>/stage/`.
//! 4. Run cargo-packager (pinned [`PACKAGER_VERSION`], installed with
//!    `cargo install --locked` when missing) with `Packager.toml` plus the
//!    per-target fields, packages land in `target/dist/<triple>/`.
//! 5. Write `SHA256SUMS` next to the packages.
//!
//! `--sign` (Windows host): signs `autorename-pdf.exe` before packaging and
//! lets cargo-packager sign the launcher, uninstaller and installer through
//! `cargo xtask sign %1`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::natives;

/// cargo-packager release the NSIS template in `packaging/nsis/` is based on.
pub const PACKAGER_VERSION: &str = "0.11.8";

pub struct Opts {
    pub target: String,
    pub formats: Option<String>,
    pub package: bool,
    pub sign: bool,
}

impl Opts {
    pub fn parse(args: &[String]) -> Option<Opts> {
        let mut o = Opts {
            target: natives::host_triple()?.to_string(),
            formats: None,
            package: true,
            sign: false,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--target" => o.target = it.next()?.clone(),
                "--formats" => o.formats = Some(it.next()?.clone()),
                "--no-package" => o.package = false,
                "--sign" => o.sign = true,
                _ => return None,
            }
        }
        Some(o)
    }

    fn windows(&self) -> bool {
        self.target.contains("windows")
    }

    fn macos(&self) -> bool {
        self.target.contains("apple")
    }

    fn default_formats(&self) -> &'static str {
        if self.windows() {
            "nsis"
        } else if self.macos() {
            "app,dmg"
        } else {
            "appimage,pacman,deb"
        }
    }
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn exec(cmd: &mut Command) -> Result<(), String> {
    println!("$ {cmd:?}");
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed ({status})"))
    }
}

fn workspace_version(root: &Path) -> Result<String, String> {
    let text = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let v: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    v.get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "no [workspace.package] version in Cargo.toml".into())
}

/// ONNX Runtime's static lib asks the linker for `PathCch.lib` and
/// `DirectML.lib`; the xwin splat on a case-sensitive FS only has the
/// lower/upper-case names.
#[cfg(unix)]
fn xwin_case_links(target: &str) {
    let arch = if target.starts_with("aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dir = Path::new(&home)
        .join(".cache/cargo-xwin/xwin/sdk/lib/um")
        .join(arch);
    for want in ["PathCch.lib", "DirectML.lib"] {
        let have = want.to_lowercase();
        if dir.join(&have).is_file() && !dir.join(want).exists() {
            let _ = std::os::unix::fs::symlink(have, dir.join(want));
        }
    }
}

#[cfg(not(unix))]
fn xwin_case_links(_: &str) {}

fn build(o: &Opts, root: &Path) -> Result<PathBuf, String> {
    let cross_windows = o.windows() && !cfg!(windows);
    let mut cmd = Command::new(cargo());
    cmd.current_dir(root);
    if cross_windows {
        xwin_case_links(&o.target);
        cmd.arg("xwin");
        // aws-lc-sys: prebuilt NASM objects (no nasm on the host).
        cmd.env("AWS_LC_SYS_PREBUILT_NASM", "1");
    }
    cmd.args(["build", "--release", "--locked", "--target", &o.target]);
    cmd.args(["-p", "autorename-app"]);
    if o.windows() {
        cmd.args(["-p", "autorename-winlaunch"]);
    }
    if o.target == "x86_64-apple-darwin" {
        // No prebuilt ONNX Runtime for Intel Macs (known.md).
        cmd.args(["--no-default-features", "--features", "gui"]);
    }
    exec(&mut cmd)?;
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|d| if d.is_absolute() { d } else { root.join(d) })
        .unwrap_or_else(|| root.join("target"));
    Ok(target_dir.join(&o.target).join("release"))
}

fn copy(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(d) = dst.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    fs::copy(src, dst).map_err(|e| format!("{} -> {}: {e}", src.display(), dst.display()))?;
    Ok(())
}

/// Stage layout: `bin/` (binaries), `res/` (installed beside the exe on
/// Windows, `/usr/lib/autorename-pdf/` on Linux, `Contents/Resources/` on
/// macOS: the app finds PDFium in all three).
fn stage(o: &Opts, root: &Path, release: &Path, n: &natives::Native) -> Result<PathBuf, String> {
    let stage = root.join("target/dist").join(&o.target).join("stage");
    let _ = fs::remove_dir_all(&stage);
    let exe = if o.windows() { ".exe" } else { "" };
    let mut bins = vec!["autorename-pdf"];
    if o.windows() {
        bins.push("autorename-pdf-gui");
    }
    for b in bins {
        let name = format!("{b}{exe}");
        copy(&release.join(&name), &stage.join("bin").join(&name))?;
    }
    // ORT's DirectML build (Windows OCR) imports DirectML.dll; ort-sys copies
    // the redistributable next to the binaries. Ship it beside the exe.
    let dml = release.join("DirectML.dll");
    if o.windows() && dml.is_file() {
        copy(&dml, &stage.join("res/DirectML.dll"))?;
    }
    let lib = natives::lib_path(n);
    copy(
        &lib,
        &stage.join("res").join(lib.file_name().unwrap_or_default()),
    )?;
    let pdfium_dir = lib.parent().unwrap_or(root);
    copy(
        &pdfium_dir.join("LICENSE"),
        &stage.join("res/licenses/PDFium-LICENSE.txt"),
    )?;
    copy(
        &root.join("LICENSE"),
        &stage.join("res/licenses/AutoRename-PDF-LICENSE.txt"),
    )?;
    for f in ["Inter-OFL.txt", "JetBrainsMono-OFL.txt", "Lucide-ISC.txt"] {
        copy(
            &root.join("crates/app/assets/fonts/LICENSES").join(f),
            &stage.join("res/licenses").join(f),
        )?;
    }
    Ok(stage)
}

/// `cargo-packager 0.11.8` → `0.11.8`.
pub fn parse_packager_version(out: &str) -> Option<&str> {
    out.split_whitespace()
        .nth(1)
        .filter(|v| v.split('.').count() == 3 && v.split('.').all(|p| p.parse::<u32>().is_ok()))
}

/// The NSIS template is tied to one cargo-packager release: install it when
/// missing, refuse any other version.
fn ensure_packager() -> Result<(), String> {
    let out = Command::new(cargo())
        .args(["packager", "--version"])
        .output();
    if let Ok(o) = &out
        && o.status.success()
    {
        let text = String::from_utf8_lossy(&o.stdout);
        return match parse_packager_version(&text) {
            Some(v) if v == PACKAGER_VERSION => Ok(()),
            other => Err(format!(
                "cargo-packager {} found, but packaging/nsis/installer.nsi needs exactly {PACKAGER_VERSION}: \
                 cargo install cargo-packager --locked --force --version {PACKAGER_VERSION}",
                other.unwrap_or(text.trim())
            )),
        };
    }
    exec(Command::new(cargo()).args([
        "install",
        "cargo-packager",
        "--locked",
        "--version",
        PACKAGER_VERSION,
    ]))
}

/// Release tags must be `v<workspace version>` with a semver version
/// (`MAJOR.MINOR.PATCH[-pre]`), so a tag can never ship other binaries.
pub fn check_tag(tag: &str, version: &str) -> Result<(), String> {
    let v = tag
        .strip_prefix('v')
        .ok_or_else(|| format!("tag {tag:?} must start with `v`"))?;
    let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
    let nums: Vec<&str> = core.split('.').collect();
    let num_ok = |n: &&str| {
        !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
            && (n.len() == 1 || !n.starts_with('0'))
    };
    let pre_ok = pre.is_none_or(|p| {
        !p.is_empty()
            && p.split('.').all(|id| {
                !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
    });
    if nums.len() != 3 || !nums.iter().all(num_ok) || !pre_ok {
        return Err(format!("tag {tag:?} is not v<semver> (e.g. v4.0.0)"));
    }
    if v != version {
        return Err(format!(
            "tag {tag:?} does not match the workspace version {version} (Cargo.toml [workspace.package])"
        ));
    }
    Ok(())
}

/// `cargo xtask check-tag <tag>`.
pub fn check_tag_cmd(tag: &str) -> Result<(), String> {
    let version = workspace_version(&natives::workspace_root())?;
    check_tag(tag, &version)?;
    println!("tag {tag} matches workspace version {version}");
    Ok(())
}

/// `Packager.toml` + per-target fields, as the JSON passed to `--config`.
fn packager_config(o: &Opts, root: &Path, stage: &Path, out: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(root.join("Packager.toml")).map_err(|e| e.to_string())?;
    let base: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut cfg = serde_json::to_value(base).map_err(|e| e.to_string())?;
    let m = cfg.as_object_mut().ok_or("Packager.toml is not a table")?;
    m.insert("version".into(), json!(workspace_version(root)?));
    m.insert("binariesDir".into(), json!(stage.join("bin")));
    m.insert("outDir".into(), json!(out));
    m.insert("targetTriple".into(), json!(o.target));
    let binaries = if o.windows() {
        // The launcher is "main": Start-menu shortcut + finish-page run.
        json!([{"path": "autorename-pdf-gui", "main": true}, {"path": "autorename-pdf", "main": false}])
    } else {
        json!([{"path": "autorename-pdf", "main": true}])
    };
    m.insert("binaries".into(), binaries);
    m.insert(
        "resources".into(),
        json!([{"src": stage.join("res"), "target": "."}]),
    );
    if o.sign {
        m.insert(
            "windows".into(),
            json!({"signCommand": "cargo xtask sign %1"}),
        );
    }
    Ok(cfg)
}

fn checksums(out: &Path) -> Result<(), String> {
    let mut lines = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(out)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .is_some_and(|n| n != "SHA256SUMS" && n != "packager.json")
        })
        .collect();
    entries.sort();
    for p in entries {
        let bytes = fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        lines.push(format!("{}  {name}", natives::hex(&Sha256::digest(&bytes))));
        println!("{:>10}  {name}", bytes.len());
    }
    fs::write(out.join("SHA256SUMS"), lines.join("\n") + "\n").map_err(|e| e.to_string())
}

pub fn run(o: &Opts) -> Result<(), String> {
    let root = natives::workspace_root();
    let n = natives::find(&o.target).ok_or_else(|| format!("no pinned PDFium for {}", o.target))?;
    natives::fetch(n)?;
    let release = build(o, &root)?;
    let stage = stage(o, &root, &release, n)?;
    println!("staged {}", stage.display());
    if !o.package {
        return Ok(());
    }
    if o.sign {
        crate::sign::sign_files(&[stage.join("bin/autorename-pdf.exe")])?;
    }
    ensure_packager()?;
    let out = root.join("target/dist").join(&o.target);
    let cfg = packager_config(o, &root, &stage, &out)?;
    let cfg_text = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
    fs::write(
        out.join("packager.json"),
        serde_json::to_string_pretty(&cfg).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    let formats = o.formats.as_deref().unwrap_or(o.default_formats());
    // Raw JSON config: relative paths in Packager.toml resolve against the
    // repo root (cwd); a config *file* would make them relative to its dir.
    let mut packager = Command::new(cargo());
    if !o.windows() && !o.macos() {
        packager.env("XDG_CACHE_HOME", appimage_tools(&root, &out)?);
    }
    exec(
        packager
            .current_dir(&root)
            .env("APPIMAGE_EXTRACT_AND_RUN", "1")
            .args(["packager", "--formats", formats, "--config", &cfg_text]),
    )?;
    rename_installer(&out)?;
    checksums(&out)
}

/// cargo-packager copies its tools from `<cache>/.cargo-packager/AppImage/`
/// (downloading missing ones), including a desktop-file AppRun that prepends
/// the `.desktop` Exec args (`gui`) to every call. Use a per-dist tool cache
/// seeded with our pass-through `packaging/linux/AppRun` (and the shared
/// linuxdeploy downloads, when present) and point `XDG_CACHE_HOME` at it.
fn appimage_tools(root: &Path, out: &Path) -> Result<PathBuf, String> {
    let cache = out.join("packager-cache");
    let dir = cache.join(".cargo-packager/AppImage");
    let arch = std::env::consts::ARCH;
    let apprun = dir.join(format!("AppRun-{arch}"));
    copy(&root.join("packaging/linux/AppRun"), &apprun)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&apprun, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", apprun.display()))?;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let shared = Path::new(&home).join(".cache/.cargo-packager/AppImage");
        for f in [
            format!("linuxdeploy-{arch}.AppImage"),
            "linuxdeploy-plugin-appimage.AppImage".to_string(),
        ] {
            if shared.join(&f).is_file() && !dir.join(&f).is_file() {
                copy(&shared.join(&f), &dir.join(&f))?;
            }
        }
    }
    Ok(cache)
}

/// cargo-packager names the NSIS installer after the main binary (the
/// `autorename-pdf-gui` launcher); publish it under the product name.
fn rename_installer(out: &Path) -> Result<(), String> {
    for e in fs::read_dir(out).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix("autorename-pdf-gui_") {
            let to = out.join(format!("AutoRename-PDF_{rest}"));
            fs::rename(e.path(), &to).map_err(|e| format!("{}: {e}", to.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packager_version_must_match_exactly() {
        assert_eq!(
            parse_packager_version("cargo-packager 0.11.8\n"),
            Some("0.11.8")
        );
        assert_eq!(
            parse_packager_version("cargo-packager 0.11.80"),
            Some("0.11.80")
        );
        assert_ne!(
            parse_packager_version("cargo-packager 0.11.80"),
            Some(PACKAGER_VERSION)
        );
        assert_eq!(parse_packager_version("garbage"), None);
    }

    #[test]
    fn release_tag_must_equal_the_workspace_version() {
        assert!(check_tag("v4.0.0", "4.0.0").is_ok());
        assert!(check_tag("v4.1.0-rc.1", "4.1.0-rc.1").is_ok());
        assert!(
            check_tag("v4.0.1", "4.0.0")
                .unwrap_err()
                .contains("does not match")
        );
        for bad in [
            "4.0.0", "v4.0", "v04.0.0", "v4.0.0-", "vfoo", "v4.0.0+x", "v4..0",
        ] {
            assert!(check_tag(bad, "4.0.0").is_err(), "{bad}");
        }
    }
}
