# Follow-ups

Open work after 4.0. Done items are removed; the history is in git and `docs/superpowers/specs/`.

## TOC

- [Next beta (4.0.0-beta.2)](#next-beta-400-beta2)
- [Validation gaps](#validation-gaps)
- [Scanner / image ingest](#scanner--image-ingest)
- [Enrichment](#enrichment)
- [OCR language packs](#ocr-language-packs)
- [Decision models](#decision-models)
- [Packaging and OS integration](#packaging-and-os-integration)
- [Auto-updater](#auto-updater)

## Next beta (4.0.0-beta.2)

User feedback on beta.1 (2026-11-10). One session per block, in this order; each ends with `/test` gates green and a commit on its own branch. Done: details label column spacing (`0841ab7`); own company names, one decimal mark, Decisions API shape + `ai` fallback, no `Unknown`/repeated type, legal form stripped, vision only for unreadable pages (block 1); GUI mouse/keyboard parity, scan-language line, custom profile naming, details amount fields (block 2).

3. **International screenshots**: demo data (`gui/demo.rs`) is German-centric (Telekom Deutschland, Stadtwerke München, `Rechnung (2).pdf`, Würth, Raiffeisen, `Kontoauszug`). Make it English/international (English vendors and file names; one or two non-English documents to show multi-language reading). Add a marketing set to `docs/images/` from the snapshot tests (`gui::tests::readme_screenshots`, 1280 × 800, light + dark): preview, details, name matching, settings, offline, done + undo; use them in `README.md` and `docs/guide.md`. Every GUI snapshot changes: look at each PNG.
4. **Release 4.0.0-beta.2**: `playbooks/release.md` (version bump; README install table and AGENTS.md runbook name the new tag; signed Windows build on the VM, signed + notarized DMG on the Mac mini, which the user must switch on; GitHub serves the deb as `X.Y.Z.beta.N`, fix the Linux `SHA256SUMS` before publishing). Notes: list the beta.1 fixes above.

- **Details grid for custom profiles**: `screens/detail.rs` always shows the business rows (Company, Date, Type, Total/Subject); a custom profile plan (e.g. `receipts`) shows "Type —", "Subject —". Derive the rows from the plan profile's fields.
- **Real test PDFs**: `.todo/real-pdfs/` holds 18 real documents for local runs (git-ignored; never commit, never move into fixtures). See its README.

## Validation gaps

- Native Wayland drag-and-drop (`gui/wayland_dnd.rs`) is verified on Hyprland only; test GNOME (Mutter sends DnD only to a client's first data device) and KDE.
- Visual cap (`extract/visual.rs`, 0.75): calibrate on a real invoice corpus (false positives from rounded boxes, icons, signatures, QR codes; annotation stamps are not scanned).
- `image_only_lines` (OCR evidence merge): cost on dense pages unmeasured; multi-column reading order may append duplicate lines (duplicates are tolerated, losses are not).

## Scanner / image ingest

- Out of scope for 4.0. Photos and scans (JPEG/PNG/HEIC) → deskewed, cleaned PDF, then the normal pipeline. No live camera.
- Hard gate before shipping: on par with Genius Scan, Adobe Scan and Lens on a corpus of ~40 photos in 8 categories plus ~10 bad scans (CER and corner IoU at or above the best competitor, no category below the median, blind A/B ≥ 50% ours-or-tie).
- Candidate stack from the feasibility study (`docs/superpowers/specs/2026-11-06-rust-rewrite-feasibility.md`): oar-ocr UVDoc + DocAligner, imageproc linear-light cleanup, pdf-writer output. Avoid mupdf (AGPL), opencv-rust, DocTr/DewarpNet weights.

## Enrichment

- **PDF/A-3**: PDF/A files are skipped (`pdf_a`). PDF/A-3 allows `/AF` attachments; enriching needs an XMP extension schema for the `arp:` namespace and a conformance check (veraPDF). Do it when users report many skipped PDF/A scans.
- **Invisible OCR text layer**: write recognized text as an invisible layer so OCR'd scans become searchable (same incremental update, undoable). Check that PDFium and pdf.js read it back in the right order.

## OCR language packs

- Only the `latin` PP-OCRv6 pack (48 languages) is in `crates/core/src/ocr/manifest.rs`. Add CJK, Cyrillic, Arabic, Devanagari etc. (PP-OCRv5 script recognizers; URLs + sha256 from oar-ocr `docs/models.md`) when a user needs them. Steps: `playbooks/natives-and-models.md`.
- Settings › Reading already lists every pack in `PACKS` with Download/Remove; with ≥ 2 packs the scan-language select should offer "Automatic" plus installed packs only.
- `ocr.language: auto` uses the installed pack (today only `latin`, which reads English, German and 46 more at once). With ≥ 2 script packs installed, `auto` needs script detection to choose a pack per page (or run several and keep the better result); `ocr::pack_for_setting` is the hook.

## Decision models

Evaluated 2026-11-07. The default `ai` decider uses the main provider. `harmonization.decider: openai-decisions | jev` are implemented best-effort from the notes below (`crates/core/src/providers/decide.rs`) and need a live check against the real APIs before being recommended (keys: `OPENAI_API_KEY`, `JEV_API_KEY`).

- **OpenAI Decisions API** (`POST /v1/decisions`, gpt-6-luna, $0.10/1M input, no output charge) and **Jev** (TypeSafe, `POST api.typesafe.ai/v1/systemone`, early access, $0.042/1M) return only a predicate/choice/score over given options. They cannot replace extraction.
- Worth adding only when needed: (1) a calibrated `needs_review` flag for low-confidence renames; (2) tie-break of ambiguous company matches as a `choice` over ≤ 255 aliases. Docs: developers.openai.com/api/docs/guides/decisions, docs.typesafe.ai/models.

## Packaging and OS integration
- Unsigned Windows installers can trip Defender ML heuristics (seen once: `Program:Win32/Contebrew.A!ml` when the NSIS script deleted old program files; cleanup moved into `integrate install`). Ship only Azure-signed installers (`cargo xtask sign`).
- AppImage without FUSE: menu entries rely on the documented extract-to-`~/.local/opt/autorename-pdf` + `integrate install` route; an Exec that works mounted and extracted is not implemented.

- **Linux-side NSIS build**: Arch has no `makensis`, so Windows installers are built on Windows (CI `release.yml`). A Linux build needs NSIS from source or a container.
- **macOS Finder Quick Actions**: `integrate install` workflows (`crates/app/src/integrate/macos.rs`) are untested on a real Mac (needs someone at the Mac mini to right-click a PDF; see `playbooks/macos-release.md` step 4). Signing + notarization are done (`xtask dist --sign`).
- **Windows 11 modern context menu**: today the verbs are classic HKCU entries under "Show more options". The top-level menu needs a signed sparse package plus an `IExplorerCommand` COM handler.
- **CI GUI tests**: hosted runners have no GPU, so CI skips `gui::tests`. Try mesa lavapipe (`mesa-vulkan-drivers`) and check that the snapshots stay within tolerance.

## Auto-updater

- No updater in 4.0. Needs a hosted, signed manifest (version, per-OS URL, sha256), a check on start (opt-out, never in offline mode), and an in-place update that keeps the per-user install and context menu.
