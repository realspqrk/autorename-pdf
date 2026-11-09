# Spike: text extraction + offline OCR (Rust rebuild gate)

Date 2026-11-07 · Host Arch x86_64, 8 cores · Rust 1.99.1 · Gate from rust-rebuild §Pipeline 1–2.
Code: `/tmp/claude-1000/-home-spqrk-Work-autorename-pdf/c1e2ef4f-94cd-4c3b-ac5b-10d15ed84bda/scratchpad/spike-extract` (`src/text.rs` → `crates/core/src/extract/text.rs`, `src/ocr.rs` → `crates/core/src/ocr.rs`; `scripts/` = Python reference + OCR scorer).

## Verdict: **GO**

Text: 9/9 fixtures byte-identical to pdfplumber (`_pdf_utils.extract_text`), quality identical to 4 d.p.; 70/72 extra real PDFs exact, 0 threshold flips. Render: pixel-identical to pypdfium2. OCR: oar-ocr PP-OCRv6 small reads both image fixtures with all key fields, ~0.5 s/page CPU, statically linked, no Python.

## Pinned versions / artifacts

| Item | Pin | Source / hash |
|---|---|---|
| pdfium-render | `=0.9.5`, features `pdfium_8125, image_latest, thread_safe` (no default) | crates.io |
| PDFium | bblanchon/pdfium-binaries **`chromium/8125`** (PDFium 158.0.8125.0) | `https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8125/pdfium-<plat>.tgz` |
|  linux-x64 | 3 828 800 B tgz, `libpdfium.so` 8.0 MB | `e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659` (verified) |
|  linux-arm64 | | `8b8818f2ec57d1b213d952ca6d7f0ffeca9d0110487d0e20ad83ba918bc92efa` |
|  win-x64 | | `9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b` |
|  win-arm64 | | `d394ba61991fb53f4d87f70988035f337d9e73f2bd2c3c1b966df8c43fcabb05` |
|  mac-arm64 / mac-x64 | | `4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d` / `8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0` |
|  mac-univ | | `0cf06803b9a4acc489fc967a1b05990622ca1cf41fa027ff9cc89eeab9b19279` |
| oar-ocr | `=0.10.2` (MSRV 1.95, Apache-2.0); pins `ort =2.0.0-rc.13` (`std, ndarray, tracing, copy-dylibs`) | crates.io |
| ONNX Runtime | **1.28.0** (pyke static build via ort-sys `download-binaries`) | linux-x64 `e454f710f8a49f53aa5b4ff51e3454ae1835777e431c6c35c5255ce6f205fd68`; win-x64 (`directml` set) `f7c654b3…b527d`; mac-arm64 (`coreml`) `6934874e…674c7` |
| Models (GitHub release; same files on ModelScope `greatv/oar-ocr`, hashes = oar-ocr `REGISTRY`) | `https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/<f>` | all 5 verified locally |
|  `pp-ocrv6_small_det.onnx` 9 880 512 B | | `d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e` |
|  `pp-ocrv6_small_rec.onnx` 21 159 378 B | | `5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634` |
|  `ppocrv6_dict.txt` 74 947 B | | `b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d` |
|  `pp-lcnet_x1_0_doc_ori.onnx` 6 787 248 B (`…/v0.3.0/`) | | `bbcd6c2b43ab15d2e605455aef2cd280ab87b570824522d24a69e9298875a1ac` |
|  `pp-lcnet_x0_25_textline_ori.onnx` 1 018 629 B (`…/v0.3.0/`) | | `fb402220f39b183d64a68cd48d4bd53267a21354b6fc39370c2a83fbdad85b10` |

Note: pypdfium2 5.15 in the Python venv bundles PDFium 8146; 8125 is pdfium-render 0.9.5's newest binding set (`pdfium_latest`). Renders were still pixel-identical.

## Text parity (max_pages 3, threshold 0.3)

