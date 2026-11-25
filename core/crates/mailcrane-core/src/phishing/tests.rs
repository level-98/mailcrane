use super::*;
use mailcrane_mime::parse_message;

const RECEIVED: &str = "Received: from out.example.net (out.example.net [192.0.2.1]) \
    by mx2.example.org (Postfix) with ESMTPS id 1; Wed, 25 Nov 2026 08:00:00 +0000\r\n";

/// A message with these Authentication-Results headers, topmost first, and
/// the provider's Received header above them when `received`.
fn check(results: &[&str], received: bool) -> SenderCheck {
    let mut m = String::new();
    if received {
        m.push_str(RECEIVED);
    }
    for r in results {
        m.push_str(&format!("Authentication-Results: {r}\r\n"));
    }
    m.push_str(
        "From: News <news@example.net>\r\nTo: a@example.org\r\nSubject: Hi\r\n\r\nHello\r\n",
    );
    sender_check(&parse_message(m.as_bytes()).expect("parses"))
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
    // The sender wrote the second header; the provider's (topmost) wins.
    let c = check(
        &[
            "mx1.example.org; spf=fail; dkim=fail header.d=example.net; dmarc=fail",
            "mx.example.org; spf=pass; dkim=pass header.d=example.net; dmarc=pass",
        ],
        true,
    );
    assert!(c.trusted);
    assert_eq!(c.verdict, Some(AuthVerdict::Failed));
    assert_eq!(c.auth.dmarc, AuthResult::Fail);
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
        ("plain-ascii", None),
    ] {
        let path = format!(
            "{}/../../../tests/fixtures/mime/{name}.eml",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw = std::fs::read(&path).expect("fixture");
        let check = sender_check(&parse_message(&raw).expect("parses"));
        assert_eq!(check.verdict, want, "{name}");
        assert_eq!(check.trusted, want.is_some(), "{name}");
    }
}
