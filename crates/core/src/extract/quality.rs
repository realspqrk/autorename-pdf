//! Per-page readability score (behavior contract §4.2; not Python parity)
//! and Python `str` predicates.

use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};

use super::mojibake;

/// Python `str.isspace()`: Rust's White_Space plus U+001C..U+001F.
pub fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1C}'..='\u{1F}').contains(&c)
}

/// Python `str.isalnum()` ≈ general category L* or N*. Rust's `is_alphanumeric`
/// uses the derived Alphabetic property, which also accepts combining marks.
pub fn py_isalnum(c: char) -> bool {
    matches!(
        c.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
    )
}

/// True when `s` is empty or whitespace only (Python `not s.strip()`).
pub fn is_blank(s: &str) -> bool {
    s.chars().all(py_isspace)
}

/// A char that only appears when the text layer is broken: U+FFFD, a
/// private-use code point (unmapped glyphs), or a control char other than
/// tab/CR/LF.
fn is_broken(c: char) -> bool {
    c == '\u{FFFD}'
        || c.general_category() == GeneralCategory::PrivateUse
        || (c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

/// Readability score 0.0–1.0 of one page's text (after NFC and mojibake
/// repair, without the `Page N:` header). Not Python parity.
///
/// `N` = non-whitespace chars; a blank page or one without any letter or
/// digit scores 0. `B` = broken chars (U+FFFD, private use, control chars
/// except tab/CR/LF; counted before whitespace is dropped), `M` = chars in
/// residual multi-char mojibake sequences ([`mojibake::residual_mask`]),
/// each char counted once. `Q = clamp(1 − 10·(B+M)/N, 0, 1)`: 2 % broken
/// chars → 0.8, 10 % → 0. Length does not matter: a short clean page
/// scores 1.0.
///
/// Limitation: only *visibly* broken text is caught. A text layer with a
/// wrong ToUnicode map that decodes to plausible letters ("Kbwfsfj")
/// passes as 1.0; so does a clean text layer that misses content printed
/// only as an image (logos) — [`super::visual`] caps such pages instead.
pub fn page_quality(text: &str) -> f64 {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.iter().filter(|&&c| !py_isspace(c)).count();
    if n == 0 || !chars.iter().any(|&c| py_isalnum(c)) {
        return 0.0;
    }
    let mojibake = mojibake::residual_mask(&chars);
    let bad = chars
        .iter()
        .zip(&mojibake)
        .filter(|&(&c, &m)| m || is_broken(c))
        .count();
    (1.0 - 10.0 * bad as f64 / n as f64).clamp(0.0, 1.0)
}

/// Document summary: the worst page (never an average, so one broken page
/// is not hidden by good ones). No pages → 0.
pub fn document_quality(pages: impl IntoIterator<Item = f64>) -> f64 {
    pages.into_iter().reduce(f64::min).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_or_symbol_only_is_zero() {
        assert_eq!(page_quality(""), 0.0);
        assert_eq!(page_quality(" \n\t\u{1F}"), 0.0);
        assert_eq!(page_quality("----- ... ///"), 0.0);
    }

    #[test]
    fn short_clean_text_is_perfect() {
        assert_eq!(page_quality("ref: 42"), 1.0);
        assert_eq!(page_quality("Müller – Straße 5\nSÃO PAULO\tÂGE"), 1.0);
    }

    #[test]
    fn broken_chars_scale_tenfold() {
        // 2 % replacement chars -> 0.8.
        let text = format!("{}{}", "a".repeat(98), "\u{FFFD}".repeat(2));
        assert!((page_quality(&text) - 0.8).abs() < 1e-9);
        // Private-use glyph garbage -> 0.
        let pua: String = "Invoice "
            .chars()
            .chain("\u{E000}\u{E001}\u{F8FF}".chars())
            .collect();
        assert_eq!(page_quality(&pua), 0.0);
        // Control chars count even when Python calls them whitespace (U+001C
        // is not in N, but is in B).
        let ctl = format!("{}\u{1C}\u{7}", "a".repeat(98));
        assert!((page_quality(&ctl) - (1.0 - 20.0 / 99.0)).abs() < 1e-9);
        // Residual mojibake: 2 chars of `Ã¼` per char each, counted once.
        let moji = format!("{}Ã¼", "a".repeat(98));
        assert!((page_quality(&moji) - 0.8).abs() < 1e-9);
        // C1 control inside a mojibake pair is not counted twice.
        let both = format!("{}Ã\u{81}", "a".repeat(98));
        assert!((page_quality(&both) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn document_is_the_worst_page() {
        assert_eq!(document_quality([1.0, 0.2, 0.9]), 0.2);
        assert_eq!(document_quality([]), 0.0);
    }

    #[test]
    fn python_space_and_alnum_semantics() {
        assert!(py_isspace('\u{1C}'));
        assert!(!py_isalnum('\u{0345}')); // combining mark: Alphabetic in Rust, not isalnum
        assert!(py_isalnum('²'));
    }
}
