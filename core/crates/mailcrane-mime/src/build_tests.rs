use super::*;
use crate::parse_message;
use proptest::prelude::*;

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

fn draft() -> OutgoingMessage {
    OutgoingMessage {
        from: addr(Some("Ann Example"), "ann@example.org"),
        to: vec![addr(None, "bob@example.net")],
        cc: vec![],
        bcc: vec![],
        subject: "Lunch".into(),
        body_text: "Hello Bob,\nsee you at noon.\n".into(),
        encoding: TransferEncoding::QuotedPrintable,
    }
}

const RANDOM: [u8; 16] = [0xab; 16];
/// 24 Nov 2026 21:32:00 UTC.
const DATE: i64 = 1_795_555_920;

fn build(d: &OutgoingMessage) -> Vec<u8> {
    build_with(d, DATE, &RANDOM).expect("builds")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("ascii")
}

/// Every line ends in CRLF, there is no bare CR or LF, and no line is over
/// 998 octets.
fn assert_wire_format(bytes: &[u8]) {
    assert!(bytes.ends_with(b"\r\n"));
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            assert_eq!(bytes.get(i.wrapping_sub(1)), Some(&b'\r'), "bare LF at {i}");
        }
        if b == b'\r' {
            assert_eq!(bytes.get(i + 1), Some(&b'\n'), "bare CR at {i}");
        }
    }
    for line in bytes.split(|&b| b == b'\n') {
        assert!(line.len() <= MAX_LINE + 1, "line of {} octets", line.len());
    }
}

fn header_block(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    s.split("\r\n\r\n").next().unwrap_or_default().to_owned()
}

#[test]
fn simple_message_is_exact() {
    let got = text(&build(&draft()));
    let id = "ab".repeat(16);
    assert_eq!(
        got,
        format!(
            "Date: Tue, 24 Nov 2026 21:32:00 +0000\r\n\
             From: Ann Example <ann@example.org>\r\n\
             To: bob@example.net\r\n\
             Subject: Lunch\r\n\
             Message-ID: <{id}@example.org>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Transfer-Encoding: quoted-printable\r\n\
             \r\n\
             Hello Bob,\r\n\
             see you at noon.\r\n"
        )
    );
}

#[test]
fn round_trips_through_the_parser() {
    let mut d = draft();
    d.from = addr(Some("Zoë Ångström"), "zoe@example.org");
    d.to = vec![
        addr(Some("Bob \"The Builder\" B."), "bob@example.net"),
        addr(Some("Example, Carol"), "carol@example.com"),
        addr(None, "dan@example.org"),
    ];
    d.cc = vec![addr(Some("Émile 日本"), "emile@example.org")];
    d.bcc = vec![addr(Some("Secret"), "hidden@example.org")];
    d.subject = "Grüße aus Köln — 会議の件 ✈ and a fairly long tail to force folding".into();
    d.body_text = "Grüße!\n\nA line with trailing space \nand = signs\r\n.leading dot\n".into();

    let raw = build(&d);
    assert_wire_format(&raw);
    let parsed = parse_message(&raw).expect("parses");
    assert_eq!(parsed.subject.as_deref(), Some(d.subject.as_str()));
    assert_eq!(parsed.from, vec![d.from.clone()]);
    assert_eq!(parsed.to, d.to);
    assert_eq!(parsed.cc, d.cc);
    assert!(parsed.bcc.is_empty());
    assert_eq!(
        parsed.message_id.as_deref(),
        Some(format!("{}@example.org", "ab".repeat(16)).as_str())
    );
    assert_eq!(parsed.date, Some(DATE));
    let body = parsed.text_plain.unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(body, d.body_text.replace("\r\n", "\n"));
}

#[test]
fn never_writes_bcc() {
    let mut d = draft();
    d.to.clear();
    d.bcc = vec![addr(Some("Hidden"), "hidden@example.org")];
    let raw = text(&build(&d));
    assert!(!raw.to_ascii_lowercase().contains("bcc"));
    assert!(!raw.contains("hidden@example.org"));
    assert!(!raw.contains("\r\nTo:"));
}

