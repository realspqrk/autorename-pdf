# Maintenance: cleaning up a dev checkout after 4.0

The 4.0 cleanup removed every tracked 3.x file (Python, Tauri GUI, scripts) and the Python caches. The items below are **yours** (untracked or outside the repo), so nothing deleted them. Run the commands from the repo root on Linux. Every path was checked on 2026-11-07 (sections 2–3 updated 2026-11-08).

## TOC
- [1. 3.x leftovers in the checkout](#1-3x-leftovers-in-the-checkout)
- [2. App data from dev and test runs](#2-app-data-from-dev-and-test-runs)
- [3. Build caches](#3-build-caches)
- [4. Old 3.x installs (Windows)](#4-old-3x-installs-windows)
- [5. After the 2026-11-10 history rewrite](#5-after-the-2026-11-10-history-rewrite)

## 1. 3.x leftovers in the checkout

| Path | What | Action |
|---|---|---|
| `venv/` (~250 MB) | 3.x Python virtualenv | Delete. |
| `config.yaml` | 3.x v2 config (`config_version: 2`, `paddleocr:`); 4.0 rejects it | Delete, or move your settings into a 4.x config with `autorename-pdf config init`. Don't commit it. |
| `.env` | API keys, used by the live tests (`AUTORENAME_LIVE=1`) | Keep. `OLLAMA_MODEL` in it is unused now. |

```bash
rm -rf venv/
rm -i config.yaml          # asks first; holds your 3.x keys/settings
git status --short --ignored   # should show only .env, target/, vendor/ as ignored
```

`.gitignore` still lists `/venv/` and `__pycache__/` so old checkouts stay clean. Drop those two lines once nobody has a 3.x venv any more.

## 2. App data from dev and test runs

`~/.local/share/autorename-pdf/` (Linux OS data dir; `autorename-pdf config path` prints it):

| Path | What | Action |
|---|---|---|
| `autorename.log` | 3.x log, written by the old pytest runs | Delete. |
| `journal.json.dev-2026-11-07.bak` | Old 4.0-dev undo journal, moved aside on 2026-11-09 (its format predates per-field name lists; both runs in it were already undone) | Delete when you like. |
| `models/latin/` (~38 MB) | OCR models | Keep for OCR. Remove with `autorename-pdf models remove latin`. |

```bash
cd ~/.local/share/autorename-pdf
rm -f autorename.log                             # 3.x log
rm -f journal.json.dev-2026-11-07.bak            # old dev journal, already moved aside
```

## 3. Build caches

| Path | What | Action |
|---|---|---|
| `target/` (~100 GB on 2026-11-08) | cargo output: `debug` (62 GB, `cargo run`), `main` (26 GB, a second dev build dir used by agents), `xwin`, `release`, `dist-build*` | `rm -rf target/main` frees 26 GB if you only use `cargo run`. |
| `target/` in general | cargo output, incl. `target/xwin` and `target/dist` | `cargo clean -n` shows the size, `cargo clean` removes it. Keep `target/dist/` if you still need built packages. |
| `vendor/pdfium/` | PDFium from `cargo xtask fetch-natives` | Keep (tests need it); re-fetch any time. |
| `~/.cache/dfbin/` (~450 MB) | ONNX Runtime downloads (ort-sys) | Safe to delete; the next build downloads it again. |
| `~/.cache/cargo-xwin/` (~1.2 GB) | Windows SDK splat for `cargo xwin` | Safe to delete; the next cross-build re-downloads it (and `xtask dist` re-creates the `PathCch.lib` link, see known.md). |

```bash
cargo clean -n                       # dry run: what would be deleted
cargo clean
rm -rf ~/.cache/dfbin ~/.cache/cargo-xwin
```

## 4. Old 3.x installs (Windows)

3.x and 4.0 share `%LOCALAPPDATA%\autorename-pdf`: 3.x put PaddleOCR there (`python-embed\`, `paddleocr-venv\`), 4.0 keeps its undo journal and OCR models there.

1. Run the 3.x portable folder's `setup.ps1`, choose uninstall, and answer **y** to "Remove context menu entries?". 4.0 neither reads nor removes the 3.x entries.
2. Answer **N** to "Remove PaddleOCR installation?": it deletes the whole `%LOCALAPPDATA%\autorename-pdf` folder, including 4.0's journal and models. Remove only the 3.x parts instead (PowerShell):
   ```powershell
   Remove-Item "$env:LOCALAPPDATA\autorename-pdf\paddleocr-venv","$env:LOCALAPPDATA\autorename-pdf\python-embed" -Recurse -Force -ErrorAction SilentlyContinue
   ```
3. Delete the portable `AutoRename-PDF-Portable-*` folder (it held the 3.x `config.yaml`, EXEs and logs).

On Linux/macOS the 3.x OCR venv was `~/.local/share/autorename-pdf/paddleocr-venv/` (not present on this machine): `rm -rf ~/.local/share/autorename-pdf/paddleocr-venv`.

## 5. After the 2026-11-10 history rewrite

Real-document details (a customer name, one invoice number and amount, a document title) were removed from the 16 commits after `0841ab7` with `git filter-repo` and force-pushed (`main`, tag `v4.0.0-beta.2`). Old copies still exist:

| Where | What | Action |
|---|---|---|
| GitHub `refs/pull/25/head` (`cb3e591`) and cached views | old commits, reachable from PR #25 | Ask GitHub Support to purge them: https://support.github.com/contact, "Remove cached views / sensitive data", repo `realspqrk/autorename-pdf`, PR #25, old SHA `cb3e591`. |
| `~/.cache/arp-backup/pre-rewrite-*.bundle` (~217 MB) | full pre-rewrite backup | Delete once the rewritten history looks right: `rm -rf ~/.cache/arp-backup`. |
| `~/Windows/arp.bundle` and the VM checkout `C:\src\arp` | old branch commits | Local only. Next VM build: `git fetch` the new bundle and `git reset --hard`; delete the old bundle: `rm ~/Windows/arp.bundle`. |
| Other clones (Mac `~/Code/autorename-pdf` if it fetched) | old history | `git fetch origin && git reset --hard origin/main && git tag -d v4.0.0-beta.2 && git fetch --tags`. |

