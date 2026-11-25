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
    /// `account:<address or name>` (M4-21).
    pub account: Option<String>,
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

/// `days` since 1970-01-01 as `YYYY-MM-DD`, or `None` outside years
/// 1–9999 (what [`parse_date`] accepts).
pub fn format_day(days: i64) -> Option<String> {
    // Civil from days (Howard Hinnant's algorithm).
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe.checked_add(era.checked_mul(400)?)? + i64::from(month <= 2);
    if !(1..=9999).contains(&year) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// A filter the search chips can add or remove (M4-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// `from:<value>`.
    From,
    /// `has:attachment`.
    HasAttachment,
    /// `is:unread`.
    Unread,
    /// `is:flagged`.
    Flagged,
    /// `after:<YYYY-MM-DD>`.
    After,
    /// `before:<YYYY-MM-DD>`.
    Before,
    /// `account:<value>`.
    Account,
}

impl Filter {
    /// The key, and for on/off filters the one value they take.
    fn key_value(self) -> (&'static str, Option<&'static str>) {
        match self {
            Filter::From => ("from", None),
            Filter::HasAttachment => ("has", Some("attachment")),
            Filter::Unread => ("is", Some("unread")),
            Filter::Flagged => ("is", Some("flagged")),
            Filter::After => ("after", None),
            Filter::Before => ("before", None),
            Filter::Account => ("account", None),
        }
    }

    /// Whether `token` is this filter (an empty `from:` being typed counts).
    fn matches(self, token: &str) -> bool {
        let Some((key, value)) = token.split_once(':') else {
            return false;
        };
        let (want_key, want_value) = self.key_value();
        key.eq_ignore_ascii_case(want_key)
            && want_value.is_none_or(|w| value.trim().eq_ignore_ascii_case(w))
    }
}

/// A token written back as query text, quoted where it has spaces.
fn render(token: &str, quoted: bool) -> String {
    match token.split_once(':') {
        Some((key, value)) if quoted || value.chars().any(char::is_whitespace) => {
            format!("{key}:\"{value}\"")
        }
        _ if quoted => format!("\"{token}\""),
        _ => token.to_owned(),
    }
}

/// `text` with every `filter` term removed, then `filter` added with
/// `value` when it is `Some` (for on/off filters any `Some` turns it on).
/// Everything else in `text` is kept, in order. A value is cleaned of
/// quotes and control characters; an empty value adds nothing.
pub fn with_filter(text: &str, filter: Filter, value: Option<&str>) -> String {
    let mut parts: Vec<String> = tokens(text)
        .into_iter()
        .filter(|(token, _)| !filter.matches(token))
        .map(|(token, quoted)| render(&token, quoted))
        .collect();
    if let Some(value) = value {
        let (key, fixed) = filter.key_value();
        let clean: String = fixed
            .unwrap_or(value)
            .chars()
            .filter(|c| *c != '"' && !c.is_control())
            .take(MAX_QUERY_CHARS)
            .collect();
        let clean = clean.trim();
        if !clean.is_empty() {
            parts.push(render(&format!("{key}:{clean}"), false));
        }
    }
    parts.join(" ")
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
            ("account", v) => {
                q.account = Some(v.to_owned());
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
    fn account_filter_is_understood() {
        let q = parse_query("account:work@example.org lunch account:\"Ann Home\"");
        assert_eq!(q.account.as_deref(), Some("Ann Home"), "the last one wins");
        assert_eq!(q.words, ["lunch"]);
    }

    #[test]
    fn filters_are_added_replaced_and_removed_keeping_the_rest() {
        let text = r#"lunch from:ana "big plan" is:unread"#;
        let added = with_filter(text, Filter::HasAttachment, Some(""));
        assert_eq!(
            added,
            r#"lunch from:ana "big plan" is:unread has:attachment"#
        );
        // Turning it on twice leaves one.
        assert_eq!(with_filter(&added, Filter::HasAttachment, Some("")), added);
        // Removing one `is:` keeps the other.
        let flagged = with_filter(&added, Filter::Flagged, Some(""));
        let unflagged = with_filter(&flagged, Filter::Unread, None);
        assert_eq!(
            unflagged,
            r#"lunch from:ana "big plan" has:attachment is:flagged"#
        );
        // Values replace, and get quotes when they have spaces.
        let account = with_filter(text, Filter::Account, Some("Ann Home"));
        assert_eq!(
            account,
            r#"lunch from:ana "big plan" is:unread account:"Ann Home""#
        );
        assert_eq!(parse_query(&account).account.as_deref(), Some("Ann Home"));
        let other = with_filter(&account, Filter::Account, Some("bob@example.org"));
        assert!(
            other.ends_with("is:unread account:bob@example.org"),
            "{other}"
        );
        // Removing `from:` also removes one still being typed.
        assert_eq!(with_filter("from:ana x from:", Filter::From, None), "x");
        // Quoted values survive a rewrite.
        assert_eq!(
            with_filter(r#"from:"Ana Lopez" x"#, Filter::Unread, Some("")),
            r#"from:"Ana Lopez" x is:unread"#
        );
    }

    #[test]
    fn hostile_filter_values_cannot_inject_terms() {
        let text = with_filter("x", Filter::Account, Some("a\" is:flagged \"b\n"));
        let q = parse_query(&text);
        assert!(!q.flagged, "{text}");
        assert_eq!(q.account.as_deref(), Some("a is:flagged b"));
        assert_eq!(with_filter("x", Filter::Account, Some(" \" ")), "x");
    }

    #[test]
    fn days_format_and_round_trip() {
        assert_eq!(format_day(0).as_deref(), Some("1970-01-01"));
        assert_eq!(format_day(20_757).as_deref(), Some("2026-10-31"));
        for days in [-719_162, -1, 0, 59, 10_956, 20_758, 2_932_896] {
            let text = format_day(days).expect("in range");
            assert_eq!(parse_date(&text), Some(days * 86_400), "{text}");
        }
        assert_eq!(format_day(-719_163), None, "year 0");
        assert_eq!(format_day(2_932_897), None, "year 10000");
        assert_eq!(format_day(i64::MAX), None);
        assert_eq!(format_day(i64::MIN), None);
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
