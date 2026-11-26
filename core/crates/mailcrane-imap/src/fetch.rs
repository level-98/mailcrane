//! Selecting a folder and fetching headers and bodies (RFC 9051 §6.3.2,
//! §6.4.4 and §6.4.5).

use std::collections::HashSet;

use crate::append::SearchKey;
use crate::command::Arg;
use crate::error::ImapError;
use crate::parser::{Code, FetchData, Untagged};
use crate::session::{expect_ok, Session};

/// The most messages one `UID SEARCH` asks about. Its `* SEARCH` answer is
/// then at most 4096 UIDs of at most 10 digits and a space each, about
/// 44 KiB, under the 64 KiB line limit. Keep it below `max_line / 11`.
pub const SEARCH_WINDOW: u32 = 4096;

/// What `SELECT` told us about a folder (RFC 9051 §6.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SelectInfo {
    /// Number of messages.
    pub exists: u32,
    /// `UIDVALIDITY` (RFC 9051 §2.3.1.1).
    pub uidvalidity: u32,
    /// `UIDNEXT`.
    pub uidnext: u32,
    /// `HIGHESTMODSEQ` (RFC 7162 §3.1.2.1), or `None` if the server did not
    /// send one or said `NOMODSEQ` (§3.1.2.2): then mod-sequences cannot be
    /// used for this folder.
    pub highestmodseq: Option<u64>,
    /// Whether new keywords can be stored permanently: `PERMANENTFLAGS`
    /// lists `\*`, or the server sent no `PERMANENTFLAGS`, in which case all
    /// flags can be changed permanently (RFC 9051 §7.1).
    pub custom_keywords: bool,
}

/// The result of fetching headers for a set of UIDs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchedHeaders {
    /// Messages that were read.
    pub messages: Vec<FetchData>,
    /// UIDs (where known) of messages whose response could not be read.
    /// Their content is never logged, only the UID.
    pub skipped: Vec<Option<u32>>,
}

/// The items fetched for each message in the message list.
const HEADER_ITEMS: &str = "(UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE BODYSTRUCTURE \
    BODY.PEEK[HEADER.FIELDS (REFERENCES IN-REPLY-TO LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST LIST-ID)])";

fn code_number<T: std::str::FromStr>(done: &[Untagged], wanted: &str) -> Option<T> {
    done.iter().find_map(|u| match u {
        Untagged::Condition {
            text:
                crate::parser::RespText {
                    code: Some(Code::Other { name, args }),
                    ..
                },
            ..
        } if name.eq_ignore_ascii_case(wanted) => args.as_deref()?.trim().parse().ok(),
        _ => None,
    })
}

fn code_args<'a>(done: &'a [Untagged], wanted: &str) -> Option<&'a str> {
    done.iter().find_map(|u| match u {
        Untagged::Condition {
            text:
                crate::parser::RespText {
                    code: Some(Code::Other { name, args }),
                    ..
                },
            ..
        } if name.eq_ignore_ascii_case(wanted) => Some(args.as_deref().unwrap_or_default()),
        _ => None,
    })
}

fn has_code(done: &[Untagged], wanted: &str) -> bool {
    done.iter().any(|u| {
        matches!(u, Untagged::Condition {
            text: crate::parser::RespText {
                code: Some(Code::Other { name, .. }),
                ..
            },
            ..
        } if name.eq_ignore_ascii_case(wanted))
    })
}

