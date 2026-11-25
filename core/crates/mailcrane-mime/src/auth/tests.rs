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
