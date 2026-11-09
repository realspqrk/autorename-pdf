# Follow-ups

Open work after 4.0. Done items are removed; the history is in git and `docs/superpowers/specs/`.

## TOC

- [Validation gaps](#validation-gaps)
- [Scanner / image ingest](#scanner--image-ingest)
- [Enrichment](#enrichment)
- [OCR language packs](#ocr-language-packs)
- [Decision models](#decision-models)
- [Packaging and OS integration](#packaging-and-os-integration)
- [Auto-updater](#auto-updater)

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
- `ocr.language: auto` uses the installed pack (today only `latin`, which reads English, German and 46 more at once). With ≥ 2 script packs installed, `auto` needs script detection to choose a pack per page (or run several and keep the better result); `ocr::pack_for_setting` is the hook.

## Decision models

Evaluated 2026-11-07. The default `ai` decider uses the main provider. `harmonization.decider: openai-decisions | jev` are implemented best-effort from the notes below (`crates/core/src/providers/decide.rs`) and need a live check against the real APIs before being recommended (keys: `OPENAI_API_KEY`, `JEV_API_KEY`).

- **OpenAI Decisions API** (`POST /v1/decisions`, gpt-6-luna, $0.10/1M input, no output charge) and **Jev** (TypeSafe, `POST api.typesafe.ai/v1/systemone`, early access, $0.042/1M) return only a predicate/choice/score over given options. They cannot replace extraction.
- Worth adding only when needed: (1) a calibrated `needs_review` flag for low-confidence renames; (2) tie-break of ambiguous company matches as a `choice` over ≤ 255 aliases. Docs: developers.openai.com/api/docs/guides/decisions, docs.typesafe.ai/models.

## Packaging and OS integration
- Unsigned Windows installers can trip Defender ML heuristics (seen once: `Program:Win32/Contebrew.A!ml` when the NSIS script deleted old program files; cleanup moved into `integrate install`). Ship only Azure-signed installers (`cargo xtask sign`).
- AppImage without FUSE: menu entries rely on the documented extract-to-`~/.local/opt/autorename-pdf` + `integrate install` route; an Exec that works mounted and extracted is not implemented.

- **Linux-side NSIS build**: Arch has no `makensis`, so Windows installers are built on Windows (CI `release.yml`). A Linux build needs NSIS from source or a container.
- **macOS signing + notarization**: unsigned Apple silicon `.app`/`.dmg` build, smoke test, full `cargo test` and GUI launch verified on the Mac mini 2026-11-09; signing/notarization (`playbooks/macos-release.md` step 3) never run; Finder Quick Actions untested. Mac is ready via the `mac-remote` skill; open: a local `Packager.toml` `signing-identity` plus `APPLE_KEYCHAIN_PROFILE=<notarytool profile>` (cargo-packager checks it first; `APPLE_API_*` / `APPLE_ID` env also work).
- **Windows 11 modern context menu**: today the verbs are classic HKCU entries under "Show more options". The top-level menu needs a signed sparse package plus an `IExplorerCommand` COM handler.
- **CI GUI tests**: hosted runners have no GPU, so CI skips `gui::tests`. Try mesa lavapipe (`mesa-vulkan-drivers`) and check that the snapshots stay within tolerance.

## Auto-updater

- No updater in 4.0. Needs a hosted, signed manifest (version, per-OS URL, sha256), a check on start (opt-out, never in offline mode), and an in-place update that keeps the per-user install and context menu.
