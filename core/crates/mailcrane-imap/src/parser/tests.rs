use super::*;
use proptest::prelude::*;

fn parse(input: &[u8]) -> Response {
    let (response, used) = parse_response(input).expect("parses");
    assert_eq!(used, input.len());
    response
}

#[test]
fn tagged_ok_with_capability_code() {
    let r = parse(b"a1 OK [CAPABILITY IMAP4rev2 SPECIAL-USE] LOGIN completed\r\n");
    assert_eq!(
        r,
        Response::Tagged {
            tag: "a1".into(),
            status: StatusKind::Ok,
            text: RespText {
                code: Some(Code::Capability(vec![
                    "IMAP4rev2".into(),
                    "SPECIAL-USE".into()
                ])),
                text: "LOGIN completed".into(),
            },
        }
    );
}

#[test]
fn tagged_without_text_is_accepted() {
    let r = parse(b"a2 no\r\n");
    assert!(matches!(
        r,
        Response::Tagged {
            status: StatusKind::No,
            ..
        }
    ));
}

#[test]
fn untagged_capability_and_greeting() {
    assert_eq!(
        parse(b"* CAPABILITY IMAP4rev2 STARTTLS LOGINDISABLED\r\n"),
        Response::Untagged(Untagged::Capability(vec![
            "IMAP4rev2".into(),
            "STARTTLS".into(),
            "LOGINDISABLED".into()
        ]))
    );
    let r = parse(b"* OK [ALERT] hello\r\n");
    assert_eq!(
        r,
        Response::Untagged(Untagged::Condition {
            condition: Condition::Ok,
            text: RespText {
                code: Some(Code::Other {
                    name: "ALERT".into(),
                    args: None
                }),
                text: "hello".into()
            }
        })
    );
    assert!(matches!(
        parse(b"* BYE shutting down\r\n"),
        Response::Untagged(Untagged::Condition {
            condition: Condition::Bye,
            ..
        })
    ));
}

#[test]
fn list_forms() {
    assert_eq!(
        parse(b"* LIST (\\HasNoChildren \\Sent) \"/\" Sent\r\n"),
        Response::Untagged(Untagged::List(ListItem {
            attributes: vec!["\\HasNoChildren".into(), "\\Sent".into()],
            delimiter: Some('/'),
            name: b"Sent".to_vec(),
        }))
    );
    assert_eq!(
        parse(b"* LIST () NIL \"a \\\"b\\\"\"\r\n"),
        Response::Untagged(Untagged::List(ListItem {
            attributes: vec![],
            delimiter: None,
            name: b"a \"b\"".to_vec(),
        }))
    );
    assert_eq!(
        parse(b"* LIST (\\Noselect) \"\\\\\" {5}\r\nfo\r\no\r\n"),
        Response::Untagged(Untagged::List(ListItem {
            attributes: vec!["\\Noselect".into()],
            delimiter: Some('\\'),
            name: b"fo\r\no".to_vec(),
        }))
    );
    // RFC 5258 extended data is skipped.
    assert!(matches!(
        parse(b"* LIST () \".\" INBOX (\"CHILDINFO\" (\"SUBSCRIBED\"))\r\n"),
        Response::Untagged(Untagged::List(_))
    ));
}

#[test]
fn unknown_responses_are_framed_over_literals() {
    let input = b"* 1 FETCH (BODY[] {6}\r\nab\r\ncd)\r\n* 2 EXISTS\r\n";
    let (r, used) = parse_response(input).expect("parses");
    let Response::Untagged(Untagged::Fetch(data)) = r else {
        panic!("expected a FETCH response");
    };
    assert_eq!(data.body.as_deref(), Some(&b"ab\r\ncd"[..]));
    assert_eq!(&input[used..], b"* 2 EXISTS\r\n");
    assert_eq!(
        parse(b"* 2 EXISTS\r\n"),
        Response::Untagged(Untagged::Exists(2))
    );
    assert_eq!(
        parse(b"* 1 RECENT\r\n"),
        Response::Untagged(Untagged::Other {
            name: "RECENT".into()
        })
    );
}

const ENV: &str = "ENVELOPE (\"Wed, 24 Nov 2026 10:46:51 +0000\" \"Hi\" ((\"A\" NIL \"a\" \"example.org\")) NIL NIL ((NIL NIL \"b\" \"example.net\")) NIL NIL NIL \"<1@example.org>\")";

fn fetch(input: &str) -> Untagged {
    match parse(input.as_bytes()) {
        Response::Untagged(u) => u,
        other => panic!("not untagged: {other:?}"),
    }
}