#[test]
fn header_injection_is_impossible() {
    let mut d = draft();
    d.subject = "Hi\r\nBcc: evil@example.net\r\n\r\nbody".into();
    d.from.name = Some("Ann\nX-Evil: 1".into());
    d.to = vec![addr(
        Some("Bob\r\nBcc: evil@example.net"),
        "bob@example.net",
    )];
    let raw = build(&d);
    assert_wire_format(&raw);
    let headers = header_block(&raw);
    for line in headers.split("\r\n") {
        assert!(
            line.starts_with(' ')
                || [
                    "Date:",
                    "From:",
                    "To:",
                    "Subject:",
                    "Message-ID:",
                    "MIME-Version:",
                    "Content-Type:",
                    "Content-Transfer-Encoding:"
                ]
                .iter()
                .any(|h| line.starts_with(h)),
            "unexpected header line {line:?}"
        );
    }
    let parsed = parse_message(&raw).expect("parses");
    assert!(parsed.bcc.is_empty());
    assert_eq!(
        parsed.subject.as_deref(),
        Some("Hi  Bcc: evil@example.net    body")
    );

    for email in [
        "bob@example.net\r\nBcc: evil@example.net",
        "bob@example.net\n",
        "bob\0@example.net",
    ] {
        let mut d = draft();
        d.to = vec![addr(None, email)];
        assert_eq!(
            build_with(&d, DATE, &RANDOM),
            Err(BuildError::InvalidAddress {
                field: AddressField::To,
                index: 0,
                problem: AddressProblem::ControlCharacter
            })
        );
    }
    let mut d = draft();
    d.from.email = "ann@example.org\r\nX: y".into();
    assert!(matches!(
        build_with(&d, DATE, &RANDOM),
        Err(BuildError::InvalidAddress {
            field: AddressField::From,
            ..
        })
    ));
    let mut d = draft();
    d.bcc = vec![addr(None, "ok@example.org"), addr(None, "x@y\r\n")];
    assert!(matches!(
        build_with(&d, DATE, &RANDOM),
        Err(BuildError::InvalidAddress {
            field: AddressField::Bcc,
            index: 1,
            ..
        })
    ));
}

#[test]
fn long_lines_and_long_words_stay_within_limits() {
    let mut d = draft();
    d.subject = "x".repeat(3000);
    d.body_text = format!("{}\n{}", "y".repeat(5000), "é".repeat(2000));
    d.from.name = Some("N".repeat(2000));
    d.to = (0..40)
        .map(|i| addr(Some("Some Body"), &format!("person{i}@example.org")))
        .collect();
    let raw = build(&d);
    assert_wire_format(&raw);
    let parsed = parse_message(&raw).expect("parses");
    assert_eq!(parsed.subject.as_deref(), Some(d.subject.as_str()));
    assert_eq!(parsed.to.len(), 40);
    assert_eq!(parsed.from[0].name, d.from.name);
    let body = parsed.text_plain.unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(body.trim_end(), d.body_text);
}

#[test]
fn ascii_subject_keeps_spacing_when_folded() {
    let mut d = draft();
    d.subject = format!("{}  double  spaced {}", "word ".repeat(30), "end");
    let raw = build(&d);
    assert_wire_format(&raw);
    assert!(header_block(&raw).contains("\r\n "), "folded");
    let parsed = parse_message(&raw).expect("parses");
    assert_eq!(parsed.subject.as_deref(), Some(d.subject.as_str()));
}

#[test]
fn subject_that_looks_encoded_is_encoded() {
    let mut d = draft();
    d.subject = "=?UTF-8?B?aGk=?=".into();
    let parsed = parse_message(&build(&d)).expect("parses");
    assert_eq!(parsed.subject.as_deref(), Some("=?UTF-8?B?aGk=?="));
}