/// Reads what a successful `SELECT` reported. Fails if `UIDVALIDITY` or
/// `UIDNEXT` is missing.
pub(crate) fn select_info(untagged: &[Untagged]) -> Result<SelectInfo, ImapError> {
    let exists = untagged
        .iter()
        .find_map(|u| match u {
            Untagged::Exists(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0);
    let uidvalidity = code_number(untagged, "UIDVALIDITY").ok_or(ImapError::Protocol)?;
    let uidnext = code_number(untagged, "UIDNEXT").ok_or(ImapError::Protocol)?;
    // RFC 7162 §3.1.2.2: NOMODSEQ wins over anything else; §7 limits a
    // mod-sequence to 63 bits, and zero is not a valid HIGHESTMODSEQ.
    let highestmodseq = if has_code(untagged, "NOMODSEQ") {
        None
    } else {
        code_number::<u64>(untagged, "HIGHESTMODSEQ")
            .filter(|m| *m > 0 && i64::try_from(*m).is_ok())
    };
    let custom_keywords = code_args(untagged, "PERMANENTFLAGS").is_none_or(|args| {
        args.split_whitespace()
            .any(|f| f.trim_matches(['(', ')']) == "\\*")
    });
    Ok(SelectInfo {
        exists,
        uidvalidity,
        uidnext,
        highestmodseq,
        custom_keywords,
    })
}

pub(crate) fn uid_set(uids: &[u32]) -> String {
    let mut out = String::new();
    for (i, uid) in uids.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&uid.to_string());
    }
    out
}

impl Session {
    /// `SELECT` (RFC 9051 §6.3.2). `raw_name` is the folder's name exactly as
    /// the server listed it. Fails if the server does not report
    /// `UIDVALIDITY` and `UIDNEXT`.
    pub async fn select(&mut self, raw_name: &str) -> Result<SelectInfo, ImapError> {
        let done = self
            .run_command("SELECT", &[Arg::Str(raw_name.as_bytes())])
            .await?;
        expect_ok(&done)?;
        let info = select_info(&done.untagged)?;
        self.selected = Some((raw_name.to_owned(), info));
        Ok(info)
    }

    /// Like [`Session::select`], but when `raw_name` is already the selected
    /// folder of this session, returns what its `SELECT` reported without
    /// sending anything. For a reused connection that opens messages one
    /// after another in the same folder (issue #294). The cached
    /// `UIDVALIDITY` is the one from that `SELECT`; a server can't change it
    /// while the folder stays selected without ending the session.
    pub async fn select_reusing(&mut self, raw_name: &str) -> Result<SelectInfo, ImapError> {
        match &self.selected {
            Some((name, info)) if name == raw_name => Ok(*info),
            _ => self.select(raw_name).await,
        }
    }

    /// `NOOP` (RFC 9051 §6.1.2): a cheap check that the connection still
    /// works, and lets the server send pending updates. Never retried
    /// automatically by this crate.
    pub async fn noop(&mut self) -> Result<(), ImapError> {
        let done = self.run_command("NOOP", &[]).await?;
        expect_ok(&done)
    }

