//! Parsing the recipient lists people type: `Name <a@b>`, bare `a@b`,
//! `"Quoted, Name" <a@b>`, separated by commas or semicolons.
//!
//! This is deliberately stricter than RFC 5322 §3.4: it accepts what a
//! person types into a To field and what an SMTP server will take, and
//! reports everything else per entry so the apps can mark it. Groups,
//! comments, quoted local parts and domain literals are not accepted.

use crate::model::Address;

/// Longest input accepted by [`parse_address_list`], in bytes.
pub const MAX_LIST_BYTES: usize = 64 * 1024;

/// Most entries in one list.
pub const MAX_ENTRIES: usize = 500;

/// Longest address (RFC 5321 §4.5.3.1.3 path minus the angle brackets).
const MAX_ADDRESS: usize = 254;
/// RFC 5321 §4.5.3.1.1.
const MAX_LOCAL: usize = 64;
/// RFC 1035 §2.3.4.
const MAX_LABEL: usize = 63;
/// Longest display name kept, in chars.
const MAX_NAME_CHARS: usize = 256;
/// Longest entry text kept in an [`InvalidAddress`], in chars.
const MAX_ECHO_CHARS: usize = 320;

/// What is wrong with one entry of a recipient list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressProblem {
    /// There is no `@`, or more than one outside quotes.
    MissingAt,
    /// The part before `@` is empty or has characters an address can't have.
    InvalidLocalPart,
    /// The part after `@` is not a valid domain name.
    InvalidDomain,
    /// A line break or other control character. Rejected rather than
    /// stripped, because it could start a new header line.
    ControlCharacter,
    /// The address has characters outside ASCII. Sending to those needs
    /// SMTPUTF8 (RFC 6531), which isn't supported yet.
    NonAscii,
    /// A quote or `<` is never closed, or there is text after `>`.
    Malformed,
    /// The address, or the whole list, is too long.
    TooLong,
    /// The list has more than [`MAX_ENTRIES`] entries.
    TooMany,
}

/// One entry of a list that is not a usable address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidAddress {
    /// Position of the entry in the list, counting from 0. Empty entries
    /// (such as after a trailing comma) are not counted.
    pub index: usize,
    /// The entry as typed, trimmed, with control characters removed and cut
    /// to 320 chars, so it can be shown back to the user.
    pub input: String,
    /// What is wrong with it.
    pub problem: AddressProblem,
}

/// One entry of a list, parsed or not, with the text it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressEntry {
    /// The entry as typed, trimmed. Control characters are kept here so the
    /// caller can see exactly what was typed; never put this in a header.
    pub text: String,
    /// The address, or what is wrong with it.
    pub result: Result<Address, AddressProblem>,
}

/// Entries of a list and any text after the last separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitList {
    /// Entries that ended with a separator, or every entry when the caller
    /// asked for the whole input.
    pub entries: Vec<AddressEntry>,
    /// Text after the last separator, when the caller did not ask for the
    /// whole input. Empty otherwise.
    pub rest: String,
}

/// Parses a recipient list. Returns every address, or every invalid entry
/// (with its position) if there are any. Empty entries are skipped, so
/// `"a@example.org,"` is one address.
pub fn parse_address_list(input: &str) -> Result<Vec<Address>, Vec<InvalidAddress>> {
    if input.len() > MAX_LIST_BYTES {
        return Err(vec![InvalidAddress {
            index: 0,
            input: echo(input),
            problem: AddressProblem::TooLong,
        }]);
    }
    let entries = split_address_list(input, true).entries;
    let mut good = Vec::with_capacity(entries.len());
    let mut bad = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let problem = if index >= MAX_ENTRIES {
            Err(AddressProblem::TooMany)
        } else {
            entry.result
        };
        match problem {
            Ok(address) => good.push(address),
            Err(problem) => bad.push(InvalidAddress {
                index,
                input: echo(&entry.text),
                problem,
            }),
        }
    }
    if bad.is_empty() {
        Ok(good)
    } else {
        Err(bad)
    }
}

