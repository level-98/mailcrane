//! Feeds arbitrary bytes to the MIME parser. Any panic, hang or runaway
//! allocation is a finding; an `Err` is a correct answer to garbage.
#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = mailcrane_mime::parse_message(data);
});