    /// The UIDs in `low..=high` that match every key in `keys`, newest
    /// first, at most `want` of them (RFC 9051 §6.4.4). `exists` is the
    /// folder's message count from `SELECT`.
    ///
    /// A large folder's `* SEARCH` answer would be one line longer than
    /// [`Limits::max_line`](crate::Limits), and raising that limit would
    /// weaken the defence against hostile servers. So the folder is searched
    /// by message sequence number, newest first, [`SEARCH_WINDOW`] messages
    /// at a time: `UID SEARCH [CHARSET UTF-8] <a:b> [keys]`. No answer can
    /// hold more than `SEARCH_WINDOW` UIDs, and the number of round trips
    /// depends on how many messages are scanned, not on gaps between UIDs
    /// (Gmail folders are sparse). The scan stops once `want` UIDs are
    /// found, or, without keys, once a window reaches below `low`.
    ///
    /// Sequence numbers and UIDs rise together, so windows come newest
    /// first. A message expunged during the scan only shifts newer messages
    /// down into the next window, so nothing is missed (duplicates are
    /// dropped). The first window ends at `*`, the current last message.
    pub async fn uid_search_newest(
        &mut self,
        exists: u32,
        low: u32,
        high: u32,
        keys: &[SearchKey],
        want: usize,
    ) -> Result<Vec<u32>, ImapError> {
        let mut found: Vec<u32> = Vec::new();
        if low == 0 || low > high || want == 0 || exists == 0 {
            return Ok(found);
        }
        let mut seen: HashSet<u32> = HashSet::new();
        let mut exists = exists;
        let mut top = exists;
        let mut first = true;
        loop {
            let bottom = top.saturating_sub(SEARCH_WINDOW.saturating_sub(1)).max(1);
            let set = if first {
                format!("{bottom}:*")
            } else {
                format!("{bottom}:{top}")
            };
            first = false;
            let mut args = vec![Arg::Raw("SEARCH")];
            if !keys.is_empty() {
                args.extend([Arg::Raw("CHARSET"), Arg::Raw("UTF-8")]);
            }
            args.push(Arg::Built(&set));
            crate::append::push_search_keys(&mut args, keys);
            let done = self.run_command("UID", &args).await?;
            expect_ok(&done)?;

            let mut window: Vec<u32> = Vec::new();
            for untagged in &done.untagged {
                match untagged {
                    Untagged::Search(list) => window.extend_from_slice(list),
                    Untagged::Exists(n) => exists = *n,
                    Untagged::Expunge(_) => exists = exists.saturating_sub(1),
                    Untagged::Vanished {
                        earlier: false,
                        uids,
                    } => {
                        let gone = uids.iter().fold(0u32, |n, (a, b)| {
                            n.saturating_add(b.abs_diff(*a)).saturating_add(1)
                        });
                        exists = exists.saturating_sub(gone);
                    }
                    _ => {}
                }
            }
            window.sort_unstable_by(|a, b| b.cmp(a));
            let window_oldest = window.last().copied();
            for uid in window {
                // A server may answer outside the range we asked for.
                if (low..=high).contains(&uid) && seen.insert(uid) {
                    found.push(uid);
                    if found.len() >= want {
                        return Ok(found);
                    }
                }
            }
            // Without keys the window lists every message in it, so its
            // oldest UID shows whether older windows can still be in range.
            let below_range = keys.is_empty() && window_oldest.is_some_and(|uid| uid < low);
            if below_range || bottom <= 1 {
                break;
            }
            top = bottom.saturating_sub(1).min(exists);
            if top == 0 {
                break;
            }
        }
        found.sort_unstable_by(|a, b| b.cmp(a));
        Ok(found)
    }

    /// `UID FETCH` of the list fields for `uids` (RFC 9051 §6.4.5). Uses
    /// `BODY.PEEK`, so nothing is marked `\Seen`. A message whose response
    /// cannot be read is skipped and reported in
    /// [`FetchedHeaders::skipped`].
    pub async fn uid_fetch_headers(&mut self, uids: &[u32]) -> Result<FetchedHeaders, ImapError> {
        if uids.is_empty() {
            return Ok(FetchedHeaders::default());
        }
        let set = uid_set(uids);
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("FETCH"),
                    Arg::Str(set.as_bytes()),
                    Arg::Raw(HEADER_ITEMS),
                ],
            )
            .await?;
        expect_ok(&done)?;
        let mut out = FetchedHeaders::default();
        for untagged in done.untagged {
            match untagged {
                Untagged::Fetch(data) => match data.uid {
                    // Only messages we asked for, each once.
                    Some(uid)
                        if uids.contains(&uid)
                            && !out.messages.iter().any(|m| m.uid == Some(uid)) =>
                    {
                        out.messages.push(*data);
                    }
                    Some(_) => {}
                    None => out.skipped.push(None),
                },
                Untagged::FetchMalformed { uid } => out.skipped.push(uid),
                _ => {}
            }
        }
        Ok(out)
    }

    /// `UID FETCH <uid> BODY.PEEK[]`: the whole raw message, without
    /// setting `\Seen`. `None` if the server returned no such message.
    pub async fn uid_fetch_body(&mut self, uid: u32) -> Result<Option<Vec<u8>>, ImapError> {
        let set = uid.to_string();
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("FETCH"),
                    Arg::Str(set.as_bytes()),
                    Arg::Raw("(UID BODY.PEEK[])"),
                ],
            )
            .await?;
        expect_ok(&done)?;
        Ok(done.untagged.into_iter().find_map(|u| match u {
            Untagged::Fetch(data) if data.uid == Some(uid) => data.body,
            _ => None,
        }))
    }

    /// `UID FETCH <uids> (UID RFC822.SIZE BODY.PEEK[]<0.n>)` for downloading
    /// bodies ahead of time (M4-25): whole raw messages without setting
    /// `\Seen`, at most [`PREFETCH_MAX_OCTETS`] each. The partial fetch
    /// (RFC 9051 §6.4.5, `<origin.octets>`) asks for one octet more than
    /// the limit, so a longer message shows up as too long instead of
    /// being cut off. Only complete bodies come back: one longer than the
    /// limit, or whose length is not the `RFC822.SIZE` the server sent with
    /// it, is left out, as are UIDs not asked for and repeats.
    pub async fn uid_fetch_complete_bodies(
        &mut self,
        uids: &[u32],
    ) -> Result<Vec<(u32, Vec<u8>)>, ImapError> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set = uid_set(uids);
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("FETCH"),
                    Arg::Str(set.as_bytes()),
                    Arg::Raw(PREFETCH_ITEMS),
                ],
            )
            .await?;
        expect_ok(&done)?;
        Ok(complete_bodies(done.untagged, uids, PREFETCH_MAX_OCTETS))
    }
}

