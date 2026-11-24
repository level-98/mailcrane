//! Selecting a folder and fetching headers and bodies (RFC 9051 §6.3.2,
//! §6.4.4 and §6.4.5).

use crate::command::Arg;
use crate::error::ImapError;
use crate::parser::{Code, FetchData, Untagged};
use crate::session::{expect_ok, Session};

/// What `SELECT` told us about a folder (RFC 9051 §6.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    BODY.PEEK[HEADER.FIELDS (REFERENCES IN-REPLY-TO LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST)])";

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
    Ok(SelectInfo {
        exists,
        uidvalidity,
        uidnext,
        highestmodseq,
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
        select_info(&done.untagged)
    }

    /// `UID SEARCH UID <low>:<high>` in the selected folder (RFC 9051
    /// §6.4.4): the UIDs in that range, ascending.
    pub async fn uid_search_range(&mut self, low: u32, high: u32) -> Result<Vec<u32>, ImapError> {
        if low == 0 || low > high {
            return Ok(Vec::new());
        }
        let range = format!("{low}:{high}");
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("SEARCH"),
                    Arg::Raw("UID"),
                    Arg::Str(range.as_bytes()),
                ],
            )
            .await?;
        expect_ok(&done)?;
        let mut uids: Vec<u32> = done
            .untagged
            .iter()
            .filter_map(|u| match u {
                Untagged::Search(list) => Some(list.iter().copied()),
                _ => None,
            })
            .flatten()
            // A server may answer outside the range we asked for.
            .filter(|uid| (low..=high).contains(uid))
            .collect();
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
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
}

/// Parses `INTERNALDATE` (`24-Nov-2026 10:46:51 +0000`, RFC 9051 §9
/// `date-time`) to Unix seconds, UTC.
pub fn parse_internal_date(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once(' ')?;
    let (time, zone) = rest.split_once(' ')?;
    let mut d = date.split('-');
    let day: i64 = d.next()?.trim().parse().ok()?;
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
    let year: i64 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let (h, m, s): (i64, i64, i64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    let zone = zone.trim();
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = zone.get(1..)?;
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let offset = sign
        * (digits.get(..2)?.parse::<i64>().ok()? * 3600
            + digits.get(2..)?.parse::<i64>().ok()? * 60);
    if !(1..=31).contains(&day)
        || !(0..24).contains(&h)
        || !(0..60).contains(&m)
        || !(0..61).contains(&s)
    {
        return None;
    }
    // Days since 1970-01-01 (civil-from-days, Howard Hinnant).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
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

    #[test]
    fn uid_sets_are_comma_lists() {
        assert_eq!(uid_set(&[5, 3, 9]), "5,3,9");
    }
}
