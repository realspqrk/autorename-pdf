//! Printed invoice amounts → one filename style. The model copies the amount
//! as printed (`1.234,56`, `4,258.80`, `€0.00`); [`normalize`] parses it and
//! writes it with the configured decimal mark and no grouping, so names do
//! not depend on the document's locale.

/// Currency symbols with one obvious ISO code (`$`, `¥` are ambiguous).
const SYMBOLS: [(&str, &str); 3] = [("€", "EUR"), ("£", "GBP"), ("₹", "INR")];

/// `printed` as `[-]digits[<sep>decimals]` with `sep` as decimal mark.
/// Grouping (`.` `,` `'` spaces) is dropped; when both `.` and `,` occur the
/// last one is the decimal mark; a single mark followed by exactly three
/// digits is grouping (`1.234` = 1234). Currency text around the number is
/// ignored. `None` when the text is not one number (kept as printed).
pub fn normalize(printed: &str, sep: &str) -> Option<String> {
    let t = printed.trim();
    let negative = t.starts_with(['-', '−'])
        || t.ends_with(['-', '−'])
        || (t.starts_with('(') && t.ends_with(')'));
    let start = t.find(|c: char| c.is_ascii_digit())?;
    let end = t.rfind(|c: char| c.is_ascii_digit())? + 1;
    let core: String = t[start..end]
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}' | '\u{2009}' | '\'' | '’'))
        .collect();
    if !core
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return None;
    }
    let decimal_at = match (core.rfind('.'), core.rfind(',')) {
        (Some(d), Some(c)) => Some(d.max(c)),
        (Some(i), None) | (None, Some(i)) => {
            let mark = core.as_bytes()[i];
            let single = core.bytes().filter(|b| *b == mark).count() == 1;
            (single && core.len() - i - 1 != 3).then_some(i)
        }
        (None, None) => None,
    };
    let (int, frac) = match decimal_at {
        Some(i) => (&core[..i], &core[i + 1..]),
        None => (core.as_str(), ""),
    };
    // The decimal mark occurs once; grouping uses the other mark.
    if decimal_at.is_some_and(|i| int.contains(core.as_bytes()[i] as char)) {
        return None;
    }
    let int: String = int.chars().filter(char::is_ascii_digit).collect();
    let int = match int.trim_start_matches('0') {
        "" => "0",
        s => s,
    };
    let sign = if negative { "-" } else { "" };
    Some(match frac {
        "" => format!("{sign}{int}"),
        f => format!("{sign}{int}{sep}{f}"),
    })
}

/// A currency symbol with one obvious ISO code becomes that code
/// (`€` → `EUR`); anything else is kept.
pub fn currency_code(currency: &str) -> String {
    let c = currency.trim();
    SYMBOLS
        .iter()
        .find(|(s, _)| *s == c)
        .map_or_else(|| c.to_string(), |(_, iso)| (*iso).to_string())
}

/// The obvious currency of a symbol printed inside the amount (`€0.00`).
pub fn currency_in(printed: &str) -> Option<&'static str> {
    SYMBOLS
        .iter()
        .find(|(s, _)| printed.contains(s))
        .map(|(_, iso)| *iso)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printed_amounts_follow_one_decimal_mark() {
        let cases = [
            ("12,13", "12.13"),
            ("100.00", "100.00"),
            ("1.234,56", "1234.56"),
            ("4,258.80", "4258.80"),
            ("1 234,56", "1234.56"),
            ("1'234.50", "1234.50"),
            ("1.234", "1234"),
            ("1,234,567", "1234567"),
            ("€0.00", "0.00"),
            ("EUR 6.29", "6.29"),
            ("-10,00", "-10.00"),
            ("10,00-", "-10.00"),
            ("(5.00)", "-5.00"),
            ("0,5", "0.5"),
            ("007", "7"),
        ];
        for (printed, want) in cases {
            assert_eq!(normalize(printed, ".").as_deref(), Some(want), "{printed}");
        }
        assert_eq!(normalize("1.234,56", ",").as_deref(), Some("1234,56"));
        for bad in ["", "n/a", "12,3,4.5.6", "12 EUR 13"] {
            assert_eq!(normalize(bad, "."), None, "{bad}");
        }
        assert_eq!(currency_code("€"), "EUR");
        assert_eq!(currency_code("$"), "$");
        assert_eq!(currency_in("€0.00"), Some("EUR"));
    }
}
