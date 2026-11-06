# Follow-ups

## TOC

- [Ideas](#ideas)
- [Rust rewrite + scanner (parked 2026-11-06)](#rust-rewrite--scanner-parked-2026-11-06)

## Ideas

- **Embed extraction in PDF (opt-in `pdf.embed_extraction`)**: attach `autorename.json` as a PDF embedded file via `pypdf` incremental update (original bytes stay an intact prefix; record original length + SHA-256 for truncation-restore/undo). Skip signed PDFs and ZUGFeRD/Factur-X (PDF/A-3). Default instead: local content-hash cache (JSON/SQLite next to undo log) for re-render/skip-on-rerun without mutating files. Rejected: ZIP/PDF polyglot (fragile, AV-flagged), NTFS ADS (lost on copy/cloud), XMP-only (cramped). Validate first: Acrobat / Edge / pdf.js behavior on a sample.

## Rust rewrite + scanner (parked 2026-11-06)

The full study is in `.todo/rust-rewrite-feasibility.md`. Note that `.todo/` is gitignored, so the file exists on this machine only. Codex (Astra) and Grok 4.7 reviewed it and both said go-with-changes.

- **Verdict**: feasible and nothing is blocked. Full port plus scanner is 95–135 dev-days; scanner only, in front of the Python core, is 33–50.
- **Spike first (5–8 days, go/no-go)**:
  - Compare pdfium-render and pdfplumber text on the fixtures plus about 50 real invoices, scored with `assess_text_quality`.
  - Compare oar-ocr and PaddleOCR on the scanned fixtures.
  - Run DocAligner plus a linear-light cleanup on the scanner gate corpus.
  - Test Windows.Media.Ocr from an unpackaged EXE. This is disputed: Codex thinks it needs package identity.
  - Check that pdfplumber can read back a searchable PDF we write.
- **Biggest risk**: text parity. Text comes from pdfplumber/pdfminer, and pypdfium2 only renders. A different engine shifts both the LLM input and the OCR/vision routing.
- **Stack**:
  - PDF: pdfium-render.
  - OCR: oar-ocr on ort, which bundles PP-OCRv6/v5, PP-LCNet orientation, UVDoc and word boxes.
  - Providers: async-openai (`create_byot`) plus a thin Anthropic client using `output_config.format`.
  - Utilities: the rapidfuzz Rust crate, serde-saphyr, dotenvy, clap.
  - Scanner: imageproc, plus pdf-writer for the PDF output.
  - Avoid mupdf (AGPL), opencv-rust, and the DocTr/DewarpNet weights.
- **Compatibility traps**:
  - YAML 1.1 → 1.2 changes `yes`/`no`: fail hard and show a migration hint.
  - Moving off dateparser narrows the accepted date formats.
  - Keep instructor's TOOLS vs JSON modes per provider.
  - Keep `store=false` and `reasoning.effort=none`.
  - Keep the v5 script-specific OCR recognizers.
  - Keep the sidecar contract: stdout JSON, stderr `Processing [i/n]`, exit codes 0–11, `config inspect`.
  - Keep undo log v2 readable.
- **Tests**: never byte-diff live LLM JSON. Gate on golden extraction text, replayed provider responses, and filename rendering with mocked metadata.
- **GUI + CLI**: one codebase and installer, two EXEs. A single EXE can't be both a GUI and a console app on Windows, and PhraseVault also has two (Electron app + Go console relay over a named pipe, `electron-workspace/packages/shared/cli-relay`).
  - Option A, recommended: the CLI links `crates/core` directly.
  - Option B: PhraseVault-style relay. Only worth it if the GUI becomes a resident tray app.
  - Either way, native PDF/OCR work runs in a worker process so a crash there can't take down the GUI.
- **Scanner scope**: photos and scans only, no live camera.
  - **Hard gate**: on par with or better than Genius Scan, Adobe Scan and Lens.
  - **Corpus**: about 40 photos in 8 categories plus about 10 bad scans, each run through every app via gallery import.
  - **Pass criteria**: CER and corner IoU at or above the best competitor; no category below the median; blind A/B prefers ours or calls a tie on at least 50%.
  - If it fails, it doesn't ship.
- **Open decisions**:
  - Full port, or scanner only (after the spike).
  - CLI option A or B.
  - Download OCR models on first use, or keep OCR optional.
  - Keep GPU and non-Latin OCR?
  - Add an updater (needs a hosted manifest), or keep manual releases?
