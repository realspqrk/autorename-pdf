//! Document date parsing and strftime-subset formatting.
//!
//! Only complete calendar dates parse: missing parts are never filled in and
//! relative words (`today`, `yesterday`) are rejected.
//!
//! Accepted shapes (DMY preference for numeric dates):
//! - ISO `2024-03-15` / `2024/03/15` / `2024.3.5` (optionally `T…`/` …` time suffix)
//! - `15.03.2024`, `15/03/2024`, `15-03-2024`, `15. 3. 2024`, two-digit years
//! - `15 March 2024`, `15. März 2024`, `March 15, 2024`, `15th Mar 2024`,
//!   `1er janvier 2024`, `15 de marzo de 2024` (EN/DE/FR/IT/ES, full + abbreviated)

use chrono::{Datelike, NaiveDate};

/// Parse a model-supplied date string. `None` when blank, partial or invalid.
pub fn parse_date(input: &str) -> Option<NaiveDate> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    parse_numeric(s).or_else(|| parse_textual(s))
}

fn ymd(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, d)
}

fn expand_year(digits: &str) -> Option<i32> {
    let y: i32 = digits.parse().ok()?;
    match digits.len() {
        4 => Some(y),
        2 => Some(if y < 70 { 2000 + y } else { 1900 + y }),
        _ => None,
    }
}

fn parse_numeric(s: &str) -> Option<NaiveDate> {
    // ISO with optional time suffix.
    let head = match s.find(['T', ' ']) {
        Some(i) if i == 10 && s.as_bytes().get(4).is_some_and(|b| *b == b'-') => &s[..i],
        _ => s,
    };
    let parts: Vec<&str> = head.split(['.', '/', '-']).map(str::trim).collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    // Separator must be consistent ignoring the optional space after a dot.
    let seps: Vec<char> = head
        .chars()
        .filter(|c| matches!(c, '.' | '/' | '-'))
        .collect();
    if seps.len() != 2 || seps[0] != seps[1] {
        return None;
    }
    if parts[0].len() == 4 {
        let y = parts[0].parse().ok()?;
        return ymd(y, parts[1].parse().ok()?, parts[2].parse().ok()?);
    }
    if parts[0].len() > 2 || parts[1].len() > 2 {
        return None;
    }
    let year = expand_year(parts[2])?;
    ymd(year, parts[1].parse().ok()?, parts[0].parse().ok()?)
}

/// Month table: (name, month). Lowercase, accents kept; ASCII fallbacks included.
const MONTHS: &[(&str, u32)] = &[
    // English
    ("january", 1),
    ("jan", 1),
    ("february", 2),
    ("feb", 2),
    ("march", 3),
    ("mar", 3),
    ("april", 4),
    ("apr", 4),
    ("may", 5),
    ("june", 6),
    ("jun", 6),
    ("july", 7),
    ("jul", 7),
    ("august", 8),
    ("aug", 8),
    ("september", 9),
    ("sep", 9),
    ("sept", 9),
    ("october", 10),
    ("oct", 10),
    ("november", 11),
    ("nov", 11),
    ("december", 12),
    ("dec", 12),
    // German
    ("januar", 1),
    ("jänner", 1),
    ("jaenner", 1),
    ("jän", 1),
    ("februar", 2),
    ("feber", 2),
    ("märz", 3),
    ("maerz", 3),
    ("mär", 3),
    ("mrz", 3),
    ("mai", 5),
    ("juni", 6),
    ("juli", 7),
    ("oktober", 10),
    ("okt", 10),
    ("dezember", 12),
    ("dez", 12),
    // French
    ("janvier", 1),
    ("janv", 1),
    ("février", 2),
    ("fevrier", 2),
    ("févr", 2),
    ("fevr", 2),
    ("mars", 3),
    ("avril", 4),
    ("avr", 4),
    ("juin", 6),
    ("juillet", 7),
    ("juil", 7),
    ("août", 8),
    ("aout", 8),
    ("septembre", 9),
    ("octobre", 10),
    ("novembre", 11),
    ("décembre", 12),
    ("decembre", 12),
    ("déc", 12),
    // Italian
    ("gennaio", 1),
    ("gen", 1),
    ("febbraio", 2),
    ("marzo", 3),
    ("aprile", 4),
    ("maggio", 5),
    ("mag", 5),
    ("giugno", 6),
    ("giu", 6),
    ("luglio", 7),
    ("lug", 7),
    ("agosto", 8),
    ("ago", 8),
    ("settembre", 9),
    ("set", 9),
    ("ottobre", 10),
    ("ott", 10),
    ("dicembre", 12),
    ("dic", 12),
    // Spanish
    ("enero", 1),
    ("ene", 1),
    ("febrero", 2),
    ("abril", 4),
    ("abr", 4),
    ("mayo", 5),
    ("junio", 6),
    ("julio", 7),
    ("septiembre", 9),
    ("setiembre", 9),
    ("octubre", 10),
    ("noviembre", 11),
    ("diciembre", 12),
];

