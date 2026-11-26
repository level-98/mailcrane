use super::*;
use mailcrane_mime::parse_message;

const RECEIVED: &str = "Received: from out.example.net (out.example.net [192.0.2.1]) \
    by mx2.example.org (Postfix) with ESMTPS id 1; Wed, 25 Nov 2026 08:00:00 +0000\r\n";

/// A message with these Authentication-Results headers, topmost first,
/// above the provider's Received header (when `received`), as a provider
/// adds them.
fn check(results: &[&str], received: bool) -> SenderCheck {
    check_with(results, &[], received)
}

/// Like [`check`], plus headers the sender wrote, below the Received header.
fn check_with(above: &[&str], below: &[&str], received: bool) -> SenderCheck {
    let mut m = String::new();
    for r in above {
        m.push_str(&format!("Authentication-Results: {r}\r\n"));
    }
    if received {
        m.push_str(RECEIVED);
    }
    for r in below {
        m.push_str(&format!("Authentication-Results: {r}\r\n"));
    }
    m.push_str(
        "From: News <news@example.net>\r\nTo: a@example.org\r\nSubject: Hi\r\n\r\nHello\r\n",
    );
    sender_check(&parse_message(m.as_bytes()).expect("parses"), None)
}

#[test]
fn dmarc_pass_is_verified() {
    let c = check(
        &["mx1.example.org; spf=pass smtp.mailfrom=example.net; dkim=pass header.d=example.net; dmarc=pass header.from=example.net"],
        true,
    );
    assert!(c.trusted);
    assert_eq!(c.verdict, Some(AuthVerdict::Verified));
    assert_eq!(
        (c.auth.spf, c.auth.dkim, c.auth.dmarc),
        (AuthResult::Pass, AuthResult::Pass, AuthResult::Pass)
    );
}

#[test]
fn dmarc_fail_is_failed() {
    let c = check(
        &["mx1.example.org; spf=fail; dkim=none; dmarc=fail header.from=example.net"],
        true,
    );
    assert_eq!(c.verdict, Some(AuthVerdict::Failed));
}

#[test]
fn verdict_rules() {
    let s = |spf, dkim, aligned, dmarc| AuthSummary {
        spf,
        spf_domain: Option::None,
        dkim,
        dkim_domain: Option::None,
        dkim_aligned: aligned,
        dmarc,
        dmarc_domain: Option::None,
    };
    use AuthResult::*;
    use AuthVerdict::*;
    for (summary, want) in [
        (s(Fail, Fail, false, Pass), Verified),
        (s(None, Pass, true, None), Verified),
        // An unaligned DKIM pass isn't enough.
        (s(None, Pass, false, None), NotVerified),
        // With a DMARC result, DKIM alone doesn't decide.
        (s(None, Pass, true, TempError), NotVerified),
        (s(Pass, Pass, true, Fail), Failed),
        (s(Fail, Fail, false, None), Failed),
        (s(Fail, None, false, None), NotVerified),
        (s(Pass, Fail, false, None), NotVerified),
        (s(None, None, false, None), NotVerified),
        (s(Unknown, Unknown, false, Unknown), NotVerified),
    ] {
        assert_eq!(verdict(&summary), want, "{summary:?}");
    }
}

#[test]
fn a_forged_pass_below_the_providers_header_is_ignored() {
    // The sender wrote the second header, below the provider's Received.
    let c = check_with(
        &["mx1.example.org; spf=fail; dkim=fail header.d=example.net; dmarc=fail"],
        &["mx.example.org; spf=pass; dkim=pass header.d=example.net; dmarc=pass"],
        true,
    );
    assert!(c.trusted);
    assert_eq!(c.verdict, Some(AuthVerdict::Failed));
    assert_eq!(c.auth.dmarc, AuthResult::Fail);
}

