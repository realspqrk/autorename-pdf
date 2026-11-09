# Release (4.x)

## TOC
- [1. Version bump](#1-version-bump)
- [2. Gates](#2-gates)
- [3. Build packages per OS](#3-build-packages-per-os)
- [4. Windows signing](#4-windows-signing)
- [5. Verify, checksums, publish](#5-verify-checksums-publish)
- [What is where](#what-is-where)

## 1. Version bump
- Edit `[workspace.package] version` in `Cargo.toml` (one place: every crate, the exe VERSIONINFO, and cargo-packager read it).
- `cargo check --workspace` to refresh `Cargo.lock`. Then commit as `Bump version to X.Y.Z`.

## 2. Gates
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask icons --check            # generated icons match the SVG masters
cargo xtask notices --check          # THIRD_PARTY_NOTICES.md is current (cargo-about 0.9.3)
cargo deny check licenses           # license allowlist (cargo-deny 0.20.2; deny.toml)
AWS_LC_SYS_PREBUILT_NASM=1 CARGO_TARGET_DIR=target/xwin \
  cargo xwin clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

## 3. Build packages per OS
`cargo xtask dist [--target <triple>] [--formats a,b] [--no-package] [--sign]` does all of this:
it fetches PDFium, builds release with `--locked`, stages binaries, natives and licenses in `target/dist/<triple>/stage/`, runs cargo-packager 0.11.8 (it installs that version when missing), and writes `SHA256SUMS`.

| OS (build host) | Command | Output in `target/dist/<triple>/` |
|---|---|---|
| Linux x86_64 | `cargo xtask dist` | `.AppImage`, `.deb`, pacman `.tar.gz` + `PKGBUILD` |
| Windows x64 (Windows host) | `cargo xtask dist --sign` | `AutoRename-PDF_X.Y.Z_x64-setup.exe` (NSIS, per user) |
| Windows from Linux (unsigned test build) | `cargo xtask dist --target x86_64-pc-windows-msvc` | needs `makensis` (`nsis` package) and `cargo-xwin` |
| macOS arm64 | `cargo xtask dist` | `.app`, `.dmg`; signing + notarization: [macos-release.md](macos-release.md) |

- AppImage: cargo-packager downloads linuxdeploy from GitHub, and the build sets `APPIMAGE_EXTRACT_AND_RUN=1` (works without FUSE).
- Pacman: the output is a source tarball plus a PKGBUILD. Install it with `makepkg -si` in a directory that holds both files.
- Smoke test (Linux): `./autorename-pdf_X.Y.Z_x86_64.AppImage --version`, then `AUTORENAME_GUI_DEMO=1 ./…AppImage gui` (the window must open). `inspect <pdf>` proves PDFium loads (`/usr/lib/autorename-pdf/libpdfium.so` inside the image).
- NSIS: `packaging/nsis/installer.nsi` is the upstream 0.11.8 template plus two hooks and a check for the running `autorename-pdf.exe`. On Windows, `integrate install` also puts the install folder on the user PATH (`HKCU\Environment` `Path`, REG_EXPAND_SZ, no duplicates), and `integrate remove` takes exactly that entry off again. On Linux, it adds an owned `~/.local/bin/autorename-pdf` symlink (AppImage) unless PATH already resolves to the same exe. Post-install runs `autorename-pdf.exe integrate install`, pre-uninstall runs `integrate remove`. When you bump cargo-packager, re-sync the template (diff it against `~/.cargo/registry/src/*/cargo-packager-<ver>/src/package/nsis/installer.nsi`) and update `PACKAGER_VERSION` in `crates/xtask/src/dist.rs`.
- Icons: edit the SVG masters in `assets/icons/`, run `cargo xtask icons`, and commit `assets/icons/generated/`. The exes embed `generated/autorename-pdf.ico` via `packaging/windows/winres.rs`.

## 4. Windows signing
Azure Trusted Signing (Artifact Signing), ported from 3.x `build.py`:
- Needs `signtool` (Windows SDK) on `PATH` or in `$SIGNTOOL`.
- Needs the dlib: `$AZURE_CODE_SIGNING_DLIB`, else `%LOCALAPPDATA%\Microsoft\MicrosoftArtifactSigningClientTools\Azure.CodeSigning.Dlib.dll`.
- Needs the metadata JSON: `$AZURE_CODE_SIGNING_METADATA`, else `<repo>/metadata.json` (committed; no secrets). It holds `{"Endpoint": "...", "CodeSigningAccountName": "...", "CertificateProfileName": "..."}`.
- Needs Azure credentials: `az login`, or `AZURE_CLIENT_ID` / `AZURE_TENANT_ID` / `AZURE_CLIENT_SECRET`.
- CI: `.github/workflows/release.yml` builds Linux and Windows packages on a `v*` tag and signs on Windows when the `AZURE_CLIENT_*` secrets exist (it installs the dlib from NuGet `Microsoft.Trusted.Signing.Client`). The result is a draft release.
- `cargo xtask dist --sign` signs `autorename-pdf.exe` before packaging. It then sets cargo-packager's `windows.signCommand = "cargo xtask sign %1"`, which signs the launcher, the uninstaller and the installer.
- Manual signing: `cargo xtask sign <file>...` (sha256, timestamp `http://timestamp.acs.microsoft.com`).
- Verify with `signtool verify /pa /v <file>`. `pdfium.dll` (third party) is not signed by us.

### Local signed build in the Windows VM
Validated for 4.0.0 (Linux has no `makensis` host that can sign the uninstaller, so build natively). Toolchain, once per VM (each as a `vm.sh run` job):
- `winget install -e --silent --accept-source-agreements --accept-package-agreements` with `Rustlang.Rustup`, `Microsoft.AzureCLI`, `Git.Git`, and MSVC Build Tools (`windows-vm` skill).
- Signing dlib: download the NuGet package `Microsoft.Trusted.Signing.Client` (a zip) and extract it to `C:\ats\pkg`. `AZURE_CODE_SIGNING_DLIB=C:\ats\pkg\bin\x64\Azure.CodeSigning.Dlib.dll`.
- signtool: `C:\Program Files (x86)\Windows Kits\10\bin\<sdk>\x64\signtool.exe`.
- Code: `git bundle create ~/Windows/arp.bundle <branch>`, then clone the bundle to `C:\src\arp` in the guest.

Then:
1. The user runs `az login` in a PowerShell window in the RDP session. The runner runs in that same session, so it shares the CLI login. Check it with `az account get-access-token --resource https://codesigning.azure.net --query expiresOn -o tsv`.
2. Run the build detached. It takes longer than the 10-minute `vm.sh run` timeout (about 20 min cold). A script in `~/Windows` sets `AWS_LC_SYS_PREBUILT_NASM=1`, `AZURE_CODE_SIGNING_DLIB` and `SIGNTOOL`, runs `cargo xtask dist --sign 2>&1 | % { "$_" } | Out-File -Append -Encoding ascii \\host.lan\Data\<log>`, and ends with an `exit $LASTEXITCODE` line. Start it with `Start-Process powershell -File <local copy> -WindowStyle Hidden`, then wait on the log from the host. Plain `*>>` writes UTF-16, which is hard to grep.
3. `signtool verify /pa /v` on `autorename-pdf.exe`, `autorename-pdf-gui.exe`, `uninstall.exe` (from the install dir) and the setup exe.

## 5. Verify, checksums, publish
1. Run `sha256sum -c SHA256SUMS` in each `target/dist/<triple>/`.
2. Windows: follow [windows-vm-validation.md](windows-vm-validation.md) (install, context menu, CLI smoke, uninstall).
3. Linux: smoke-test the AppImage (above). `autorename-pdf integrate install && autorename-pdf integrate status && autorename-pdf integrate remove` must leave nothing behind.
4. Create the GitHub release (CI `release.yml` does the same as a draft). Rename each manifest per triple first so the names don't collide:
   ```bash
   for d in target/dist/*/; do t=$(basename "$d"); mv "$d/SHA256SUMS" "$d/SHA256SUMS-$t"; done
   gh release create vX.Y.Z --draft $(ls target/dist/*/*.{exe,AppImage,deb,tar.gz,dmg} target/dist/*/PKGBUILD target/dist/*/SHA256SUMS-* 2>/dev/null)
   ```
   Step 1's `sha256sum -c` must run before the rename (or use `sha256sum -c SHA256SUMS-<triple>`).

## What is where
| Item | Path |
|---|---|
| Packager base config | `Packager.toml`. Per-target fields are added by `crates/xtask/src/dist.rs`, and the final config goes to `target/dist/<triple>/packager.json` |
| NSIS template | `packaging/nsis/installer.nsi` |
| Linux .desktop template (deb, pacman, AppImage) | `packaging/linux/autorename-pdf.desktop.hbs` |
| Exe icon + VERSIONINFO | `packaging/windows/winres.rs`, included by `crates/{app,winlaunch}/build.rs` |
| PDFium in packages | Windows: beside the exe. Linux: `/usr/lib/autorename-pdf/`. macOS: `Contents/Resources/` (`main.rs::locate_packaged_pdfium`) |
| Desktop integration | `autorename-pdf integrate install\|remove\|status` (`crates/app/src/integrate/`) |