/// Filler words that may appear between date parts.
const FILLERS: &[&str] = &["de", "del", "of", "the", "le", "der", "den", "am", "il"];

const WEEKDAYS: &[&str] = &[
    "monday",
    "mon",
    "tuesday",
    "tue",
    "tues",
    "wednesday",
    "wed",
    "thursday",
    "thu",
    "thur",
    "thurs",
    "friday",
    "fri",
    "saturday",
    "sat",
    "sunday",
    "sun",
    "montag",
    "dienstag",
    "mittwoch",
    "donnerstag",
    "freitag",
    "samstag",
    "sonnabend",
    "sonntag",
    "lundi",
    "mardi",
    "mercredi",
    "jeudi",
    "vendredi",
    "samedi",
    "dimanche",
    "lunedì",
    "lunedi",
    "martedì",
    "martedi",
    "mercoledì",
    "mercoledi",
    "giovedì",
    "giovedi",
    "venerdì",
    "venerdi",
    "sabato",
    "domenica",
    "lunes",
    "martes",
    "miércoles",
    "miercoles",
    "jueves",
    "viernes",
    "sábado",
    "domingo",
];

fn month_of(word: &str) -> Option<u32> {
    MONTHS.iter().find(|(n, _)| *n == word).map(|(_, m)| *m)
}

/// Strip English/French/Spanish ordinal suffixes from a day token (`15th`, `1er`, `1º`).
fn strip_ordinal(tok: &str) -> &str {
    for suf in ["st", "nd", "rd", "th", "er", "re", "º", "°", "o", "a"] {
        if let Some(rest) = tok.strip_suffix(suf)
            && !rest.is_empty()
            && rest.bytes().all(|b| b.is_ascii_digit())
        {
            return rest;
        }
    }
    tok
}

