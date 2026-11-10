# AutoRename-PDF

Rename PDFs from their content (AI + local OCR). Rust workspace, one binary `autorename-pdf` (CLI + egui GUI). Default name `YYYYMMDD COMPANY AP EUR 12.13.pdf`; academic profile: date, author, venue, title.

Tool-agnostic source of truth for agents. Be concise; smallest correct diff.

## Setup runbook

For agents asked to "set up AutoRename-PDF" for a user. Non-interactive; ask the user only for the provider + API key (or "offline with Ollama") and their company name. Never put a key on a command line or in chat output.

1. **Get the binary** (`$APP` below). 4.x is in beta: always pass the tag `v4.0.0-beta.2` (without it `gh` fetches 3.x, the latest stable release).
   - Windows x64: `gh release download v4.0.0-beta.2 --repo realspqrk/autorename-pdf --pattern "*_x64-setup.exe"`, run it with `/S` (per-user, no admin). `$APP` = `autorename-pdf` (on the user PATH in new shells) or `%LOCALAPPDATA%\Programs\AutoRename-PDF\autorename-pdf.exe`.
   - Linux x86_64: `gh release download v4.0.0-beta.2 --repo realspqrk/autorename-pdf --pattern "*.AppImage"`, `chmod +x`. `$APP` = that file. Preflight FUSE: `ls /dev/fuse && (command -v fusermount3 || command -v fusermount)`; if that fails: `$APP --appimage-extract && mkdir -p ~/.local/opt && mv squashfs-root ~/.local/opt/autorename-pdf` and use `$APP` = `~/.local/opt/autorename-pdf/AppRun` (stable path for `--integrate`).
   - macOS Apple silicon: `gh release download v4.0.0-beta.2 --repo realspqrk/autorename-pdf --pattern "*_aarch64.dmg"`, then `hdiutil attach -nobrowse -mountpoint /tmp/arp-dmg <dmg> && ditto /tmp/arp-dmg/AutoRename-PDF.app ~/Applications/AutoRename-PDF.app && hdiutil detach /tmp/arp-dmg`. `$APP` = `~/Applications/AutoRename-PDF.app/Contents/MacOS/autorename-pdf`.
   - Else (other arch), from source: `git clone https://github.com/realspqrk/autorename-pdf && cd autorename-pdf && cargo xtask fetch-natives && cargo build --release -p autorename-app`. `$APP` = `target/release/autorename-pdf` (run it from the clone; it finds PDFium in `vendor/`). Intel Macs are not supported.
   Then read `$APP --help` and `$APP setup --help`. Before first use of any other command read `$APP <command> --help`: the installed help is the usage reference.
2. `$APP setup --check -o json` → read `steps[]` (`missing` + `hint`). Exit 0 = ready.
3. Configure (idempotent; safe to re-run):
   - Cloud: key in an env var → `$APP setup --provider openai --api-key-env OPENAI_API_KEY --company "<Company>" -o json`; key only in your context → `printf %s "$KEY" | $APP setup --provider <p> --api-key-stdin --company "<Company>" -o json`.
   - Offline: `ollama pull gemma4` (CPU only: `gemma4:e2b`). OCR models first, while not yet offline: `$APP setup --ocr-models`; air-gapped: copy the `models` folder from a connected machine and run `$APP models import <dir>` (`<dir>` = that `models` folder or its `latin` subfolder; `--pack` only for other languages). Then `$APP setup --offline --ollama-model <model> --company "<Company>" -o json` (add `--ollama-url` if not local). Saved offline mode refuses every download.
   - Offline → cloud: `$APP setup --cloud --provider <p> --api-key-env VAR -o json` (saved offline mode is never left implicitly). A provider change resets `ai.base_url`; pass `--base-url URL` for a gateway.
   - Exit 0 = ready; 3 = see the failed step's `hint`; 2 = bad flags.
