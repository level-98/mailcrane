//! Feeds arbitrary bytes to the IMAP response parser, then keeps parsing
//! whatever follows each complete response, the way a connection reads a
//! stream of responses. Any panic or hang is a finding.
#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut rest = data;
    while let Ok((_response, used)) = mailcrane_imap::parse_response(rest) {
        // A response that uses no bytes would loop forever; that is a bug
        // in itself, so stop rather than hide it behind a timeout.
        match rest.get(used..) {
            Some(next) if used > 0 => rest = next,
            _ => break,
        }
    }
});
