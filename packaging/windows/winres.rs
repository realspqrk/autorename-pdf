// Shared by crates/app/build.rs and crates/winlaunch/build.rs (`include!`):
// embeds the app icon (`assets/icons/generated/autorename-pdf.ico`, resource
// id 1 = what Explorer shows and what `Icon=<exe>,0` refers to) and a
// VERSIONINFO block into Windows executables. No-op for other targets.
// Cross builds from Linux need `llvm-rc` (cargo xwin, see known.md).

fn embed_windows_resources(exe_name: &str, description: &str) {
    use std::path::Path;
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let root = Path::new(&manifest).join("../..");
    let ico = root.join("assets/icons/generated/autorename-pdf.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    println!("cargo:rerun-if-changed=../../packaging/windows/winres.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = std::fs::canonicalize(&ico).expect("generated icon (cargo xtask icons)");
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let mut nums: Vec<u16> = version
        .split(['.', '-'])
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    nums.resize(4, 0);
    let commas = nums
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {commas}
PRODUCTVERSION {commas}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Gerhard Petermeir"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{exe_name}"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "{exe_name}.exe"
      VALUE "ProductName", "AutoRename-PDF"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = ico.display().to_string().replace('\\', "/"),
    );
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("app.rc");
    std::fs::write(&out, rc).expect("write app.rc");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("embed Windows resources");
}
