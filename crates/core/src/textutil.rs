//! Small shared text helpers.

use unicode_normalization::UnicodeNormalization;

pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

pub fn nfkc(s: &str) -> String {
    s.nfkc().collect()
}

/// Python `str.isidentifier()` approximation (Unicode letters/digits/underscore).
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}
