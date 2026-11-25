//! One-click unsubscribe (M4-10) against a local HTTPS server.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::unsubscribe::{post_one_click, UnsubscribeError};
use mailcrane_testkit::FakeHttps;
use std::time::Duration;

#[tokio::test]
async fn the_post_is_exactly_rfc_8058_with_nothing_identifying() {
    let server = FakeHttps::builder().start().await.expect("server");
    let roots = vec![server.root_cert_der().to_vec()];
    post_one_click(&server.url("/u/abc?list=7"), &roots)
        .await
        .expect("2xx is success");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    let r = &requests[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.target, "/u/abc?list=7");
    assert_eq!(r.body, b"List-Unsubscribe=One-Click");
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    let names: Vec<&str> = r.headers.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["host", "content-type", "content-length", "connection"]
    );
    for absent in ["cookie", "referer", "user-agent", "origin", "authorization"] {
        assert_eq!(r.header(absent), None, "{absent}");
    }
}

#[tokio::test]
async fn redirects_are_not_followed_and_count_as_failure() {
    let target = FakeHttps::builder().start().await.expect("server");
    let server = FakeHttps::builder()
        .status(302)
        .location(target.url("/elsewhere"))
        .start()
        .await
        .expect("server");
    let roots = vec![
        server.root_cert_der().to_vec(),
        target.root_cert_der().to_vec(),
    ];
    let err = post_one_click(&server.url("/u"), &roots)
        .await
        .expect_err("redirect");
    assert!(matches!(err, UnsubscribeError::Refused(302)), "{err:?}");
    assert!(target.requests().await.is_empty(), "not followed");
}

#[tokio::test]
async fn non_2xx_is_failure() {
    for status in [400, 404, 500, 503] {
        let server = FakeHttps::builder()
            .status(status)
            .start()
            .await
            .expect("server");
        let roots = vec![server.root_cert_der().to_vec()];
        let err = post_one_click(&server.url("/u"), &roots)
            .await
            .expect_err("fails");
        assert!(
            matches!(err, UnsubscribeError::Refused(s) if s == status),
            "{err:?}"
        );
    }
    let server = FakeHttps::builder()
        .status(204)
        .start()
        .await
        .expect("server");
    let roots = vec![server.root_cert_der().to_vec()];
    post_one_click(&server.url("/u"), &roots)
        .await
        .expect("204 is fine");
}

#[tokio::test]
async fn an_untrusted_certificate_is_refused_before_sending() {
    let server = FakeHttps::builder().start().await.expect("server");
    let err = post_one_click(&server.url("/u"), &[])
        .await
        .expect_err("untrusted");
    assert!(matches!(err, UnsubscribeError::Tls), "{err:?}");
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn http_urls_are_never_posted_to() {
    let err = post_one_click("http://localhost:1/u", &[])
        .await
        .expect_err("http");
    assert!(matches!(err, UnsubscribeError::NotOffered));
}

#[tokio::test]
async fn a_server_that_never_answers_times_out() {
    let server = FakeHttps::builder()
        .delay(Duration::from_secs(60))
        .start()
        .await
        .expect("server");
    let roots = vec![server.root_cert_der().to_vec()];
    let started = std::time::Instant::now();
    let err = post_one_click(&server.url("/u"), &roots)
        .await
        .expect_err("timeout");
    assert!(matches!(err, UnsubscribeError::Timeout), "{err:?}");
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(14) && took < Duration::from_secs(20),
        "{took:?}"
    );
}
