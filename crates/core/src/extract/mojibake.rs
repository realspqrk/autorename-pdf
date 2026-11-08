//! UTF-8-read-as-cp1252 repair (behavior contract §4.1).

const MARKERS: [&str; 7] = [
    "\u{00c3}",
    "\u{00c2}",
    "\u{00e2}\u{201a}",
    "\u{00e2}\u{20ac}\u{0153}",
    "\u{00e2}\u{20ac}",
    "\u{00e2}\u{20ac}\u{201c}",
    "\u{00e2}\u{20ac}\u{201d}",
];

/// Sum of non-overlapping counts per marker (overlapping markers double-count).
pub fn marker_count(s: &str) -> usize {
    MARKERS.iter().map(|m| s.matches(m).count()).sum()
}

/// cp1252 code points that map into 0x80..0x9F (the 27 defined ones).
const CP1252_HIGH: [(char, u8); 27] = [
    ('€', 0x80),
    ('‚', 0x82),
    ('ƒ', 0x83),
    ('„', 0x84),
    ('…', 0x85),
    ('†', 0x86),
    ('‡', 0x87),
    ('ˆ', 0x88),
    ('‰', 0x89),
    ('Š', 0x8A),
    ('‹', 0x8B),
    ('Œ', 0x8C),
    ('Ž', 0x8E),
    ('‘', 0x91),
    ('’', 0x92),
    ('“', 0x93),
    ('”', 0x94),
    ('•', 0x95),
    ('–', 0x96),
    ('—', 0x97),
    ('˜', 0x98),
    ('™', 0x99),
    ('š', 0x9A),
    ('›', 0x9B),
    ('œ', 0x9C),
    ('ž', 0x9E),
    ('Ÿ', 0x9F),
];

/// Strict cp1252 encoder (Python semantics): only the 27 defined code points map
/// into 0x80..0x9F; U+0080..U+009F themselves are not encodable.
fn cp1252_encode(s: &str) -> Option<Vec<u8>> {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if u < 0x80 || (0xA0..=0xFF).contains(&u) {
                Some(u as u8)
            } else {
                CP1252_HIGH.iter().find(|(ch, _)| *ch == c).map(|&(_, b)| b)
            }
        })
        .collect()
}

/// A UTF-8 continuation byte (0x80..0xBF) as it looks after a cp1252 or
/// latin-1 decode.
fn is_continuation(c: char) -> bool {
    (0x80..=0xBF).contains(&(c as u32)) || CP1252_HIGH.iter().any(|(ch, _)| *ch == c)
}

/// Chars covered by residual, unmistakable UTF-8-as-cp1252 sequences (what
/// [`repair`] could not fix), one flag per char of `chars`:
/// - `Â`/`Ã` + continuation (`Â°`, `Ã©`, `Ã¼`; 2-byte sequences),
/// - `â€`/`â‚` + optional continuation (`â€™`, `â€œ`, `â‚¬`),
/// - `ï»¿` (BOM), `ð` + three continuations (4-byte, emoji).
///
/// A lone `Ã`/`Â` (`SÃO PAULO`, `ÂGE`) or another accented capital before a
/// quote is legitimate text and is not flagged.
pub fn residual_mask(chars: &[char]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let cont = |i: usize| chars.get(i).copied().is_some_and(is_continuation);
    let mut i = 0;
    while i < chars.len() {
        let len = match chars[i] {
            '\u{00c2}' | '\u{00c3}' if cont(i + 1) => 2,
            '\u{00e2}' if matches!(chars.get(i + 1), Some('€' | '‚')) => {
                if cont(i + 2) {
                    3
                } else {
                    2
                }
            }
            '\u{00ef}' if chars.get(i + 1) == Some(&'»') && chars.get(i + 2) == Some(&'¿') => 3,
            '\u{00f0}' if cont(i + 1) && cont(i + 2) && cont(i + 3) => 4,
            _ => 1,
        };
        if len > 1 {
            mask[i..i + len].iter_mut().for_each(|m| *m = true);
        }
        i += len;
    }
    mask
}

type Encoder = fn(&str) -> Option<Vec<u8>>;

fn latin1_encode(s: &str) -> Option<Vec<u8>> {
    s.chars().map(|c| u8::try_from(c as u32).ok()).collect()
}

/// Try cp1252 then latin-1 round trip through strict UTF-8; the first that
/// succeeds and strictly reduces the marker count wins.
pub fn repair(text: &str) -> String {
    let orig = marker_count(text);
    if orig == 0 {
        return text.to_string();
    }
    let encoders: [Encoder; 2] = [cp1252_encode, latin1_encode];
    for enc in encoders {
        if let Some(bytes) = enc(text)
            && let Ok(fixed) = String::from_utf8(bytes)
            && !fixed.is_empty()
            && marker_count(&fixed) < orig
        {
            return fixed;
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp1252_roundtrip() {
        assert_eq!(repair("MÃ¼ller â€“ StraÃŸe"), "Müller – Straße");
    }

    #[test]
    fn undefined_cp1252_byte_falls_back_to_latin1() {
        assert_eq!(repair("Ã\u{81}"), "\u{c1}");
    }

    #[test]
    fn clean_text_untouched() {
        assert_eq!(repair("Müller"), "Müller");
        // Marker present but not repairable -> unchanged.
        assert_eq!(repair("Ã alone €\u{0100}"), "Ã alone €\u{0100}");
    }

    #[test]
    fn residual_sequences_are_flagged_lone_capitals_are_not() {
        let flags = |s: &str| {
            let chars: Vec<char> = s.chars().collect();
            residual_mask(&chars).iter().filter(|f| **f).count()
        };
        assert_eq!(flags("MÃ¼ller"), 2);
        assert_eq!(flags("Preis 5Â°C â€“ ok"), 2 + 3);
        assert_eq!(flags("â‚¬ 10"), 3);
        assert_eq!(flags("\u{00ef}»¿Text"), 3);
        assert_eq!(flags("SÃO PAULO, ÂGE, Ä“"), 0);
        assert_eq!(flags("Müller – Straße"), 0);
    }

    #[test]
    fn overlapping_markers_double_count() {
        assert_eq!(marker_count("\u{00e2}\u{20ac}\u{0153}"), 2);
    }
}
