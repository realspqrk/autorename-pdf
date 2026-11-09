---
name: build
description: Build AutoRename-PDF release packages (Windows NSIS installer, Linux AppImage/deb/pacman, macOS app/dmg) with cargo xtask dist, optionally signed. Use when the user wants a release build.
user-invocable: true
allowed-tools:
  - Bash
  - Read
  - Glob
---

# Build release packages

Follow `playbooks/release.md`; this is the short path.

1. Gates first (stop on failure):
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo xtask icons --check
   ```
2. Build for the host (600 s timeout; first builds are slow):
   - Linux: `cargo xtask dist` → `.AppImage`, `.deb`, pacman `.tar.gz` + `PKGBUILD`
   - Windows host: `cargo xtask dist --sign` (Azure Trusted Signing; needs signtool, the dlib, `metadata.json`, Azure login) or without `--sign`
   - Windows from Linux (unsigned test build): `cargo xtask dist --target x86_64-pc-windows-msvc` (needs `cargo-xwin`, `makensis`); `--no-package` for bare exes
   - macOS (Apple silicon only): `cargo xtask dist` (unsigned; signing: `playbooks/macos-release.md`)
3. Verify in `target/dist/<triple>/`: `sha256sum -c SHA256SUMS`, list files and sizes, smoke `--version` and `inspect tests/fixtures/text_invoice_acme.pdf`.
4. Windows installers: validate per `playbooks/windows-vm-validation.md` before publishing.

Never put `config.yaml`, `.env` or `name-lists/*.yaml` into a package. Version lives only in `[workspace.package] version` in `Cargo.toml`.
