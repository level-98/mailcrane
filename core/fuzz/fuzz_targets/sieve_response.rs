//! Feeds arbitrary bytes to the ManageSieve response parser, then keeps
//! parsing whatever follows each complete response, the way a session reads
//! a stream of responses. Any panic or hang is a finding.
#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut rest = data;
    while let Ok((_response, used)) = mailcrane_sieve::parse_response(rest) {
        match rest.get(used..) {
            Some(next) if used > 0 => rest = next,
            _ => break,
        }
    }
});