#[test]
fn eight_bit_when_asked_and_possible() {
    let mut d = draft();
    d.encoding = TransferEncoding::EightBit;
    d.body_text = "Grüße\n".into();
    let raw = build(&d);
    let s = String::from_utf8(raw.clone()).expect("utf-8");
    assert!(s.contains("Content-Transfer-Encoding: 8bit\r\n"));
    assert!(s.ends_with("\r\n\r\nGrüße\r\n"));
    assert_eq!(
        parse_message(&raw).expect("parses").text_plain.as_deref(),
        Some("Grüße\r\n")
    );

    d.body_text = "z".repeat(1200);
    let s = text(&build(&d));
    assert!(s.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
}

#[test]
fn message_ids_are_unique_and_use_the_sender_domain() {
    let d = draft();
    let ids: std::collections::HashSet<String> = (0..50)
        .map(|_| {
            let raw = build_message(&d).expect("builds");
            parse_message(&raw)
                .expect("parses")
                .message_id
                .expect("has id")
        })
        .collect();
    assert_eq!(ids.len(), 50);
    assert!(ids.iter().all(|id| id.ends_with("@example.org")));
}

#[test]
fn refuses_bad_drafts() {
    let mut d = draft();
    d.to.clear();
    assert_eq!(build_with(&d, DATE, &RANDOM), Err(BuildError::NoRecipients));

    let mut d = draft();
    d.to = vec![addr(None, "a@example.org"); MAX_RECIPIENTS + 1];
    assert_eq!(
        build_with(&d, DATE, &RANDOM),
        Err(BuildError::TooManyRecipients)
    );

    let mut d = draft();
    d.body_text = "a".repeat(MAX_BODY_BYTES + 1);
    assert_eq!(build_with(&d, DATE, &RANDOM), Err(BuildError::TooLarge));

    let mut d = draft();
    d.cc = vec![addr(None, "not-an-address")];
    assert_eq!(
        build_with(&d, DATE, &RANDOM),
        Err(BuildError::InvalidAddress {
            field: AddressField::Cc,
            index: 0,
            problem: AddressProblem::MissingAt
        })
    );
}

#[test]
fn dates_format_correctly() {
    assert_eq!(format_date(0), "Thu, 1 Jan 1970 00:00:00 +0000");
    assert_eq!(format_date(951_782_400), "Tue, 29 Feb 2000 00:00:00 +0000");
    assert_eq!(format_date(-5), "Thu, 1 Jan 1970 00:00:00 +0000");
    assert_eq!(format_date(i64::MAX), "Fri, 31 Dec 9999 23:59:59 +0000");
}

#[test]
fn base64_matches_rfc_4648_vectors() {
    let cases = [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foobar", "Zm9vYmFy"),
    ];
    for (input, want) in cases {
        assert_eq!(base64(input.as_bytes()), want);
    }
}

proptest! {
    #[test]
    fn any_text_builds_to_valid_wire_format_and_round_trips(
        subject in "\\PC{0,300}",
        name in "\\PC{0,100}",
        body in "(\\PC|\r|\n|\t){0,2000}",
        eight_bit in any::<bool>(),
    ) {
        let mut d = draft();
        d.subject = subject;
        d.to = vec![addr(Some(&name), "bob@example.net")];
        d.body_text = body;
        d.encoding = if eight_bit { TransferEncoding::EightBit } else { TransferEncoding::QuotedPrintable };
        let raw = build(&d);
        assert_wire_format(&raw);
        prop_assert!(!header_block(&raw).to_ascii_lowercase().contains("\r\nbcc"));
        let parsed = parse_message(&raw).expect("parses");
        prop_assert_eq!(parsed.to.len(), 1);
        prop_assert_eq!(parsed.to[0].email.as_str(), "bob@example.net");
    }
}

/// The builder's output for a fixed draft, kept as a fixture so changes to
/// the wire format show up in review. Run with `MAILCRANE_BLESS=1` to
/// rewrite it after an intended change.
#[test]
fn matches_golden_fixture() {
    let path = format!(
        "{}/../../../tests/fixtures/mime/composed-utf8.eml",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut d = draft();
    d.from = addr(Some("Zoë Example"), "zoe@example.org");
    d.to = vec![
        addr(Some("Bob \"B\" Example"), "bob@example.net"),
        addr(Some("Example, Carol"), "carol@example.com"),
    ];
    d.cc = vec![addr(None, "dan@example.org")];
    d.bcc = vec![addr(Some("Hidden"), "hidden@example.org")];
    d.subject = "Grüße — meeting notes for Thursday's planning session (draft)".into();
    d.body_text = "Hi all,\n\nNotes: café = 3 €, tea = 2 €.  \n.\nBye\n".into();
    let built = build(&d);
    if std::env::var_os("MAILCRANE_BLESS").is_some() {
        std::fs::write(&path, &built).expect("write fixture");
    }
    let want = std::fs::read(&path).expect("fixture exists");
    assert_eq!(text(&built), text(&want));
}
