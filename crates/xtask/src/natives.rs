//! `cargo xtask fetch-natives [--target <triple>|--all]`
//!
//! Downloads the pinned PDFium build (bblanchon/pdfium-binaries) into
//! `vendor/pdfium/<triple>/`, verifying sha256. Core finds it there in dev builds
//! (after `AUTORENAME_PDFIUM_DIR` and the executable's directory).
//! Bumping the pin: `playbooks/natives-and-models.md`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// bblanchon/pdfium-binaries release tag. Must match pdfium-render's
/// `pdfium_<build>` feature in crates/core/Cargo.toml.
const PDFIUM_TAG: &str = "chromium/8125";

pub struct Native {
    pub triple: &'static str,
    pub asset: &'static str,
    pub sha256: &'static str,
    /// Library path inside the archive.
    pub lib: &'static str,
}

pub const PDFIUM: &[Native] = &[
    Native {
        triple: "x86_64-unknown-linux-gnu",
        asset: "pdfium-linux-x64.tgz",
        sha256: "e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659",
        lib: "lib/libpdfium.so",
    },
    Native {
        triple: "aarch64-unknown-linux-gnu",
        asset: "pdfium-linux-arm64.tgz",
        sha256: "8b8818f2ec57d1b213d952ca6d7f0ffeca9d0110487d0e20ad83ba918bc92efa",
        lib: "lib/libpdfium.so",
    },
    Native {
        triple: "x86_64-pc-windows-msvc",
        asset: "pdfium-win-x64.tgz",
        sha256: "9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b",
        lib: "bin/pdfium.dll",
    },
    Native {
        triple: "aarch64-pc-windows-msvc",
        asset: "pdfium-win-arm64.tgz",
        sha256: "d394ba61991fb53f4d87f70988035f337d9e73f2bd2c3c1b966df8c43fcabb05",
        lib: "bin/pdfium.dll",
    },
    Native {
        triple: "aarch64-apple-darwin",
        asset: "pdfium-mac-arm64.tgz",
        sha256: "4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d",
        lib: "lib/libpdfium.dylib",
    },
    Native {
        triple: "x86_64-apple-darwin",
        asset: "pdfium-mac-x64.tgz",
        sha256: "8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0",
        lib: "lib/libpdfium.dylib",
    },
];

pub fn host_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        _ => return None,
    })
}

pub fn workspace_root() -> PathBuf {
    // crates/xtask -> workspace root
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .unwrap_or(manifest)
        .to_path_buf()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn fetch(n: &Native) -> Result<(), String> {
    let dir = workspace_root().join("vendor/pdfium").join(n.triple);
    let stamp = dir.join(".sha256");
    let lib_name = Path::new(n.lib).file_name().expect("lib has a file name");
    if fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == n.sha256)
        && dir.join(lib_name).is_file()
    {
        println!("{}: up to date ({})", n.triple, dir.display());
        return Ok(());
    }
    let url = format!(
        "https://github.com/bblanchon/pdfium-binaries/releases/download/{PDFIUM_TAG}/{}",
        n.asset
    );
    println!("{}: downloading {url}", n.triple);
    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    client
        .get(&url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("download failed: {e}"))?
        .read_to_end(&mut bytes)
        .map_err(|e| format!("download failed: {e}"))?;
    let got = hex(&Sha256::digest(&bytes));
    if got != n.sha256 {
        return Err(format!(
            "sha256 mismatch for {url}: expected {}, got {got}",
            n.sha256
        ));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
    let mut found = false;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let rel = path.to_string_lossy().trim_start_matches("./").to_string();
        let out = if rel == n.lib {
            found = true;
            dir.join(lib_name)
        } else if rel == "LICENSE" || rel == "VERSION" {
            dir.join(&rel)
        } else {
            continue;
        };
        entry
            .unpack(&out)
            .map_err(|e| format!("{}: {e}", out.display()))?;
    }
    if !found {
        return Err(format!("{} not found in {}", n.lib, n.asset));
    }
    fs::write(&stamp, n.sha256).map_err(|e| format!("{}: {e}", stamp.display()))?;
    println!("{}: ok -> {}", n.triple, dir.display());
    Ok(())
}

/// Fetched library path for `triple` (`vendor/pdfium/<triple>/<lib>`).
pub fn lib_path(n: &Native) -> PathBuf {
    let name = Path::new(n.lib).file_name().expect("lib has a file name");
    workspace_root()
        .join("vendor/pdfium")
        .join(n.triple)
        .join(name)
}

pub fn find(triple: &str) -> Option<&'static Native> {
    PDFIUM.iter().find(|n| n.triple == triple)
}
