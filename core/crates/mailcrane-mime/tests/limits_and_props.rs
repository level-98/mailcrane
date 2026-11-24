//! Limits and property tests.

use mailcrane_mime::{limits, parse_message, MimeError};
use proptest::prelude::*;
use std::time::{Duration, Instant};

fn nested_multipart(depth: usize) -> Vec<u8> {
    let mut s = String::from("Subject: deep\r\n");
    for i in 0..depth {
        s.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=\"b{i}\"\r\n\r\n--b{i}\r\n"
        ));
    }
    s.push_str("Content-Type: text/plain\r\n\r\nleaf\r\n");
    for i in (0..depth).rev() {
        s.push_str(&format!("--b{i}--\r\n"));
    }
    s.into_bytes()
}

fn many_parts(n: usize) -> Vec<u8> {
    let mut s =
        String::from("Subject: wide\r\nContent-Type: multipart/mixed; boundary=\"w\"\r\n\r\n");
    for i in 0..n {
        s.push_str(&format!(
            "--w\r\nContent-Type: text/plain\r\n\r\npart {i}\r\n"
        ));
    }
    s.push_str("--w--\r\n");
    s.into_bytes()
}

#[test]
fn depth_at_limit_is_accepted() {
    assert!(parse_message(&nested_multipart(limits::MAX_DEPTH)).is_ok());
}

#[test]
fn depth_over_limit_is_too_deep() {
    assert_eq!(
        parse_message(&nested_multipart(limits::MAX_DEPTH + 1)),
        Err(MimeError::TooDeep)
    );
    assert_eq!(
        parse_message(&nested_multipart(500)),
        Err(MimeError::TooDeep)
    );
}

#[test]
fn nested_messages_over_limit_are_too_deep() {
    let mut s = String::new();
    for _ in 0..40 {
        s.push_str("Content-Type: message/rfc822\r\n\r\n");
    }
    s.push_str("Subject: x\r\n\r\nbody\r\n");
    assert_eq!(parse_message(s.as_bytes()), Err(MimeError::TooDeep));
}

#[test]
fn parts_at_limit_are_accepted() {
    // 999 leaves + the multipart container = 1000 parts.
    assert!(parse_message(&many_parts(limits::MAX_PARTS - 1)).is_ok());
}

#[test]
fn parts_over_limit_are_too_many() {
    assert_eq!(
        parse_message(&many_parts(limits::MAX_PARTS)),
        Err(MimeError::TooManyParts)
    );
    assert_eq!(
        parse_message(&many_parts(5000)),
        Err(MimeError::TooManyParts)
    );
}

#[test]
fn input_over_limit_is_too_large() {
    let big = vec![b'a'; limits::MAX_INPUT_BYTES + 1];
    assert_eq!(parse_message(&big), Err(MimeError::TooLarge));
}

#[test]
fn empty_input_is_an_error_not_a_panic() {
    assert!(parse_message(b"").is_err());
}

fn no_bad_chars(s: &str) -> bool {
    !s.contains(['\r', '\n', '\0'])
}

fn all_header_values_clean(m: &mailcrane_mime::ParsedMessage) -> bool {
    let addrs = [&m.from, &m.sender, &m.reply_to, &m.to, &m.cc, &m.bcc];
    addrs.iter().all(|list| {
        list.iter()
            .all(|a| no_bad_chars(&a.email) && a.name.as_deref().is_none_or(no_bad_chars))
    }) && m.subject.as_deref().is_none_or(no_bad_chars)
        && m.message_id.as_deref().is_none_or(no_bad_chars)
        && m.in_reply_to.iter().all(|s| no_bad_chars(s))
        && m.references.iter().all(|s| no_bad_chars(s))
        && m.list_unsubscribe.as_deref().is_none_or(no_bad_chars)
        && m.list_unsubscribe_post.as_deref().is_none_or(no_bad_chars)
        && m.attachments.iter().all(|a| {
            no_bad_chars(&a.mime_type)
                && a.filename.as_deref().is_none_or(no_bad_chars)
                && a.content_id.as_deref().is_none_or(no_bad_chars)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_bytes_never_panic_and_are_fast(bytes in proptest::collection::vec(any::<u8>(), 0..65536)) {
        let start = Instant::now();
        let _ = parse_message(&bytes);
        prop_assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn arbitrary_header_bytes_never_leak_cr_lf_nul(
        subject in proptest::collection::vec(any::<u8>(), 0..300),
        from in proptest::collection::vec(any::<u8>(), 0..300),
        other in proptest::collection::vec(any::<u8>(), 0..300),
    ) {
        let mut msg = Vec::new();
        for (name, value) in [
            ("Subject", &subject), ("From", &from), ("To", &other), ("Cc", &other),
            ("Reply-To", &from), ("Sender", &from), ("Bcc", &other),
            ("Message-ID", &other), ("In-Reply-To", &other), ("References", &other),
            ("List-Unsubscribe", &other), ("List-Unsubscribe-Post", &other),
        ] {
            msg.extend_from_slice(name.as_bytes());
            msg.extend_from_slice(b": ");
            msg.extend_from_slice(value);
            msg.extend_from_slice(b"\r\n");
        }
        msg.extend_from_slice(b"\r\nbody");
        if let Ok(m) = parse_message(&msg) {
            prop_assert!(all_header_values_clean(&m));
        }
    }

    #[test]
    fn encoded_word_injection_is_clean(hex in "[0-9A-F]{2}", tail in "[a-z]{0,10}") {
        let msg = format!("Subject: =?UTF-8?Q?a=0D=0A=00={hex}{tail}?=\r\nFrom: =?UTF-8?Q?x=0D=0A?= <a@example.org>\r\n\r\nbody");
        if let Ok(m) = parse_message(msg.as_bytes()) {
            prop_assert!(all_header_values_clean(&m));
        }
    }
}

#[test]
fn one_mebibyte_of_junk_is_fast() {
    let junk: Vec<u8> = (0..1024 * 1024u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let start = Instant::now();
    let _ = parse_message(&junk);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn one_mebibyte_of_mime_boundaries_is_fast() {
    let mut s = String::from("Content-Type: multipart/mixed; boundary=\"x\"\r\n\r\n");
    while s.len() < 1024 * 1024 {
        s.push_str("--x\r\nContent-Type: multipart/mixed; boundary=\"x\"\r\n\r\n");
    }
    let start = Instant::now();
    let _ = parse_message(s.as_bytes());
    assert!(start.elapsed() < Duration::from_secs(1));
}
