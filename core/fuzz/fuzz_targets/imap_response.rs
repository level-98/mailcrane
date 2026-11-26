//! Feeds arbitrary bytes to the IMAP response parser, then keeps parsing
//! whatever follows each complete response, the way a connection reads a
//! stream of responses. Any panic or hang is a finding.
//!
//! The `INTERNALDATE` of each FETCH, and the input itself as text, also go
//! through `parse_internal_date`, which the sync runs on every fetched
//! message (#410).
#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use mailcrane_imap::parser::{Response, Untagged};

fuzz_target!(|data: &[u8]| {
    let _ = mailcrane_imap::parse_internal_date(&String::from_utf8_lossy(data));
    let mut rest = data;
    while let Ok((response, used)) = mailcrane_imap::parse_response(rest) {
        if let Response::Untagged(Untagged::Fetch(fetch)) = &response {
            if let Some(date) = fetch.internal_date.as_deref() {
                let _ = mailcrane_imap::parse_internal_date(date);
            }
        }
        // A response that uses no bytes would loop forever; that is a bug
        // in itself, so stop rather than hide it behind a timeout.
        match rest.get(used..) {
            Some(next) if used > 0 => rest = next,
            _ => break,
        }
    }
});
