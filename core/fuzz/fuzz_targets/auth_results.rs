//! Feeds arbitrary text to the Authentication-Results parser and the
//! trust check (M5-07). Any panic or hang is a finding.
#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use mailcrane_mime::auth;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = auth::parse(&text);
    let _ = auth::summarize(&text, "example.net");
    let _ = auth::received_by(&text);
    let headers = [text.to_string()];
    let _ = auth::provider_results(&headers, Some("mx.example.org"));
    // The same text as each kind of trace header, in both orders.
    let (r, a) = (
        auth::TraceHeader::Received(text.to_string()),
        auth::TraceHeader::AuthenticationResults(text.to_string()),
    );
    let _ = auth::trusted_results(&[r.clone(), a.clone()]);
    let _ = auth::trusted_results(&[a, r]);
});
