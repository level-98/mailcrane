use super::*;
use crate::oauth::begin;

#[test]
fn the_configuration_is_googles() {
    let p = provider().expect("provider");
    assert!(
        p.check().is_ok(),
        "https endpoints, client id and redirect set"
    );
    assert_eq!(
        p.authorization_endpoint.host_str(),
        Some("accounts.google.com")
    );
    assert_eq!(p.token_endpoint.host_str(), Some("oauth2.googleapis.com"));
    assert_eq!(p.scopes, ["https://mail.google.com/"]);
    assert!(p.client_id.ends_with(".apps.googleusercontent.com"));
    assert_eq!(p.redirect_uri, "app.mailcrane:/oauth2redirect");
    assert_eq!(super::super::by_id("google"), Some(p));
    assert_eq!(super::super::by_id("example"), None);
}

#[test]
fn the_authorization_url_has_pkce_the_hint_and_consent() {
    let (request, pending) =
        begin(&provider().expect("provider"), Some("ann@gmail.com")).expect("begin");
    let q: std::collections::HashMap<String, String> =
        request.url.query_pairs().into_owned().collect();
    assert_eq!(q.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(
        q.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(
        q.get("login_hint").map(String::as_str),
        Some("ann@gmail.com")
    );
    assert_eq!(q.get("prompt").map(String::as_str), Some("consent"));
    assert_eq!(q.get("scope").map(String::as_str), Some(SCOPE));
    assert_eq!(q.get("state").map(String::as_str), Some(pending.state()));
}

#[test]
fn gmail_addresses() {
    for yes in ["ann@gmail.com", "Ann@GoogleMail.com", "a@gmail.com."] {
        assert!(is_gmail_address(yes), "{yes}");
    }
    for no in [
        "ann@example.com",
        "ann@notgmail.com",
        "ann@gmail.com.example.net",
        "@gmail.com",
        "",
    ] {
        assert!(!is_gmail_address(no), "{no}");
    }
}
