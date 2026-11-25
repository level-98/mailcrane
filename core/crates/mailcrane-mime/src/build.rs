//! Building a new plain-text message (RFC 5322, MIME RFC 2045–2047).
//!
//! [`build_message`] turns an [`OutgoingMessage`] into bytes ready for SMTP:
//! CRLF line endings, no line over 998 octets, folded headers, encoded words
//! for non-ASCII text, and never a `Bcc:` header. Header injection is not
//! possible: line breaks in the subject and names are replaced, and
//! addresses containing them are rejected.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::address::{check_addr_spec, AddressProblem};
use crate::model::Address;

/// Largest body accepted, in bytes. Quoted-printable can triple it, which
/// still fits in [`crate::limits::MAX_INPUT_BYTES`].
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Most recipients (To, Cc and Bcc together) in one message.
pub const MAX_RECIPIENTS: usize = 500;

/// RFC 5322 §2.1.1: lines SHOULD be at most 78 characters.
const FOLD_AT: usize = 78;
/// RFC 2045 §6.7 rule 5: encoded lines at most 76 characters.
const QP_LINE: usize = 76;
/// RFC 5322 §2.1.1: lines MUST be at most 998 characters.
const MAX_LINE: usize = 998;
/// Longest word kept as plain text in a header before switching to encoded
/// words, so a folded line can never pass [`MAX_LINE`].
const MAX_PLAIN_WORD: usize = 900;
/// Raw bytes per RFC 2047 encoded word: 45 bytes is 60 base64 chars, plus
/// `=?UTF-8?B?` and `?=` makes 72, under the limit of 75 (§2).
const WORD_BYTES: usize = 45;

/// How the body is encoded for transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransferEncoding {
    /// Quoted-printable (RFC 2045 §6.7): 7-bit safe, works with any server.
    #[default]
    QuotedPrintable,
    /// Raw UTF-8 (`8bit`). Only for servers that announce `8BITMIME`
    /// (RFC 6152). Falls back to quoted-printable when a line is too long
    /// for 8bit or the text contains NUL.
    EightBit,
}

/// A new message to build.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutgoingMessage {
    /// The sender. Its domain is used in the Message-ID.
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    /// Recipients that get the message but are never written into it.
    pub bcc: Vec<Address>,
    pub subject: String,
    /// The plain-text body. Any line endings; they become CRLF.
    pub body_text: String,
    pub encoding: TransferEncoding,
    /// The Message-ID this replies to, without angle brackets
    /// (RFC 5322 §3.6.4). Written as `In-Reply-To` if it is a valid id.
    pub in_reply_to: Option<String>,
    /// The thread's Message-IDs, oldest first, without angle brackets.
    /// Written as `References`; invalid ids are left out.
    pub references: Vec<String>,
    /// A fixed Message-ID (without angle brackets) instead of a random one,
    /// for a draft that keeps its id across versions. Ignored if invalid.
    pub message_id: Option<String>,
    /// A draft: may have no recipients yet.
    pub is_draft: bool,
}

/// Which address header a problem is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressField {
    From,
    To,
    Cc,
    Bcc,
}

/// Why a message could not be built. Carries no message content, so it is
/// safe to log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// An address is not valid; `index` counts from 0 within `field`.
    #[error("invalid address in {field:?} at {index}: {problem:?}")]
    InvalidAddress {
        field: AddressField,
        index: usize,
        problem: AddressProblem,
    },
    /// To, Cc and Bcc are all empty.
    #[error("no recipients")]
    NoRecipients,
    /// More than [`MAX_RECIPIENTS`] recipients.
    #[error("more than {MAX_RECIPIENTS} recipients")]
    TooManyRecipients,
    /// The body is larger than [`MAX_BODY_BYTES`].
    #[error("body is larger than {MAX_BODY_BYTES} bytes")]
    TooLarge,
    /// The system's random number source failed.
    #[error("no random numbers available")]
    Random,
}

/// Builds the message, with the current time as `Date` and a new random
/// `Message-ID` at the sender's domain.
pub fn build_message(draft: &OutgoingMessage) -> Result<Vec<u8>, BuildError> {
    let date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| BuildError::Random)?;
    build_with(draft, date, &random)
}

