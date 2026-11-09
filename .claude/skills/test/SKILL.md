---
name: test
description: Run the AutoRename-PDF gates (fmt, clippy, tests), optional live provider tests, and point to Windows VM validation. Use when tests need to be executed or verified.
user-invocable: true
argument-hint: "[crate, test target or filter]"
allowed-tools:
  - Bash
  - Read
  - Grep
---

# Run tests

`PATH=$HOME/.local/share/mise/shims:$PATH` if `cargo` is missing. Use a 600 s timeout for cold builds.

1. Once per checkout: `cargo xtask fetch-natives` (PDFium; without it extraction tests fail with `pdfium_missing`).
2. Gates:
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
3. With an argument, narrow it, e.g. `cargo test -p autorename-core --test enrich`, `cargo test -p autorename-app --test cli <filter>`, or GUI: `cargo test -p autorename-app --bin autorename-pdf gui::`.
4. Optional:
   - OCR accuracy: `AUTORENAME_MODELS_DIR=<root with latin/> cargo test -p autorename-core --features ocr --test extract ocr_`
   - Live providers (costs money, needs real keys in `.env`): `AUTORENAME_LIVE=1 cargo test -p autorename-core --test providers_live -- --ignored --nocapture`. A 401 with `sk-your-…` is a placeholder key.
   - Offline end-to-end: `playbooks/live-poc.md`. Windows installer + context menu: `playbooks/windows-vm-validation.md`.
5. Report passed/failed/ignored per target; for failures show the test name and assertion.

Snapshot changes: help text via `INSTA_UPDATE=always … help_snapshots`, GUI PNGs via `UPDATE_SNAPSHOTS=force`; review every diff. Do not change a test to make it pass unless the test is wrong.
