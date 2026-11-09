//! `cargo xtask sign <files…>`: Azure Trusted Signing (Artifact Signing)
//! through `signtool` + the Azure CodeSigning dlib, as in the 3.x
//! `build.py`. Windows only.
//!
//! - dlib: `$AZURE_CODE_SIGNING_DLIB`, else
//!   `%LOCALAPPDATA%\Microsoft\MicrosoftArtifactSigningClientTools\Azure.CodeSigning.Dlib.dll`.
//! - metadata: `$AZURE_CODE_SIGNING_METADATA`, else `<repo>/metadata.json`
//!   (gitignored; `{"Endpoint", "CodeSigningAccountName",
//!   "CertificateProfileName"}`). Credentials come from the Azure identity
//!   chain (`az login` or `AZURE_CLIENT_ID`/`AZURE_TENANT_ID`/`AZURE_CLIENT_SECRET`).
//! - `signtool`: `$SIGNTOOL`, else from `PATH`.

use std::path::{Path, PathBuf};
use std::process::Command;

const TIMESTAMP_SERVER: &str = "http://timestamp.acs.microsoft.com";

fn dlib() -> PathBuf {
    if let Some(p) = std::env::var_os("AZURE_CODE_SIGNING_DLIB") {
        return p.into();
    }
    let base = std::env::var_os("LOCALAPPDATA").unwrap_or_default();
    Path::new(&base)
        .join("Microsoft")
        .join("MicrosoftArtifactSigningClientTools")
        .join("Azure.CodeSigning.Dlib.dll")
}

fn metadata() -> PathBuf {
    std::env::var_os("AZURE_CODE_SIGNING_METADATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::natives::workspace_root().join("metadata.json"))
}

/// signtool arguments for one file (pure; printed with `--verbose` in CI logs).
pub fn signtool_args(dlib: &Path, metadata: &Path, file: &Path) -> Vec<String> {
    [
        "sign",
        "/v",
        "/dlib",
        &dlib.to_string_lossy(),
        "/dmdf",
        &metadata.to_string_lossy(),
        "/fd",
        "sha256",
        "/tr",
        TIMESTAMP_SERVER,
        "/td",
        "sha256",
        &file.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn sign_files(files: &[PathBuf]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("signing needs Windows (signtool + Azure dlib)".into());
    }
    let dlib = dlib();
    if !dlib.is_file() {
        return Err(format!(
            "Azure signing dlib not found: {} (install Microsoft Artifact Signing Client Tools or set AZURE_CODE_SIGNING_DLIB)",
            dlib.display()
        ));
    }
    let meta = metadata();
    if !meta.is_file() {
        return Err(format!(
            "signing metadata not found: {} (set AZURE_CODE_SIGNING_METADATA)",
            meta.display()
        ));
    }
    let signtool = std::env::var_os("SIGNTOOL").unwrap_or_else(|| "signtool".into());
    for f in files {
        println!("signing {}", f.display());
        let status = Command::new(&signtool)
            .args(signtool_args(&dlib, &meta, f))
            .status()
            .map_err(|e| format!("signtool: {e}"))?;
        if !status.success() {
            return Err(format!("signtool failed for {} ({status})", f.display()));
        }
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let files: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
    sign_files(&files)
}
