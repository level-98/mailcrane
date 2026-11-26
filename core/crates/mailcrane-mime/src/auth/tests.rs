use super::*;

fn method(header: &str, name: &str) -> Option<MethodResult> {
    parse(header).results.into_iter().find(|r| r.method == name)
}

#[test]
fn parses_a_typical_provider_header() {
    let header = "mx.example.org; spf=pass (sender IP is 192.0.2.1) \
                  smtp.mailfrom=bounce@news.example.net smtp.helo=out.example.net; \
                  dkim=pass (2048-bit key) header.d=example.net header.s=s1 header.b=abc; \
                  dmarc=pass (p=REJECT) header.from=example.net";
    let parsed = parse(header);
    assert_eq!(parsed.authserv_id, "mx.example.org");
    let methods: Vec<_> = parsed.results.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["spf", "dkim", "dmarc"]);
    assert!(parsed.results.iter().all(|r| r.result == AuthResult::Pass));
    let spf = &parsed.results[0];
    assert_eq!(
        spf.property("smtp.mailfrom"),
        Some("bounce@news.example.net")
    );
    assert_eq!(spf.property("smtp.helo"), Some("out.example.net"));
    assert_eq!(parsed.results[1].property("header.s"), Some("s1"));
}

#[test]
fn result_words() {
    let r = |h: &str, m: &str| method(h, m).map(|r| r.result);
    let h = "mx.example.org; spf=softfail; dkim=softfail; dmarc=policy";
    assert_eq!(r(h, "spf"), Some(AuthResult::Fail));
    assert_eq!(r(h, "dkim"), Some(AuthResult::Unknown));
    assert_eq!(r(h, "dmarc"), Some(AuthResult::Unknown));
    let h = "mx.example.org; spf=NEUTRAL; dkim=TempError; dmarc=permerror; arc=none; x=bogus";
    assert_eq!(r(h, "spf"), Some(AuthResult::Neutral));
    assert_eq!(r(h, "dkim"), Some(AuthResult::TempError));
    assert_eq!(r(h, "dmarc"), Some(AuthResult::PermError));
    assert_eq!(r(h, "arc"), Some(AuthResult::None));
    assert_eq!(r(h, "x"), Some(AuthResult::Unknown));
}

#[test]
fn methods_are_case_insensitive_and_versions_dropped() {
    let h = "mx.example.org 1; DKIM/1 = pass HEADER.D = example.net; SPF=Fail";
    let parsed = parse(h);
    assert_eq!(parsed.authserv_id, "mx.example.org");
    assert_eq!(parsed.results[0].method, "dkim");
    assert_eq!(parsed.results[0].result, AuthResult::Pass);
    assert_eq!(parsed.results[0].property("header.d"), Some("example.net"));
    assert_eq!(parsed.results[1].result, AuthResult::Fail);
}

#[test]
fn comments_and_quoted_strings() {
    // A comment can't smuggle in a result; a quoted ';' doesn't split.
    let h = "mx.example.org; dkim=none (dkim=pass header.d=example.net; dmarc=pass); \
             dmarc=fail reason=\"p=reject; aligned=no\" header.from=\"example.net\"";
    let parsed = parse(h);
    assert_eq!(parsed.results.len(), 2);
    assert_eq!(parsed.results[0].result, AuthResult::None);
    assert!(parsed.results[0].properties.is_empty());
    assert_eq!(parsed.results[1].result, AuthResult::Fail);
    assert_eq!(
        parsed.results[1].property("reason"),
        Some("p=reject; aligned=no")
    );
    assert_eq!(
        parsed.results[1].property("header.from"),
        Some("example.net")
    );
    // Escapes inside both.
    let h = r#"mx.example.org; dkim=pass (a \) b) header.d="ex\"ample.net""#;
    assert_eq!(
        method(h, "dkim").and_then(|r| r.property("header.d").map(str::to_owned)),
        Some("ex\"ample.net".to_owned())
    );
}

#[test]
fn hostile_inputs_never_panic() {
    for h in [
        "",
        ";",
        ";;;;",
        "mx.example.org",
        "mx.example.org none",
        "mx.example.org; none",
        "mx.example.org; =pass",
        "mx.example.org; dkim=",
        "mx.example.org; dkim pass",
        "mx.example.org; dkim=pass header.d=",
        "mx.example.org; dkim=pass header.d",
        "(((((",
        "mx.example.org; dkim=pass (unclosed header.d=example.net",
        "mx.example.org; dmarc=pass header.from=\"unclosed",
        "\\",
        "mx.example.org; dkim=pass header.d=\\",
        "\u{feff}é; ü=ö ä=ß",
    ] {
        let _ = parse(h);
        let _ = summarize(h, "example.net");
    }
    assert!(parse("").results.is_empty());
    assert!(parse("mx.example.org; none").results.is_empty());
    assert!(parse("mx.example.org; dkim=").results.is_empty());
    // An unclosed comment swallows the rest.
    assert!(
        method("mx.example.org; dkim=pass (unclosed header.d=x.net", "dkim")
            .is_some_and(|r| r.properties.is_empty())
    );
}

