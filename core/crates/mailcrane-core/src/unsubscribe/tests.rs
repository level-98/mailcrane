use super::*;

fn raw(auth: Option<&str>, from: &str, list: &str, post: Option<&str>) -> Vec<u8> {
    // The provider's results sit above its own Received header, where the
    // message entered it (M5-07: only those can be trusted).
    let mut m = String::new();
    if let Some(a) = auth {
        m.push_str(&format!("Authentication-Results: {a}\r\n"));
    }
    m.push_str(
        "Received: from out.example.net by mx2.example.org with ESMTPS; \
         Wed, 25 Nov 2026 08:00:00 +0000\r\n",
    );
    // Results the sender forged must not count, wherever they are and
    // whatever authserv-id they name.
    m.push_str("Authentication-Results: evil.example; dkim=pass header.d=example.net\r\n");
    m.push_str("Authentication-Results: mx.example.org; dkim=pass header.d=example.net\r\n");
    m.push_str(
        "Received: from sender.example.net by out.example.net; Wed, 25 Nov 2026 07:59:00 +0000\r\n",
    );
    m.push_str(&format!(
        "From: News <{from}>\r\nTo: alice@example.org\r\nSubject: Weekly\r\n"
    ));
    m.push_str(&format!("List-Unsubscribe: {list}\r\n"));
    if let Some(p) = post {
        m.push_str(&format!("List-Unsubscribe-Post: {p}\r\n"));
    }
    m.push_str("\r\nHello\r\n");
    m.into_bytes()
}

const LIST: &str = "<mailto:unsub@example.net?subject=stop>, <https://example.net/u/1>";
const POST: Option<&str> = Some("List-Unsubscribe=One-Click");

#[test]
fn one_click_needs_an_aligned_dkim_pass_from_the_top_header() {
    let ok = "mx.example.org; dkim=pass header.d=example.net; spf=pass";
    let o = options_from_raw(&raw(Some(ok), "news@example.net", LIST, POST)).expect("options");
    assert_eq!(o.one_click.as_deref(), Some("https://example.net/u/1"));
    assert_eq!(o.sender, "news@example.net");
    assert_eq!(
        o.mailto.as_ref().map(|m| m.address.as_str()),
        Some("unsub@example.net")
    );
    assert_eq!(o.web.as_deref(), Some("https://example.net/u/1"));

    // A subdomain of the signing domain is aligned.
    let o = options_from_raw(&raw(Some(ok), "news@mail.example.net", LIST, POST)).expect("options");
    assert!(o.one_click.is_some());

    for auth in [
        // No results from the provider: the forged one below doesn't count.
        None,
        Some("mx.example.org; dkim=fail header.d=example.net"),
        Some("mx.example.org; dkim=pass header.d=other.example"),
        Some("mx.example.org; dkim=none"),
    ] {
        let o = options_from_raw(&raw(auth, "news@example.net", LIST, POST)).expect("options");
        assert_eq!(o.one_click, None, "{auth:?}");
        assert!(o.mailto.is_some() && o.web.is_some(), "falls back");
    }
}

#[test]
fn without_the_post_header_there_is_no_one_click() {
    let ok = Some("mx.example.org; dkim=pass header.d=example.net");
    let o = options_from_raw(&raw(ok, "news@example.net", LIST, None)).expect("options");
    assert_eq!(o.one_click, None);
    let o = options_from_raw(&raw(ok, "news@example.net", "<http://example.net/u>", POST));
    assert_eq!(o, None, "http only: nothing to offer");
    let o = options_from_raw(&raw(ok, "news@example.net", "<mailto:u@example.net>", POST))
        .expect("options");
    assert_eq!((o.one_click, o.web), (None, None));
}

#[test]
fn messages_without_the_header_offer_nothing() {
    assert_eq!(options_from_raw(b"From: a@example.net\r\n\r\nHi\r\n"), None);
    assert_eq!(options_from_raw(b""), None);
}

#[test]
fn the_request_is_exactly_rfc_8058() {
    let url = url::Url::parse("https://example.net/u/1?id=a%20b").expect("url");
    assert_eq!(
        http::request(&url).expect("request"),
        "POST /u/1?id=a%20b HTTP/1.1\r\nHost: example.net\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: 26\r\nConnection: close\r\n\r\nList-Unsubscribe=One-Click"
    );
    let url = url::Url::parse("https://example.net:8443/").expect("url");
    assert!(http::request(&url)
        .expect("request")
        .contains("Host: example.net:8443\r\n"));
}

#[test]
fn status_lines() {
    assert_eq!(http::parse_status(b"HTTP/1.1 200 OK\r\n"), Some(200));
    assert_eq!(
        http::parse_status(b"HTTP/1.0 302 Found\r\nLocation: x\r\n"),
        Some(302)
    );
    assert_eq!(http::parse_status(b"HTTP/1.1 204\r\n"), Some(204));
    assert_eq!(http::parse_status(b"HTTP/2 200\r\n"), None);
    assert_eq!(http::parse_status(b"garbage\r\n"), None);
    assert_eq!(http::parse_status(b"HTTP/1.1 99 x\r\n"), None);
    assert_eq!(http::parse_status(b"HTTP/1.1 abc x\r\n"), None);
}
