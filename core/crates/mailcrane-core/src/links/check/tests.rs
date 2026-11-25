use super::*;
use proptest::prelude::*;

fn warns(href: &str, text: Option<&str>) -> Vec<LinkWarning> {
    check_link(href, text).warnings
}

#[test]
fn matching_link_has_no_warning() {
    let c = check_link(
        "https://example.com/login",
        Some("https://example.com/login"),
    );
    assert!(c.warnings.is_empty());
    assert_eq!(c.display_host, "example.com");
    assert!(!c.is_http);
}

#[test]
fn text_host_mismatch_names_the_text_host() {
    let w = warns(
        "https://example.net/login",
        Some("https://example.com/login"),
    );
    assert_eq!(
        w,
        vec![LinkWarning::TextHostMismatch {
            text_host: "example.com".into()
        }]
    );
}

#[test]
fn mismatch_forms() {
    let mismatch = |text: &str| {
        warns("https://example.net/", Some(text))
            .iter()
            .any(|w| matches!(w, LinkWarning::TextHostMismatch { .. }))
    };
    assert!(mismatch("www.example.com"));
    assert!(mismatch("example.com"));
    assert!(mismatch("EXAMPLE.com/path"));
    // Same organisation, www and case are ignored.
    assert!(!mismatch("www.example.net"));
    assert!(!mismatch("Mail.Example.NET"));
    // Not a URL or domain.
    assert!(!mismatch("Click here"));
    assert!(!mismatch("report.pdf"));
    assert!(!mismatch("v1.2"));
    assert!(!mismatch(""));
}

#[test]
fn text_is_compared_in_punycode() {
    let c = check_link(
        "https://xn--e1afmkfd.example.org/",
        Some("пример.example.org"),
    );
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
}

#[test]
fn ip_literals() {
    assert!(warns("https://192.0.2.1/", None).contains(&LinkWarning::IpAddress));
    assert!(warns("https://[2001:db8::1]/", None).contains(&LinkWarning::IpAddress));
    assert!(!warns("https://example.com/", None).contains(&LinkWarning::IpAddress));
}

#[test]
fn user_info() {
    let c = check_link("https://example.com@example.net/", None);
    assert!(c.warnings.contains(&LinkWarning::UserInfo));
    assert_eq!(c.display_host, "example.net");
}

#[test]
fn http_is_flagged_not_secure() {
    let c = check_link("http://example.com/", None);
    assert!(c.is_http);
    assert_eq!(c.warnings, vec![LinkWarning::Http]);
}

#[test]
fn other_schemes_are_unsupported() {
    for href in [
        "javascript:alert(1)",
        "mailto:a@example.com",
        "ftp://example.com/",
        "nonsense",
    ] {
        assert_eq!(
            warns(href, None),
            vec![LinkWarning::UnsupportedScheme],
            "{href}"
        );
    }
}

#[test]
fn single_script_idn_shows_unicode() {
    let c = check_link("https://пример.рф/", None);
    assert!(c.warnings.is_empty());
    assert_eq!(c.display_host, "пример.рф");
    let c = check_link("https://例え.中国/", None);
    assert!(c.warnings.is_empty());
    assert_eq!(c.display_host, "例え.中国");
}

#[test]
fn mixed_script_shows_punycode() {
    // Latin "exampl" plus Cyrillic "е".
    let c = check_link("https://exampl\u{435}.com/", None);
    assert!(c.warnings.contains(&LinkWarning::MixedScript));
    assert!(c.display_host.starts_with("xn--"), "{}", c.display_host);
    assert_eq!(c.display_host, c.ascii_host);
}

#[test]
fn unknown_script_counts_as_mixed() {
    // Ethiopic is not in the table.
    let c = check_link("https://\u{1200}\u{1201}.example.com/", None);
    assert!(c.warnings.contains(&LinkWarning::MixedScript));
}

