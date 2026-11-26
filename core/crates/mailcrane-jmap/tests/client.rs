//! The JMAP client (M3-04 part 1) against the testkit's fake server.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_jmap::{
    call, discover, get_mailboxes, Auth, ClientConfig, JmapError, MailboxRole, MethodCall,
};
use mailcrane_testkit::FakeJmap;
use serde_json::json;
use std::time::Duration;

/// Each request's limit (connect to last byte). Only a hang guard: a busy
/// machine can stall a request against the fake for seconds, and a 5 s
/// limit is the likely reason all four actions tests failed at once (#362).
const HANG_GUARD: Duration = Duration::from_secs(60);

fn config(server: &FakeJmap) -> ClientConfig {
    ClientConfig {
        timeout: HANG_GUARD,
        extra_roots: vec![server.root_cert_der().to_vec()],
    }
}

fn alice() -> Auth {
    Auth::Basic {
        username: "alice@example.org".into(),
        password: "app-password".into(),
    }
}

#[tokio::test]
async fn discovery_follows_the_well_known_redirect_and_lists_mailboxes() {
    let server = FakeJmap::builder()
        .user("alice@example.org", "app-password")
        .redirect_session()
        .mailbox("i", "Inbox", Some("inbox"), None)
        .mailbox("p", "Projects", None, Some("i"))
        .mailbox("t", "Bin", Some("trash"), None)
        .start()
        .await
        .expect("server");
    let cfg = config(&server);
    let input = format!("{}/.well-known/jmap", server.base_url());
    let session = discover(&input, &alice(), &cfg).await.expect("session");
    assert_eq!(session.mail_account_id, "A1");
    assert_eq!(session.username, "alice@example.org");
    assert_eq!(
        session.api_url.as_str(),
        format!("{}/api", server.base_url())
    );
    let paths: Vec<String> = server
        .requests()
        .await
        .into_iter()
        .map(|r| r.path)
        .collect();
    assert_eq!(paths, ["/.well-known/jmap", "/jmap/session"]);

    let (state, mailboxes) = get_mailboxes(&session, &alice(), &cfg)
        .await
        .expect("mailboxes");
    assert_eq!(state, "mb-state-1");
    let roles: Vec<(String, Option<MailboxRole>)> =
        mailboxes.iter().map(|m| (m.name.clone(), m.role)).collect();
    assert_eq!(
        roles,
        [
            ("Inbox".into(), Some(MailboxRole::Inbox)),
            ("Projects".into(), None),
            ("Bin".into(), Some(MailboxRole::Trash)),
        ]
    );
    assert_eq!(mailboxes[1].parent_id.as_deref(), Some("i"));
}

#[tokio::test]
async fn wrong_credentials_are_unauthorized() {
    let server = FakeJmap::builder()
        .user("alice@example.org", "app-password")
        .start()
        .await
        .expect("server");
    let wrong = Auth::Basic {
        username: "alice@example.org".into(),
        password: "nope".into(),
    };
    let err = discover(&server.base_url(), &wrong, &config(&server))
        .await
        .expect_err("refused");
    assert_eq!(err, JmapError::Unauthorized);
}

#[tokio::test]
async fn an_untrusted_certificate_is_refused() {
    let server = FakeJmap::builder().start().await.expect("server");
    let err = discover(&server.base_url(), &alice(), &ClientConfig::default())
        .await
        .expect_err("untrusted");
    assert_eq!(err, JmapError::Tls);
    assert!(server.requests().await.is_empty(), "nothing sent");
}

#[tokio::test]
async fn method_errors_come_back_per_call() {
    let server = FakeJmap::builder().start().await.expect("server");
    let cfg = config(&server);
    let session = discover(&server.base_url(), &Auth::Bearer("t".into()), &cfg)
        .await
        .expect("session");
    let responses = call(
        &session,
        &Auth::Bearer("t".into()),
        &cfg,
        &[
            MethodCall {
                name: "Mailbox/get".into(),
                args: json!({"accountId": "A1"}),
                id: "a".into(),
            },
            MethodCall {
                name: "Thing/explode".into(),
                args: json!({}),
                id: "b".into(),
            },
        ],
    )
    .await
    .expect("call");
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].id, "a");
    assert_eq!(
        responses[1].clone().into_result(),
        Err(JmapError::Method("unknownMethod".into()))
    );
}

#[tokio::test]
async fn http_urls_are_refused() {
    let err = discover("http://localhost:1/", &alice(), &ClientConfig::default())
        .await
        .expect_err("http");
    assert_eq!(err, JmapError::InvalidUrl);
}