/// [`build_message`] with a fixed date (Unix seconds, UTC) and Message-ID
/// randomness, for tests.
pub(crate) fn build_with(
    draft: &OutgoingMessage,
    date: i64,
    random: &[u8; 16],
) -> Result<Vec<u8>, BuildError> {
    check_addresses(draft)?;
    if draft.body_text.len() > MAX_BODY_BYTES {
        return Err(BuildError::TooLarge);
    }
    let domain = draft
        .from
        .email
        .rsplit_once('@')
        .map(|(_, d)| d)
        .unwrap_or_default();
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();

    let mut out = Vec::with_capacity(draft.body_text.len() + 1024);
    header(&mut out, "Date", &[format_date(date)]);
    header(
        &mut out,
        "From",
        &address_units(std::slice::from_ref(&draft.from)),
    );
    if !draft.to.is_empty() {
        header(&mut out, "To", &address_units(&draft.to));
    }
    if !draft.cc.is_empty() {
        header(&mut out, "Cc", &address_units(&draft.cc));
    }
    // Bcc is deliberately never written (RFC 5322 §3.6.3 allows leaving it out).
    header(&mut out, "Subject", &unstructured_units(&draft.subject));
    let message_id = match draft.message_id.as_deref().filter(|i| is_msg_id(i)) {
        Some(fixed) => format!("<{fixed}>"),
        None => format!("<{id}@{domain}>"),
    };
    header(&mut out, "Message-ID", &[message_id]);
    if let Some(parent) = draft.in_reply_to.as_deref().filter(|i| is_msg_id(i)) {
        header(&mut out, "In-Reply-To", &[format!("<{parent}>")]);
    }
    let references: Vec<String> = draft
        .references
        .iter()
        .filter(|i| is_msg_id(i))
        .map(|i| format!("<{i}>"))
        .collect();
    if !references.is_empty() {
        header(&mut out, "References", &references);
    }
    header(&mut out, "MIME-Version", &["1.0".to_owned()]);
    header(
        &mut out,
        "Content-Type",
        &["text/plain;".to_owned(), "charset=utf-8".to_owned()],
    );

    let lines = body_lines(&draft.body_text);
    let eight_bit = draft.encoding == TransferEncoding::EightBit
        && lines
            .iter()
            .all(|l| l.len() <= MAX_LINE && !l.contains('\0'));
    let cte = if eight_bit {
        "8bit"
    } else {
        "quoted-printable"
    };
    header(&mut out, "Content-Transfer-Encoding", &[cte.to_owned()]);
    out.extend_from_slice(b"\r\n");
    for line in &lines {
        if eight_bit {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        } else {
            quoted_printable_line(&mut out, line.as_bytes());
        }
    }
    Ok(out)
}

fn check_addresses(draft: &OutgoingMessage) -> Result<(), BuildError> {
    let fields = [
        (AddressField::From, std::slice::from_ref(&draft.from)),
        (AddressField::To, draft.to.as_slice()),
        (AddressField::Cc, draft.cc.as_slice()),
        (AddressField::Bcc, draft.bcc.as_slice()),
    ];
    for (field, list) in fields {
        for (index, a) in list.iter().enumerate() {
            check_addr_spec(&a.email).map_err(|problem| BuildError::InvalidAddress {
                field,
                index,
                problem,
            })?;
        }
    }
    let count = draft
        .to
        .len()
        .saturating_add(draft.cc.len())
        .saturating_add(draft.bcc.len());
    if count == 0 && !draft.is_draft {
        return Err(BuildError::NoRecipients);
    }
    if count > MAX_RECIPIENTS {
        return Err(BuildError::TooManyRecipients);
    }
    Ok(())
}

/// Writes `Name: unit unit …`, folding before a unit (RFC 5322 §2.2.3) when
/// the line would pass 78 characters. Units contain no CR or LF.
fn header(out: &mut Vec<u8>, name: &str, units: &[String]) {
    out.extend_from_slice(name.as_bytes());
    out.push(b':');
    let mut line_len = name.len() + 1;
    for unit in units {
        // A fold before an empty unit would leave a line of only spaces.
        if !unit.is_empty() && line_len + 1 + unit.len() > FOLD_AT && line_len > name.len() + 1 {
            out.extend_from_slice(b"\r\n");
            line_len = 0;
        }
        out.push(b' ');
        out.extend_from_slice(unit.as_bytes());
        line_len += 1 + unit.len();
    }
    out.extend_from_slice(b"\r\n");
}

/// Longest Message-ID written into `In-Reply-To` or `References`. Real ids are
/// far shorter; this keeps a hostile one from making a header line too long.
const MAX_MSG_ID: usize = 250;

/// Whether `id` (without angle brackets) can be written as a `msg-id`
/// (RFC 5322 §3.6.4): printable ASCII, no spaces, no `<` or `>`, and an `@`.
/// Ids from received mail are hostile input; anything else is left out
/// rather than repaired, so it can't break or inject headers.
fn is_msg_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_MSG_ID
        && id.contains('@')
        && id
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'<' && b != b'>')
}