#[test]
fn too_long_skips_the_text_comparison() {
    let href = format!("https://example.net/{}", "a".repeat(3000));
    let c = check_link(&href, Some("example.com"));
    assert_eq!(c.warnings, vec![LinkWarning::TooLong]);
    assert_eq!(c.display_host, "example.net");
}

// Hostile input (regression tests for M5-07 part A).

fn has_mismatch(w: &[LinkWarning]) -> bool {
    w.iter()
        .any(|w| matches!(w, LinkWarning::TextHostMismatch { .. }))
}

#[test]
fn homograph_mixed_into_latin_labels_is_caught() {
    // Cyrillic а (U+0430), о (U+043E), р (U+0440) and Greek ο (U+03BF) in
    // otherwise Latin labels.
    for href in [
        "https://ex\u{0430}mple.com/",
        "https://examp\u{043E}le.org/",
        "https://\u{0440}aypal-example.net/",
        "https://ex\u{03BF}mple.com/",
        "https://login.ex\u{0430}mple.com/",
    ] {
        let c = check_link(href, None);
        assert!(c.warnings.contains(&LinkWarning::MixedScript), "{href}");
        assert!(
            c.display_host.contains("xn--"),
            "{href}: {}",
            c.display_host
        );
        assert!(c.display_host.is_ascii(), "{href}");
    }
}

#[test]
fn whole_script_lookalike_is_caught_by_the_text() {
    // An all-Cyrillic label that looks like "example": one script, so it is
    // shown in Unicode without MixedScript (rule A2), but text naming
    // example.com still differs from it once both are in punycode.
    let href = "https://\u{0435}\u{0445}\u{0430}\u{043C}\u{0440}\u{04CF}\u{0435}.com/";
    let c = check_link(href, Some("https://example.com/"));
    assert!(!c.warnings.contains(&LinkWarning::MixedScript));
    assert!(has_mismatch(&c.warnings), "{:?}", c.warnings);
}

#[test]
fn fullwidth_and_ideographic_dots_in_text_are_dots() {
    for text in [
        "example\u{FF0E}com",
        "example\u{3002}com",
        "www\u{FF61}example\u{FF0E}com",
    ] {
        assert!(
            has_mismatch(&warns("https://example.net/", Some(text))),
            "{text}"
        );
        assert!(
            !has_mismatch(&warns("https://example.com/", Some(text))),
            "{text}"
        );
    }
}

#[test]
fn user_info_tricks() {
    for (href, host) in [
        ("https://example.com@example.net/", "example.net"),
        ("https://example.com:443@example.net/", "example.net"),
        ("https://user:pass@example.net/", "example.net"),
        (
            "https://www.example.com%2Flogin@example.net/",
            "example.net",
        ),
        ("HTTPS://EXAMPLE.COM@EXAMPLE.NET/", "example.net"),
        ("https://example.com@192.0.2.1/", "192.0.2.1"),
    ] {
        let c = check_link(href, None);
        assert!(c.warnings.contains(&LinkWarning::UserInfo), "{href}");
        assert_eq!(c.display_host, host, "{href}");
    }
    // Text naming the fake host before the @ is a mismatch too.
    let w = warns(
        "https://example.com@example.net/",
        Some("https://example.com/"),
    );
    assert!(w.contains(&LinkWarning::UserInfo));
    assert!(has_mismatch(&w));
    // A backslash is a path separator in http(s): the host really is
    // example.com, so there is no user info to warn about.
    let c = check_link("https://example.com\\@example.net/", None);
    assert_eq!(c.display_host, "example.com");
    assert!(!c.warnings.contains(&LinkWarning::UserInfo));
}

#[test]
fn script_and_data_urls_are_never_web_links() {
    for href in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        " javascript:alert(1)",
        "\tjava\nscript:alert(1)",
        "javascript://example.com/%0Aalert(1)",
        "data:text/html,<script>alert(1)</script>",
        "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
        "vbscript:msgbox(1)",
        "file:///etc/passwd",
        "blob:https://example.com/0000",
        "intent://example.com#Intent;scheme=https;end",
        "",
        "   ",
        "//example.com/",
        "/relative/path",
    ] {
        let c = check_link(href, Some("https://example.com/"));
        assert_eq!(c.warnings, vec![LinkWarning::UnsupportedScheme], "{href:?}");
        assert!(c.display_host.is_empty(), "{href:?}");
    }
}