#[test]
fn fetch_headers_are_read() {
    let line = format!(
        "* 3 FETCH (UID 7 FLAGS (\\Seen $x) INTERNALDATE \"24-Nov-2026 10:46:51 +0000\" RFC822.SIZE 99 {ENV} BODYSTRUCTURE ((\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 1 1)(\"APPLICATION\" \"PDF\" NIL NIL NIL \"BASE64\" 4 NIL (\"ATTACHMENT\" (\"FILENAME\" \"a.pdf\")) NIL) \"MIXED\") BODY[HEADER.FIELDS (REFERENCES)] {{4}}\r\nX: y)\r\n"
    );
    let Untagged::Fetch(d) = fetch(&line) else {
        panic!("expected FETCH");
    };
    assert_eq!((d.seq, d.uid, d.size), (3, Some(7), Some(99)));
    assert_eq!(d.flags, Some(vec!["\\Seen".to_string(), "$x".to_string()]));
    assert_eq!(d.has_attachment, Some(true));
    assert_eq!(d.header_fields.as_deref(), Some(&b"X: y"[..]));
    let env = d.envelope.expect("envelope");
    assert_eq!(env.subject.as_deref(), Some(&b"Hi"[..]));
    assert_eq!(env.from.len(), 1);
    assert_eq!(env.to.first().and_then(|a| a.name.clone()), None);
    assert_eq!(env.message_id.as_deref(), Some(&b"<1@example.org>"[..]));
}

#[test]
fn malformed_envelope_or_structure_is_flagged_with_its_uid() {
    for bad in [
        "* 1 FETCH (UID 9 ENVELOPE (\"x\" \"y\"))\r\n",
        "* 1 FETCH (UID 9 ENVELOPE ((((\r\n",
        "* 1 FETCH (UID 9 BODYSTRUCTURE (1 2 3))\r\n",
        "* 1 FETCH (UID 9 BODYSTRUCTURE ((\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 1 1)))\r\n",
    ] {
        assert_eq!(
            fetch(bad),
            Untagged::FetchMalformed { uid: Some(9) },
            "{bad}"
        );
    }
}

#[test]
fn deeply_nested_structure_is_rejected_not_overflowed() {
    let nested = "(".repeat(5000);
    let line = format!("* 1 FETCH (UID 4 BODYSTRUCTURE {nested}\r\n");
    assert_eq!(fetch(&line), Untagged::FetchMalformed { uid: Some(4) });
}

#[test]
fn search_lists_uids() {
    assert_eq!(
        fetch("* SEARCH 1 20 300\r\n"),
        Untagged::Search(vec![1, 20, 300])
    );
    assert_eq!(fetch("* SEARCH\r\n"), Untagged::Search(Vec::new()));
}

#[test]
fn modseq_is_read_from_fetch() {
    // RFC 7162 §3.1.4.1
    let Untagged::Fetch(d) = fetch("* 4 FETCH (UID 12 FLAGS (\\Seen) MODSEQ (624140003))\r\n")
    else {
        panic!("expected FETCH");
    };
    assert_eq!((d.uid, d.modseq), (Some(12), Some(624_140_003)));
    assert_eq!(
        fetch("* 4 FETCH (UID 12 MODSEQ 5)\r\n"),
        Untagged::FetchMalformed { uid: Some(12) }
    );
}

#[test]
fn vanished_is_read_as_ranges() {
    // RFC 7162 §3.2.10
    assert_eq!(
        fetch("* VANISHED (EARLIER) 41,43:116,118,120:119\r\n"),
        Untagged::Vanished {
            earlier: true,
            uids: vec![(41, 41), (43, 116), (118, 118), (119, 120)],
        }
    );
    // A huge range stays one range, never a list of four billion UIDs.
    assert_eq!(
        fetch("* VANISHED 1:4294967295\r\n"),
        Untagged::Vanished {
            earlier: false,
            uids: vec![(1, u32::MAX)],
        }
    );
    for bad in [
        "* VANISHED\r\n",
        "* VANISHED 0\r\n",
        "* VANISHED 1:*\r\n",
        "* VANISHED (LATER) 1\r\n",
        "* VANISHED 4294967296\r\n",
        "* VANISHED 1,,2\r\n",
    ] {
        assert!(parse_response(bad.as_bytes()).is_err(), "{bad}");
    }
}

#[test]
fn continuation() {
    assert_eq!(
        parse(b"+ \r\n"),
        Response::Continuation {
            text: String::new()
        }
    );
    assert_eq!(
        parse(b"+\r\n"),
        Response::Continuation {
            text: String::new()
        }
    );
}

