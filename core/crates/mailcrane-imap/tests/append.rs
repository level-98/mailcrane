//! `APPEND` and `UID SEARCH HEADER` against the fake server.

use mailcrane_imap::{AppendFlags, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_testkit::{FakeImap, Quirk};
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";

async fn session(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

fn draft(id: &str, body: &str) -> Vec<u8> {
    format!("From: alice@example.org\r\nSubject: Draft\r\nMessage-ID: <{id}>\r\n\r\n{body}\r\n")
        .into_bytes()
}

#[tokio::test]
async fn append_stores_the_exact_bytes_with_flags_and_reports_the_uid() {
    for (quirks, reports_uid) in [(vec![], true), (vec![Quirk::NoUidplus], false)] {
        let mut builder = FakeImap::builder()
            .user(USER, PASSWORD)
            .folder("INBOX", vec![])
            .folder("Drafts", vec![]);
        for q in quirks {
            builder = builder.quirk(q);
        }
        let imap = builder.start().await.expect("starts");
        let mut s = session(&imap).await;
        let raw = draft("d1@example.org", "First\r\n.\r\nline with {5}");
        let uid = s
            .append("Drafts", AppendFlags::Draft, &raw)
            .await
            .expect("appended");
        assert_eq!(uid.is_some(), reports_uid);

        let stored = imap.mailbox().messages("Drafts").await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].raw, raw, "byte-identical");
        assert_eq!(stored[0].flags, ["\\Seen", "\\Draft"]);
        if let Some(uid) = uid {
            assert_eq!(uid, stored[0].uid);
        }

        // The session still works after the literal.
        s.append("Drafts", AppendFlags::Seen, &draft("d2@example.org", "x"))
            .await
            .expect("second append");
        s.select("Drafts").await.expect("select");
        let found = s
            .uid_search_message_id("d1@example.org")
            .await
            .expect("search");
        assert_eq!(found, [stored[0].uid]);
        assert!(s
            .uid_search_message_id("nope@example.org")
            .await
            .expect("search")
            .is_empty());
        s.logout().await;
    }
}

#[tokio::test]
async fn append_to_a_missing_folder_is_refused_for_good() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![])
        .start()
        .await
        .expect("starts");
    let mut s = session(&imap).await;
    let error = s
        .append("Drafts", AppendFlags::Draft, &draft("d@example.org", "x"))
        .await
        .err()
        .expect("refused");
    assert!(!error.is_transient());
}