#[test]
fn length_limit_boundaries() {
    let base = "https://example.net/";
    let at_limit = format!("{base}{}", "a".repeat(MAX_LINK_BYTES - base.len()));
    assert_eq!(at_limit.len(), MAX_LINK_BYTES);
    assert!(!warns(&at_limit, None).contains(&LinkWarning::TooLong));
    let over = format!("{at_limit}a");
    assert!(warns(&over, None).contains(&LinkWarning::TooLong));
    // A huge link and huge text: no panic, and the host still comes back.
    let huge = format!("{base}?q={}", "%41".repeat(400_000));
    let c = check_link(&huge, Some(&"example.com".repeat(100_000)));
    assert_eq!(c.warnings, vec![LinkWarning::TooLong]);
    assert_eq!(c.display_host, "example.net");
    // A huge host is not a valid DNS name; still no panic.
    let _ = check_link(&format!("https://{}.example/", "a".repeat(100_000)), None);
}

#[test]
fn mismatched_text_variants() {
    let href = "https://example.net/login";
    for text in [
        "HTTPS://EXAMPLE.COM/login",
        "http://example.com",
        "https://www.example.com/login?next=/",
        "example.com/login",
        "www.example.com",
        "https://example.com:8443/",
        "https://192.0.2.1/",
        "https://example.co.uk/",
        "\u{00A0}example.com\u{00A0}",
    ] {
        assert!(has_mismatch(&warns(href, Some(text))), "{text:?}");
    }
    for text in [
        "https://example.net/",
        "https://login.example.net/other",
        "WWW.EXAMPLE.NET",
        "Log in to your account",
        "example.com is our partner",
        "Click https://example.com now",
        "invoice.pdf",
        "1.2.3",
        "...",
        "example.",
        ".com",
        "https://",
        "\u{200B}",
    ] {
        assert!(!has_mismatch(&warns(href, Some(text))), "{text:?}");
    }
    // Two-label public suffixes: different organisations under co.uk.
    assert!(has_mismatch(&warns(
        "https://example.co.uk/",
        Some("other-example.co.uk")
    )));
    assert!(!has_mismatch(&warns(
        "https://mail.example.co.uk/",
        Some("www.example.co.uk")
    )));
}

#[test]
fn ip_address_forms() {
    for href in [
        "https://192.0.2.1/",
        "https://3221225985/",
        "https://0xC0.0.2.1/",
        "https://0300.0.2.1/",
        "https://[::ffff:192.0.2.1]/",
        "https://[2001:db8::1]:8443/",
    ] {
        assert!(
            warns(href, None).contains(&LinkWarning::IpAddress),
            "{href}"
        );
    }
}

#[test]
fn link_text_lookup_uses_canonical_urls() {
    let links = [
        ("https://EXAMPLE.org/a", "first"),
        ("https://пример.рф/", "idn"),
        ("not a url", "junk"),
    ];
    assert_eq!(link_text_for("https://example.org/a", links), Some("first"));
    assert_eq!(
        link_text_for("https://xn--e1afmkfd.xn--p1ai/", links),
        Some("idn")
    );
    assert_eq!(link_text_for("https://example.org/b", links), None);
    assert_eq!(link_text_for(&"a".repeat(5000), links), None);
}