#[test]
fn large_values_are_bounded() {
    let big = format!(
        "mx.example.org; {}",
        "dkim=pass header.d=example.net; ".repeat(2048)
    );
    assert!(big.len() > 64 * 1024);
    assert_eq!(parse(&big).results.len(), MAX_RESULTS);
    let nested = format!("mx.example.org; {}dkim=pass", "(".repeat(70_000));
    assert!(parse(&nested).results.is_empty());
    let long_word = format!(
        "mx.example.org; dkim=pass header.d={}.net",
        "a".repeat(70_000)
    );
    assert_eq!(summarize(&long_word, "example.net").dkim_domain, None);
}

#[test]
fn summary_reads_domains() {
    let s = summarize(
        "mx.example.org; spf=pass smtp.mailfrom=bounce@News.Example.NET.; \
         dkim=pass header.i=@mail.example.net; dmarc=pass header.from=example.net",
        "example.net",
    );
    assert_eq!(s.spf, AuthResult::Pass);
    assert_eq!(s.spf_domain.as_deref(), Some("news.example.net"));
    assert_eq!(s.dkim, AuthResult::Pass);
    assert_eq!(s.dkim_domain.as_deref(), Some("mail.example.net"));
    assert!(s.dkim_aligned);
    assert_eq!(s.dmarc_domain.as_deref(), Some("example.net"));
    // SPF falls back to the HELO name; odd characters drop the domain.
    let s = summarize(
        "mx.example.org; spf=none smtp.helo=out.example.net; dmarc=fail header.from=<x>",
        "example.net",
    );
    assert_eq!(s.spf_domain.as_deref(), Some("out.example.net"));
    assert_eq!(s.dmarc, AuthResult::Fail);
    assert_eq!(s.dmarc_domain, None);
}

#[test]
fn missing_methods_are_none() {
    let s = summarize("mx.example.org; arc=pass", "example.net");
    assert_eq!(
        (s.spf, s.dkim, s.dmarc, s.dkim_aligned),
        (AuthResult::None, AuthResult::None, AuthResult::None, false)
    );
    assert_eq!(
        (s.spf_domain, s.dkim_domain, s.dmarc_domain),
        (None, None, None)
    );
}

#[test]
fn an_aligned_dkim_pass_wins() {
    // Rule B2: any aligned pass wins, wherever it is.
    let s = summarize(
        "mx.example.org; dkim=fail header.d=example.net; dkim=pass header.d=esp.example; \
         dkim=pass header.d=mail.example.net",
        "example.net",
    );
    assert_eq!(s.dkim, AuthResult::Pass);
    assert_eq!(s.dkim_domain.as_deref(), Some("mail.example.net"));
    assert!(s.dkim_aligned);
    // Otherwise the first result counts, even an unaligned pass.
    let s = summarize(
        "mx.example.org; dkim=pass header.d=esp.example; dkim=fail header.d=example.net",
        "example.net",
    );
    assert_eq!(s.dkim, AuthResult::Pass);
    assert_eq!(s.dkim_domain.as_deref(), Some("esp.example"));
    assert!(!s.dkim_aligned);
    let s = summarize(
        "mx.example.org; dkim=fail header.d=example.net; dkim=pass header.d=esp.example",
        "example.net",
    );
    assert_eq!(s.dkim, AuthResult::Fail);
    assert!(!s.dkim_aligned);
}

fn rcv(s: &str) -> TraceHeader {
    TraceHeader::Received(s.to_owned())
}

fn ar(s: &str) -> TraceHeader {
    TraceHeader::AuthenticationResults(s.to_owned())
}

/// The provider's MX taking the message from outside.
const HANDOFF: &str = "from out.example.net (out.example.net [192.0.2.1]) by mx2.example.org \
                       (Postfix) with ESMTPS id 1; Wed, 25 Nov 2026 08:00:00 +0000";
const PROVIDER_PASS: &str = "mx1.example.org; dmarc=pass header.from=example.net";
const FORGED_PASS: &str = "mx.example.org; spf=pass; dkim=pass header.d=example.net; dmarc=pass";

#[test]
fn the_providers_header_above_the_handoff_is_trusted() {
    let trace = [ar(PROVIDER_PASS), rcv(HANDOFF), ar(FORGED_PASS)];
    assert_eq!(trusted_results(&trace), Some(PROVIDER_PASS));
}

#[test]
fn a_forged_header_is_not_trusted_when_the_provider_adds_none() {
    // The provider wrote only its Received header; the sender wrote an
    // Authentication-Results header with the provider's authserv-id.
    let trace = [rcv(HANDOFF), ar(FORGED_PASS)];
    assert_eq!(trusted_results(&trace), None);
    // Also with more hops the sender made up below.
    let trace = [
        rcv(HANDOFF),
        ar(FORGED_PASS),
        rcv("from mx1.example.org (localhost [127.0.0.1]) by mx9.example.org"),
    ];
    assert_eq!(trusted_results(&trace), None);
}