#[test]
fn a_forged_pass_is_ignored_when_the_provider_adds_no_header() {
    // The provider wrote only its Received header. The sender's header
    // names the provider, but sits below the point where the message
    // entered it.
    let c = check_with(
        &[],
        &["mx1.example.org; spf=pass; dkim=pass header.d=example.net; dmarc=pass"],
        true,
    );
    assert!(!c.trusted);
    assert_eq!(c.verdict, None);
}

#[test]
fn a_header_from_another_organisation_is_ignored() {
    // Above the provider's header, but another authserv-id.
    let c = check(
        &[
            "mx.evil.example; dmarc=pass header.from=example.net",
            "mx1.example.org; dmarc=fail header.from=example.net",
        ],
        true,
    );
    assert_eq!(c.verdict, Some(AuthVerdict::Failed));
    // And alone, it means nothing at all.
    let c = check(
        &["mx.evil.example; dmarc=pass header.from=example.net"],
        true,
    );
    assert!(!c.trusted);
    assert_eq!(c.verdict, None);
    assert_eq!(c.auth.dmarc, AuthResult::None);
}

#[test]
fn no_trusted_header_means_no_chip() {
    // No Received header: nothing can be trusted.
    let c = check(&["mx1.example.org; dmarc=pass"], false);
    assert!(!c.trusted);
    assert_eq!(c.verdict, None);
    // No Authentication-Results at all.
    let c = check(&[], true);
    assert!(!c.trusted);
    assert_eq!(c.verdict, None);
    assert_eq!(
        (c.auth.spf, c.auth.dkim, c.auth.dmarc),
        (AuthResult::None, AuthResult::None, AuthResult::None)
    );
}

#[test]
fn a_trusted_header_without_results_is_not_verified() {
    let c = check(&["mx1.example.org; none"], true);
    assert!(c.trusted);
    assert_eq!(c.verdict, Some(AuthVerdict::NotVerified));
}