4. Verify with the demo (needs the repo's `examples/demo/`): `cp -r examples/demo ./demo && $APP rename --plan plan.json demo -o json` → exit 0, `summary.total` 7 (expected names: `examples/demo/README.md`). To rename for real without paying for a second analysis: `$APP apply plan.json -o json` (to change a name, edit `items[].proposed_name` first), `$APP undo -o json` reverts.
   Isolation for tests/sandboxes: `AUTORENAME_PDF_DATA_DIR=<dir>` (journal, OCR models) + `--config <file>`.
5. Optional: `$APP setup --integrate` (file-manager "Rename now/Preview"). Cloud mode: `$APP setup --ocr-models` (~40 MB, scans read locally); offline: see step 3.
6. Tell the user: GUI = run `$APP` (no arguments); CLI = `$APP rename --dry-run <folder>`, then `$APP rename <folder>` (analyzes again), `$APP undo`. For your own runs use `rename --plan` + `apply` (one analysis).

## Sources of truth

Do not copy these into this file.

| Need | Where |
|---|---|
| Product spec, decisions, milestones | `docs/superpowers/specs/2026-11-07-rust-rebuild.md` |
| GUI tokens, screens, keyboard, review checklist | `docs/superpowers/specs/2026-11-07-gui-design.md` |
| Provider request/response details | `docs/superpowers/specs/2026-11-07-provider-reference.md` |
| Parity contract (naming, quality score) | `docs/superpowers/specs/2026-11-07-python-behavior-contract.md` |
| User docs (quick start; install details, reading steps, offline, embedded data, name matching) | `README.md`, `docs/guide.md` |
| Config keys and defaults | `config.yaml.example`, `examples/` (validated by `crates/app/tests/examples.rs`) |
| CLI behavior | `autorename-pdf <cmd> --help` (snapshots in `crates/app/tests/snapshots/`) |
| Runbooks: release, natives/models, live POC, Windows VM | `playbooks/` |
| Traps already hit (read before debugging) | `known.md` |
| Open work | `followup.md` |
| 3.x → 4.0 user changes | `docs/migration-3-to-4.md` |

## Architecture

| Crate / module | Role |
|---|---|
| `crates/core` | Library, functional style (plain functions over data) |
| `core::config` | YAML v4 load, discovery, `${VAR}` + `.env`, strict schema, defaults, overrides |
| `core::profiles` | Built-in/overlay profiles, prompt interpolation, per-profile JSON Schema |
| `core::extract` | PDFium text (pdfplumber parity), per-page readability + visual cap (`extract::visual`), page render; `with_pdfium` mutex |
| `core::ocr` | oar-ocr PP-OCRv6 engine (feature `ocr`), model manifest/fetch/import |
| `core::providers` | 5 blocking reqwest adapters (openai, anthropic, gemini, xai, ollama), schema validation, name decider |
| `core::pipeline` | `plan_files` (extract → AI → harmonize → name) and `apply_plan`, shared by CLI and GUI |
| `core::naming`, `core::dates` | Filename rendering, collisions; strict date parsing |
| `core::harmonize` | Name matching: per-field name lists (`name-lists/<field>.yaml`), normalization by kind, matching bands, auto-learning, `update_db` |
| `core::enrich` | Info/XMP/attachment incremental update, skip reasons, undo truncation |
| `core::journal` | Undo journal (OS data dir), batches, locks |
| `core::fsx` | Every file mutation: atomic write, no-replace rename, verified replace, locks |
| `crates/app` | `autorename-pdf`: `cli.rs` (clap + help), `cmd.rs`, `out.rs` (output contract), `gui/` (egui), `integrate/` (context menus) |
| `crates/winlaunch` | Windows `autorename-pdf-gui.exe` launcher (no console, coalesces Explorer multi-select) |
| `crates/xtask` | `fetch-natives`, `icons`, `dist`, `sign` |
| `packaging/`, `Packager.toml` | NSIS template + hooks, Linux `.desktop`, exe resources (cargo-packager) |

## Commands

Toolchain via mise: prefix `PATH=$HOME/.local/share/mise/shims:$PATH` when `cargo` is not found. First build: use a 600 s timeout.

| Task | Command |
|---|---|
| PDFium for dev/tests (once) | `cargo xtask fetch-natives` |
| Run CLI | `cargo run -p autorename-app -- rename --dry-run <paths>` |
| Run GUI with demo data | `AUTORENAME_GUI_DEMO=1 cargo run -p autorename-app -- gui` |
| Format | `cargo fmt --all --check` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Test | `cargo test --workspace` |
| GUI tests only | `cargo test -p autorename-app --bin autorename-pdf gui::` (refresh PNGs: `UPDATE_SNAPSHOTS=force`) |
| Help snapshots | `INSTA_UPDATE=always cargo test -p autorename-app --test cli help_snapshots`, review the diff |
| OCR accuracy test | `AUTORENAME_MODELS_DIR=<root with latin/> cargo test -p autorename-core --features ocr --test extract ocr_` |
| Live providers | `AUTORENAME_LIVE=1 cargo test -p autorename-core --test providers_live -- --ignored --nocapture` (keys from `.env`) |
| Icons | `cargo xtask icons [--check]` |
| Third-party notices | `cargo xtask notices [--check]` (needs cargo-about 0.9.3; allowlist in `about.toml` + `deny.toml`; manual entries in `assets/licenses/`) |
| Packages | `cargo xtask dist [--target <triple>] [--sign]` (see `playbooks/release.md`) |
| Windows cross-check | `AWS_LC_SYS_PREBUILT_NASM=1 CARGO_TARGET_DIR=target/xwin cargo xwin clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings` |

Machine output: `--output json`. Exit codes: 0 ok, 1 error, 2 usage, 3 config, 4 no files, 5 partial, 10 provider, 11 auth.

## Hard rules

- **No backward compatibility.** Never read or emit 3.x config, flags, logs or file formats. Users get `docs/migration-3-to-4.md`.
- Never commit `config.yaml`, `.env` or `name-lists/*.yaml` (user name lists). Only the `.example` files.
- Core is functional: plain functions over data structs/enums; no trait-object service layers.
- File mutations only through `core::fsx` (no `File::create` temp files, no `exists()` + `rename`). Name-list writes only via `harmonize::update_db`; journal edits via `batch_mut`/`undo`.
- PDFium only inside `extract::pdfium::with_pdfium` (one global mutex; PDFium is not thread-safe).
- Providers use blocking reqwest. Never call them inside a tokio runtime; mock HTTP with std `TcpListener`, not wiremock.
- GUI: colors, sizes, radii, type and motion only from `crates/app/src/gui/theme.rs`; widgets from `gui/components/`. Long work on `gui/worker.rs`, never on the UI thread.
- stdout is data only; diagnostics and progress go to stderr. Help text is pre-wrapped at 80 columns.
- Models are never downloaded implicitly. Offline mode never falls back to the network.
- Windows VM job scripts (`vm.sh run`) are ASCII only.
- Update `known.md` when you hit a non-obvious trap; move finished items out of `followup.md`.
