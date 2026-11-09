<div align="center">
  <h1>AutoRename-PDF</h1>
  <p><b>Rename PDFs from their content with AI and local OCR.</b><br>
  <code>Rechnung (2).pdf</code> → <code>20251028 Telekom Deutschland AP 54,99.pdf</code></p>
  <p>
    <a href="https://github.com/realspqrk/autorename-pdf/releases"><img src="https://img.shields.io/github/v/release/realspqrk/autorename-pdf" alt="GitHub Release"></a>
    <a href="https://github.com/realspqrk/autorename-pdf/blob/main/LICENSE"><img src="https://img.shields.io/github/license/realspqrk/autorename-pdf" alt="MIT License"></a>
    <a href="https://github.com/realspqrk/autorename-pdf/releases"><img src="https://img.shields.io/github/downloads/realspqrk/autorename-pdf/total" alt="Downloads"></a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-blue" alt="Platform: Windows, Linux, macOS">
  </p>
</div>

One native app (GUI + CLI) for invoices, letters, scans and academic papers. It reads each PDF, asks an AI model (OpenAI, Anthropic, Gemini, xAI, or a local Ollama model) for date, company, document type and total, renames the file, embeds the extracted data into the PDF, and keeps company names consistent. Every change can be undone.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/gui-preview-dark.png">
  <img src="docs/images/gui-preview-light.png" alt="AutoRename-PDF preview: proposed names per PDF with extraction tier, metadata and company-match badges, and a detail pane">
</picture>

| Profile | Example filename |
|---|---|
| Business (default) | `20260906 ACME AP 12,13.pdf`: date, counterparty, AP/AR (bill received / invoice sent), printed total |
| Academic | `20260101 Smith Nature Attention Is All You Need.pdf`: date, first author, venue, title |
| Custom | your template and fields under `profiles:` ([recipes](examples/)) |

## Contents

