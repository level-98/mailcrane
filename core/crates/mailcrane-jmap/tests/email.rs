//! JMAP emails (M3-04 part 2) against the testkit's fake server.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_jmap::{
    discover, download_blob, email_changes, get_emails, query_mailbox, Auth, ClientConfig,
    JmapError, Session,
};
use mailcrane_testkit::{FakeEmail, FakeJmap};
use std::time::Duration;

fn email(id: &str, mailbox: &str, at: &str, seen: bool) -> FakeEmail {
    FakeEmail {
        id: id.into(),
        mailbox_id: mailbox.into(),
        received_at: at.into(),
        subject: format!("Subject {id}"),
        from: "ana@example.org".into(),
        seen,
        raw: format!("From: ana@example.org\r\nSubject: Subject {id}\r\n\r\nBody {id}\r\n"),
    }
}

async fn setup() -> (FakeJmap, ClientConfig, Auth, Session) {
    let server = FakeJmap::builder()
        .email(email("m1", "mb-inbox", "2026-11-20T08:00:00Z", true))
        .email(email("m2", "mb-inbox", "2026-11-25T08:00:00Z", false))
        .email(email("m3", "mb-inbox", "2026-11-22T08:00:00Z", false))
        .email(email("s1", "mb-sent", "2026-11-24T08:00:00Z", true))
        .start()
        .await
        .expect("server");
    let cfg = ClientConfig {
        timeout: Duration::from_secs(5),
        extra_roots: vec![server.root_cert_der().to_vec()],
    };
    let auth = Auth::Bearer("token".into());
    let session = discover(&server.base_url(), &auth, &cfg)
        .await
        .expect("session");
    (server, cfg, auth, session)
}

#[tokio::test]
async fn a_mailbox_pages_newest_first_and_emails_come_back() {
    let (_server, cfg, auth, session) = setup().await;
    let first = query_mailbox(&session, &auth, &cfg, "mb-inbox", 0, 2)
        .await
        .expect("query");
    assert_eq!(first.ids, ["m2", "m3"]);
    assert_eq!(first.total, Some(3));
    let second = query_mailbox(&session, &auth, &cfg, "mb-inbox", 2, 2)
        .await
        .expect("query");
    assert_eq!(second.ids, ["m1"]);
    assert_eq!(second.position, 2);

    let (state, emails) = get_emails(&session, &auth, &cfg, &first.ids)
        .await
        .expect("get");
    assert_eq!(state, "e1");
    assert_eq!(emails.len(), 2);
    assert_eq!(emails[0].subject, "Subject m2");
    assert_eq!(emails[0].mailbox_ids, ["mb-inbox"]);
    assert!(!emails[0].is_seen());
    assert_eq!(emails[0].from[0].email, "ana@example.org");
    assert_eq!(emails[0].message_id.as_deref(), Some("m2@example.org"));
    assert_eq!(emails[0].received_at, 1_795_593_600);
}

#[tokio::test]
async fn changes_and_the_resync_fallback() {
    let (_server, cfg, auth, session) = setup().await;
    let none = email_changes(&session, &auth, &cfg, "e1", 100)
        .await
        .expect("changes");
    assert!(none.created.is_empty() && none.updated.is_empty());
    assert_eq!(none.new_state, "e1");
    let all = email_changes(&session, &auth, &cfg, "e0", 100)
        .await
        .expect("changes");
    assert_eq!(all.created.len(), 4);
    let err = email_changes(&session, &auth, &cfg, "ancient", 100)
        .await
        .expect_err("too old");
    assert_eq!(err, JmapError::Method("cannotCalculateChanges".into()));
}

#[tokio::test]
async fn the_raw_message_downloads() {
    let (_server, cfg, auth, session) = setup().await;
    let (_, emails) = get_emails(&session, &auth, &cfg, &["m1".to_owned()])
        .await
        .expect("get");
    let raw = download_blob(&session, &auth, &cfg, &emails[0].blob_id)
        .await
        .expect("download");
    assert!(String::from_utf8_lossy(&raw).contains("Body m1"));
    let missing = download_blob(&session, &auth, &cfg, "blob-nope")
        .await
        .expect_err("missing");
    assert_eq!(missing, JmapError::Http(404));
}