#[test]
fn incomplete_and_invalid() {
    assert_eq!(parse_response(b"a1 OK"), Err(ParseError::Incomplete));
    assert_eq!(
        parse_response(b"* LIST () \"/\" {10}\r\nabc"),
        Err(ParseError::Incomplete)
    );
    assert!(matches!(
        parse_response(b"\xFF\xFEgarbage\r\n"),
        Err(ParseError::Invalid { .. })
    ));
    assert!(matches!(
        parse_response(b"a1 MAYBE\r\n"),
        Err(ParseError::Invalid { .. })
    ));
    assert!(matches!(
        parse_response(b"* LIST (\\x \"/\" a\r\n"),
        Err(ParseError::Invalid { .. })
    ));
}

#[test]
fn limits_are_enforced() {
    let limits = Limits {
        max_line: 16,
        max_literal: 32,
    };
    assert_eq!(
        parse_response_with(b"* OK this line is too long\r\n", &limits),
        Err(ParseError::LineTooLong)
    );
    // Even before the line ends.
    assert_eq!(
        parse_response_with(&[b'a'; 17], &limits),
        Err(ParseError::LineTooLong)
    );
    assert_eq!(
        parse_response_with(b"* X {33}\r\n", &limits),
        Err(ParseError::TooLarge)
    );
    assert_eq!(
        parse_response(b"* LIST () NIL {99999999999999999999999}\r\n"),
        Err(ParseError::TooLarge)
    );
    // Many small literals cannot add up past the limit either.
    let mut many = Vec::new();
    for _ in 0..10 {
        many.extend_from_slice(b"* X {8}\r\n12345678");
    }
    assert_eq!(
        parse_response_with(&many, &limits),
        Err(ParseError::TooLarge)
    );
}

// ---- Round trips -------------------------------------------------------

fn encode_astring(name: &[u8], out: &mut Vec<u8>) {
    if !name.is_empty() && name.iter().all(|&b| is_astring_char(b)) {
        out.extend_from_slice(name);
    } else if name.iter().all(|&b| (0x20..=0x7e).contains(&b)) {
        out.push(b'"');
        for &b in name {
            if b == b'"' || b == b'\\' {
                out.push(b'\\');
            }
            out.push(b);
        }
        out.push(b'"');
    } else {
        out.extend_from_slice(format!("{{{}}}\r\n", name.len()).as_bytes());
        out.extend_from_slice(name);
    }
}

fn encode_resp_text(t: &RespText, out: &mut Vec<u8>) {
    out.push(b' ');
    match &t.code {
        Some(Code::Capability(caps)) => {
            out.extend_from_slice(b"[CAPABILITY");
            for cap in caps {
                out.push(b' ');
                out.extend_from_slice(cap.as_bytes());
            }
            out.push(b']');
        }
        Some(Code::Other { name, args }) => {
            out.push(b'[');
            out.extend_from_slice(name.as_bytes());
            if let Some(args) = args {
                out.push(b' ');
                out.extend_from_slice(args.as_bytes());
            }
            out.push(b']');
        }
        None => {}
    }
    if t.code.is_some() && !t.text.is_empty() {
        out.push(b' ');
    }
    out.extend_from_slice(t.text.as_bytes());
}

