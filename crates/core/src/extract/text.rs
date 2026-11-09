//! Tier-1 text: PDFium char boxes → plain text matching pdfplumber 0.11
//! `page.extract_text()` defaults (x/y tolerance 3, non-layout). Validated
//! byte-identical on all fixtures and 70/72 real-world PDFs (spike doc
//! `docs/superpowers/specs/2026-11-07-spike-extraction.md`).
//!
//! Reading order (pdfplumber algorithm):
//! 1. Chars in content-stream order, split into consecutive runs of equal `upright`.
//! 2. Per run, cluster chars into lines by `top` with a *chained* tolerance
//!    (sorted unique values; new cluster when value > previous + 3), top-to-bottom.
//! 3. Per line, stable sort by x0, split into words: whitespace ends a word (dropped);
//!    a char starts a new word when `x0 < prev.x0 || x0 > prev.x1 + 3 || |Δtop| > 3`.
//! 4. Words re-clustered by word top (order preserving): `" "` within, `"\n"` between
//!    lines. Ligatures expanded.
//!
//! Geometry must be pdfminer's, not PDFium's raw boxes:
//! - x0 = glyph origin (PDFium's loose left is min(origin, ink left)).
//! - x1 = loose right, unless the ink defines that edge (overhang, e.g. italic `f`):
//!   then x0 + font advance · matrix.a.
//! - top = page_h − (loose.bottom + Tf · matrix.d) (`FPDFText_GetFontSize` is unscaled Tf).
//! - PDFium-generated chars (synthetic spaces/CRLF) are skipped; `IsHyphen` (U+0002)
//!   becomes `-`; ligature parts sharing an origin share the glyph's right edge.

use pdfium_render::prelude::*;
use unicode_normalization::UnicodeNormalization;

use super::mojibake;
use super::quality::py_isspace;

const X_TOL: f32 = 3.0;
const Y_TOL: f32 = 3.0;

/// A positioned char in pdfplumber's top-down coordinate space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PChar {
    pub text: String,
    pub x0: f32,
    pub x1: f32,
    pub top: f32,
    pub upright: bool,
    /// PDF-space baseline y; only used to detect multi-char glyphs (ligatures).
    pub origin_y: f32,
}

struct Word {
    text: String,
    top: f32,
}

/// pdfplumber `LIGATURES` (expand_ligatures=True).
fn expand_ligature(s: &str) -> &str {
    match s {
        "\u{FB00}" => "ff",
        "\u{FB03}" => "ffi",
        "\u{FB04}" => "ffl",
        "\u{FB01}" => "fi",
        "\u{FB02}" => "fl",
        "\u{FB06}" | "\u{FB05}" => "st",
        other => other,
    }
}

/// pdfplumber `cluster_list` + `make_cluster_dict`: cluster id per value, ids
/// increasing with value, chained tolerance.
fn cluster_ids(values: &[f32], tol: f32) -> Vec<usize> {
    let mut uniq = values.to_vec();
    uniq.sort_by(f32::total_cmp);
    uniq.dedup();
    let mut ids = Vec::with_capacity(uniq.len());
    let mut id = 0usize;
    for (i, v) in uniq.iter().enumerate() {
        if i > 0 && *v > uniq[i - 1] + tol {
            id += 1;
        }
        ids.push(id);
    }
    values
        .iter()
        .map(|v| ids[uniq.partition_point(|u| u < v)])
        .collect()
}