#[test]
fn whole_script_latin_lookalikes_show_in_punycode() {
    // All-Cyrillic or all-Greek labels made only of letters that look Latin,
    // under a Latin top-level domain (review of #267).
    for href in [
        // аррӏе.com
        "https://\u{0430}\u{0440}\u{0440}\u{04CF}\u{0435}.com/",
        // сосо.com
        "https://\u{0441}\u{043E}\u{0441}\u{043E}.com/",
        // ехаmрӏе is not all lookalikes (м); ехарӏе is.
        "https://\u{0435}\u{0445}\u{0430}\u{0440}\u{04CF}\u{0435}.net/",
        // In a subdomain, with digits and a hyphen.
        "https://login.\u{0430}\u{0440}\u{0440}\u{04CF}\u{0435}-2.example.com/",
        // Greek οκ.org
        "https://\u{03BF}\u{03BA}.org/",
        // Uppercase Cyrillic is lowercased by IDNA first.
        "https://\u{0410}\u{0420}\u{0420}\u{04C0}\u{0415}.com/",
    ] {
        let c = check_link(href, None);
        assert!(
            c.warnings.contains(&LinkWarning::LookalikeHost),
            "{href}: {:?}",
            c.warnings
        );
        assert!(!c.warnings.contains(&LinkWarning::MixedScript), "{href}");
        assert!(
            c.display_host.contains("xn--"),
            "{href}: {}",
            c.display_host
        );
        assert!(c.display_host.is_ascii(), "{href}");
        assert_eq!(c.display_host, c.ascii_host, "{href}");
    }
}

#[test]
fn real_words_in_their_own_script_stay_in_unicode() {
    for (href, shown) in [
        ("https://пример.рф/", "пример.рф"),
        ("https://例え.jp/", "例え.jp"),
        ("https://例え.中国/", "例え.中国"),
        // A Cyrillic word with letters that don't imitate Latin, even under .com.
        ("https://пример.com/", "пример.com"),
        // A lookalike label under its own script's domain is left alone.
        (
            "https://\u{0430}\u{0440}\u{0440}\u{04CF}\u{0435}.рф/",
            "\u{0430}\u{0440}\u{0440}\u{04CF}\u{0435}.рф",
        ),
        // Greek word with a non-lookalike letter (λ).
        ("https://\u{03BB}\u{03BF}.org/", "\u{03BB}\u{03BF}.org"),
    ] {
        let c = check_link(href, None);
        assert!(c.warnings.is_empty(), "{href}: {:?}", c.warnings);
        assert_eq!(c.display_host, shown, "{href}");
    }
}

#[test]
fn lookalike_and_text_warnings_both_show() {
    // The dialog names both the lookalike host and the text it imitates.
    let w = warns(
        "https://\u{0430}\u{0440}\u{0440}\u{04CF}\u{0435}.com/",
        Some("apple.com"),
    );
    assert!(w.contains(&LinkWarning::LookalikeHost));
    assert!(has_mismatch(&w));
    // With text that isn't a host, the lookalike warning stands alone.
    let w = warns(
        "https://\u{0430}\u{0440}\u{0440}\u{04CF}\u{0435}.com/",
        Some("Sign in"),
    );
    assert_eq!(w, vec![LinkWarning::LookalikeHost]);
}

#[test]
fn imitates_latin_edge_cases() {
    assert!(!imitates_latin(""));
    assert!(!imitates_latin("com"));
    assert!(!imitates_latin("example.com"));
    assert!(!imitates_latin("..."));
    assert!(imitates_latin("\u{0430}.com."));
    assert!(!imitates_latin("\u{0430}"));
}

proptest! {
    #[test]
    fn never_panics(href in "\\PC{0,300}", text in proptest::option::of("\\PC{0,300}")) {
        let _ = check_link(&href, text.as_deref());
    }

    #[test]
    fn url_shaped_input_never_panics(
        host in "[a-z\u{0400}-\u{04FF}\u{FF0E}.@:\\[\\]0-9-]{0,60}",
        path in "\\PC{0,60}",
        text in proptest::option::of("[a-z\u{0400}-\u{04FF}\u{3002}.:/@0-9-]{0,60}"),
    ) {
        let href = format!("https://{host}/{path}");
        let c = check_link(&href, text.as_deref());
        // A host that has an ASCII form is never shown as empty.
        prop_assert!(c.ascii_host.is_empty() || !c.display_host.is_empty());
        let _ = link_text_for(&href, [(href.as_str(), "t")]);
    }
}
