# AutoRename-PDF 4.0 — Rust rebuild (spec, 2026-11-07)

Supersedes the Windows-only parts of `2026-11-06-rust-rewrite-feasibility.md`. Parity target: `2026-11-07-python-behavior-contract.md`. Scanner / image ingest: **out of scope**.

## TOC
- [Decisions](#decisions)
- [Topology](#topology)
- [CLI](#cli)
- [Pipeline](#pipeline)
- [Enrichment (default on)](#enrichment-default-on)
- [Auto-harmonization (default on)](#auto-harmonization-default-on)
- [Offline privacy mode (opt-in)](#offline-privacy-mode-opt-in)
- [Models (Nov 2026)](#models-nov-2026)
- [Config & paths](#config--paths)
- [OS integration & packaging](#os-integration--packaging)
- [Intentional changes vs 3.x](#intentional-changes-vs-3x)
- [Milestones & gates](#milestones--gates)

## Decisions

Four-eyes: Codex (Astra) proposal, reviewed by Claude. Versions pinned in `Cargo.lock`.

| Topic | Pick | Runner-up (why it loses) |
|---|---|---|
| GUI | **egui/eframe 0.37 + rfd** (MIT/Apache, pure Rust, AccessKit, `egui_kittest` snapshot + a11y tests) | Tauri 2 (web frontend + OS webview variance, WebKitGTK on Linux); Slint (GPL/royalty-free) |
| PDF read/render | **pdfium-render 0.9** + pinned PDFium binary (BSD) | pdf_oxide (single maintainer, unproven reading order) |
| PDF write | **lopdf 0.47 incremental update** | pdf-writer (generator, not editor) |
| Offline OCR | **oar-ocr 0.10 / ort rc.13 (ORT 1.28) + PP-OCRv6 small**, models fetched on explicit request (GUI button or `models fetch`), hash-pinned, never implicitly | Tesseract (C++ deps, weaker on noisy scans); ocrs (Latin only) |
| AI | **reqwest + serde, 5 thin adapters**, canonical JSON Schema per profile | async-openai/genai/rig (abstraction without removing adapters) |
| Decision models | OpenAI Decisions API / Jev: **optional deciders only** (harmonization tie-break, review flag). Never extraction. | — |
| Config | **YAML** (serde-saphyr), schema free to simplify | TOML (YAML is friendlier for recipes/profile prompts) |
| CLI | clap 4, versioned JSON / NDJSON events, stable exit codes | MCP/daemon (YAGNI) |
| Packaging | **cargo-packager** (NSIS, AppImage/deb/pacman, app/dmg) | tauri-bundler (no Tauri left) |

Dropped: Python, PaddlePaddle venv, PyInstaller, instructor, Tauri sidecar, `setup.ps1`, tray/IPC relay, GPU OCR builds, auto-updater (later).

## Topology

```
crates/core   config, profiles, extraction, ocr, providers, naming, harmonize, enrich, journal
crates/app    single binary `autorename-pdf`: clap CLI + egui GUI (feature `gui`, default on)
crates/winlaunch  Windows-only WINDOWS-subsystem launcher `autorename-pdf-gui.exe` (spawns app in GUI mode, CREATE_NO_WINDOW)
```

- Unix/macOS: one executable. No args + display → GUI; no args + no display → help; subcommand → CLI. `gui [paths]` forces GUI.
- Windows: `autorename-pdf.exe` is CONSOLE (correct pipes/exit codes for agents); Start menu + Explorer use the tiny launcher.
- GUI and CLI call the same typed core operations. GUI runs work on a worker thread pool; PDFium access serialized.

## CLI

Agent-first: `--help` on every command lists purpose, examples, side effects, offline vs cloud, exit codes. stdout = data only; stderr = diagnostics/progress. Never prompts when stdout is not a TTY or `--output json`.

| Command | Purpose |
|---|---|
| `rename <paths…>` | Extract, plan, rename (+enrich, +learn). `--dry-run`, `--plan FILE` (write plan, no changes), `-r`, `--profile`, `--provider`, `--model`, `--ocr auto\|on\|off`, `--vision auto\|on\|off`, `--no-enrich`, `--no-learn`, `--jobs N`. No legacy flags; subcommand always explicit. |
| `apply <plan.json>` | Execute a saved plan without new AI calls; verifies source hashes. GUI preview uses the same plan. |
| `undo [--list\|--batch ID\|--all]` | Reverts renames, enrichment and auto-learned aliases of a batch. |
| `inspect <pdf…>` | Show embedded AutoRename data, extracted text tier, signature/encryption status. |
| `names [--field ID] list\|review\|merge\|rename\|add` | Name-list maintenance (Name matching); `--field` defaults to the active profile's `harmonize_field` (none → exit 2). |
| `config show\|validate\|path\|init` | `init` writes a commented starter config. |
| `models list\|fetch\|import\|remove` | Offline OCR model packs. |
| `schema [result\|plan\|config]` | JSON Schemas of machine outputs. |
| `gui [paths…]` | Open GUI (preview pre-loaded). |

`--output text|json|ndjson` (auto: json when stdout not a TTY). JSON carries `"schema_version": 4` and per-file `rename_status` + `enrich_status` + `learn_status`. Errors: `{"error":{"code","message","hint","retryable"}}`.
Exit codes unchanged: 0 ok, 1 error, 2 usage, 3 config, 4 no files, 5 partial, **10 provider, 11 auth now actually emitted** (all files failed with that cause).

## Pipeline

1. **Text** (always): PDFium chars → lines with reading-order policy, NFC, cp1252 mojibake repair (per page). Per-page readability `R` (contract §4.2, 4.0 formula; not Python parity): `R = clamp(1 − 10·(broken + mojibake chars)/non-whitespace chars, 0, 1)`, blank/no letter or digit = 0. **Visual cap** (`extract::visual`, `VISUAL_CAP = 0.75`): a selected page with a significant non-text visual (`H`) scores `Qtext = min(R, 0.75)`, else `Qtext = R`. A clean text layer says nothing about content printed only as a picture (a sender's logo, a stamp, a scan behind the text), so a limit above 0.75 reads such pages again; this replaces any profile-specific logo check. `H` uses displayed bounds in page space clipped to the crop box (area `A`): an image object (top level or inside form XObjects, recursive, ancestor matrices applied once) with area ≥ 0.001·A and both sides ≥ 2 pt (full-page images count: scans); or filled paths whose summed area is ≥ 0.001·A, where a path qualifies if it is not fully transparent, its minor extent is > 1 pt and it has a Bézier segment or ≥ 6 non-MoveTo segments, except paths whose subpaths are all axis-aligned rectangles (table borders, cell shading, backgrounds); stroke-only paths never count; shading or unsupported objects count when their area is ≥ 0.001·A. Limits: form depth ≤ 32, ≤ 100 000 objects + path segments per page; exhaustion or a PDFium error ⇒ `H = true` + warning `visual_scan_incomplete: page N: …`. `pdf.text_quality_threshold` (the limit `L`) default 0.8. Gate: fixture text diff vs pdfplumber; clean fixtures ≥ 0.95.
Tiered route, free/offline first (user policy 2026-11-07: cloud vision only for problematic PDFs). `pdf.ocr` and `pdf.vision` default to `auto`. If the user's documents keep failing, raising the limit is the one knob.

2. **OCR** (`auto` default: pages with `Qtext < L`; `on`: all selected pages; `off`). A successful OCR sets the page result `Q = R(full OCR text)` (0 when empty), never capped again and never `max`ed with `Qtext`; failed or unavailable OCR keeps `Q = Qtext` (+ warning under `auto`). Evidence merge per OCR'd page: if the page's `R < L`, every non-blank OCR line is appended; otherwise only lines where > 50 % of their lowercased alphanumeric words are missing from the page's text-layer word set (the logo line, not a second copy of the page). One merged evidence string (text layer + `--- OCR Text ---` block) serves the prompt and the enrichment attachment. oar-ocr PP-OCRv6 small (det+rec, PP-LCNet orientation). `ocr.language: auto` (default) uses the installed pack (today only `latin`: English, German and 46 more at once); a pack id or language code selects its pack; with several script packs, `auto` needs script detection (followup). Models are never downloaded implicitly. `on` = hard requirement: missing models/engine → error `ocr_models_missing`/`ocr_unavailable` with `models fetch` hint. `auto` + missing models → warning `ocr_models_missing` (same hint) and the route falls through to vision with `Qtext`.
3. **Vision** (`auto` default; `on`; `off`): page PNGs to the provider, decided on the page result `Q`: `auto` sends pages with `Q < L`, `on` every selected page, `off` none; no pre-OCR veto. PNGs are encoded only for pages sent. Offline: the local Ollama model. Document `quality` = minimum page `Q`; plans keep per page `page_quality[] {text: Qtext, visual, ocr, image}`, `inspect` shows `quality` (Qtext), `visual`, `ocr_quality` per page. Profiles with `direction` add a `counterparty_missing` review warning when the answer has no counterparty (no extra request).
4. **Extraction**: one structured-output call per document; schema from profile; local validation; bounded retries by error class.
5. **Naming**: contract §3 algorithm (separator collapse, 244-char limit, `_(n)` collisions).

Concurrency: `--jobs` (default = min(4, cores)); AI calls concurrent, PDFium serialized.

## Enrichment (default on)

Switch: `enrich.enabled: true` / `--no-enrich`.

- Writes, as **one lopdf incremental update** appended to the original bytes:
  - Info: `Title` (rendered filename stem), `Subject` (doc type + company), `Keywords`. Never overwrite `Author`/`CreationDate` with invoice semantics.
  - XMP: merged into existing packet; private namespace `https://autorename-pdf.dev/ns/1.0/` (`arp:`) with every extracted field, profile, tier, provider/model, timestamp.
  - Attachment `autorename-pdf.json` (`AFRelationship /Data`): schema version, fields, per-page text actually used (marked partial if `max_pages` cut), tier, model, original SHA-256 + length. No secrets, prompts, absolute paths.
- **Skipped** (rename still happens, `enrich_status: skipped:<reason>`): signed or certified PDFs, encrypted, PDF/A-3 with existing invoice attachments (ZUGFeRD/Factur-X), unparseable.
- Commit: write to sibling temp, verify original bytes are an exact prefix and the result re-opens in PDFium, atomic replace.
- **Undo**: journal stores original length + SHA-256 and post-enrich SHA-256. Undo restores the original prefix only if the current hash equals the post-enrich hash; otherwise refuse with a conflict.
- Commit/undo never write in place and use no mandatory locks: `fsx::replace_verified` sets the current file aside, verifies identity + hash, installs the new bytes, re-verifies the displaced file and keeps a concurrent editor's revision as `<stem> (conflict <timestamp>).pdf`; the change itself counts as done (journaled, `enriched`/`restored`) with a `conflict_copy` warning naming the kept file. Residual window: on Unix a write through an already-open handle between the final re-verification and the unlink (microseconds) can still be lost.
- Invisible OCR text layer: deferred (followup).

## Auto-harmonization (default on)

Switch: `harmonization.auto_learn: true` / `--no-learn`. UI name: "Name matching". The profile's `harmonize_field` picks the list; one file per field, `<config-dir>/name-lists/<field>.yaml` (structured format only; the file stem is the field id verbatim, and profiles may only name fields that are lowercase identifiers and not Windows device names, so no escaping is needed), shared by every profile that harmonizes that field. Business: `company_name`; academic: `null` (opt-in). Undo uses the list path + field recorded in the journal, never the active profile.

1. Normalize: `company_name` (kind Company): NFKC, casefold, strip punctuation + legal suffixes (GmbH, AG, Inc, Ltd, LLC, SA, SAS, BV, SRL, …), collapse spaces. Any other field (kind Generic): NFKC, casefold, trim surrounding quotes/periods/commas, collapse spaces; words and inner punctuation are kept. The decider prompt names the field's label (`harmonize::field_label`) and only assumes companies/invoices for kind Company. The my-company guard and AP/AR direction always use company normalization.
2. Exact hit in normalized index (canonical + aliases) → canonical, no write.
3. Candidates: Jaro-Winkler (rapidfuzz) + token-set ratio, top k.
4. Decide: ≥ 0.92 → add alias; 0.75–0.92 → decider (`harmonization.decider: ai | openai-decisions | jev | off`; default `ai` = small structured call on main provider, choice over candidates + "new"); < 0.75 → new canonical entry. Decider off, unavailable or failing (incl. offline without a local decider) → only the ambiguous band 0.75–0.92 (including capped subset/prefix matches) keeps the extracted name as a new entry with `review: true` — never a silent mapping to an existing company; < 0.75 or no candidates → plain new entry (`review: false`); ≥ 0.92 behaves as above. Subset/prefix matches ("Bank" vs "Bank Austria") are capped at 0.91 and so never auto-alias.
5. Never insert: empty, `Unknown`, names below extraction confidence, or anything whose normalized form already exists. Hand-written entries are never rewritten; auto entries get `source: auto` + date. Writes atomic; journaled for undo. Thresholds validated on harmonization fixtures.

## Offline privacy mode (opt-in)

`privacy: offline` (or `--offline`): provider forced to `ollama` (localhost/LAN `base_url` only), OCR local, vision only via a local Ollama vision model, decider `ai` → local model, `openai-decisions`/`jev` refused. Any non-local network request is a hard error (exit 3), never a silent fallback. GUI shows an "Offline" badge. Default stays cloud (`openai`).

## Models (Nov 2026)

Defaults track the current families (OpenAI GPT-6 family; Anthropic Claude 5 family; current Gemini/Grok); exact ids from `2026-11-07-provider-reference.md`, re-verified by live smoke tests. Model ids are config, never hard-coded beyond the default table.

## Config & paths

- Discovery: `--config` > `AUTORENAME_PDF_CONFIG` > OS config dir (`dirs`: `%APPDATA%\autorename-pdf`, `~/.config/autorename-pdf`, `~/Library/Application Support/autorename-pdf`). `.env` beside config; never overrides real env.
- Undo journal + OCR models: OS data dir.
- Config schema may be simplified freely; unknown keys are errors.

## OS integration & packaging

| OS | Context menu | Package |
|---|---|---|
| Windows | Per-user classic verbs (PDF + folder) with icon + submenu (Rename / Preview); Win11 modern menu via sparse package + IExplorerCommand = stretch | NSIS per-user, Azure Trusted Signing |
| Linux | Nautilus (scripts fallback; host shows no icons), Dolphin service menu, Nemo action, Thunar uca merge; hicolor icons; `.desktop` with MIME | AppImage + pacman/deb; `autorename-pdf integrate install\|remove` |
| macOS | Finder Quick Action (code only this session) | app/dmg (unsigned, not built here) |

- **Verbs** on PDFs and folders, on every host: "Rename now" = `gui --rename-now <paths>`, and "Preview in AutoRename-PDF" = `gui <paths>`.
  - `--rename-now` opens the normal GUI at its minimum size. It renames right after the analysis when there are no errors and no review flags. Otherwise it stays in the preview and shows a "Nothing renamed yet" banner.
  - The result screen offers Undo. Config (enrich, learn) applies as usual. Without a config, the window shows onboarding and renames nothing.
- **Ownership**: every generated entry carries `X-AutoRename-PDF-Integration`. On Thunar, ownership is the `unique-id` prefix `autorename-pdf-`. On Windows, it is the `AutoRenamePdfIntegration` value.
  - `integrate remove` deletes only marked entries, plus what `install` recorded in `<data>/autorename-pdf/integration-owned.txt`: the directories it created and the icons it wrote, with their SHA-256. A foreign or modified icon is never overwritten or deleted.
- **Windows multi-select**: Explorer starts one launcher per file. `autorename-pdf-gui.exe` coalesces them into one GUI process (`crates/winlaunch/src/coalesce.rs`, using an OS file lock).
  - The paths are handed over in a `req-*.txt` request file (`gui --request-file`). The app deletes the file as its acknowledgement, which avoids the 32,767-char command-line limit.
- **Packaging**: `cargo xtask dist` reads `Packager.toml` and applies the NSIS template hooks in `packaging/` (`playbooks/release.md`).

## Intentional changes vs 3.x

**Policy (user, 2026-11-07): no backward compatibility.** Legacy never shapes the design; users get `docs/migration-3-to-4.md` only. No legacy flags, config keys, log formats or file formats are read.

- Dates: no dateparser "fill missing from today", no relative words. Missing parts → unknown.
- Exit 10/11 emitted; undo JSON paths no longer swapped.
- OCR renders at scale 2.0 (was 1.5) and auto-orientation is on.
- One render for OCR and vision (scale 2.0); PNGs encoded Best+Adaptive (smaller than PIL).
- OCR detection resize is min-side (oar-ocr `LimitType::Min`), never the bridge's 736 max-side cap (destroyed small-font scans in the spike).
- OCR text lists only pages with recognized text; an all-blank OCR result is a warning, not an `ocr` source.
- Quality score is a per-page readability score (no Python parity), default threshold 0.8 (3.x: 0.3 over a length-weighted whole-document score); document summary = worst page.
- `pdf.ocr` / `pdf.vision` default to `auto` (3.x: off): text → local OCR → vision, per page, each tier only for pages whose result so far is below the quality threshold. Missing OCR models: `ocr: on` → error `ocr_models_missing` (hint `autorename-pdf models fetch`); `ocr: auto` → same code as a warning, then vision. Models are never downloaded implicitly.
- Default model re-verified at implementation time.
- Business profile: the model extracts `issuer`, `recipient`, `total_amount`, `currency`; the app derives `document_type` AP/AR (recipient/issuer vs `company.name`, else model guess + `direction_uncertain`) and the filename amount (`output.amount_format`, default `{currency} {amount}`; no currency → `currency_missing`). Profiles declare this generically (`direction`, `amount`).
- Re-runs are idempotent: a file already named `<proposed>` or `<proposed>_(n)` is unchanged; enrichment titles never carry `_(n)`.

## Milestones & gates

| M | Scope | Gate |
|---|---|---|
| 1 | Workspace, core deterministic: config, profiles, naming, dates, harmonize + auto-learn, journal | Ported golden tests green |
| 2 | Text extraction (PDFium), quality score, OCR (oar-ocr), models cmd | Fixture text/quality parity; OCR on image fixtures |
| 3 | Providers ×5 + vision, mocked HTTP fixtures | Replay tests; live smoke (`--run-live`) where keys exist |
| 4 | Enrichment + undo restore | Byte-prefix + undo byte-equality tests; signed fixture skipped |
| 5 | CLI contract, help, plan/apply, schema | E2E CLI tests; agent workflow simulation (Codex) |
| 6 | egui GUI | kittest snapshots; Codex visual review |
| 7 | OS integration + packaging | Linux install on Omarchy; Windows VM install + context menu |
| 8 | Docs, README, skills, playbooks; delete Python + Tauri | Codex code review: no flaws |