/// Splits `input` at commas and semicolons that are outside quotes and
/// angle brackets, and parses each entry.
///
/// With `whole` set, the text after the last separator is an entry too.
/// Without it, that text is returned in [`SplitList::rest`], for a field
/// that turns finished entries into chips as the user types.
///
/// Input over [`MAX_LIST_BYTES`] gives one entry with
/// [`AddressProblem::TooLong`].
pub fn split_address_list(input: &str, whole: bool) -> SplitList {
    if input.len() > MAX_LIST_BYTES {
        return SplitList {
            entries: vec![AddressEntry {
                text: echo(input),
                result: Err(AddressProblem::TooLong),
            }],
            rest: String::new(),
        };
    }
    let mut entries = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    let mut in_angle = false;
    let mut escaped = false;
    for (i, c) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quote => escaped = true,
            '"' if !in_angle => in_quote = !in_quote,
            '<' if !in_quote => in_angle = true,
            '>' if !in_quote => in_angle = false,
            ',' | ';' if !in_quote && !in_angle => {
                push_entry(&mut entries, input.get(start..i).unwrap_or_default());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let tail = input.get(start..).unwrap_or_default();
    if whole {
        push_entry(&mut entries, tail);
        SplitList {
            entries,
            rest: String::new(),
        }
    } else {
        SplitList {
            entries,
            rest: tail.trim_start().to_owned(),
        }
    }
}

fn push_entry(entries: &mut Vec<AddressEntry>, raw: &str) {
    let text = raw.trim();
    if text.is_empty() {
        return;
    }
    entries.push(AddressEntry {
        text: text.to_owned(),
        result: parse_entry(text),
    });
}

/// Parses one trimmed, non-empty entry.
fn parse_entry(text: &str) -> Result<Address, AddressProblem> {
    // Checked first, so a line break is always reported as such and never
    // reaches a header (AGENTS.md §4.6). A tab is only spacing.
    if text.chars().any(|c| c.is_control() && c != '\t') {
        return Err(AddressProblem::ControlCharacter);
    }
    let (name, address) = match text.rfind('<') {
        Some(open) if !inside_quotes(text, open) => {
            let after = text.get(open + 1..).unwrap_or_default();
            let close = after.find('>').ok_or(AddressProblem::Malformed)?;
            if !after.get(close + 1..).unwrap_or_default().trim().is_empty() {
                return Err(AddressProblem::Malformed);
            }
            let name = parse_name(text.get(..open).unwrap_or_default())?;
            (name, after.get(..close).unwrap_or_default().trim())
        }
        _ => {
            if text.contains('"') || text.contains('>') {
                return Err(AddressProblem::Malformed);
            }
            (None, text)
        }
    };
    check_addr_spec(address)?;
    Ok(Address {
        name,
        email: address.to_owned(),
    })
}

/// Whether byte `pos` of `text` is inside a quoted string.
fn inside_quotes(text: &str, pos: usize) -> bool {
    let mut in_quote = false;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if i >= pos {
            break;
        }
        if escaped {
            escaped = false;
        } else if c == '\\' && in_quote {
            escaped = true;
        } else if c == '"' {
            in_quote = !in_quote;
        }
    }
    in_quote
}

/// The display name before `<`: a quoted string, or plain words. Returns
/// `None` for an empty name.
fn parse_name(raw: &str) -> Result<Option<String>, AddressProblem> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let name = if let Some(inner) = raw.strip_prefix('"') {
        let mut out = String::new();
        let mut escaped = false;
        let mut closed_at = None;
        for (i, c) in inner.char_indices() {
            if escaped {
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                closed_at = Some(i);
                break;
            } else {
                out.push(c);
            }
        }
        let close = closed_at.ok_or(AddressProblem::Malformed)?;
        if !inner.get(close + 1..).unwrap_or_default().trim().is_empty() {
            return Err(AddressProblem::Malformed);
        }
        out
    } else {
        if raw.contains('"') {
            return Err(AddressProblem::Malformed);
        }
        raw.to_owned()
    };
    let name: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    Ok(Some(name).filter(|n| !n.is_empty()))
}