| Fixture | Norm. Levenshtein | Exact | q Python | q Rust | Same side |
|---|---|---|---|---|---|
| empty | 1.0000 | yes | 0.0000 | 0.0000 | yes |
| image_invoice_springfield | 1.0000 | yes | 0.0000 | 0.0000 | yes |
| minimal_text | 1.0000 | yes | 0.5620 | 0.5620 | yes |
| mixed_invoice_initech | 1.0000 | yes | 0.8140 | 0.8140 | yes |
| multipage_invoice_stark | 1.0000 | yes | 0.9682 | 0.9682 | yes |
| text_invoice_acme | 1.0000 | yes | 0.9167 | 0.9167 | yes |
| text_letter_globex | 1.0000 | yes | 0.8860 | 0.8860 | yes |
| text_outgoing_invoice_wayne | 1.0000 | yes | 0.8237 | 0.8237 | yes |
| text_rechnung_mustermann | 1.0000 | yes | 0.7537 | 0.7537 | yes |
| **Extra: 72 local PDFs** (/usr/share/doc: LaTeX, groff, Ghostscript, CUPS, forms incl. Cyrillic) | mean 0.9999, min 0.9984 | 70/72 | — | max Δq 0.0001 | 72/72 |

Residual diffs: `neatoguide.pdf` (two lines inside a figure cluster differently by <0.5 pt), `speex/manual.pdf` (pdfminer prints `(cid:13)` for an unmapped glyph, PDFium a blank). Both acceptable. 5 corpus files are gzip, not PDF: both sides yield `""`/0.0. Speed: 1–15 ms per document (3 pages).

## Recommended reading-order policy ("pdfplumber non-layout emulation")

Implemented in `text.rs` with raw `FPDFText_*` calls through pdfium-render's bindings (the high-level char API has no advance width):
1. Skip PDFium-generated chars (`IsGenerated`); map `IsHyphen` (U+0002) → `-`; combine UTF-16 surrogates; drop NUL.
2. Char box in pdfminer geometry: `x0` = char origin; `x1` = loose right, unless ink touches it (overhang: italic `p`, `f`), then `x0 + GlyphWidth(unicode, Tf)·m.a` (accepted only if ≤ loose right); `top = page_h − (loose.bottom + Tf·m.d)`. Upright = `a·d>0 && b≈0 && c≈0`. Ligature parts (same origin) share the glyph's right edge.
3. pdfplumber algorithm: groupby `upright` runs → chained cluster by top (tol 3) → sort by x0 → words split on whitespace / `x0 > prev.x1+3` / `x0 < prev.x0` / `|Δtop|>3` → order-preserving line cluster of word tops → `" "` / `"\n"` join, ligatures expanded.
4. `Page {n}:` assembly, NFC, strict cp1252 → latin-1 mojibake round trip, quality score: verbatim port with Python `isspace` (adds U+001C–1F) and `isalnum` = Unicode category L*/N* (not Rust `is_alphanumeric`). Unit tests included.

Each fix in step 2 was driven by a concrete diff. Without them, parity on the corpus was 0.996 (fixtures were already exact).

## Rendering (pypdfium2 `render(scale)` equivalent)

`ceil(pt·scale)` target size, white clear colour, annots and forms on, RGB8. A4 → 893×1263 @1.5, 1191×1684 @2.0. **Pixel-identical (max diff 0)** to Python on 3 fixtures × 2 scales. Render 5–30 ms/page. PNG: the `image` default is 1.6× PIL's size. Use `CompressionType::Best + FilterType::Adaptive` (`ocr::encode_png`), which comes out 10–15% *smaller* than PIL (e.g. acme @2.0 114 KB vs PIL 135 KB) at 30–75 ms.

## OCR (oar-ocr 0.10.2, PP-OCRv6 small, CPU, default ORT threads)

The Python PaddleOCR venv is absent on this host (`~/.local/share/autorename-pdf/paddleocr-venv`), so I scored against the ground truth in `tests/generate_test_pdfs.py`. CER = Levenshtein with whitespace removed. Key fields = invoice no, date, company, total.