fn encode(r: &Response) -> Vec<u8> {
    let mut out = Vec::new();
    match r {
        Response::Continuation { text } => {
            out.extend_from_slice(b"+ ");
            out.extend_from_slice(text.as_bytes());
        }
        Response::Tagged { tag, status, text } => {
            out.extend_from_slice(tag.as_bytes());
            out.extend_from_slice(match status {
                StatusKind::Ok => b" OK",
                StatusKind::No => b" NO",
                StatusKind::Bad => b" BAD",
            });
            encode_resp_text(text, &mut out);
        }
        Response::Untagged(u) => {
            out.extend_from_slice(b"* ");
            match u {
                Untagged::Condition { condition, text } => {
                    out.extend_from_slice(match condition {
                        Condition::Ok => b"OK".as_slice(),
                        Condition::No => b"NO",
                        Condition::Bad => b"BAD",
                        Condition::PreAuth => b"PREAUTH",
                        Condition::Bye => b"BYE",
                    });
                    encode_resp_text(text, &mut out);
                }
                Untagged::Capability(caps) | Untagged::Enabled(caps) => {
                    out.extend_from_slice(if matches!(u, Untagged::Capability(_)) {
                        b"CAPABILITY".as_slice()
                    } else {
                        b"ENABLED"
                    });
                    for cap in caps {
                        out.push(b' ');
                        out.extend_from_slice(cap.as_bytes());
                    }
                }
                Untagged::List(item) => {
                    out.extend_from_slice(b"LIST (");
                    out.extend_from_slice(item.attributes.join(" ").as_bytes());
                    out.extend_from_slice(b") ");
                    match item.delimiter {
                        Some(c @ ('"' | '\\')) => {
                            out.extend_from_slice(format!("\"\\{c}\"").as_bytes())
                        }
                        Some(c) => out.extend_from_slice(format!("\"{c}\"").as_bytes()),
                        None => out.extend_from_slice(b"NIL"),
                    }
                    out.push(b' ');
                    encode_astring(&item.name, &mut out);
                }
                Untagged::Other { name } => out.extend_from_slice(name.as_bytes()),
                Untagged::Exists(n) => out.extend_from_slice(format!("{n} EXISTS").as_bytes()),
                Untagged::Search(uids) => {
                    out.extend_from_slice(b"SEARCH");
                    for uid in uids {
                        out.extend_from_slice(format!(" {uid}").as_bytes());
                    }
                }
                Untagged::Vanished { earlier, uids } => {
                    out.extend_from_slice(b"VANISHED ");
                    if *earlier {
                        out.extend_from_slice(b"(EARLIER) ");
                    }
                    let set: Vec<String> = uids
                        .iter()
                        .map(|(a, b)| {
                            if a == b {
                                a.to_string()
                            } else {
                                format!("{a}:{b}")
                            }
                        })
                        .collect();
                    out.extend_from_slice(set.join(",").as_bytes());
                }
                // Not produced by the generators below.
                Untagged::Fetch(_) | Untagged::FetchMalformed { .. } => {}
            }
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}

fn atom() -> impl Strategy<Value = String> {
    "[A-Za-z0-9=.+-]{1,12}"
}

// Free text: printable ASCII without '{' (a trailing "{n}" would announce
// a literal) and not starting with '['.
fn text() -> impl Strategy<Value = String> {
    "([ -Z\\\\^-z|}~][ -z|}~]{0,30})?"
}

fn resp_text() -> impl Strategy<Value = RespText> {
    let code = prop_oneof![
        Just(None),
        prop::collection::vec(atom(), 0..4).prop_map(|c| Some(Code::Capability(c))),
        (
            atom().prop_filter("not CAPABILITY", |a| !a.eq_ignore_ascii_case("CAPABILITY")),
            prop::option::of("[ -\\\\^-z|}~]{0,10}")
        )
            .prop_map(|(name, args)| Some(Code::Other { name, args })),
    ];
    (code, text()).prop_map(|(code, text)| RespText { code, text })
}

fn response() -> impl Strategy<Value = Response> {
    let status = prop_oneof![
        Just(StatusKind::Ok),
        Just(StatusKind::No),
        Just(StatusKind::Bad)
    ];
    let condition = prop_oneof![
        Just(Condition::Ok),
        Just(Condition::No),
        Just(Condition::Bad),
        Just(Condition::PreAuth),
        Just(Condition::Bye)
    ];
    let attribute = (any::<bool>(), atom()).prop_map(|(b, a)| if b { format!("\\{a}") } else { a });
    let list = (
        prop::collection::vec(attribute, 0..4),
        prop::option::of(prop::char::range(' ', '~')),
        prop::collection::vec(any::<u8>(), 0..40),
    )
        .prop_map(|(attributes, delimiter, name)| {
            Response::Untagged(Untagged::List(ListItem {
                attributes,
                delimiter,
                name,
            }))
        });
    prop_oneof![
        text().prop_map(|text| Response::Continuation { text }),
        ("[A-Za-z0-9]{1,8}", status, resp_text())
            .prop_map(|(tag, status, text)| Response::Tagged { tag, status, text }),
        (condition, resp_text()).prop_map(|(condition, text)| Response::Untagged(
            Untagged::Condition { condition, text }
        )),
        prop::collection::vec(atom(), 0..6)
            .prop_map(|c| Response::Untagged(Untagged::Capability(c))),
        prop::collection::vec(atom(), 0..6).prop_map(|c| Response::Untagged(Untagged::Enabled(c))),
        list,
        (
            any::<bool>(),
            prop::collection::vec((1u32.., 1u32..), 1..6).prop_map(|ranges| ranges
                .into_iter()
                .map(|(a, b)| (a.min(b), a.max(b)))
                .collect::<Vec<_>>())
        )
            .prop_map(|(earlier, uids)| Response::Untagged(Untagged::Vanished { earlier, uids })),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = parse_response(&bytes);
    }

    #[test]
    fn arbitrary_imap_like_bytes_never_panic(s in "[*+a-z0-9 ()\\[\\]{}\"\\\\\r\n]{0,200}") {
        let _ = parse_response(s.as_bytes());
    }

    #[test]
    fn valid_responses_round_trip(r in response()) {
        let bytes = encode(&r);
        let (parsed, used) = parse_response(&bytes).map_err(|e| TestCaseError::fail(format!("{e:?} for {:?}", String::from_utf8_lossy(&bytes))))?;
        prop_assert_eq!(used, bytes.len());
        prop_assert_eq!(parsed, r);
    }
}
