use super::*;

fn mailto(address: &str, subject: Option<&str>, body: Option<&str>) -> MailtoTarget {
    MailtoTarget {
        address: address.into(),
        subject: subject.map(Into::into),
        body: body.map(Into::into),
    }
}

/// Header, Post header, https URIs, mailto targets, one-click.
type Case<'a> = (
    &'a str,
    Option<&'a str>,
    Vec<&'a str>,
    Vec<MailtoTarget>,
    bool,
);

#[test]
fn list_unsubscribe_table() {
    let one_click = Some("List-Unsubscribe=One-Click");
    let cases: Vec<Case<'_>> = vec![
        (
            "<mailto:unsub@example.net?subject=Remove%20me>, <https://example.net/u/abc>",
            one_click,
            vec!["https://example.net/u/abc"],
            vec![mailto("unsub@example.net", Some("Remove me"), None)],
            true,
        ),
        // Folding and whitespace inside and between the brackets.
        (
            "<https://example.net/u/\r\n  abc?x=1>,\r\n\t<mailto:u@example.net?body=Please%0Aremove>",
            Some("  List-Unsubscribe = One-Click "),
            vec!["https://example.net/u/abc?x=1"],
            vec![mailto("u@example.net", None, Some("Please\nremove"))],
            true,
        ),
        // http: is ignored, so no one-click even with the Post header.
        ("<http://example.net/u/abc>", one_click, vec![], vec![], false),
        // https without the Post header: a web page only.
        ("<https://example.net/u>", None, vec!["https://example.net/u"], vec![], false),
        // A Post header with another value.
        ("<https://example.net/u>", Some("List-Unsubscribe=Yes"), vec!["https://example.net/u"], vec![], false),
        // Several of each, in order.
        (
            "<https://a.example/1>, <https://b.example/2>, <mailto:x@a.example>, <mailto:y@b.example>",
            one_click,
            vec!["https://a.example/1", "https://b.example/2"],
            vec![mailto("x@a.example", None, None), mailto("y@b.example", None, None)],
            true,
        ),
        // Garbage.
        ("", one_click, vec![], vec![], false),
        ("no brackets here https://example.net", one_click, vec![], vec![], false),
        ("<javascript:alert(1)>, <ftp://example.net/x>, <data:text/html,x>", one_click, vec![], vec![], false),
        ("<https://>, <https://user@example.net/x>, <https://example.net/\"x>", one_click, vec![], vec![], false),
        ("<mailto:>, <mailto:a@b@c>, <mailto:a@x.example,b@y.example>, <mailto:%FF@x.example>", None, vec![], vec![], false),
        ("<https://example.net/u", one_click, vec![], vec![], false),
    ];
    for (header, post, https, mailtos, one) in cases {
        let got = parse_list_unsubscribe(header, post);
        assert_eq!(got.https, https, "{header:?}");
        assert_eq!(got.mailto, mailtos, "{header:?}");
        assert_eq!(got.one_click, one, "{header:?}");
    }
    let long = format!("<https://example.net/{}>", "a".repeat(3000));
    assert!(parse_list_unsubscribe(&long, None).is_empty());
}

#[test]
fn dkim_results() {
    let header = "mx.example.org; spf=pass smtp.mailfrom=news.example.net; \
                  dkim=pass (2048-bit key) header.d=example.net header.s=s1; \
                  dkim=fail header.d=evil.example; dkim=pass header.i=@Mail.Example.COM.; dmarc=pass";
    assert_eq!(
        dkim_pass_domains(header),
        ["example.net", "mail.example.com"]
    );
    assert!(dkim_pass_domains("mx.example.org; dkim=fail header.d=example.net").is_empty());
    assert!(dkim_pass_domains("mx.example.org; none").is_empty());
    assert!(dkim_pass_domains("").is_empty());
    // A comment can't smuggle in a result.
    assert!(
        dkim_pass_domains("mx.example.org; dkim=none (dkim=pass header.d=example.net)").is_empty()
    );
}

#[test]
fn alignment() {
    assert!(dkim_aligned("example.net", "example.net"));
    assert!(dkim_aligned("example.net", "news.example.net"));
    assert!(dkim_aligned("mail.example.net", "example.net"));
    assert!(dkim_aligned("Example.NET.", "example.net"));
    assert!(!dkim_aligned("example.org", "example.net"));
    assert!(!dkim_aligned("notexample.net", "example.net"));
    assert!(!dkim_aligned("co.uk", "shop.co.uk"));
    assert!(!dkim_aligned("net", "example.net"));
    assert!(dkim_aligned("shop.co.uk", "news.shop.co.uk"));
    assert!(!dkim_aligned("", "example.net"));
}

#[test]
fn org_domains() {
    assert_eq!(org_domain("mx1.mail.Example.COM."), "example.com");
    assert_eq!(org_domain("news.shop.co.uk"), "shop.co.uk");
    assert_eq!(org_domain("example.net"), "example.net");
    assert_eq!(org_domain("net"), "net");
}

#[test]
fn received_and_provider_results() {
    assert_eq!(
        received_by("from out.example.net (out.example.net [192.0.2.1]) by mx2.example.org (Postfix) with ESMTPS id 1; Wed, 25 Nov 2026 08:00:00 +0000").as_deref(),
        Some("mx2.example.org")
    );
    assert_eq!(received_by("from x (by evil.example) with SMTP"), None);
    assert_eq!(received_by("garbage"), None);
    let results = vec![
        "evil.example; dkim=pass header.d=example.net".to_owned(),
        "mx1.example.org; dkim=pass header.d=example.net".to_owned(),
        "mx9.example.org; dkim=fail".to_owned(),
    ];
    assert_eq!(
        provider_results(&results, Some("mx2.example.org")),
        Some("mx1.example.org; dkim=pass header.d=example.net")
    );
    assert_eq!(provider_results(&results, Some("mx.other.example")), None);
    assert_eq!(provider_results(&results, None), None);
    assert_eq!(provider_results(&[], Some("mx2.example.org")), None);
}