- [Install](#install) · [Quick start](#quick-start) · [Set up with an AI agent](#set-up-with-an-ai-agent) · [How it works](#how-it-works)
- [Offline privacy mode](#offline-privacy-mode) · [Metadata embedding](#metadata-embedding) · [Name matching](#name-matching-company-names)
- [Configuration](#configuration) · [Agents and scripts](#agents-and-scripts) · [Upgrading from 3.x](#upgrading-from-3x)
- [Development](#development) · [Support](#support) · [License](#license)

## Install

Download from [Releases](https://github.com/realspqrk/autorename-pdf/releases). Every package contains the CLI, the GUI and PDFium.

| OS | Package | Install |
|---|---|---|
| Windows 10/11 x64 | `AutoRename-PDF_<ver>_x64-setup.exe` | Run it. Per-user install to `%LOCALAPPDATA%\Programs\AutoRename-PDF`, no admin prompt. Adds the Start menu entry, the Explorer context menu and `autorename-pdf` on your user `PATH`. Uninstall (Settings > Apps) removes only those; your settings, name lists and undo history stay in `%APPDATA%\autorename-pdf` and `%LOCALAPPDATA%\autorename-pdf`. |
| Linux x86_64 (any distro) | `autorename-pdf_<ver>_x86_64.AppImage` | `chmod +x autorename-pdf_*.AppImage`, then `./autorename-pdf_*.AppImage integrate install` for the menu entry and file-manager actions. Needs FUSE ([no FUSE?](#appimage-without-fuse)). |
| Debian / Ubuntu | `autorename-pdf_<ver>_amd64.deb` | `sudo apt install ./autorename-pdf_*_amd64.deb`, then `autorename-pdf integrate install` (file-manager actions are per user). |
| Arch | `autorename-pdf_<ver>_x86_64.tar.gz` + `PKGBUILD` | `makepkg -si` in a folder holding both, then `autorename-pdf integrate install`. |
| macOS | from source | See [Development](#development). Release builds for macOS are not published yet. |

#### AppImage without FUSE

Containers, WSL and minimal systems often lack FUSE (`fusermount: not found`, exit 127). Check with `ls /dev/fuse && command -v fusermount3 || command -v fusermount`. Without it:

```bash
./autorename-pdf_*.AppImage --appimage-extract-and-run --help   # one-off (or: export APPIMAGE_EXTRACT_AND_RUN=1)
./autorename-pdf_*.AppImage --appimage-extract                  # permanent: unpack once …
mkdir -p ~/.local/opt && mv squashfs-root ~/.local/opt/autorename-pdf
~/.local/opt/autorename-pdf/AppRun integrate install            # … menu entries then point to this stable path
```

`integrate install` adds "Rename now" and "Preview in AutoRename-PDF" for PDFs and folders to Windows Explorer, Nautilus (Scripts submenu), Dolphin, Nemo, Thunar and Finder Quick Actions. `integrate remove` deletes exactly those entries again.

## Quick start

1. **Configure.** Open the app: the first start asks where documents are processed (cloud provider + API key, or offline Ollama). Or from a terminal (idempotent; the key goes to a `.env` beside the config, never into it):
   ```bash
   export OPENAI_API_KEY=sk-...
   autorename-pdf setup --provider openai --api-key-env OPENAI_API_KEY --company "Spqrk"
   autorename-pdf setup --check      # what is still missing; changes nothing
   ```
   Windows PowerShell (`autorename-pdf` is on `PATH` after the installer; otherwise use `& "$env:LOCALAPPDATA\Programs\AutoRename-PDF\autorename-pdf.exe"`):
   ```powershell
   $env:OPENAI_API_KEY = "sk-..."
   autorename-pdf setup --provider openai --api-key-env OPENAI_API_KEY --company "Spqrk"
   Copy-Item -Recurse examples\demo .\demo
   autorename-pdf rename --dry-run demo
   autorename-pdf rename --plan plan.json demo   # review plan.json, then:
   autorename-pdf apply plan.json
   autorename-pdf undo
   ```
   Offline instead: `autorename-pdf setup --offline --company "Spqrk"` (see [Offline privacy mode](#offline-privacy-mode)).
2. **Optional: offline OCR models** for scans (~40 MB, downloaded only on request): Settings > Processing > Reading in the GUI, or `autorename-pdf setup --ocr-models`.
3. **Try the demo** ([7 sample PDFs](examples/demo/), expected outputs in its README):
   ```console
   $ cp -r examples/demo ./demo && autorename-pdf rename --dry-run demo
     acme-invoice.pdf           →  20240315 ACME Corporation AP EUR 4,258.80.pdf
     globex-letter.pdf          →  20250122 Globex Corporation Letter.pdf
     invoice-to-wayne.pdf       →  20250310 Wayne Enterprises AR EUR 10,700.00.pdf
     rechnung-mustermann.pdf    →  20250208 Mustermann Consulting AP EUR 1.320,00.pdf
     springfield-scan.pdf       →  20241103 Springfield Power Co AP USD 4,300.00.pdf
     ...
   Dry run: 7 to rename (nothing changed)
   $ autorename-pdf rename demo          # rename + embed metadata + learn names
   $ autorename-pdf undo                 # everything back, byte-identical
   ```
4. **Rename your files.**

| Where | How |
|---|---|
| GUI | Drop PDFs or folders, review the proposed names (edit any row), press **Rename**. **Undo** is on the result screen and under History. |
| File manager | Right-click PDFs or a folder > AutoRename-PDF > **Rename now** (renames when nothing needs review, otherwise opens the preview) or **Preview**. Windows 11: under "Show more options". |
| CLI | `autorename-pdf rename --dry-run ~/Scans` to preview, `autorename-pdf rename ~/Scans` to rename, `autorename-pdf undo` to revert. |

On Windows the installer puts `autorename-pdf` on your user `PATH` (new terminals); the full path is `"%LOCALAPPDATA%\Programs\AutoRename-PDF\autorename-pdf.exe"`. `autorename-pdf --help` and `<command> --help` document every option, side effect and exit code.

```bash
autorename-pdf rename -r ~/Documents/Inbox                 # include subfolders
autorename-pdf rename --profile academic paper.pdf         # another profile for one run
autorename-pdf rename --provider anthropic invoices/       # another provider for one run
autorename-pdf rename --ocr=on --vision=off scan.pdf       # force local OCR, never send images
autorename-pdf inspect renamed.pdf                         # embedded data, signature, text quality
autorename-pdf undo --list                                 # all batches; undo --batch <id>
```

## Set up with an AI agent

Paste this into Claude Code, Codex, Cursor or any coding agent with a shell:

> Set up AutoRename-PDF from https://github.com/realspqrk/autorename-pdf for me. Follow AGENTS.md › Setup runbook.

The agent installs the app, runs `autorename-pdf setup --check`, asks you only for your API key (or whether to stay offline) and your company name, configures everything with `autorename-pdf setup`, and verifies it on the demo. Agents start at [`llms.txt`](llms.txt).

## How it works

Each PDF goes through free, local tiers first, decided page by page: a tier runs only for pages whose score so far is below `pdf.text_quality_threshold` (default 0.8). A clean text layer scores 1.0; broken or unmapped characters lower it, and a page with pictures or drawings (a logo, a stamp, a scan) scores at most 0.75, so at the default it is read again with local OCR, and a sender printed only in a logo is found. OCR adds only the lines the text layer lacks. Without OCR models such pages go to vision; set the threshold to 0.75 to keep them text-only. OCR reads English, German and 46 more Latin-script languages at once (`ocr.language: auto`).

| Tier | What | Cost | Setting |
|---|---|---|---|
| 1. Text layer | PDFium text extraction with reading-order and mojibake repair | free, instant | always |
| 2. Local OCR | PP-OCRv6 small (det + rec + orientation) on the CPU, 48 Latin-script languages | free, offline, ~0.5 s/page | `pdf.ocr: auto \| true \| false`, `--ocr=…` |
| 3. Vision | Page images sent to the model | provider price per image | `pdf.vision: auto \| true \| false`, `--vision=…` |

Then one structured-output call per document returns the profile's fields. The app validates them locally, harmonizes the company name, renders the filename (`_(1)`, `_(2)`, … on collisions; existing files are never overwritten; a re-run keeps `_(n)` names and changes nothing — that is also why the embedded title of a collision copy is the canonical name without `_(n)`), renames, embeds metadata and records the batch in the undo journal.

| Provider | Default model | Key |
|---|---|---|
| `openai` (default) | `gpt-6-luna` | `OPENAI_API_KEY` |
| `anthropic` | `claude-haiku-5-5` | `ANTHROPIC_API_KEY` |
| `gemini` | `gemini-3.5-flash-lite` | `GEMINI_API_KEY` |
| `xai` | `grok-4.7` | `XAI_API_KEY` |
| `ollama` | `gemma4` | none (local) |

Any model id works through `ai.model` or `--model`. Keys go into the config as `${VAR}`, read from the environment or a `.env` beside the config.

## Offline privacy mode

`privacy: offline` in the config (or `--offline` for one run) keeps every document on your network:

- The provider is forced to Ollama at `localhost` or a LAN address. Any other network request is an error (exit 3), never a silent fallback. Proxy variables (`HTTPS_PROXY`, …) are ignored and redirects are never followed; DNS answers must be loopback/LAN addresses.
- OCR runs locally. Vision, when needed, uses the local Ollama model.
- The GUI shows an **Offline** badge.

Offline checklist:

1. **Pick a model for your hardware** and pull it: GPU or Apple silicon `gemma4`; CPU only `gemma4:e2b` (smaller, slower to read scans).
   ```bash
   ollama pull gemma4:e2b
   ```
2. **Configure** (no API key; checks that Ollama answers and has the model):
   ```bash
   autorename-pdf setup --offline --ollama-model gemma4:e2b --company "Spqrk"
   ```
3. **OCR models** (optional, for scans): online first `autorename-pdf models fetch`; on an air-gapped machine copy the `models` folder (see `autorename-pdf config path`) from a connected one and run `autorename-pdf models import /media/usb/models` (the pack folder `…/models/latin` works too).
4. **Use it**: `autorename-pdf rename --dry-run --jobs 1 ~/Private` (one file at a time keeps a CPU model responsive), then `rename --plan plan.json …` → `apply plan.json`, `inspect <file>`, `undo`.

Back to a cloud provider later: `autorename-pdf setup --cloud --provider openai --api-key-env OPENAI_API_KEY` (offline mode is only left explicitly).

Recipe: [`examples/ollama-offline.yaml`](examples/ollama-offline.yaml). In the GUI: Settings > Processing > Privacy.

## Metadata embedding

On by default (`enrich.enabled`, `--no-enrich`). After renaming, the app appends one incremental update to the PDF. The original bytes stay an exact prefix of the file.

| Written | Content |
|---|---|
| Info dictionary | `Title` (the proposed filename without `.pdf` and without a collision suffix `_(n)`, so copies and re-runs embed the same data), `Subject` (document type + company), `Keywords`. `Author` and `CreationDate` are never touched. |
| XMP | Merged into the existing packet; every extracted field, profile, tier, provider/model and timestamp in the `arp:` namespace. |
| Attachment `autorename-pdf.json` | Fields, the text actually used per page, tier, model, original SHA-256 and length. No prompts, keys or absolute paths. |

- **Skipped** (the rename still happens): signed or certified PDFs, encrypted PDFs, PDF/A, e-invoices with embedded XML (ZUGFeRD / Factur-X), unparseable files.
- **Undo** truncates the file back to its original bytes, only when it is unchanged since enrichment; otherwise that entry reports a conflict.
- `autorename-pdf inspect <pdf>` shows the embedded data, signature/encryption state and text quality.

## Name matching (company names)

Name matching gives every spelling of the same name one canonical form in your file names. A profile's `harmonize_field` picks the field (business: `company_name`; academic: none, opt in with e.g. `harmonize_field: journal_name`), and each field has its own name list, `name-lists/<field>.yaml` beside the config. `name-lists/company_name.yaml` maps company variants to one canonical name, so `ACME Corp.`, `ACME Corporation` and `Acme` all become `ACME`. Profiles that harmonize the same field share its list.

- Company matching ignores case, punctuation and legal forms (GmbH, AG, Inc, Ltd, LLC, SA, BV, …); other fields ignore only case, spacing and surrounding punctuation. Candidates are scored with Jaro-Winkler and token-set similarity.
- **Auto-learning** (on by default, `--no-learn` per run): ≥ 0.92 adds an alias; 0.75-0.92 asks a small AI decider (`harmonization.decider`); below that creates a new entry. Without a decider, ambiguous names become new entries flagged `review: true`, never silent merges. Deciders `openai-decisions` / `jev` take their key from `harmonization.decider_api_key` (e.g. `"${JEV_API_KEY}"`, or `setup --decider jev --decider-key-env JEV_API_KEY`).
- Learned names are part of the undo batch. Hand-written aliases are never changed.
- Lifecycle: the app learns (`source: auto`) → an ambiguous name becomes a `review` entry that stores its evidence (candidates, scores, source PDF name, batch) → you decide with `names merge NEW KNOWN` (same company) or `names review --accept NEW` (new company); both make the entry yours (`source: user`), as does `names rename OLD NEW` (spelling fix). `undo` removes what a run learned, wherever a learned alias lives by now, but never an entry you accepted, merged or renamed (reported as `kept`).
- Maintain it in the GUI (Companies tab) or with `autorename-pdf names list | review | merge | rename | add` (the active profile's list; `--field journal_name` picks another).

### Company names over time

A recorded walkthrough (company Spqrk, `harmonization.decider: off` so the app does not ask an AI about ambiguous names; `acme-holding-invoice.pdf` stands for a second ACME invoice that prints the name differently, answered by a scripted model so the output is reproducible):

```console
$ autorename-pdf rename demo/acme-invoice.pdf
  acme-invoice.pdf  →  20240315 ACME Corporation AP EUR 4,258.80.pdf  enriched · new entry
Done: 1 renamed, 1 enriched, 1 name learned  (revert: autorename-pdf undo)

$ autorename-pdf rename demo/acme-holding-invoice.pdf      # prints the name differently
  acme-holding-invoice.pdf  →  20240402 ACME Corp Holding AP EUR 1,190.00.pdf  enriched · new entry
Done: 1 renamed, 1 enriched, 1 name learned  (revert: autorename-pdf undo)

$ autorename-pdf names review
ACME Corp Holding
  ambiguous match (0.75-0.92) without a decider verdict
  candidate  ACME Corporation  (0.91)
  from acme-holding-invoice.pdf (batch 20261107T155324-aa210c)
  $ autorename-pdf names merge 'ACME Corp Holding' 'ACME Corporation'
  $ autorename-pdf names review --accept 'ACME Corp Holding'
1 of 2 need review

$ autorename-pdf names merge "ACME Corp Holding" "ACME Corporation"       # same company
  ACME Corporation  ACME Corp Holding
1 entries · * = learned · ~/.config/autorename-pdf/name-lists/company_name.yaml

$ autorename-pdf undo                                      # reverts the second run
  20240402 ACME Corp Holding AP EUR 1,190.00.pdf  →  acme-holding-invoice.pdf
  ACME Corp Holding: kept: merged into "ACME Corporation" by user
Undo: 1 restored, 0 failed, 1 metadata reverted, 0 names forgotten  (batch 20261107T155324-aa210c)
```

The file is back under its old name, but your merge decision stays: the next invoice printing "ACME Corp Holding" is named `ACME Corporation`. `-o json` gives the same as JSON (`names review` includes the ready-to-run `commands`).

```yaml
"ACME":
  aliases: ["ACME Corporation", "Acme Corp"]   # yours
  learned: ["ACME Coporation"]                  # added by the app (OCR typo)
  source: user
```

Starter file: [name-lists/company_name.yaml.example](name-lists/company_name.yaml.example).

## Configuration

| Need | Where |
|---|---|
| Every key, with defaults and comments | [config.yaml.example](config.yaml.example) (`config init` writes a shorter version) |
| Recipes: academic, German ER/AR, invoice number, offline Ollama, cloud vision | [examples/](examples/) |
| Config location | `--config` > `$AUTORENAME_PDF_CONFIG` > `%APPDATA%\autorename-pdf\` · `~/.config/autorename-pdf/` · `~/Library/Application Support/autorename-pdf/` |
| All paths (config, `.env`, name lists, journal, OCR models) | `autorename-pdf config path` |
| Effective config / check | `autorename-pdf config show` · `config validate` |
| First-time setup / what is missing | `autorename-pdf setup …` · `setup --check` (idempotent, keys only in `.env`) |

Unknown keys are errors. Profiles: `business` and `academic` are built in; overlay or add your own under `profiles:` (fields, template, truncation).

## Agents and scripts

The CLI is built for automation: stdout carries only the result document, diagnostics go to stderr, and it never prompts.

| Feature | How |
|---|---|
| JSON output | `--output json` (default when stdout is not a terminal); `--output ndjson` streams progress events, last line = result |
| Review before changing anything | `rename --plan plan.json <paths>` (AI calls, no changes) → inspect → `apply plan.json` (no AI calls; refuses files whose SHA-256 changed) |
| Schemas | `autorename-pdf schema [result\|plan\|config\|error\|event\|inspect]` |
| Errors | `{"schema_version":4,"error":{"code","message","hint","retryable"}}`; per-file errors in `files[].error` (plans: `items[].error`) |
| Per-file status | `rename_status`, `enrich_status`, `learn_status`; sub-step failures in `enrich_error` / `learn_error` (exit 5) |
| Undo order | `undo` = newest batch. `undo --batch X` is refused (`undo_blocked`, exit 1, nothing changed) while a newer batch still touches the same files; undo those first or use `undo --all` |
| Editing a plan | Change `items[].proposed_name`, then `apply plan.json`. The file is renamed inside its own folder; `target` is informational and ignored. Any editor works: UTF-8 (BOM ok) and UTF-16 are read (PowerShell: `Set-Content -Encoding utf8`) |
| Isolated state | `AUTORENAME_PDF_DATA_DIR=/tmp/x` puts the undo journal, OCR models and locks there (`config path` shows it); `--config FILE` for the config |
| Windows | JSON output and plan files escape non-ASCII as `\uXXXX` (any console code page); `*`/`?` in paths are expanded by the app (cmd/PowerShell pass them through) |

| Exit code | Meaning |
|---|---|
| 0 | Success |
| 1 | Error (or every file failed for mixed reasons) |
| 2 | Usage error (bad arguments, unreadable plan) |
| 3 | Config error (missing/invalid config, offline-mode violation) |
| 4 | No PDF files found |
| 5 | Partial failure: some files failed |
| 10 | Provider error: every file failed at the AI provider |
| 11 | Auth error: every file failed authentication |

Claude Code users: the repo ships a `/rename-pdfs` skill (dry-run, review, apply) in `.claude/skills/`.

## Upgrading from 3.x

4.0 is a rewrite in Rust and reads none of the 3.x files: the config, company-names file and undo log all have new formats, and the Python CLI, PaddleOCR venv and Tauri GUI are gone. See [docs/migration-3-to-4.md](docs/migration-3-to-4.md) for every change, with before/after examples.

## Development

Rust 1.99+ ([rustup](https://rustup.rs)). Workspace: `crates/core` (library), `crates/app` (the `autorename-pdf` binary: CLI + egui GUI), `crates/winlaunch` (Windows GUI launcher), `crates/xtask` (build tooling).

```bash
cargo xtask fetch-natives                                   # pinned PDFium into vendor/ (needed by tests and runs)
cargo run -p autorename-app -- rename --dry-run tests/fixtures
cargo run -p autorename-app -- gui                          # AUTORENAME_GUI_DEMO=1: demo data, no disk or network
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
AUTORENAME_LIVE=1 cargo test -p autorename-core --test providers_live -- --ignored --nocapture   # real providers, keys from .env
cargo xtask icons [--check]                                 # regenerate / verify icons from assets/icons/*.svg
cargo xtask dist [--target <triple>] [--sign]               # release packages into target/dist/<triple>/
```

| Topic | Where |
|---|---|
| macOS build | `cargo xtask fetch-natives && cargo build --release -p autorename-app`. Intel Macs: add `--no-default-features --features gui` (no ONNX Runtime build, OCR off). |
| Release, signing, packaging | [playbooks/release.md](playbooks/release.md) |
| PDFium and OCR model updates | [playbooks/natives-and-models.md](playbooks/natives-and-models.md) |
| Windows VM validation | [playbooks/windows-vm-validation.md](playbooks/windows-vm-validation.md) |
| Design specs | [docs/superpowers/specs/](docs/superpowers/specs/) |
| Contributor and agent rules | [AGENTS.md](AGENTS.md) |

Issues and forks are welcome; pull requests are not being accepted right now.

## Support

Enjoying AutoRename-PDF? It may save you hours. Even in the age of AI, building and maintaining it takes a lot of time and resources. Please consider a donation — or try [PhraseVault](https://phrasevault.app), our text expander. You may enjoy it too.

[GitHub Sponsors](https://github.com/sponsors/realspqrk) · [Ko-fi](https://ko-fi.com/spqrk) · [PayPal](https://paypal.me/realSPQRK) · [Star the repo](https://github.com/realspqrk/autorename-pdf). Thank you [@claus82](https://github.com/claus82).

## License

[MIT](LICENSE). Bundles [PDFium](https://pdfium.googlesource.com/pdfium/) (BSD-3-Clause) and, with OCR, ONNX Runtime (MIT); OCR models are PaddleOCR (Apache-2.0). Made by [Gerhard Petermeir, SPQRK Web Solutions](https://github.com/realspqrk).

**Legal:** provided as is, without warranty of any kind; no liability for renamed or modified files — keep backups. Imprint: SPQRK IT-Services · Gerhard Petermeir, BA MSc · Haag 8, 8344 Bad Gleichenberg, Austria · office@spqrk.net · VAT ATU69739438.
