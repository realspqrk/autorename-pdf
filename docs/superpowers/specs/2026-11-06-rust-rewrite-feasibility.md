# Rust Rewrite + Scan-to-PDF — Feasibility (2026-11-06, rev. 2)

Codex (Astra, high) and Grok 4.7 (high) reviewed rev. 1. Both returned **go-with-changes**. Their corrections are merged into this revision.

## TOC
- [Verdict](#verdict)
- [Current baseline](#current-baseline)
- [Component mapping](#component-mapping)
- [Scan-to-PDF feature](#scan-to-pdf-feature)
- [Architecture: one app, GUI + CLI](#architecture-one-app-gui--cli)
- [Effort](#effort)
- [Risks](#risks)
- [Recommended plan](#recommended-plan)
- [Open decisions](#open-decisions)
- [Sources](#sources)

## Verdict

**Technically feasible, nothing is blocked. It is still a 4–6 month project, and one risk dominates: text extraction parity with pdfplumber.**

| Question | Answer |
|---|---|
| Can all of it be done in Rust? | Yes. Every dependency has a permissively licensed Rust path. The one exception is mupdf, which is AGPL, so we avoid it. |
| GUI + CLI combined? | Yes as one codebase and one installer, with all logic in `core`. It's still two EXEs on Windows, the same as PhraseVault (Electron app + Go console relay). A thin console EXE is needed because of how the Windows subsystem works. |
| Scanner like Genius Scan? | Photos and scans only. **Gate: on par with or better than Genius Scan, Adobe Scan and Lens**, measured on a shared corpus. Achievable with DocAligner + classic fallback + linear-light cleanup, but not guaranteed. The gate decides. |
| Effort | **95–135 dev-days** for full port + scanner incl. gate. Scanner alone, in front of the current Python core: **33–50 dev-days**. |
| Biggest risk | PDFium text output differs from pdfplumber/pdfminer in reading order, spacing and repair. That changes the LLM input and the OCR/vision routing (quality score). |

**Recommendation:** start with a **5–8 day spike**. Its results decide whether to port the core at all. The scanner can proceed either way.

## Current baseline

| Item | Now |
|---|---|
| Core | ~3.9k LOC Python (+6.5k LOC tests). CLI floor is Python 3.11. PyInstaller EXE. |
| GUI | Tauri 2 + TypeScript (~3k LOC). Runs the CLI as a sidecar (`gui/src/lib/sidecar.ts`). Contract: JSON on stdout, `Processing [i/n]` progress on stderr, exit codes, `config inspect`. |
| Text | **pdfplumber/pdfminer** (`extract_text`, NFC, `repair=True`, cp1252 mojibake round-trip). pypdfium2 is used **only for rendering**. |
| OCR | PaddleOCR 3.7 venv (~500 MB, optional). PP-OCRv6_small for Latin scripts, **v5 script-specific recognizers** for CJK, Cyrillic, Arabic and others. GPU/device and custom detection-model settings. Text only; the bridge discards geometry. |
| AI | OpenAI and Anthropic native parse. instructor TOOLS mode for Gemini and xAI, JSON mode for Ollama. `reasoning.effort=none`, `store=false`. |
| Release | 65 MB portable ZIP + `setup.ps1` (context menu, optional OCR). |
| Config | YAML v2, PyYAML (**YAML 1.1**), legacy-key migration, dotenv precedence. The GUI only reads it. |

## Component mapping

| Python | Rust replacement | Confidence | Caveat |
|---|---|---|---|
| pdfplumber (text) | **pdfium-render 0.9.x** (or pdf_oxide, which is pure Rust but has one maintainer) | **Medium. Spike gate.** | Different engine. Reading order, spacing, repair and the quality score all shift. Word/line grouping has to be rebuilt on character boxes. The mojibake fix stays a cp1252 round-trip; this crate's `has_unicode_map_error()` does not replace it. |
| pypdfium2 (render) | pdfium-render | High | Same engine. Ships `pdfium.dll` (~5–6 MB). |
| PaddleOCR venv | **oar-ocr 0.10 on ort 2.0-rc** (Apache): PP-OCRv6 + v5 script models, PP-LCNet orientation, UVDoc, word boxes | Med-High | Inventory every script, device and det-model setting. Pin model hashes and dictionaries. GPU via ort execution providers, or document it as a dropped setting. Windows.Media.Ocr fallback is **disputed**: Codex says it needs package identity, while `winocr` works unpackaged. The spike settles it. |
| openai + anthropic + instructor | **async-openai 0.43** (`create_byot`, Responses) for OpenAI-compatible providers, plus a thin reqwest client for Anthropic `output_config.format` | Medium | Keep TOOLS vs JSON modes per provider, validation retries, refusal/incomplete handling and the error-to-exit-code mapping. 8–10 days, not 4–5. |
| pydantic dynamic models | Runtime JSON-schema builder + `jsonschema` validation | High | Per-provider schema rules: OpenAI strict means all fields required; Anthropic allows no min/max. |
| dateparser (DMY) | Own parser: ISO, `d.m.yyyy`, `d. Monat yyyy`, `15 March 2024`, EN/DE/FR/IT/ES months. Strftime-subset formatter for `output.date_format`. | Medium | A deliberate narrowing. Document it as a compatibility change. |
| rapidfuzz JaroWinkler | **rapidfuzz (Rust)** | High | Same metric as today, which keeps the `> 0.85` threshold stable. strsim scores differ on short strings. Replay the harmonization fixtures. |
| PyYAML | serde-saphyr 2.x | Medium | YAML **1.2**: `yes`/`no`/`on`/`off` are no longer booleans. Fail hard with a migration hint, and parse every example config with both parsers. |
| python-dotenv, argparse | dotenvy, clap 4 | High | Keep subcommands, JSON shapes, stderr progress and exit codes 0–11. |
| mupdf | **Do not use** | — | AGPL. |

## Scan-to-PDF feature

Scope (confirmed by Gerhard, 2026-11-06): **photos and scans only, no live camera.** Still images become a clean, **searchable** PDF, which then goes through the existing rename pipeline.

**Hard gate (Gerhard): output must be on par with or better than Genius Scan, Adobe Scan and Microsoft Lens. Otherwise the feature does not ship.**

Gate protocol (fair comparison, because every one of those apps can import photos from the gallery):
1. **Corpus:** about 40 photos in 8 categories (low-contrast desk, cluttered background, shadow or hand, glare, crumpled or curled, receipt, multi-column letter, colored paper or stamps), plus about 10 bad flatbed scans.
2. **Process each image** with Genius Scan, Adobe Scan, Lens (gallery import, default auto settings) and our pipeline (auto only, no manual corner adjust).
3. **Metrics:**
   - Corner IoU against hand-labelled quads.
   - OCR character error rate on each output. Run the same OCR engine over all of them.
   - File size per page.
   - Failed detections.
4. **Blind A/B review:** shuffled side-by-side pages, judged by a human and by Codex visual review.
5. **Pass:** we match or beat the best competitor on CER and IoU averaged over the corpus. No category is worse than the median competitor. The blind review prefers ours or calls it a tie on ≥ 50%.

If the gate fails, the fallback is to iterate: fine-tune the corner model on failing categories, or adopt a better cleanup model. Ship only after it passes.

| Stage | Technique / crate | Days |
|---|---|---|
| Load, EXIF rotate, downscale | `image`, `kamadak-exif` | 0.5 |
| Corner detection | **DocAligner** ONNX (Apache-2.0) via ort | 2–3 |
| Corner fallback | imageproc 0.27: Canny, contours, Douglas-Peucker, quad scoring, Hough | 3 |
| Perspective warp | `imageproc::Projection::from_control_points` at full resolution | 1 |
| Orientation + OCR (+ dewarp v2) | One **oar-ocr** session: PP-LCNet, PP-OCRv6, UVDoc | 2 |
| Shadow removal / white balance | Background division done **in linear light** (not sRGB), percentile stretch | 2–3 |
| Color modes | Color / gray / B&W (Sauvola) | 2 |
| PDF writer | pdf-writer: JPEG (color/gray), **Flate for bilevel**, one embedded font with ToUnicode, OCR boxes mapped through the homography, invisible text mode 3 | 4–6 |
| Corner-adjust UI | Small JPEG preview over IPC, drag handles + loupe, batch review grid; the warp runs in Rust | 5 |
| Benchmark set | Corner IoU, OCR CER, memory and latency on categories of difficult photos | 3 |
| **MVP total** | | **25–35** |

Parked for v2: CCITT G4 (no Rust encoder exists), PDF/A (krilla), UVDoc curl dewarp, live camera.

Must-haves: keep the original files, make processing adjustable, and always offer manual corner adjust.

Avoid: opencv-rust (Windows build pain) and the DocTr/DewarpNet weights (non-commercial or unclear licenses).

## Architecture: one app, GUI + CLI

**The PhraseVault reference** (PhraseVault repo, `packages/shared/cli-relay`): PhraseVault feels like one combined GUI + CLI, but on Windows it is also two binaries.
- The Electron app holds all the logic and runs a CLI server.
- A thin **Go console relay** (`phrasevault.exe`) forwards argv, stdin/stdout/stderr and the exit code over a named pipe. If the app isn't running, the relay starts it hidden with `--cli-launch`.
- So the rule stands: one Windows EXE can't be both GUI and console. "Combined" means **one codebase, one installer, all logic in one place**, plus a tiny console front end.

Two options for AutoRename-PDF:

| Option | How | Pro | Con |
|---|---|---|---|
| **A. CLI links `core` directly** (recommended) | `autorename-pdf-cli.exe` statically links `crates/core` | Stateless batch tool, so no resident app is needed. Fast context-menu calls. No pipe protocol. | Context-menu runs show console output, not GUI progress |
| B. PhraseVault-style relay | Port the Go relay to Rust (or reuse it). The GUI runs a CLI server on a named pipe. | One running instance. Context-menu runs could show progress in the GUI. Same pattern across your apps. | The app must launch hidden per call (cold start). More moving parts. Only worth it if the GUI becomes resident (tray). |

Both options share `crates/core`. Moving from A to B later is cheap.

```
crates/core      -- config, profiles, providers, extraction, rename/undo, scanner
crates/cli       -- autorename-pdf-cli.exe  (console subsystem; also the GUI's worker)
gui/src-tauri    -- autorename-pdf.exe      (windows subsystem; Tauri commands)
```

- **Native PDF and OCR work stays in a worker process.** The GUI spawns the Rust CLI the way it spawns the sidecar today. That keeps the crash boundary, timeouts, cancellation and restart, and it preserves the existing sidecar contract (PDFium calls are serialized behind a mutex anyway). Cheap operations like config, undo listing and scanner previews can be direct Tauri commands.
- One NSIS installer (per user) with **both EXEs and the DLLs signed**. It registers today's per-user classic context menu. An updater needs a plugin and a hosted manifest, so that is a separate decision.
- **User-data migration:** config and log paths from the resource dir, portable mode, retention across uninstall, keeping secrets redacted in `config inspect`.

## Effort

| Workstream | Dev-days |
|---|---|
| Spike / feasibility gates | 5–8 |
| Config, profiles, schema builder, YAML 1.1 migration | 6–8 |
| Provider adapters (5 providers, modes, retries, error mapping) | 8–10 |
| Text extraction parity + quality score + mojibake | 6–10 |
| OCR (oar-ocr, model inventory, download/cache, device) | 5–7 |
| Rename, undo v2, harmonization, dates | 4–5 |
| CLI contract + GUI rewire | 5–7 |
| Tests: replayed provider responses, golden extraction text, filename rendering with mocked metadata | 8–12 |
| Installer, signing, context menu, data migration, release playbook | 6–8 |
| **Port subtotal** | **55–75** |
| Scanner MVP | 25–35 |
| Scanner par-or-better gate (corpus labelling, comparison tooling, tuning loop) | 8–15 (open-ended if the corner model needs fine-tuning) |
| Hardening + buffer | 5–10 |
| **Total** | **95–135** ≈ 5–7 months full time |

## Risks

| Risk | Sev. | Mitigation |
|---|---|---|
| pdfium/pdf_oxide text ≠ pdfplumber, so naming and routing drift | **High** | Spike gate: diff text on fixtures plus about 50 real invoices (tables, columns, broken fonts, rotation). No pass, no port. |
| Scanner falls short of commercial quality | High | Define the MVP as still images only. Benchmark, use adjustable presets, always do manual adjust. |
| Provider behavior drift once instructor is gone | Med | Replay tests for recorded failure responses; per-provider live smoke tests |
| OCR parity (scripts, GPU, custom models) | Med | Config inventory; explicitly drop or migrate unsupported settings |
| YAML 1.1 → 1.2, dateparser narrowing | Med | Hard error + migration hint; document the compatibility change |
| ort still RC; oar-ocr, hayro and pdf_oxide each mostly one maintainer | Med | Pin versions and model hashes; vendor if needed |
| Unsigned DLLs trigger SmartScreen / Smart App Control | Low-Med | Sign every binary |

Testing note: live LLM output varies, so a byte-diff against the Python JSON can never pass. Gate on golden extraction text, deterministic filename rendering and replayed provider responses instead. Live calls stay as statistical smoke tests.

## Recommended plan

0. **Spike, 5–8 days. This is the go/no-go.**
   - Text from pdfium-render vs pdfplumber on fixtures plus real invoices, scored with the existing quality function.
   - oar-ocr vs PaddleOCR on scanned fixtures.
   - DocAligner and a linear-light cleanup on the gate corpus, compared against Genius Scan, Adobe Scan and Lens (see the scanner gate protocol).
   - Windows.Media.Ocr from an unpackaged EXE.
   - A minimal searchable PDF that pdfplumber can read back.
1. **Spike fails on text parity:** build only the scanner in Rust, in front of the Python sidecar (33–50 days incl. gate), and stop the rewrite. **The scanner fails its par-or-better gate:** the scanner doesn't ship, and the port decision is made separately.
2. **Spike passes:** port in strangler order — deterministic logic (config, profiles, dates, harmonization, undo), then providers, then extraction and OCR last. Build the scanner on the same oar-ocr/ort stack in parallel. Python stays the reference until the gates pass.
3. **Cutover = v4.0:** NSIS installer, Python and PyInstaller deleted, the `/rename-pdfs` dev skill switches to `cargo run -p cli`.

## Open decisions

- Port the whole core, or only the scanner (running in front of the Python core)? Decide after the spike.
- CLI option A (links core directly) or B (PhraseVault-style relay)? B only makes sense if the GUI becomes a resident tray app.
- OCR default: download oar-ocr models on first use (~30–60 MB including models) or keep them optional.
- Drop GPU OCR and the non-Latin scripts, or carry them over?
- Updater: Tauri updater + hosted manifest, or keep manual ZIP/installer releases?

## Sources

Crate data comes from the crates.io API on 2026-11-06.
- https://github.com/ajrcarey/pdfium-render
- https://github.com/yfedoseev/pdf_oxide
- https://github.com/GreatV/oar-ocr
- https://github.com/pykeio/ort
- https://huggingface.co/collections/PaddlePaddle/pp-ocrv6
- https://github.com/DocsaidLab/DocAligner
- https://github.com/tanguymagne/UVDoc
- https://docs.rs/imageproc/latest/imageproc/all.html
- https://github.com/marquaye/scanic
- https://docs.rs/pdf-writer
- https://docs.rs/krilla
- https://github.com/64bit/async-openai
- https://platform.claude.com/docs/en/build-with-claude/structured-outputs
- https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr
- https://v2.tauri.app/distribute/sign/windows/
- https://zng-ui.github.io/doc/zng_env/windows_subsystem/index.html
