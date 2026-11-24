//! Removing tracking parameters from links before they are opened.

mod params;

/// Removes known tracking parameters (`utm_*`, `fbclid`, `gclid` and others,
/// listed in `links/params.rs`) from an `http` or `https` link.
///
/// Everything else is kept byte for byte: the scheme, host (including
/// internationalised names), path, the order and exact encoding of the other
/// parameters, and the `#fragment`. The `?` goes when no parameters are left.
/// Other schemes (`mailto:` and so on) and input that does not parse as a URL
/// come back unchanged. Running it twice gives the same result as once.
pub fn clean_url(input: &str) -> String {
    match clean(input) {
        Some(cleaned) => cleaned,
        None => input.to_string(),
    }
}

/// The cleaned link, or `None` to leave `input` as it is.
fn clean(input: &str) -> Option<String> {
    // Parsing only decides whether this is a valid http(s) URL. The output is
    // built from `input` itself, because `Url` normalises (IDN hosts become
    // punycode, escapes change) and the link must stay as the sender wrote it.
    let parsed = url::Url::parse(input).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    // The fragment starts at the first `#`, and the query at the first `?`
    // before it (RFC 3986 §3).
    let (before_fragment, fragment) = match input.find('#') {
        Some(at) => (input.get(..at)?, input.get(at..)?),
        None => (input, ""),
    };
    let at = before_fragment.find('?')?;
    let base = before_fragment.get(..at)?;
    let query = before_fragment.get(at + 1..)?;

    let mut removed = false;
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split_once('=').map_or(*pair, |(name, _)| name);
            let tracking = params::is_tracking(name);
            removed |= tracking;
            !tracking
        })
        .collect();
    if !removed {
        return None;
    }

    let mut out = String::with_capacity(input.len());
    out.push_str(base);
    // Empty segments (`a=1&&b=2`) are kept as they were; `?` is dropped only
    // when nothing at all is left.
    if !kept.iter().all(|pair| pair.is_empty()) {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    out.push_str(fragment);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn table() {
        let cases: &[(&str, &str, &str)] = &[
            (
                "only tracking params",
                "https://example.com/a?utm_source=news&utm_medium=email&fbclid=x1",
                "https://example.com/a",
            ),
            (
                "mixed params keep their order",
                "https://example.com/p?id=7&utm_source=n&lang=en&gclid=g&page=2",
                "https://example.com/p?id=7&lang=en&page=2",
            ),
            (
                "no params",
                "https://example.com/path/to/page",
                "https://example.com/path/to/page",
            ),
            (
                "empty query left alone",
                "https://example.com/?",
                "https://example.com/?",
            ),
            (
                "fragment kept",
                "https://example.com/doc?utm_campaign=c&x=1#section-2",
                "https://example.com/doc?x=1#section-2",
            ),
            (
                "fragment kept when no params remain",
                "https://example.com/doc?utm_campaign=c#top",
                "https://example.com/doc#top",
            ),
            (
                "question mark in fragment is not a query",
                "https://example.com/doc#a?utm_source=x",
                "https://example.com/doc#a?utm_source=x",
            ),
            (
                "repeated params",
                "https://example.com/?tag=a&utm_source=x&tag=b&utm_source=y",
                "https://example.com/?tag=a&tag=b",
            ),
            (
                "encoded values keep their exact encoding",
                "https://example.com/s?q=caf%C3%A9+au+lait&r=%2Fa%2Fb&mc_eid=abc%3D",
                "https://example.com/s?q=caf%C3%A9+au+lait&r=%2Fa%2Fb",
            ),
            (
                "utm_ prefix is case-insensitive",
                "https://example.com/?UTM_Source=x&Utm_Medium=y&keep=1",
                "https://example.com/?keep=1",
            ),
            (
                "exact names are case-sensitive",
                "https://example.com/?FBCLID=x&fbclid=y",
                "https://example.com/?FBCLID=x",
            ),
            (
                "names that only look like a tracker are kept",
                "https://example.com/?gclid_info=1&utm=2&xutm_source=3",
                "https://example.com/?gclid_info=1&utm=2&xutm_source=3",
            ),
            (
                "param without a value",
                "https://example.com/?fbclid&a",
                "https://example.com/?a",
            ),
            (
                "empty segments kept",
                "https://example.com/?a=1&&utm_term=t&b=2",
                "https://example.com/?a=1&&b=2",
            ),
            (
                "http works too",
                "http://example.org/?yclid=1&n=2",
                "http://example.org/?n=2",
            ),
            (
                "uppercase scheme",
                "HTTPS://example.org/?utm_id=1",
                "HTTPS://example.org/",
            ),
            (
                "IDN host is not converted to punycode",
                "https://bücher.example/katalog?utm_source=n&seite=2",
                "https://bücher.example/katalog?seite=2",
            ),
            (
                "port, user info and path kept exactly",
                "https://user@example.net:8443/a/%7Eb/?_hsenc=p&x=y",
                "https://user@example.net:8443/a/%7Eb/?x=y",
            ),
            (
                "mailto unchanged",
                "mailto:news@example.com?subject=hi&utm_source=x",
                "mailto:news@example.com?subject=hi&utm_source=x",
            ),
            (
                "other schemes unchanged",
                "ftp://example.com/f?utm_source=x",
                "ftp://example.com/f?utm_source=x",
            ),
            (
                "invalid URL unchanged",
                "https://exa mple.com/?utm_source=x",
                "https://exa mple.com/?utm_source=x",
            ),
            (
                "relative link unchanged",
                "/path?utm_source=x",
                "/path?utm_source=x",
            ),
            ("empty input", "", ""),
        ];
        for (name, input, expected) in cases {
            assert_eq!(clean_url(input), *expected, "{name}");
            assert_eq!(clean_url(expected), *expected, "{name} (idempotent)");
        }
    }

    #[test]
    fn every_listed_param_is_removed() {
        for name in params::EXACT {
            let link = format!("https://example.com/?{name}=1&keep=2");
            assert_eq!(clean_url(&link), "https://example.com/?keep=2", "{name}");
        }
    }

    proptest! {
        #[test]
        fn never_panics_and_is_idempotent(input in any::<String>()) {
            let once = clean_url(&input);
            prop_assert_eq!(clean_url(&once), once);
        }

        #[test]
        fn idempotent_and_complete_on_url_shaped_input(
            path in "[a-z/%0-9]{0,12}",
            pairs in proptest::collection::vec(
                ("(utm_[a-z]{1,5}|UTM_X|fbclid|gclid|[a-z]{1,6}|)", "[a-z0-9%=]{0,6}"),
                0..6,
            ),
            fragment in proptest::option::of("[a-z?&=#]{0,8}"),
        ) {
            let query: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let mut link = format!("https://example.com/{path}?{}", query.join("&"));
            if let Some(fragment) = fragment {
                link.push('#');
                link.push_str(&fragment);
            }
            let once = clean_url(&link);
            prop_assert_eq!(clean_url(&once), once.clone());
            // No tracking parameter is left in the query.
            let before_fragment = once.split('#').next().unwrap_or("");
            let query = before_fragment.split_once('?').map_or("", |(_, q)| q);
            for pair in query.split('&') {
                let name = pair.split('=').next().unwrap_or("");
                prop_assert!(!params::is_tracking(name), "{} left in {}", name, once);
            }
        }
    }
}