/// Replaces line breaks and tabs with spaces and drops other control
/// characters, so the text fits on one header line (AGENTS.md §4.6).
fn one_line(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\r' | '\n' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Whether text can go into a header as it is, rather than as encoded words.
fn plain_ok(text: &str) -> bool {
    text.is_ascii() && !text.contains("=?") && text.split(' ').all(|w| w.len() <= MAX_PLAIN_WORD)
}

/// Units for an unstructured header such as Subject (RFC 5322 §3.2.5).
fn unstructured_units(text: &str) -> Vec<String> {
    let text = one_line(text);
    if plain_ok(&text) {
        // Splitting on single spaces keeps runs of spaces when rejoined.
        text.split(' ').map(str::to_owned).collect()
    } else {
        encoded_words(&text)
    }
}

/// RFC 2047 encoded words, base64, each under 75 characters, never
/// splitting a character. Whitespace between them is ignored when decoding
/// (§6.2), so they can be folded freely.
fn encoded_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in text.chars() {
        if chunk.len() + c.len_utf8() > WORD_BYTES {
            words.push(format!("=?UTF-8?B?{}?=", base64(chunk.as_bytes())));
            chunk.clear();
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(format!("=?UTF-8?B?{}?=", base64(chunk.as_bytes())));
    }
    words
}

/// Units for a list of addresses: `name <addr>,` each, the last without a
/// comma.
fn address_units(list: &[Address]) -> Vec<String> {
    let mut units = Vec::new();
    for (i, a) in list.iter().enumerate() {
        let name = a.name.as_deref().map(one_line).unwrap_or_default();
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        let last = i + 1 == list.len();
        let comma = if last { "" } else { "," };
        if name.is_empty() {
            units.push(format!("{}{comma}", a.email));
            continue;
        }
        // A quoted string is one unit, so it must fit on a line by itself.
        if !plain_ok(&name) || name.len() > MAX_PLAIN_WORD {
            units.extend(encoded_words(&name));
        } else if name.split(' ').all(|w| w.chars().all(is_atext)) {
            units.extend(name.split(' ').map(str::to_owned));
        } else {
            // RFC 5322 §3.2.4 quoted-string.
            let escaped: String = name
                .chars()
                .flat_map(|c| match c {
                    '"' | '\\' => vec!['\\', c],
                    c => vec![c],
                })
                .collect();
            units.push(format!("\"{escaped}\""));
        }
        units.push(format!("<{}>{comma}", a.email));
    }
    units
}

fn is_atext(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c)
}

/// The body split into lines at CRLF, CR or LF, without the endings.
fn body_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(['\r', '\n']) {
        lines.push(rest.get(..pos).unwrap_or_default());
        let skip = if rest.get(pos..pos + 2) == Some("\r\n") {
            2
        } else {
            1
        };
        rest = rest.get(pos + skip..).unwrap_or_default();
    }
    if !rest.is_empty() || lines.is_empty() {
        lines.push(rest);
    }
    lines
}

/// One line of quoted-printable (RFC 2045 §6.7), with soft line breaks to
/// keep each output line within 76 characters, ending in CRLF.
fn quoted_printable_line(out: &mut Vec<u8>, line: &[u8]) {
    let mut len = 0;
    for (i, &b) in line.iter().enumerate() {
        let last = i + 1 == line.len();
        // Rule 3: whitespace at the end of a line must be encoded.
        let literal = matches!(b, 33..=60 | 62..=126) || (matches!(b, b' ' | b'\t') && !last);
        let width = if literal { 1 } else { 3 };
        // Leave room for the `=` of a soft break, unless this is the end.
        let limit = if last { QP_LINE } else { QP_LINE - 1 };
        if len + width > limit {
            out.extend_from_slice(b"=\r\n");
            len = 0;
        }
        if literal {
            out.push(b);
        } else {
            out.extend_from_slice(format!("={b:02X}").as_bytes());
        }
        len += width;
    }
    out.extend_from_slice(b"\r\n");
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let sym = |i: u32| char::from(TABLE[(i & 63) as usize]);
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(sym(n >> 18));
        out.push(sym(n >> 12));
        out.push(if chunk.len() > 1 { sym(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(n) } else { '=' });
    }
    out
}

/// A new random Message-ID for `domain`, without angle brackets: 32 hex
/// characters, `@`, the domain. For drafts, which keep one id across
/// versions.
pub fn new_message_id(domain: &str) -> Result<String, BuildError> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| BuildError::Random)?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let domain: String = domain
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
        .collect();
    let domain = if domain.is_empty() {
        "localhost".to_owned()
    } else {
        domain
    };
    Ok(format!("{id}@{domain}"))
}

/// RFC 5322 §3.3 date in UTC, such as `Tue, 24 Nov 2026 21:32:00 +0000`.
pub fn format_date(secs: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Clamp to years 1970–9999 so the arithmetic can't overflow.
    let secs = secs.clamp(0, 253_402_300_799);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let weekday = DAYS
        .get(usize::try_from(days.rem_euclid(7)).unwrap_or(0))
        .copied()
        .unwrap_or("Thu");
    let month = MONTHS
        .get(usize::try_from(m - 1).unwrap_or(0))
        .copied()
        .unwrap_or("Jan");
    format!(
        "{weekday}, {d} {month} {y} {:02}:{:02}:{:02} +0000",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