fn chars_to_words(chars: &[PChar]) -> Vec<Word> {
    let mut words = Vec::new();
    let flush = |cur: &mut Vec<&PChar>, words: &mut Vec<Word>| {
        if !cur.is_empty() {
            let text = cur.iter().map(|c| expand_ligature(&c.text)).collect();
            let top = cur.iter().map(|c| c.top).fold(f32::INFINITY, f32::min);
            words.push(Word { text, top });
            cur.clear();
        }
    };
    for run in chars.chunk_by(|a, b| a.upright == b.upright) {
        let upright = run[0].upright;
        // Non-upright text: pdfplumber clusters by x0; approximated the same way.
        let keys: Vec<f32> = run
            .iter()
            .map(|c| if upright { c.top } else { c.x0 })
            .collect();
        let ids = cluster_ids(&keys, if upright { Y_TOL } else { X_TOL });
        let mut order: Vec<usize> = (0..run.len()).collect();
        order.sort_by_key(|&i| ids[i]); // stable: stream order inside a cluster
        for line_idx in order.chunk_by(|&a, &b| ids[a] == ids[b]) {
            let mut line: Vec<&PChar> = line_idx.iter().map(|&i| &run[i]).collect();
            line.sort_by(|a, b| a.x0.total_cmp(&b.x0));
            let mut cur: Vec<&PChar> = Vec::new();
            for c in line {
                if c.text.chars().all(py_isspace) {
                    flush(&mut cur, &mut words);
                    continue;
                }
                if let Some(prev) = cur.last()
                    && (c.x0 < prev.x0
                        || c.x0 > prev.x1 + X_TOL
                        || (c.top - prev.top).abs() > Y_TOL)
                {
                    flush(&mut cur, &mut words);
                }
                cur.push(c);
            }
            flush(&mut cur, &mut words);
        }
    }
    words
}

/// Pure layout step: positioned chars → page text.
pub(crate) fn layout_text(chars: &[PChar]) -> String {
    let words = chars_to_words(chars);
    let tops: Vec<f32> = words.iter().map(|w| w.top).collect();
    let ids = cluster_ids(&tops, Y_TOL);
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            out.push(if ids[i] == ids[i - 1] { ' ' } else { '\n' });
        }
        out.push_str(&w.text);
    }
    out
}

/// Accessor for pdfium-render's raw bindings: the high-level char API has no
/// glyph advance widths, which pdfminer geometry needs.
struct Raw;
impl<'a> PdfiumLibraryBindingsAccessor<'a> for Raw {}

/// Decode one PDFium char (UTF-16 unit) into text; `None` = skip.
fn decode_unit(u: u32, hyphen: bool, pending_high: &mut Option<u32>) -> Option<String> {
    if hyphen {
        return Some("-".into());
    }
    if (0xD800..0xDC00).contains(&u) {
        *pending_high = Some(u);
        return None;
    }
    if (0xDC00..0xE000).contains(&u) {
        let h = pending_high.take()?;
        return char::from_u32(0x10000 + ((h - 0xD800) << 10) + (u - 0xDC00)).map(String::from);
    }
    match char::from_u32(u) {
        Some('\0') | None => None,
        Some(c) => Some(c.to_string()),
    }
}

/// Collect pdfminer-equivalent char geometry for one page.
#[allow(unsafe_code)] // raw FPDFText_* calls; handles are valid for the page's lifetime
fn page_chars(page: &PdfPage) -> Result<Vec<PChar>, PdfiumError> {
    let b = Raw.bindings();
    let page_h = page.height().value;
    // SAFETY: the page handle is live for the duration of `page`; the text page is
    // closed before returning and all out-pointers reference local stack values.
    unsafe {
        let tp = b.FPDFText_LoadPage(b.get_handle_from_page(page));
        if tp.is_null() {
            return Err(PdfiumError::PdfiumLibraryInternalError(
                PdfiumInternalError::Unknown,
            ));
        }
        let n = b.FPDFText_CountChars(tp).max(0);
        let mut chars: Vec<PChar> = Vec::with_capacity(n as usize);
        let mut pending_high = None;
        for i in 0..n {
            if b.FPDFText_IsGenerated(tp, i) == 1 {
                continue;
            }
            let u = b.FPDFText_GetUnicode(tp, i);
            let Some(text) = decode_unit(u, b.FPDFText_IsHyphen(tp, i) == 1, &mut pending_high)
            else {
                continue;
            };
            let mut r = FS_RECTF {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            };
            if !b.is_true(b.FPDFText_GetLooseCharBox(tp, i, &mut r)) {
                continue;
            }
            let mut m = FS_MATRIX {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            };
            b.FPDFText_GetMatrix(tp, i, &mut m);
            // pdfminer LTChar: upright = a·d > 0 and b·c <= 0; we also require no skew.
            let upright =
                m.a * m.d > 0.0 && m.b * m.c <= 0.0 && m.b.abs() < 1e-3 && m.c.abs() < 1e-3;
            let size = b.FPDFText_GetFontSize(tp, i) as f32;
            let (mut ox, mut oy) = (f64::from(r.left), f64::from(r.bottom));
            b.FPDFText_GetCharOrigin(tp, i, &mut ox, &mut oy);
            let (mut x0, mut x1) = (r.left, r.right);
            if upright {
                x0 = ox as f32;
                let (mut tl, mut tr, mut tb, mut tt) = (0f64, 0f64, 0f64, 0f64);
                let tight =
                    b.is_true(b.FPDFText_GetCharBox(tp, i, &mut tl, &mut tr, &mut tb, &mut tt));
                // Loose box = union(advance, ink). Only when ink defines the right edge
                // is the advance unknown; then ask the font.
                if tight && (r.right - tr as f32).abs() < 0.01 {
                    let obj = b.FPDFText_GetTextObject(tp, i);
                    let font = if obj.is_null() {
                        std::ptr::null_mut()
                    } else {
                        b.FPDFTextObj_GetFont(obj)
                    };
                    let mut w: f32 = 0.0;
                    if !font.is_null()
                        && b.is_true(b.FPDFFont_GetGlyphWidth(font, u, size, &mut w))
                        && w > 0.0
                        && x0 + w * m.a <= r.right + 0.01
                    {
                        x1 = x0 + w * m.a;
                    }
                }
                if let Some(prev) = chars.last_mut()
                    && (prev.x0 - x0).abs() < 0.01
                    && (prev.origin_y - oy as f32).abs() < 0.01
                {
                    x1 = x1.max(r.right).max(prev.x1);
                    prev.x1 = x1;
                }
            }
            let y_top_up = if upright {
                r.bottom + size * m.d
            } else {
                r.top
            };
            chars.push(PChar {
                text,
                x0,
                x1,
                top: page_h - y_top_up,
                upright,
                origin_y: oy as f32,
            });
        }
        b.FPDFText_ClosePage(tp);
        Ok(chars)
    }
}