#[test]
fn fixtures_show_the_expected_chip() {
    use AuthVerdict::*;
    for (name, want) in [
        ("auth-pass", Some(Verified)),
        ("auth-fail", Some(Failed)),
        ("auth-dkim-only", Some(Verified)),
        ("auth-forged-below", Some(NotVerified)),
        ("auth-forged-foreign", Some(Failed)),
        ("auth-no-received", None),
        ("auth-malformed", Some(NotVerified)),
        ("auth-forged-no-provider", None),
        ("plain-ascii", None),
    ] {
        let path = format!(
            "{}/../../../tests/fixtures/mime/{name}.eml",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw = std::fs::read(&path).expect("fixture");
        let check = sender_check(&parse_message(&raw).expect("parses"), None);
        assert_eq!(check.verdict, want, "{name}");
        assert_eq!(check.trusted, want.is_some(), "{name}");
    }
}

#[test]
fn the_postfix_layout_gets_a_chip_only_with_the_accounts_authserv_id() {
    // M5-08: Postfix with OpenDMARC writes its header right below its own
    // Received header.
    let m = format!(
        "{RECEIVED}Authentication-Results: mx2.example.org; spf=pass smtp.mailfrom=example.net; \
         dkim=pass header.d=example.net; dmarc=pass header.from=example.net\r\n\
         From: News <news@example.net>\r\nTo: a@example.org\r\nSubject: Hi\r\n\r\nHello\r\n"
    );
    let parsed = parse_message(m.as_bytes()).expect("parses");
    let c = sender_check(&parsed, None);
    assert!(!c.trusted);
    assert_eq!(c.verdict, None);
    let c = sender_check(&parsed, Some("mx2.example.org"));
    assert!(c.trusted);
    assert_eq!(c.verdict, Some(AuthVerdict::Verified));
    // Another server name changes nothing.
    assert_eq!(sender_check(&parsed, Some("mx9.example.org")).verdict, None);

    // A forged header with that name further down is still ignored.
    let forged = format!(
        "{RECEIVED}Received: from sender.example.net by out.example.net; \
         Wed, 25 Nov 2026 07:59:00 +0000\r\n\
         Authentication-Results: mx2.example.org; dmarc=pass header.from=example.net\r\n\
         From: News <news@example.net>\r\nTo: a@example.org\r\nSubject: Hi\r\n\r\nHello\r\n"
    );
    let parsed = parse_message(forged.as_bytes()).expect("parses");
    assert_eq!(sender_check(&parsed, Some("mx2.example.org")).verdict, None);
}

#[test]
fn the_postfix_fixture() {
    let path = format!(
        "{}/../../../tests/fixtures/mime/auth-postfix.eml",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read(&path).expect("fixture");
    let parsed = parse_message(&raw).expect("parses");
    assert_eq!(sender_check(&parsed, None).verdict, None);
    assert_eq!(
        sender_check(&parsed, Some("mx2.example.org")).verdict,
        Some(AuthVerdict::Verified)
    );
}

#[test]
fn a_forged_pass_below_the_providers_postfix_header_is_ignored() {
    let m = format!(
        "{RECEIVED}Authentication-Results: mx2.example.org; spf=fail; dkim=fail; dmarc=fail\r\n\
         Authentication-Results: mx2.example.org; spf=pass; dkim=pass header.d=example.net; dmarc=pass\r\n\
         From: News <news@example.net>\r\nTo: a@example.org\r\nSubject: Hi\r\n\r\nHello\r\n"
    );
    let parsed = parse_message(m.as_bytes()).expect("parses");
    assert_eq!(
        sender_check(&parsed, Some("mx2.example.org")).verdict,
        Some(AuthVerdict::Failed)
    );
}

fn display_name_check(from: &str) -> Option<DisplayNameWarning> {
    let m = format!("From: {from}\r\nTo: a@example.org\r\nSubject: Hi\r\n\r\nHello\r\n");
    sender_check(&parse_message(m.as_bytes()).expect("parses"), None).display_name_warning
}

#[test]
fn display_name_warnings_come_with_the_sender_check() {
    let w = display_name_check("\"ceo@example.com\" <attacker@example.net>").expect("warns");
    assert_eq!(w.claimed, "ceo@example.com");
    assert_eq!(w.actual, "attacker@example.net");
    assert_eq!(
        display_name_check("\"Support (support@example.com)\" <noreply@mail.example.com>"),
        None
    );
    assert_eq!(display_name_check("Ann Example <ann@example.org>"), None);
    assert_eq!(display_name_check("ann@example.org"), None);
}

#[test]
fn encoded_display_names_are_checked_after_decoding() {
    // RFC 2047: "ceo＠example.com" (fullwidth @), Q and B encodings.
    for from in [
        "=?UTF-8?Q?ceo=EF=BC=A0example.com?= <x@example.net>",
        "=?UTF-8?B?Y2Vv77ygZXhhbXBsZS5jb20=?= <x@example.net>",
        // A zero-width space inside the address.
        "=?UTF-8?Q?ceo@exa=E2=80=8Bmple.com?= <x@example.net>",
    ] {
        let w = display_name_check(from).unwrap_or_else(|| panic!("warns: {from}"));
        assert_eq!(w.claimed, "ceo@example.com", "{from}");
    }
}

#[test]
fn display_name_fixtures() {
    for (name, want) in [
        (
            "display-name-spoof",
            Some(("ceo@example.com", "x@example.net")),
        ),
        ("display-name-same-org", None),
        (
            "display-name-encoded",
            Some(("ceo@example.com", "x@example.net")),
        ),
        ("auth-pass", None),
        ("plain-ascii", None),
    ] {
        let path = format!(
            "{}/../../../tests/fixtures/mime/{name}.eml",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw = std::fs::read(&path).expect("fixture");
        let w = sender_check(&parse_message(&raw).expect("parses"), None).display_name_warning;
        assert_eq!(
            w.as_ref().map(|w| (w.claimed.as_str(), w.actual.as_str())),
            want,
            "{name}"
        );
    }
}
