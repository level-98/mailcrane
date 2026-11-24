//! Cleaning of header values and address lists.

use crate::model::Address;
use mail_parser::AddressList;

/// Removes CR, LF and NUL, so a value can't smuggle in a new header line
/// (AGENTS.md §4.6).
pub fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

/// [`clean`], and `None` if nothing is left.
pub fn clean_opt(value: Option<&str>) -> Option<String> {
    value.map(clean).filter(|v| !v.trim().is_empty())
}

/// Cleaned list of ids (Message-ID style).
pub fn clean_list<'a>(items: impl Iterator<Item = &'a str>) -> Vec<String> {
    items.map(clean).filter(|v| !v.trim().is_empty()).collect()
}

/// Flattens an address list (groups included) into cleaned addresses.
/// Entries without an address are dropped.
pub fn addresses(list: Option<AddressList<'_>>) -> Vec<Address> {
    let Some(list) = list else {
        return Vec::new();
    };
    list.mailboxes()
        .filter_map(|mailbox| {
            let email = clean(mailbox.address()?);
            if email.trim().is_empty() {
                return None;
            }
            Some(Address {
                name: clean_opt(mailbox.name()),
                email,
            })
        })
        .collect()
}

/// A raw header value as cleaned text.
pub fn raw_value(bytes: &[u8]) -> Option<String> {
    clean_opt(Some(&String::from_utf8_lossy(bytes))).map(|v| v.trim().to_owned())
}