| Config | Springfield (scan) CER / keys | Initech (mixed) CER / keys | ms/page (warm) |
|---|---|---|---|
| scale 1.5, oar default det resize (min-side 736) | 0.013 / 3/4 (`4300.00`, comma lost) | 0.000 / 4/4 | 450–650 |
| **scale 2.0**, default det | 0.013 / **4/4** | 0.000 / 4/4 | 470–500 |
| det limit **736 max** (= Python bridge setting) | **0.538 / 1/4** | 0.027 / 4/4 | 220–360 |
| + doc+textline orientation (PP-LCNet), page rotated 90° / 180° | 0.013 / 4/4 | 0.000 / 4/4 | ~480–530 |
| rotated 90° / 180° **without** orientation | 0.79 / 4/4 · 0.82 / 0/4 | 0.77 / 4/4 · 0.85 / 0/4 | — |

Engine init 150–190 ms (5 sessions). Peak RSS 390 MB. Output is in Python-bridge shape (`Page n:\n` + lines in sorted-box order joined `\n`). Typical residual errors are lost spaces (`Net30`, `Date:03.11.2024`). Compared with PaddleOCR subprocess (~2–5 s/page per CLAUDE.md), this is 4–10× faster with no venv.

Recommendations:
- Keep oar-ocr's det resize (`LimitType::Min`). **Do not port the bridge's `det_limit_side_len 736 / max`**, which destroys small-font scans.
- Render OCR at 2.0, not 1.5. This is an intentional change vs contract §4.4: it makes vision and OCR share one render.
- Enable doc orientation and textline orientation by default. They add +7.8 MB of models and ≈0 ms measurable cost.

## Binary size / distribution

| Build (linux x64, stripped) | Size |
|---|---|
| text + render only (`--no-default-features`) | 3.5 MB (+ `libpdfium.so` 8.0 MB beside it) |
| + oar-ocr + **static** ORT 1.28 | 31.9 MB (ORT ≈ 28 MB; `libonnxruntime.a` 105 MB). `ldd`: only libstdc++/libgcc_s/libm/libc |
| Models (det+rec+dict+2 ori) | 38 MB, downloaded on first use, hash-pinned |

- **PDFium**: dynamic `bind_to_library(exe_dir)`. Ship `pdfium.dll` / `libpdfium.so` / `libpdfium.dylib` from the pinned bblanchon tag next to the binary (cargo-packager resources; AppImage `usr/lib`; mac `Contents/Frameworks`). A build script downloads it and verifies the sha256 above. pdfium-render `static` needs a self-built static PDFium, so it is not worth it.
- **ORT**: keep `download-binaries` (the default) = pyke static link, sha-verified by ort-sys itself, one self-contained exe. Available pyke dists: x86_64/aarch64 linux (`none`), x86_64/aarch64 windows-msvc (**`directml`** feature set is the only CPU-usable one, so it gets picked), aarch64-apple-darwin (`coreml`). **No x86_64-apple-darwin dist, and Microsoft 1.28.0 ships no osx-x64 either.** Intel Mac options: build with `--no-default-features` (OCR off, clear error) or a self-built ORT via `ORT_LIB_LOCATION`. `load-dynamic` is not needed anywhere else. CI cache: `~/.cache/dfbin/<target>/<hash>`. Offline builds: `ORT_LIB_LOCATION` + oar-ocr `--no-default-features --features simd`.
- Windows linking needs MSVC v143 (VS 2022) STL (oar-ocr FAQ: unresolved `__std_*` with VS 2019).

## Caveats / follow-ups

1. **Windows untested.** Must check whether the static `directml` ORT pulls in `DirectML.dll` at load time (delay-load expected), the MSVC link, and that PDFium DLL lookup works beside the exe. Do this in the Windows VM before M1 closes.
2. PDFium thread safety: `thread_safe` feature serialises calls globally. That matches the spec ("PDFium serialized").
3. Rotated pages (`/Rotate`) and vertical/CJK text were only lightly covered. Non-upright chars use the loose box and x-clustering approximation. Add a rotated + vertical fixture to the golden set.
4. Repair fallback (pdfplumber `repair=True` via Ghostscript) has no PDFium analogue. PDFium is already tolerant; on open failure return `("", 0.0)` as today.
5. OCR accuracy was only measured on 2 synthetic fixtures. Re-score on real scans (and against PaddleOCR when a venv exists) before the M-OCR gate. Latin langs reuse the v6 small rec. Other script packs come from the `*_pp-ocrv5_mobile_rec` registry entries.