#[test]
fn a_header_right_below_the_handoff_is_not_trusted() {
    // Postfix puts milter headers below its own Received. That spot is
    // where a sender connecting straight to the MX puts theirs, so it can't
    // be trusted: no chip rather than a wrong one.
    let trace = [rcv(HANDOFF), ar(PROVIDER_PASS)];
    assert_eq!(trusted_results(&trace), None);
}

#[test]
fn a_helo_name_alone_does_not_make_a_hop_internal() {
    for handoff in [
        // The client claims the provider's name; the provider's lookup says otherwise.
        "from mx.example.org (out.example.net [192.0.2.1]) by mx2.example.org",
        "from mx.example.org (unknown [192.0.2.1]) by mx2.example.org",
        // No lookup comment at all.
        "from mx.example.org by mx2.example.org",
        // Exim style: an address and the HELO name in the comment.
        "from [192.0.2.1] (helo=mx.example.org) by mx2.example.org with esmtp",
        "from [127.0.0.1] (helo=mx.example.org) by mx2.example.org with esmtp",
    ] {
        let trace = [rcv(handoff), ar(FORGED_PASS)];
        assert_eq!(trusted_results(&trace), None, "{handoff}");
    }
}

#[test]
fn internal_hops_above_the_handoff_are_walked_past() {
    let trace = [
        rcv("by mx2.example.org (Postfix, from userid 0) id 7"),
        rcv("from localhost (localhost [127.0.0.1]) by mx2.example.org (amavis)"),
        ar("mx.other.example; dmarc=pass"),
        ar(PROVIDER_PASS),
        rcv("from mx1.example.org (mx1.example.org [10.0.0.1]) by mx2.example.org"),
        rcv("from out.example.net (out.example.net [192.0.2.1]) by mx1.example.org"),
        ar(FORGED_PASS),
    ];
    assert_eq!(trusted_results(&trace), Some(PROVIDER_PASS));
}

#[test]
fn with_every_hop_internal_only_headers_above_the_last_received_count() {
    let trace = [
        ar(PROVIDER_PASS),
        rcv("from webmail.example.org (localhost [127.0.0.1]) by mx2.example.org"),
        ar(FORGED_PASS),
    ];
    assert_eq!(trusted_results(&trace), Some(PROVIDER_PASS));
    let trace = [
        rcv("from webmail.example.org (localhost [127.0.0.1]) by mx2.example.org"),
        ar(FORGED_PASS),
    ];
    assert_eq!(trusted_results(&trace), None);
}

#[test]
fn a_hop_received_by_another_organisation_ends_the_walk() {
    let trace = [
        rcv("from mx1.example.org (mx1.example.org [10.0.0.1]) by mx2.example.org"),
        rcv("from x.example.org (x.example.org [10.0.0.9]) by relay.example.com"),
        ar(FORGED_PASS),
    ];
    assert_eq!(trusted_results(&trace), None);
}

#[test]
fn no_usable_received_header_means_nothing_is_trusted() {
    assert_eq!(trusted_results(&[]), None);
    assert_eq!(trusted_results(&[ar(PROVIDER_PASS)]), None);
    // The topmost Received names no host.
    assert_eq!(
        trusted_results(&[rcv("by 2002:a05:6a10::1 with SMTP id x"), ar(PROVIDER_PASS)]),
        None
    );
    assert_eq!(trusted_results(&[ar(PROVIDER_PASS), rcv("garbage")]), None);
}

#[test]
fn hostile_received_headers_never_panic() {
    for r in [
        "",
        "from",
        "from ",
        "from (",
        "from x (",
        "from x ((((( by mx2.example.org",
        "from x (a [b by mx2.example.org",
        "from x (a ]] [[ ) by mx2.example.org",
        "FROM é (ü [ö]) BY mx2.example.org",
        "from\u{feff} by mx2.example.org",
    ] {
        let _ = trusted_results(&[rcv(r), ar(PROVIDER_PASS)]);
        let _ = trusted_results(&[rcv(HANDOFF), rcv(r), ar(PROVIDER_PASS)]);
    }
    let long = format!("from {} by mx2.example.org", "(".repeat(70_000));
    assert_eq!(trusted_results(&[rcv(&long), ar(FORGED_PASS)]), None);
}

mod props {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parse_never_panics(header in "\\PC{0,400}") {
            let parsed = parse(&header);
            prop_assert!(parsed.results.len() <= MAX_RESULTS);
            let _ = summarize(&header, "example.net");
        }

        #[test]
        fn header_shaped_input_never_panics(
            header in "[a-z0-9 ;=()\"\\\\./@-]{0,200}",
            from in "[a-z.@-]{0,30}",
        ) {
            let _ = summarize(&header, &from);
            let _ = received_by(&header);
        }

        #[test]
        fn trusted_results_never_panics(
            trace in proptest::collection::vec(
                (any::<bool>(), "[a-z0-9 ;=()\\[\\]./-]{0,80}"),
                0..12,
            ),
        ) {
            let trace: Vec<TraceHeader> = trace
                .into_iter()
                .map(|(received, v)| {
                    if received {
                        TraceHeader::Received(v)
                    } else {
                        TraceHeader::AuthenticationResults(v)
                    }
                })
                .collect();
            let _ = trusted_results(&trace);
        }
    }
}