/// The complete bodies among `untagged` for the asked-for `uids`, each UID
/// once: a body longer than `limit`, or whose length isn't the
/// `RFC822.SIZE` sent with it, was cut off and is left out.
fn complete_bodies(untagged: Vec<Untagged>, uids: &[u32], limit: u32) -> Vec<(u32, Vec<u8>)> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut out: Vec<(u32, Vec<u8>)> = Vec::new();
    for response in untagged {
        let Untagged::Fetch(data) = response else {
            continue;
        };
        let (Some(uid), Some(body)) = (data.uid, data.body) else {
            continue;
        };
        let complete = body.len() <= limit
            && data
                .size
                .is_none_or(|size| u64::try_from(body.len()).is_ok_and(|len| len == size));
        if complete && uids.contains(&uid) && !out.iter().any(|(u, _)| *u == uid) {
            out.push((uid, body));
        }
    }
    out
}

/// The largest message [`Session::uid_fetch_complete_bodies`] returns: 1 MiB.
pub const PREFETCH_MAX_OCTETS: u32 = 1_048_576;

/// The items for [`Session::uid_fetch_complete_bodies`]; the partial length
/// is `PREFETCH_MAX_OCTETS + 1` (an `Arg::Raw` has to be a fixed string).
const PREFETCH_ITEMS: &str = "(UID RFC822.SIZE BODY.PEEK[]<0.1048577>)";