/// RFC 5322 §3.2.3 `atext`.
fn is_atext(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c)
}

/// Checks `local@domain`: a dot-atom local part (RFC 5322 §3.4.1) and a
/// host name (RFC 1035 §2.3.1, RFC 5321 §4.1.2), within the RFC 5321 size
/// limits. Used both for typed input and before building a message.
pub(crate) fn check_addr_spec(address: &str) -> Result<(), AddressProblem> {
    if address.chars().any(|c| c.is_control()) {
        return Err(AddressProblem::ControlCharacter);
    }
    if !address.is_ascii() {
        return Err(AddressProblem::NonAscii);
    }
    if address.len() > MAX_ADDRESS {
        return Err(AddressProblem::TooLong);
    }
    let mut parts = address.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(AddressProblem::MissingAt);
    };
    let local_ok = !local.is_empty()
        && local.len() <= MAX_LOCAL
        && local
            .split('.')
            .all(|atom| !atom.is_empty() && atom.chars().all(is_atext));
    if !local_ok {
        return Err(AddressProblem::InvalidLocalPart);
    }
    let domain_ok = !domain.is_empty()
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= MAX_LABEL
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !domain_ok {
        return Err(AddressProblem::InvalidDomain);
    }
    Ok(())
}

fn echo(text: &str) -> String {
    text.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_ECHO_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    #[test]
    fn accepts_the_usual_forms() {
        let cases: &[(&str, Vec<Address>)] = &[
            ("ann@example.org", vec![addr(None, "ann@example.org")]),
            ("  ann@example.org  ", vec![addr(None, "ann@example.org")]),
            ("<ann@example.org>", vec![addr(None, "ann@example.org")]),
            (
                "Ann Example <ann@example.org>",
                vec![addr(Some("Ann Example"), "ann@example.org")],
            ),
            (
                "\"Example, Ann\" <ann@example.org>",
                vec![addr(Some("Example, Ann"), "ann@example.org")],
            ),
            (
                "\"Ann \\\"The Ace\\\" E.\" <ann@example.org>",
                vec![addr(Some("Ann \"The Ace\" E."), "ann@example.org")],
            ),
            (
                "Zoë Ångström <zoe@example.org>",
                vec![addr(Some("Zoë Ångström"), "zoe@example.org")],
            ),
            (
                "a@example.org, b@example.net; Cé <c@example.com>",
                vec![
                    addr(None, "a@example.org"),
                    addr(None, "b@example.net"),
                    addr(Some("Cé"), "c@example.com"),
                ],
            ),
            (
                "a@example.org,,; ,b@example.org,",
                vec![addr(None, "a@example.org"), addr(None, "b@example.org")],
            ),
            ("", vec![]),
            (
                "first.last+tag@mail.example.org",
                vec![addr(None, "first.last+tag@mail.example.org")],
            ),
            (
                "Ann   Example\t<ann@example.org>",
                vec![addr(Some("Ann Example"), "ann@example.org")],
            ),
            (
                "\"\" <ann@example.org>",
                vec![addr(None, "ann@example.org")],
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_address_list(input).as_ref(),
                Ok(expected),
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn reports_each_invalid_entry() {
        use AddressProblem::*;
        let cases: &[(&str, AddressProblem)] = &[
            ("ann", MissingAt),
            ("ann@b@example.org", MissingAt),
            ("@example.org", InvalidLocalPart),
            ("ann.@example.org", InvalidLocalPart),
            ("a..b@example.org", InvalidLocalPart),
            ("a b@example.org", InvalidLocalPart),
            ("ann@", InvalidDomain),
            ("ann@example..org", InvalidDomain),
            ("ann@-example.org", InvalidDomain),
            ("ann@exa_mple.org", InvalidDomain),
            ("ann@example.org.", InvalidDomain),
            ("zoë@example.org", NonAscii),
            ("ann@exämple.org", NonAscii),
            ("Ann <ann@example.org", Malformed),
            ("Ann <ann@example.org> extra", Malformed),
            ("\"Ann <ann@example.org>", Malformed),
            ("\"Ann\" Bee <ann@example.org>", Malformed),
            ("ann@example.org>", Malformed),
            (
                "Ann\r\nBcc: x@example.org <ann@example.org>",
                ControlCharacter,
            ),
            ("ann@example.org\r\nBcc: x@example.org", ControlCharacter),
            ("Ann\0 <ann@example.org>", ControlCharacter),
        ];
        for (input, problem) in cases {
            let got = parse_address_list(input);
            assert_eq!(
                got,
                Err(vec![InvalidAddress {
                    index: 0,
                    input: echo(input),
                    problem: *problem
                }]),
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn reports_positions_of_bad_entries_among_good_ones() {
        let got = parse_address_list("a@example.org, nope, b@example.org; x@");
        assert_eq!(
            got,
            Err(vec![
                InvalidAddress {
                    index: 1,
                    input: "nope".into(),
                    problem: AddressProblem::MissingAt
                },
                InvalidAddress {
                    index: 3,
                    input: "x@".into(),
                    problem: AddressProblem::InvalidDomain
                },
            ])
        );
    }

    #[test]
    fn limits_are_enforced() {
        let long_local = format!("{}@example.org", "a".repeat(65));
        assert_eq!(
            parse_address_list(&long_local).map_err(|e| e[0].problem),
            Err(AddressProblem::InvalidLocalPart)
        );
        let long = format!("a@{}.org", "b.".repeat(130));
        assert_eq!(
            parse_address_list(&long).map_err(|e| e[0].problem),
            Err(AddressProblem::TooLong)
        );
        let huge = "a@example.org,".repeat(MAX_LIST_BYTES / 10);
        assert_eq!(
            parse_address_list(&huge).map_err(|e| e[0].problem),
            Err(AddressProblem::TooLong)
        );
        let many = "a@example.org,".repeat(MAX_ENTRIES + 2);
        let errors = parse_address_list(&many).err().unwrap_or_default();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|e| e.problem == AddressProblem::TooMany));
        assert_eq!(errors[0].index, MAX_ENTRIES);
    }

    #[test]
    fn split_keeps_unfinished_text() {
        let split = split_address_list("a@example.org, \"Doe, J", false);
        assert_eq!(split.entries.len(), 1);
        assert_eq!(split.rest, "\"Doe, J");

        let split = split_address_list("\"Doe, J\" <j@example.org>; b", false);
        assert_eq!(
            split.entries[0].result,
            Ok(addr(Some("Doe, J"), "j@example.org"))
        );
        assert_eq!(split.rest, "b");

        let split = split_address_list("nope,", false);
        assert_eq!(split.entries[0].text, "nope");
        assert_eq!(split.entries[0].result, Err(AddressProblem::MissingAt));
        assert_eq!(split.rest, "");
    }

    #[test]
    fn echo_never_contains_line_breaks() {
        let errors = parse_address_list("x\r\ny").err().unwrap_or_default();
        assert_eq!(errors[0].input, "xy");
    }

    mod prop {
        use super::super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn never_panics_and_valid_output_is_clean(s in "\\PC{0,200}|[a-z@<>\",; \r\n\\\\]{0,60}") {
                if let Ok(list) = parse_address_list(&s) {
                    for a in list {
                        prop_assert!(check_addr_spec(&a.email).is_ok());
                        let name = a.name.unwrap_or_default();
                        prop_assert!(!name.chars().any(|c| c.is_control()));
                    }
                }
                let _ = split_address_list(&s, false);
            }
        }
    }
}
