//! Runs the HTML sanitiser on arbitrary input. The sanitiser only accepts HTML
//! that came out of the MIME parser, so the input is wrapped in a minimal
//! `text/html` message first. Any panic or hang is a finding.
#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use mailcrane_mime::{parse_message, sanitize_html, SanitizeOptions};

const HEADER: &[u8] = b"Content-Type: text/html; charset=utf-8\r\n\r\n";

fn cid_url(id: &str) -> String {
    format!("mailcrane-cid://fuzz/{id}")
}

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let mut raw = Vec::with_capacity(HEADER.len() + html.len());
    raw.extend_from_slice(HEADER);
    raw.extend_from_slice(html.as_bytes());
    let Ok(message) = parse_message(&raw) else {
        return;
    };
    let Some(html) = message.text_html.as_ref() else {
        return;
    };
    for allow_remote_content in [false, true] {
        let opts = SanitizeOptions {
            allow_remote_content,
            cid_url,
        };
        let _ = sanitize_html(html, &opts);
    }
});
