# Natives (PDFium) and OCR models

## TOC
- [Fetch PDFium for dev](#fetch-pdfium-for-dev)
- [Fetch OCR models](#fetch-ocr-models)
- [Where things live](#where-things-live)
- [Run the tests](#run-the-tests)
- [Bump PDFium](#bump-pdfium)
- [Bump ORT / oar-ocr](#bump-ort--oar-ocr)
- [Bump or add a model pack](#bump-or-add-a-model-pack)

## Fetch PDFium for dev
```bash
cargo xtask fetch-natives                                  # host triple
cargo xtask fetch-natives --target x86_64-pc-windows-msvc  # one target
cargo xtask fetch-natives --all                            # every pinned target
```
- This downloads the bblanchon/pdfium-binaries tag `PDFIUM_TAG` (in `crates/xtask/src/natives.rs`) and verifies its sha256. It writes `vendor/pdfium/<triple>/{libpdfium.so|pdfium.dll|libpdfium.dylib, LICENSE, VERSION, .sha256}`.
- `vendor/` is gitignored. Re-running is a no-op when the `.sha256` stamp matches.
- Pinned triples: x86_64/aarch64 linux-gnu, x86_64/aarch64 windows-msvc, aarch64/x86_64 apple-darwin.

## Fetch OCR models
- The product command is `autorename-pdf models fetch [pack]`. In core it is `ocr::models::fetch(root, pack, progress)`. Other operations: `list`, `verify`, `import(root, pack, dir)` for offline installs, and `remove`.
- Extraction never downloads. Missing models with `ocr: on` (or `--ocr`): the file fails with `ocr_models_missing`. With `ocr: auto` (default): a warning with the same code, then the route falls through to vision.
- The only pack today is `latin` (≈39 MB): PP-OCRv6 small det+rec+dict plus PP-LCNet doc and text-line orientation. `ocr.language` defaults to `auto` (the installed pack, `ocr::pack_for_setting`); a pack id or language code (en, de, fr, …) maps to a pack via `manifest::pack_for_language`. A second script pack needs script detection for `auto` (followup.md).
- Dev shortcut: `AUTORENAME_MODELS_DIR=/some/dir` and then either `fetch`, or copy the manifest's files into `/some/dir/latin/`.

## Where things live
| What | Resolution order |
|---|---|
| PDFium lib | `$AUTORENAME_PDFIUM_DIR` → dir of the running exe → `<workspace>/vendor/pdfium/<triple>/` (dev) → system library path |
| Models root | `$AUTORENAME_MODELS_DIR` → OS data dir + `autorename-pdf/models` (Linux `~/.local/share/…`, Windows `%LOCALAPPDATA%\…`, macOS `~/Library/Application Support/…`) |
| Pack files | `<models root>/<pack>/<file name from manifest>` |
| ORT | statically linked by ort-sys `download-binaries` at build time; cache `~/.cache/dfbin/<triple>/<sha>/` |

Release packaging puts the PDFium lib next to the executable on Windows, in `/usr/lib/autorename-pdf/` for deb/pacman/AppImage, and in `Contents/Resources/` of the macOS bundle (`main.rs::locate_packaged_pdfium` sets `AUTORENAME_PDFIUM_DIR` for the last two). No x86_64-apple-darwin ORT exists, so build Intel Macs with `--no-default-features --features gui` (GUI kept, OCR off).

## Run the tests
```bash
cargo xtask fetch-natives
cargo test --workspace                                        # parity + routing + render
AUTORENAME_MODELS_DIR=<root with latin/> cargo test -p autorename-core --features ocr --test extract ocr_
cargo test -p autorename-core --test extract fetch_latin -- --ignored   # live download
```
Golden text lives in `crates/core/tests/golden/`. It holds pdfplumber 0.11 `extract_text()` dumps and `quality.json`, made from the committed `tests/fixtures/*.pdf`. Fixture PDFs do not change. A new fixture needs a pdfplumber reference: `_pdf_utils.extract_text(p, 3)` from the v3.2.0 tag in a throwaway Python venv, written unchanged with no trailing newline.

## Bump PDFium
1. `gh api repos/bblanchon/pdfium-binaries/releases --jq '.[0:3][].tag_name'`. Pick a `chromium/<build>` for which pdfium-render has a `pdfium_<build>` feature (or `pdfium_latest`).
2. Get the hashes: `gh api repos/bblanchon/pdfium-binaries/releases/tags/chromium/<build> --jq '.assets[]|.name+" "+.digest'`.
3. Update `PDFIUM_TAG` and every `sha256` in `crates/xtask/src/natives.rs`. Update the `pdfium_<build>` feature in `crates/core/Cargo.toml`.
4. Run `cargo xtask fetch-natives --all`, then `cargo test --workspace`. The golden parity test must stay byte-identical.

## Bump ORT / oar-ocr
- oar-ocr pins `ort` exactly, so bump oar-ocr (`cargo add -p autorename-core oar-ocr@=<v> --optional`) and let it carry ort along.
- Check `ort-sys-<v>/build/download/dist.tsv` for the targets that have prebuilt ORT. Re-run the OCR test with models, and re-check the binary size (stripped exe ≈ 32 MB with OCR).

## Bump or add a model pack
1. Get the files and sha256 from the oar-ocr `oar-ocr-core/src/core/download/registry.rs` at the pinned tag. Get the URLs from the oar-ocr `docs/models.md` (GitHub releases).
2. Add or modify the `ModelFile` consts and the `Pack` in `crates/core/src/ocr/manifest.rs`. List the pack in `PACKS` with its `languages`. Update the `total_size` assertion.
3. Verify with `fetch_latin_pack_live` (or an equivalent test for the new pack) and with the OCR accuracy test.
- Followup: no non-Latin packs yet. Candidates are the `*_pp-ocrv5_mobile_rec.onnx` models and their dicts for korean, th, el, te, ta, eslav, arabic, cyrillic and devanagari, plus server rec for ch/japan.
