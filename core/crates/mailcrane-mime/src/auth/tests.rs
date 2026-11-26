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

/// Postfix with OpenDMARC: the milter's header right below Postfix's own
/// Received header.
const POSTFIX_AR: &str = "mx2.example.org; dmarc=pass header.from=example.net";

#[test]
fn the_postfix_layout_needs_the_accounts_authserv_id() {
    let trace = [rcv(HANDOFF), ar(POSTFIX_AR)];
    // Without the setting: no chip, as before.
    assert_eq!(trusted_results(&trace), None);
    assert_eq!(trusted_results_with(&trace, None), None);
    // With it: trusted.
    assert_eq!(
        trusted_results_with(&trace, Some("mx2.example.org")),
        Some(POSTFIX_AR)
    );
    assert_eq!(
        trusted_results_with(&trace, Some("MX2.Example.org.")),
        Some(POSTFIX_AR)
    );
    // A different id, or the same organisation but another host, isn't enough.
    assert_eq!(trusted_results_with(&trace, Some("mx1.example.org")), None);
    assert_eq!(trusted_results_with(&trace, Some("example.org")), None);
    assert_eq!(trusted_results_with(&trace, Some("")), None);
}

#[test]
fn the_whole_run_below_the_handoff_is_searched_but_nothing_further() {
    // OpenDKIM's header first, then OpenDMARC's.
    let dkim = "mx2.example.org.dkim; dkim=pass header.d=example.net";
    let trace = [rcv(HANDOFF), ar(dkim), ar(POSTFIX_AR)];
    assert_eq!(
        trusted_results_with(&trace, Some("mx2.example.org")),
        Some(POSTFIX_AR)
    );
    // A forged header with that id lower down, below another Received, is
    // never trusted.
    let forged = [
        rcv(HANDOFF),
        rcv("from sender.example.net by out.example.net"),
        ar(POSTFIX_AR),
    ];
    assert_eq!(trusted_results_with(&forged, Some("mx2.example.org")), None);
    // Nor when the handoff wasn't received by the provider.
    let elsewhere = [
        rcv("from mx1.example.org (mx1.example.org [10.0.0.1]) by mx2.example.org"),
        rcv("from x.example.org (x.example.org [10.0.0.9]) by relay.example.com"),
        ar(POSTFIX_AR),
    ];
    assert_eq!(
        trusted_results_with(&elsewhere, Some("mx2.example.org")),
        None
    );
}

#[test]
fn a_header_above_the_handoff_still_wins() {
    let trace = [ar(PROVIDER_PASS), rcv(HANDOFF), ar(POSTFIX_AR)];
    assert_eq!(
        trusted_results_with(&trace, Some("mx2.example.org")),
        Some(PROVIDER_PASS)
    );
}

#[test]
fn the_setting_changes_nothing_else() {
    // Every layout the default rule decides comes out the same with an
    // unrelated id set.
    let layouts: [&[TraceHeader]; 4] = [
        &[ar(PROVIDER_PASS), rcv(HANDOFF), ar(FORGED_PASS)],
        &[rcv(HANDOFF), ar(FORGED_PASS)],
        &[
            ar(PROVIDER_PASS),
            rcv("from webmail.example.org (localhost [127.0.0.1]) by mx2.example.org"),
        ],
        &[],
    ];
    for trace in layouts {
        assert_eq!(
            trusted_results_with(trace, Some("mx9.example.com")),
            trusted_results(trace)
        );
    }
}

