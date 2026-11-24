//! Feeds arbitrary text to the recipient list parser, whole and as typed.
//! Any panic or hang is a finding; invalid entries are a correct answer.
#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = mailcrane_mime::parse_address_list(&text);
    let _ = mailcrane_mime::split_address_list(&text, false);
});
