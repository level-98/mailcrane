//! Parsing a search query. Never fails: anything it doesn't understand is
//! searched as words.

/// A parsed query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free words, in order; the last one is matched as a prefix.
    pub words: Vec<String>,
    /// `"exact phrases"`.
    pub phrases: Vec<String>,
    /// `from:` values.
    pub from: Vec<String>,
    /// `to:` values (To and Cc).
    pub to: Vec<String>,
    /// `subject:` values.
    pub subject: Vec<String>,
    /// `has:attachment`.
    pub has_attachment: bool,
    /// `is:unread`.
    pub unread: bool,
    /// `is:flagged`.
    pub flagged: bool,
    /// `after:YYYY-MM-DD`, as Unix seconds at midnight UTC.
    pub after: Option<i64>,
    /// `before:YYYY-MM-DD`, as Unix seconds at midnight UTC.
    pub before: Option<i64>,
    /// `in:<folder>`.
    pub folder: Option<String>,
}

impl SearchQuery {
    /// Whether the query asks for anything at all.
    pub fn is_empty(&self) -> bool {
        *self == SearchQuery::default()
    }
}

/// Longest query looked at, in characters; the rest is ignored.
const MAX_QUERY_CHARS: usize = 1_000;
/// Most terms used; more add little and slow the search.
const MAX_TERMS: usize = 32;

/// Splits `text` into tokens: whitespace-separated, with `"..."` kept whole
/// (and `key:"..."` too). An unbalanced quote runs to the end.
fn tokens(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    for c in text.chars().take(MAX_QUERY_CHARS) {
        match c {
            '"' => {
                if quoted {
                    quoted = false;
                } else {
                    quoted = true;
                    was_quoted = true;
                }
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push((std::mem::take(&mut current), was_quoted));
                }
                was_quoted = false;
            }
            c if c.is_control() => {}
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push((current, was_quoted));
    }
    out
}

/// `YYYY-MM-DD` as Unix seconds at midnight UTC.
fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let max_day = *days_in_month.get(usize::try_from(month - 1).ok()?)?;
    if day == 0 || day > max_day {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400)
}

/// Parses `text`. See [`SearchQuery`] for what's understood.
pub fn parse_query(text: &str) -> SearchQuery {
    let mut q = SearchQuery::default();
    for (token, quoted) in tokens(text).into_iter().take(MAX_TERMS) {
        // A whole quoted token is a phrase, whatever it contains.
        if quoted && !token.contains(':') {
            q.phrases.push(token);
            continue;
        }
        let Some((key, value)) = token.split_once(':') else {
            q.words.push(token);
            continue;
        };
        let value = value.trim();
        let handled = match (key.to_ascii_lowercase().as_str(), value) {
            (_, "") => false,
            ("from", v) => {
                q.from.push(v.to_owned());
                true
            }
            ("to" | "cc", v) => {
                q.to.push(v.to_owned());
                true
            }
            ("subject", v) => {
                q.subject.push(v.to_owned());
                true
            }
            ("in", v) => {
                q.folder = Some(v.to_owned());
                true
            }
            ("has", v) if v.eq_ignore_ascii_case("attachment") => {
                q.has_attachment = true;
                true
            }
            ("is", v) if v.eq_ignore_ascii_case("unread") => {
                q.unread = true;
                true
            }
            ("is", v) if v.eq_ignore_ascii_case("flagged") => {
                q.flagged = true;
                true
            }
            ("after", v) => parse_date(v).map(|d| q.after = Some(d)).is_some(),
            ("before", v) => parse_date(v).map(|d| q.before = Some(d)).is_some(),
            _ => false,
        };
        if !handled {
            // An unknown key, or a value that doesn't fit it: plain words.
            q.words.push(token);
        }
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_filter_is_understood() {
        let q = parse_query(
            "lunch from:ana@example.org to:bob subject:plans has:attachment is:unread is:flagged \
             after:2026-11-01 before:2026-12-01 in:Work",
        );
        assert_eq!(q.words, ["lunch"]);
        assert_eq!(q.from, ["ana@example.org"]);
        assert_eq!(q.to, ["bob"]);
        assert_eq!(q.subject, ["plans"]);
        assert!(q.has_attachment && q.unread && q.flagged);
        assert_eq!(q.after, Some(1_793_491_200)); // 2026-11-01T00:00:00Z
        assert_eq!(q.before, Some(1_796_083_200)); // 2026-12-01T00:00:00Z
        assert_eq!(q.folder.as_deref(), Some("Work"));
    }

    #[test]
    fn quoted_phrases_and_quoted_values() {
        let q = parse_query(r#"see "the big plan" from:"Ana Lopez" end"#);
        assert_eq!(q.words, ["see", "end"]);
        assert_eq!(q.phrases, ["the big plan"]);
        assert_eq!(q.from, ["Ana Lopez"]);
    }

    #[test]
    fn garbage_never_errors_and_falls_back_to_words() {
        for (input, words) in [
            ("unknown:thing", vec!["unknown:thing"]),
            ("after:yesterday", vec!["after:yesterday"]),
            ("before:2026-02-30", vec!["before:2026-02-30"]),
            ("has:pony", vec!["has:pony"]),
            ("is:", vec!["is:"]),
            ("from:", vec!["from:"]),
            (":", vec![":"]),
        ] {
            assert_eq!(parse_query(input).words, words, "{input}");
        }
        // Unbalanced quote: the rest is one phrase.
        assert_eq!(parse_query(r#"a "open ended"#).phrases, ["open ended"]);
        assert!(parse_query("").is_empty());
        assert!(parse_query("   \t\n").is_empty());
        // Control characters are dropped, and huge input is cut.
        assert_eq!(parse_query("a\u{0}b").words, ["ab"]);
        let huge = "word ".repeat(10_000);
        assert!(parse_query(&huge).words.len() <= MAX_TERMS);
    }

    #[test]
    fn dates_are_validated() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2024-02-29"), Some(1_709_164_800));
        assert_eq!(parse_date("2023-02-29"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-1-1"), Some(1_767_225_600));
        assert_eq!(parse_date("20261101"), None);
    }
}
