use super::*;
use proptest::prelude::*;

fn claimed(name: &str, email: &str) -> Option<String> {
    display_name_warning(name, email).map(|w| w.claimed)
}

#[test]
fn an_address_from_another_organisation_warns() {
    let w = display_name_warning("ceo@example.com", "attacker@example.net").expect("warns");
    assert_eq!(w.claimed, "ceo@example.com");
    assert_eq!(w.actual, "attacker@example.net");
    // Inside brackets, quotes or among other words.
    for name in [
        "Boss <ceo@example.com>",
        "\"ceo@example.com\"",
        "[ceo@example.com]",
        "The CEO (ceo@example.com) via portal",
        "CEO@EXAMPLE.COM",
    ] {
        assert_eq!(
            claimed(name, "x@example.net").as_deref(),
            Some("ceo@example.com"),
            "{name}"
        );
    }
}

#[test]
fn the_same_organisation_does_not_warn() {
    for (name, email) in [
        ("Support (support@example.com)", "noreply@mail.example.com"),
        ("support@example.com", "noreply@example.com"),
        ("x@example.net", "x@example.net"),
        ("Billing <billing@news.Example.COM>", "bounce@example.com"),
        ("a@shop.co.uk", "b@mail.shop.co.uk"),
    ] {
        assert_eq!(claimed(name, email), None, "{name} <{email}>");
    }
    assert_eq!(
        claimed("a@other.co.uk", "b@shop.co.uk").as_deref(),
        Some("a@other.co.uk")
    );
}

#[test]
fn fullwidth_and_zero_width_tricks_still_warn() {
    for name in [
        "ceo\u{FF20}example.com",
        "ceo@example\u{FF0E}com",
        "ceo\u{FE6B}example\u{FE52}com",
        "ceo@exa\u{200B}mple.com",
        "c\u{200D}eo@example.\u{FEFF}com",
        "ceo\u{2060}@example.com",
    ] {
        assert_eq!(
            claimed(name, "x@example.net").as_deref(),
            Some("ceo@example.com"),
            "{name:?}"
        );
    }
}

#[test]
fn bidi_controls_cannot_hide_an_address() {
    // #314. RLO: stored reversed, shown as "ceo@example.com".
    assert_eq!(
        claimed("\u{202E}moc.elpmaxe@oec", "x@example.net").as_deref(),
        Some("ceo@example.com")
    );
    // RLO up to a PDF, with ordinary text after it.
    assert_eq!(
        claimed("\u{202E}moc.elpmaxe@oec\u{202C} (Billing)", "x@example.net").as_deref(),
        Some("ceo@example.com")
    );
    // LRO, RLI, FSI and PDI don't reverse Latin text: they're dropped.
    for name in [
        "\u{202D}ceo@example.com\u{202C}",
        "ceo@\u{2067}example.com\u{2069}",
        "\u{2068}ceo@example.com\u{2069}",
        "ceo\u{2069}@example.com",
    ] {
        assert_eq!(
            claimed(name, "x@example.net").as_deref(),
            Some("ceo@example.com"),
            "{name:?}"
        );
    }
    // The claimed address never carries a control.
    let w = display_name_warning("\u{202E}moc.elpmaxe@oec", "x@example.net").expect("warns");
    assert!(!w.claimed.chars().any(mailcrane_mime::is_bidi_control));
    // The same organisation, reversed or not, doesn't warn; nor does a plain name.
    assert_eq!(
        claimed("\u{202E}moc.elpmaxe@nna", "ann@mail.example.com"),
        None
    );
    assert_eq!(
        claimed("Ann\u{2069} \u{202E}Example", "ann@example.org"),
        None
    );
}

#[test]
fn any_tld_and_international_domains() {
    assert_eq!(
        claimed("ceo@example.photography", "x@example.net").as_deref(),
        Some("ceo@example.photography")
    );
    // The same domain in Unicode and in punycode is the same organisation.
    assert_eq!(claimed("admin@пример.рф", "x@xn--e1afmkfd.xn--p1ai"), None);
    assert_eq!(
        claimed("admin@пример.рф", "x@example.net").as_deref(),
        Some("admin@пример.рф")
    );
}

#[test]
fn any_foreign_address_in_the_name_warns() {
    assert_eq!(
        claimed("me@example.net / boss@example.com", "me@example.net").as_deref(),
        Some("boss@example.com")
    );
}

#[test]
fn a_bare_domain_name_from_elsewhere_warns() {
    assert_eq!(
        claimed("example.com", "x@example.net").as_deref(),
        Some("example.com")
    );
    assert_eq!(
        claimed("  \"Example.COM\" ", "x@example.net").as_deref(),
        Some("example.com")
    );
    assert_eq!(claimed("Example.com", "news@mail.example.com"), None);
}

#[test]
fn ordinary_names_do_not_warn() {
    for name in [
        "",
        "Ann Example",
        "Mr. Smith",
        "J.R",
        "v1.2",
        "Dr. Ann Example, Ph.D.",
        "Ann @ Example",
        "@example.com",
        "ann@",
        "ann@localhost",
        "ann@example.",
        "Ann (at) example dot com",
    ] {
        assert_eq!(claimed(name, "x@example.net"), None, "{name:?}");
    }
}

#[test]
fn no_real_address_means_no_warning() {
    assert_eq!(claimed("ceo@example.com", ""), None);
    assert_eq!(claimed("ceo@example.com", "undisclosed-recipients"), None);
}

#[test]
fn long_names_are_bounded() {
    let name = format!("{}ceo@example.com", "a".repeat(10_000));
    assert_eq!(claimed(&name, "x@example.net"), None);
    let name = format!("{}@example.com", "a".repeat(900));
    let w = display_name_warning(&name, "x@example.net").expect("warns");
    assert_eq!(w.claimed.chars().count(), MAX_SHOWN_CHARS);
    let _ = claimed(&"@".repeat(5_000), "x@example.net");
    let _ = claimed(&"a@b.".repeat(2_000), "x@example.net");
}

proptest! {
    #[test]
    fn never_panics(name in "\\PC{0,300}", email in "\\PC{0,80}") {
        let _ = display_name_warning(&name, &email);
    }

    #[test]
    fn address_shaped_names_never_panic(
        name in "[a-z\u{FF20}\u{FF0E}\u{200B}\u{0400}-\u{04FF}@.<>()\"' -]{0,80}",
        email in "[a-z@.\u{0400}-\u{04FF}-]{0,40}",
    ) {
        let _ = display_name_warning(&name, &email);
    }
}
