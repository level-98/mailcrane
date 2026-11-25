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

proptest! {
    #[test]
    fn never_panics(href in "\\PC{0,300}", text in proptest::option::of("\\PC{0,300}")) {
        let _ = check_link(&href, text.as_deref());
    }
}