fn parse_textual(s: &str) -> Option<NaiveDate> {
    let lower = s.to_lowercase();
    let mut month: Option<u32> = None;
    let mut month_pos = 0usize;
    let mut numbers: Vec<(usize, &str)> = Vec::new();
    let tokens: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '/' | '-'))
        .filter(|t| !t.is_empty())
        .collect();
    for (pos, tok) in tokens.iter().enumerate() {
        let tok = strip_ordinal(tok);
        if tok.bytes().all(|b| b.is_ascii_digit()) {
            numbers.push((pos, tok));
        } else if let Some(m) = month_of(tok) {
            if month.is_some() {
                return None;
            }
            month = Some(m);
            month_pos = pos;
        } else if FILLERS.contains(&tok) || WEEKDAYS.contains(&tok) {
            continue;
        } else {
            return None;
        }
    }
    let month = month?;
    if numbers.len() != 2 {
        return None; // `March 2024` → unknown day: no filling from today.
    }
    // The year is the 4-digit number, else the number after the month in D M Y order.
    let (day, year) = match (numbers[0], numbers[1]) {
        ((_, a), (_, b)) if b.len() == 4 && a.len() <= 2 => (a, b),
        ((_, a), (_, b)) if a.len() == 4 && b.len() <= 2 => (b, a),
        ((pa, a), (pb, b)) if a.len() <= 2 && b.len() == 2 && pa < month_pos && pb > month_pos => {
            (a, b)
        }
        _ => return None,
    };
    ymd(expand_year(year)?, month, day.parse().ok()?)
}

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAY_NAMES: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Format a date with a Python-`strftime` subset (C locale).
///
/// Supported: `%Y %y %m %d %e %j %B %b %h %A %a %u %w %F %%` and glibc-style
/// `%-d %-m %-j`. Time fields `%H %M %S` render `00`. Unknown directives are
/// copied literally (glibc behavior).
pub fn format_date(date: NaiveDate, fmt: &str) -> String {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut no_pad = false;
        if chars.peek() == Some(&'-') {
            no_pad = true;
            chars.next();
        }
        let Some(d) = chars.next() else {
            out.push('%');
            break;
        };
        let month = date.month0() as usize;
        let wd = date.weekday().num_days_from_monday() as usize;
        let num = |v: u32, width: usize| {
            if no_pad {
                v.to_string()
            } else {
                format!("{v:0width$}")
            }
        };
        match d {
            'Y' => out.push_str(&format!("{:04}", date.year())),
            'y' => out.push_str(&format!("{:02}", date.year().rem_euclid(100))),
            'm' => out.push_str(&num(date.month(), 2)),
            'd' => out.push_str(&num(date.day(), 2)),
            'e' => out.push_str(&format!("{:>2}", date.day())),
            'j' => out.push_str(&num(date.ordinal(), 3)),
            'B' => out.push_str(MONTH_NAMES[month]),
            'b' | 'h' => out.push_str(&MONTH_NAMES[month][..3]),
            'A' => out.push_str(DAY_NAMES[wd]),
            'a' => out.push_str(&DAY_NAMES[wd][..3]),
            'u' => out.push_str(&(wd + 1).to_string()),
            'w' => out.push_str(&date.weekday().num_days_from_sunday().to_string()),
            'F' => out.push_str(&format!(
                "{:04}-{:02}-{:02}",
                date.year(),
                date.month(),
                date.day()
            )),
            'H' | 'M' | 'S' => out.push_str(if no_pad { "0" } else { "00" }),
            '%' => out.push('%'),
            other => {
                out.push('%');
                if no_pad {
                    out.push('-');
                }
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, day)
    }

    #[test]
    fn golden_parse_table() {
        let table: &[(&str, Option<NaiveDate>)] = &[
            // Contract §3.5 observed dateparser results (complete dates).
            ("06.09.2026", d(2026, 9, 6)),
            ("2026-09-06", d(2026, 9, 6)),
            ("09/06/2026", d(2026, 6, 9)),
            ("15.03.24", d(2024, 3, 15)),
            ("6 Sept. 2026", d(2026, 9, 6)),
            ("15 March 2024", d(2024, 3, 15)),
            ("15.03.2024", d(2024, 3, 15)),
            ("31.02.2026", None),
            ("not a date", None),
            ("", None),
            ("   ", None),
            ("1.234,56", None),
            ("20240315", None),
            // More shapes observed with dateparser.
            ("1. März 2024", d(2024, 3, 1)),
            ("2024-3-5", d(2024, 3, 5)),
            ("5/3/2024", d(2024, 3, 5)),
            ("03-05-2024", d(2024, 5, 3)),
            ("2024/03/05", d(2024, 3, 5)),
            ("15 Mar 2024", d(2024, 3, 15)),
            ("March 15, 2024", d(2024, 3, 15)),
            ("Mar 15 2024", d(2024, 3, 15)),
            ("15th March 2024", d(2024, 3, 15)),
            ("1er janvier 2024", d(2024, 1, 1)),
            ("15 marzo 2024", d(2024, 3, 15)),
            ("15 de marzo de 2024", d(2024, 3, 15)),
            ("15.3.2024", d(2024, 3, 15)),
            ("15. 3. 2024", d(2024, 3, 15)),
            ("15. Okt. 2024", d(2024, 10, 15)),
            ("3 août 2025", d(2025, 8, 3)),
            ("12 dicembre 2023", d(2023, 12, 12)),
            ("Montag, 4. Jänner 2021", d(2021, 1, 4)),
            ("2024-03-15T10:00:00", d(2024, 3, 15)),
            ("29.02.2024", d(2024, 2, 29)),
            ("29.02.2023", None),
            // Intentional changes: no fill-from-today, no relative words.
            ("2026", None),
            ("March 2024", None),
            ("März 2024", None),
            ("today", None),
            ("yesterday", None),
            ("15.03", None),
            ("15 March", None),
            ("15.03-2024", None),
            ("March April 2024", None),
        ];
        for (input, expected) in table {
            assert_eq!(parse_date(input), *expected, "input {input:?}");
        }
    }

    #[test]
    fn strftime_subset() {
        let date = d(2024, 3, 5).unwrap();
        let table = [
            ("%Y%m%d", "20240305"),
            ("%Y-%m-%d", "2024-03-05"),
            ("%d-%m-%Y", "05-03-2024"),
            ("%d.%m.%y", "05.03.24"),
            ("%-d.%-m.%Y", "5.3.2024"),
            ("%B %Y", "March 2024"),
            ("%b %a %A", "Mar Tue Tuesday"),
            ("%j %F", "065 2024-03-05"),
            ("100%%", "100%"),
            ("%Q", "%Q"),
            ("trail%", "trail%"),
        ];
        for (fmt, expected) in table {
            assert_eq!(format_date(date, fmt), expected, "fmt {fmt:?}");
        }
    }
}