/// `text` as a number if it is `min` to `max` ASCII digits, with no sign.
fn digits(text: &str, min: usize, max: usize) -> Option<i64> {
    if !(min..=max).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Days in `month` (1–12) of `year`, Gregorian.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Parses `INTERNALDATE` (`24-Nov-2026 10:46:51 +0000`, RFC 9051 §9
/// `date-time`) to Unix seconds, UTC. Anything that isn't a real date in
/// that form gives `None`, and the caller falls back to the `Date` header.
pub fn parse_internal_date(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once(' ')?;
    let (time, zone) = rest.split_once(' ')?;
    let mut d = date.split('-');
    // RFC 9051 §9: date-day-fixed = (SP DIGIT) / 2DIGIT; the space is
    // already trimmed.
    let day = digits(d.next()?, 1, 2)?;
    let month = match d.next()?.to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    // RFC 9051 §9 `date-year` is 4DIGIT, so a sign, a fifth digit or a
    // short year is a broken server. Bounding it before the arithmetic
    // below keeps every step in range, as `search::query::parse_date` in
    // the core does (#410).
    let year = digits(d.next()?, 4, 4)?;
    if d.next().is_some() || !(1..=9999).contains(&year) {
        return None;
    }
    // time = 2DIGIT ":" 2DIGIT ":" 2DIGIT
    let mut t = time.split(':');
    let (h, m, s) = (
        digits(t.next()?, 2, 2)?,
        digits(t.next()?, 2, 2)?,
        digits(t.next()?, 2, 2)?,
    );
    if t.next().is_some() {
        return None;
    }
    // zone = ("+" / "-") 4DIGIT, hours then minutes.
    let zone = zone.trim();
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hhmm = zone.get(1..)?;
    if hhmm.len() != 4 {
        return None;
    }
    let (zone_h, zone_m) = (digits(hhmm.get(..2)?, 2, 2)?, digits(hhmm.get(2..)?, 2, 2)?);
    if zone_h > 23 || zone_m > 59 {
        return None;
    }
    let offset = sign * (zone_h * 3600 + zone_m * 60);
    // Seconds may be 60 for a leap second.
    if !(1..=days_in_month(year, month)).contains(&day)
        || !(0..24).contains(&h)
        || !(0..60).contains(&m)
        || !(0..61).contains(&s)
    {
        return None;
    }
    // Days since 1970-01-01 (days-from-civil, Howard Hinnant). The year is
    // bounded above, but each step is checked anyway, so a wider bound
    // later can't bring the overflow back (#410).
    let y = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = y.div_euclid(400);
    let yoe = y.checked_sub(era.checked_mul(400)?)?;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe
        .checked_mul(365)?
        .checked_add(yoe / 4)?
        .checked_sub(yoe / 100)?
        .checked_add(doy)?;
    let days = era
        .checked_mul(146_097)?
        .checked_add(doe)?
        .checked_sub(719_468)?;
    days.checked_mul(86_400)?
        .checked_add(h * 3600 + m * 60 + s)?
        .checked_sub(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_date_parses() {
        assert_eq!(parse_internal_date("01-Jan-1970 00:00:00 +0000"), Some(0));
        assert_eq!(
            parse_internal_date("24-Nov-2026 10:46:51 +0000"),
            Some(1_795_517_211)
        );
        assert_eq!(parse_internal_date(" 1-Jan-1970 01:00:00 +0100"), Some(0));
        assert_eq!(parse_internal_date("garbage"), None);
        assert_eq!(parse_internal_date("32-Jan-2026 00:00:00 +0000"), None);
        assert_eq!(parse_internal_date("01-Foo-2026 00:00:00 +0000"), None);
    }

    /// #410: a server can send any year. Out-of-range years give `None`
    /// instead of overflowing (a panic in debug builds, a garbage date in
    /// release); years 1 and 9999 still parse.
    #[test]
    fn internal_date_with_an_out_of_range_year_is_none_not_a_panic() {
        for text in [
            "01-Jan-9223372036854775807 00:00:00 +0000",
            "01-Mar-99999999999999999 00:00:00 +0000",
            "01-Jan--9223372036854775808 00:00:00 +0000",
            "01-Mar--1 00:00:00 +0000",
            "01-Jan-0 00:00:00 +0000",
            "01-Jan-0000 00:00:00 +0000",
            "01-Jan-10000 00:00:00 +0000",
            "31-Dec-99999999999999 23:59:60 -2359",
        ] {
            assert_eq!(parse_internal_date(text), None, "{text}");
        }
        assert_eq!(
            parse_internal_date("01-Jan-0001 00:00:00 +0000"),
            Some(-62_135_596_800)
        );
        assert_eq!(
            parse_internal_date("31-Dec-9999 23:59:59 +0000"),
            Some(253_402_300_799)
        );
    }

    /// #415 review: dates that aren't real, or aren't in the RFC 9051 §9
    /// form, give `None` instead of a shifted date.
    #[test]
    fn internal_date_must_be_a_real_date_in_rfc_form() {
        for text in [
            // A day past the month's end used to roll into the next month.
            "31-Feb-2026 00:00:00 +0000",
            "29-Feb-2025 00:00:00 +0000",
            "31-Apr-2026 00:00:00 +0000",
            "29-Feb-1900 00:00:00 +0000",
            // Zone hours over 23 or minutes over 59 used to be applied.
            "01-Jan-2026 00:00:00 -9999",
            "01-Jan-2026 00:00:00 +0060",
            "01-Jan-2026 00:00:00 +2400",
            "01-Jan-2026 00:00:00 +00:0",
            // date-year is 4DIGIT; time fields are 2DIGIT.
            "01-Jan-+2026 00:00:00 +0000",
            "01-Jan-02026 00:00:00 +0000",
            "01-Jan-7 00:00:00 +0000",
            "01-Jan-2026 1:00:00 +0000",
            "01-Jan-2026 +1:00:00 +0000",
            "+1-Jan-2026 00:00:00 +0000",
            "001-Jan-2026 00:00:00 +0000",
            // Nothing may be left over.
            "01-Jan-2026-05 00:00:00 +0000",
            "01-Jan-2026 00:00:00:00 +0000",
            "01-Jan-2026 00:00:00 +0000 x",
        ] {
            assert_eq!(parse_internal_date(text), None, "{text}");
        }
        // Real dates at the edges still parse.
        assert_eq!(
            parse_internal_date("29-Feb-2024 00:00:00 +0000"),
            Some(1_709_164_800)
        );
        assert_eq!(
            parse_internal_date("29-Feb-2000 00:00:00 +0000"),
            Some(951_782_400)
        );
        assert_eq!(parse_internal_date("01-Jan-1970 23:59:00 +2358"), Some(60));
        assert_eq!(parse_internal_date("31-Dec-1969 23:59:60 +0000"), Some(0));
    }

    fn code(name: &str, args: Option<&str>) -> Untagged {
        Untagged::Condition {
            condition: crate::parser::Condition::Ok,
            text: crate::parser::RespText {
                code: Some(Code::Other {
                    name: name.into(),
                    args: args.map(Into::into),
                }),
                text: String::new(),
            },
        }
    }

    #[test]
    fn permanentflags_decides_custom_keywords() {
        let base = || vec![code("UIDVALIDITY", Some("7")), code("UIDNEXT", Some("9"))];
        let with = |flags: &str| {
            let mut u = base();
            u.push(code("PERMANENTFLAGS", Some(flags)));
            select_info(&u).expect("info").custom_keywords
        };
        assert!(with("(\\Seen \\Flagged \\*)"));
        assert!(with("(\\*)"));
        assert!(!with("(\\Seen \\Flagged \\Deleted)"));
        assert!(!with("()"));
        // Missing: every flag can be changed permanently (RFC 9051 §7.1).
        assert!(select_info(&base()).expect("info").custom_keywords);
    }

    #[test]
    fn only_complete_asked_for_bodies_are_kept() {
        let fetch = |uid: u32, size: Option<u64>, len: usize| {
            Untagged::Fetch(Box::new(FetchData {
                uid: Some(uid),
                size,
                body: Some(vec![b'x'; len]),
                ..FetchData::default()
            }))
        };
        let got = complete_bodies(
            vec![
                // Fine: at the limit, size matches.
                fetch(1, Some(10), 10),
                // Cut off by the partial fetch: one octet over the limit.
                fetch(2, Some(50), 11),
                // Shorter than its RFC822.SIZE: incomplete.
                fetch(3, Some(9), 5),
                // No RFC822.SIZE sent: judged by the limit only.
                fetch(4, None, 3),
                // Not asked for.
                fetch(9, Some(1), 1),
                // A repeat of uid 1.
                fetch(1, Some(2), 2),
                // No body at all.
                Untagged::Fetch(Box::new(FetchData {
                    uid: Some(5),
                    ..FetchData::default()
                })),
            ],
            &[1, 2, 3, 4, 5],
            10,
        );
        let uids: Vec<(u32, usize)> = got.iter().map(|(u, b)| (*u, b.len())).collect();
        assert_eq!(uids, [(1, 10), (4, 3)]);
    }

    #[test]
    fn prefetch_items_ask_for_one_octet_over_the_limit() {
        let wanted = format!("BODY.PEEK[]<0.{}>", PREFETCH_MAX_OCTETS + 1);
        assert!(PREFETCH_ITEMS.contains(&wanted));
    }

    #[test]
    fn uid_sets_are_comma_lists() {
        assert_eq!(uid_set(&[5, 3, 9]), "5,3,9");
    }
}
