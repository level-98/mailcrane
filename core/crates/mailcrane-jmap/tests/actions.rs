//! JMAP actions and sending (M3-04 part 3) against the testkit's fake.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_jmap::{
    apply, discover, get_identities, send_message, Auth, ClientConfig, EmailAction, JmapError,
    Outgoing, Session,
};
use mailcrane_testkit::{FakeEmail, FakeJmap};
use std::time::Duration;

fn email(id: &str) -> FakeEmail {
    FakeEmail {
        id: id.into(),
        mailbox_id: "mb-inbox".into(),
        received_at: "2026-11-25T08:00:00Z".into(),
        subject: format!("Subject {id}"),
        from: "ana@example.org".into(),
        seen: false,
        raw: format!("Subject: Subject {id}\r\n\r\nBody\r\n"),
    }
}

async fn setup() -> (FakeJmap, ClientConfig, Auth, Session) {
    let server = FakeJmap::builder()
        .email(email("m1"))
        .email(email("m2"))
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

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
async fn each_action_lands_on_the_server() {
    let (server, cfg, auth, session) = setup().await;
    let seen = EmailAction::Keyword {
        keyword: "$seen".into(),
        on: true,
    };
    let out = apply(&session, &auth, &cfg, &ids(&["m1", "m2"]), &seen)
        .await
        .expect("seen");
    assert_eq!(out.done.len(), 2);
    assert_eq!(server.email_keywords("m2"), Some(ids(&["$seen"])));

    let flag = EmailAction::Keyword {
        keyword: "$flagged".into(),
        on: true,
    };
    apply(&session, &auth, &cfg, &ids(&["m1"]), &flag)
        .await
        .expect("flag");
    let unseen = EmailAction::Keyword {
        keyword: "$seen".into(),
        on: false,
    };
    apply(&session, &auth, &cfg, &ids(&["m1"]), &unseen)
        .await
        .expect("unseen");
    assert_eq!(server.email_keywords("m1"), Some(ids(&["$flagged"])));

    let archive = EmailAction::Move {
        from: "mb-inbox".into(),
        to: "mb-archive".into(),
    };
    apply(&session, &auth, &cfg, &ids(&["m1"]), &archive)
        .await
        .expect("move");
    assert_eq!(server.email_mailboxes("m1"), Some(ids(&["mb-archive"])));

    let gone = apply(
        &session,
        &auth,
        &cfg,
        &ids(&["m2", "nope"]),
        &EmailAction::Destroy,
    )
    .await
    .expect("destroy");
    assert_eq!(gone.done, ["m2"]);
    assert_eq!(gone.failed, [("nope".to_owned(), "notFound".to_owned())]);
    assert_eq!(server.email_mailboxes("m2"), None);
}

#[tokio::test]
async fn sending_uploads_submits_and_files_the_copy_in_sent() {
    let (server, cfg, auth, session) = setup().await;
    let identities = get_identities(&session, &auth, &cfg)
        .await
        .expect("identities");
    assert_eq!(identities[0].email, "alice@example.org");
    let raw = b"From: alice@example.org\r\nTo: bob@example.net\r\nSubject: Hello\r\n\r\nHi Bob\r\n";
    let out = Outgoing {
        raw: raw.to_vec(),
        identity_id: identities[0].id.clone(),
        mail_from: "alice@example.org".into(),
        rcpt_to: ids(&["bob@example.net", "secret@example.net"]),
        drafts_mailbox_id: "mb-drafts".into(),
        sent_mailbox_id: Some("mb-sent".into()),
    };
    let sent = send_message(&session, &auth, &cfg, &out)
        .await
        .expect("sent");

    let subs = server.submissions();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].email_id, sent.email_id);
    assert_eq!(subs[0].id, sent.submission_id);
    assert_eq!(
        subs[0].rcpt_to,
        ids(&["bob@example.net", "secret@example.net"])
    );
    // The server moved it from Drafts to Sent and dropped $draft.
    assert_eq!(
        server.email_mailboxes(&sent.email_id),
        Some(ids(&["mb-sent"]))
    );
    assert_eq!(server.email_keywords(&sent.email_id), Some(ids(&["$seen"])));
    assert_eq!(server.email_raw(&sent.email_id).as_deref(), Some(&raw[..]));
    let paths: Vec<String> = server
        .requests()
        .await
        .into_iter()
        .map(|r| r.path)
        .collect();
    assert!(paths.contains(&"/upload/A1/".to_owned()));
}

#[tokio::test]
async fn an_unknown_identity_is_a_method_error_and_nothing_is_sent() {
    let (server, cfg, auth, session) = setup().await;
    let out = Outgoing {
        raw: b"Subject: x\r\n\r\nx\r\n".to_vec(),
        identity_id: "someone-else".into(),
        mail_from: "alice@example.org".into(),
        rcpt_to: ids(&["bob@example.net"]),
        drafts_mailbox_id: "mb-drafts".into(),
        sent_mailbox_id: Some("mb-sent".into()),
    };
    let err = send_message(&session, &auth, &cfg, &out)
        .await
        .expect_err("refused");
    assert_eq!(err, JmapError::Method("invalidProperties".into()));
    assert!(server.submissions().is_empty());
    // The imported copy was destroyed again (#277).
    assert!(server.emails_in("mb-drafts").is_empty());
}

#[tokio::test]
async fn a_bad_envelope_is_refused_before_anything_is_uploaded() {
    let (server, cfg, auth, session) = setup().await;
    let out = Outgoing {
        raw: b"x".to_vec(),
        identity_id: "id-1".into(),
        mail_from: "alice@example.org".into(),
        rcpt_to: ids(&["bob@example.net\r\nRCPT TO:<x@y>"]),
        drafts_mailbox_id: "mb-drafts".into(),
        sent_mailbox_id: Some("mb-sent".into()),
    };
    let before = server.requests().await.len();
    assert_eq!(
        send_message(&session, &auth, &cfg, &out).await,
        Err(JmapError::Malformed)
    );
    assert_eq!(server.requests().await.len(), before, "nothing sent");
}