#[test]
fn in_the_run_below_the_handoff_the_first_header_with_the_id_wins() {
    // The provider's real result first, a sender's forged pass with the
    // same authserv-id below it: the real one counts.
    let real = "mx2.example.org; dmarc=fail header.from=example.net";
    let forged = "mx2.example.org; dmarc=pass header.from=example.net";
    let trace = [rcv(HANDOFF), ar(real), ar(forged)];
    assert_eq!(
        trusted_results_with(&trace, Some("mx2.example.org")),
        Some(real)
    );
    // A forged pass first in the run under another authserv-id is skipped.
    let other = "mx9.example.org; dmarc=pass header.from=example.net";
    let trace = [rcv(HANDOFF), ar(other), ar(real)];
    assert_eq!(
        trusted_results_with(&trace, Some("mx2.example.org")),
        Some(real)
    );
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

// --- #398: leading hops whose `by` is an address literal (Gmail) -----------

const GMAIL_INTERNAL: &str =
    "by 2001:db8:6a10:abcd:b0:5f1:1234:5678 with SMTP id x1csp123456pxb; Thu, 26 Nov 2026 14:30:00 -0800";
const GMAIL_ENTRY: &str =
    "from mail.example.net (mail.example.net. [192.0.2.10]) by mx.google.com \
     with ESMTPS id abc123; Thu, 26 Nov 2026 14:29:59 -0800";
const GMAIL_PASS: &str =
    "mx.google.com; dkim=pass header.i=@example.net; dmarc=pass header.from=example.net";

#[test]
fn ip_literal_by_hosts_are_recognised() {
    for literal in [
        "by 192.0.2.1 with SMTP",
        "by 2001:db8::1 with SMTP",
        "by [192.0.2.1]; Thu",
        "by [IPv6:2001:db8::1] id 7",
        "from x by 192.0.2.1.",
    ] {
        assert!(by_is_ip_literal(literal), "{literal}");
    }
    for name in [
        "by mx.google.com with SMTP",
        "by 2001.example.net",
        "by",
        "by [2001:db8::zz]",
        "",
    ] {
        assert!(!by_is_ip_literal(name), "{name}");
    }
}

/// Gmail's trace: an internal hop with an IPv6 `by`, then mx.google.com,
/// which writes its header just below its own Received (postfix layout).
/// The provider is found; the header counts with the server name setting,
/// and not without it (as for any Postfix layout).
#[test]
fn gmail_internal_ipv6_hops_are_skipped_to_find_the_provider() {
    let trace = [rcv(GMAIL_INTERNAL), rcv(GMAIL_ENTRY), ar(GMAIL_PASS)];
    assert_eq!(
        trusted_results_with(&trace, Some("mx.google.com")),
        Some(GMAIL_PASS)
    );
    assert_eq!(
        trusted_results_with(&trace, Some("mx.google.com.")),
        Some(GMAIL_PASS)
    );
    assert_eq!(
        trusted_results(&trace),
        None,
        "postfix layout: needs the setting"
    );
    // Before #398 the IPv6 hop ended it even with the setting.
    let two = [
        rcv(GMAIL_INTERNAL),
        rcv(GMAIL_INTERNAL),
        rcv(GMAIL_ENTRY),
        ar(GMAIL_PASS),
    ];
    assert_eq!(
        trusted_results_with(&two, Some("mx.google.com")),
        Some(GMAIL_PASS)
    );
    // A provider header above the entry hop counts once the setting
    // anchors the skip; without the setting there is no provider.
    let above = [rcv(GMAIL_INTERNAL), ar(GMAIL_PASS), rcv(GMAIL_ENTRY)];
    assert_eq!(
        trusted_results_with(&above, Some("mx.google.com")),
        Some(GMAIL_PASS)
    );
    assert_eq!(trusted_results(&above), None);
}

/// #442 review (security): a self-hosted server writes its top hop with a
/// literal `by`. The first named hop below it is the sender's forged one;
/// it must never become the provider, with no setting or an unrelated one.
#[test]
fn a_literal_top_hop_never_makes_the_senders_hop_the_provider() {
    let forged_ar =
        "mx.bank.example; dkim=pass header.d=bank.example; dmarc=pass header.from=bank.example";
    let trace = [
        rcv("from relay.example.org by [192.168.1.2]; Thu, 26 Nov 2026 14:30:00 +0000"),
        ar(forged_ar),
        rcv("from x.bank.example by mx.bank.example; Thu, 26 Nov 2026 14:29:00 +0000"),
        ar(forged_ar),
    ];
    assert_eq!(trusted_results(&trace), None);
    assert_eq!(trusted_results_with(&trace, Some("mail.example.org")), None);
    assert_eq!(trusted_results_with(&trace, Some("mx.google.com")), None);
    assert_eq!(trusted_results_with(&trace, Some("localhost")), None);
}

/// Skipping literal hops loosens nothing else: forged or foreign headers
/// stay untrusted, and past the cap there is no provider at all.
#[test]
fn skipping_literal_hops_trusts_no_forged_header() {
    let forged = "mx.google.com; dkim=pass; dmarc=pass header.from=example.net";
    // Below the sender's own hop, under the entry: never trusted.
    let below = [
        rcv(GMAIL_INTERNAL),
        rcv(GMAIL_ENTRY),
        rcv("from laptop.example.net by mail.example.net; Thu, 26 Nov 2026 14:29:00 -0800"),
        ar(forged),
    ];
    assert_eq!(trusted_results_with(&below, Some("mx.google.com")), None);
    // Another authserv-id right below the entry: not the setting's, not trusted.
    let other = [
        rcv(GMAIL_INTERNAL),
        rcv(GMAIL_ENTRY),
        ar("evil.example.net; dmarc=pass"),
    ];
    assert_eq!(trusted_results_with(&other, Some("mx.google.com")), None);
    assert_eq!(trusted_results(&other), None);
    // A literal hop further down (not leading) still ends the walk.
    let late = [rcv(GMAIL_ENTRY), rcv(GMAIL_INTERNAL), ar(forged)];
    assert_eq!(trusted_results(&late), None);
    // Past the cap of leading literal hops: no provider, as before.
    let mut many: Vec<TraceHeader> = (0..=MAX_LITERAL_HOPS)
        .map(|_| rcv(GMAIL_INTERNAL))
        .collect();
    many.push(ar(GMAIL_PASS));
    many.push(rcv(GMAIL_ENTRY));
    assert_eq!(trusted_results(&many), None);
    // Only literal hops: no provider.
    assert_eq!(
        trusted_results(&[rcv(GMAIL_INTERNAL), ar(GMAIL_PASS)]),
        None
    );
}