/// Text of one page (pdfplumber-equivalent, before NFC).
pub fn page_text(page: &PdfPage) -> Result<String, PdfiumError> {
    page_chars(page).map(|c| layout_text(&c))
}

/// Contract §4.1, per page: pages `0..min(max_pages, n)` in order, each NFC
/// and mojibake-repaired (`Err` = the warning for a page that failed).
pub fn document_pages(doc: &PdfDocument, max_pages: usize) -> Vec<Result<String, String>> {
    let n = doc.pages().len().max(0) as usize;
    (0..n.min(max_pages))
        .map(|i| {
            doc.pages()
                .get(i as PdfPageIndex)
                .and_then(|p| page_text(&p))
                .map(|t| mojibake::repair(&t.nfc().collect::<String>()))
                .map_err(|e| format!("Could not extract text from page {}: {e}", i + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(text: &str, x0: f32, x1: f32, top: f32) -> PChar {
        PChar {
            text: text.into(),
            x0,
            x1,
            top,
            upright: true,
            origin_y: 800.0 - top,
        }
    }

    #[test]
    fn chained_clusters() {
        assert_eq!(cluster_ids(&[0.0, 2.5, 5.0, 9.0], 3.0), vec![0, 0, 0, 1]);
    }

    #[test]
    fn gap_and_space_split_words_lines_sorted_top_down() {
        let chars = vec![
            ch("b", 10.0, 15.0, 50.0), // second line, emitted first in stream
            ch("a", 10.0, 15.0, 10.0),
            ch("c", 16.0, 20.0, 10.0), // gap 1 -> same word
            ch(" ", 20.0, 22.0, 10.0), // explicit space -> word break
            ch("d", 22.0, 26.0, 10.5), // |Δtop| <= 3 -> same line
            ch("e", 30.0, 34.0, 10.0), // gap 4 > 3 -> new word
            ch("\u{FB01}", 40.0, 45.0, 50.0),
        ];
        assert_eq!(layout_text(&chars), "ac d e\nb fi");
    }

    #[test]
    fn decode_surrogates_hyphen_nul() {
        let mut hi = None;
        assert_eq!(decode_unit(0x2, true, &mut hi).as_deref(), Some("-"));
        assert_eq!(decode_unit(0, false, &mut hi), None);
        assert_eq!(decode_unit(0xD83D, false, &mut hi), None);
        assert_eq!(decode_unit(0xDE00, false, &mut hi).as_deref(), Some("😀"));
        assert_eq!(decode_unit(0xDE00, false, &mut hi), None); // orphan low surrogate
    }
}
