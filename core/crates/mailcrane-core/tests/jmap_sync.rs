//! Syncing a JMAP account into the store (M3-04), against the fake server.

#![allow(clippy::expect_used)] // test helpers may panic

use mailcrane_core::events::NoListener;
use mailcrane_core::sync::{JmapSyncer, SyncConfig};
use mailcrane_jmap::{Auth, ClientConfig};
use mailcrane_store::{AccountId, FolderId, FolderRole, NewAccount, Security, Store};
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

struct Setup {
    server: FakeJmap,
    store: Store,
    account: AccountId,
    config: ClientConfig,
}

async fn setup() -> Setup {
    let server = FakeJmap::builder()
        .email(email("m1", "mb-inbox", "2026-11-20T08:00:00Z", true))
        .email(email("m2", "mb-inbox", "2026-11-25T08:00:00Z", false))
        .email(email("s1", "mb-sent", "2026-11-24T08:00:00Z", true))
        .start()
        .await
        .expect("server");
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Alice".into(),
                // The fake server's identity, so sending finds it.
                email: "alice@example.org".into(),
                imap_host: String::new(),
                imap_port: 0,
                imap_security: Security::Tls,
                smtp_host: String::new(),
                smtp_port: 0,
                smtp_security: Security::Tls,
                username: "alice@example.org".into(),
            },
            &server.base_url(),
        )
        .expect("account");
    let config = ClientConfig {
        timeout: Duration::from_secs(5),
        extra_roots: vec![server.root_cert_der().to_vec()],
    };
    Setup {
        server,
        store,
        account,
        config,
    }
}

async fn syncer<'a>(s: &'a Setup) -> JmapSyncer<'a> {
    JmapSyncer::connect(
        &s.store,
        &NoListener,
        s.account,
        &s.server.base_url(),
        Auth::Bearer("token".into()),
        s.config.clone(),
        SyncConfig::default(),
    )
    .await
    .expect("connect")
}

fn subjects(store: &Store, folder: FolderId) -> Vec<String> {
    let mut v: Vec<String> = store
        .list_messages(folder, 100, 0)
        .expect("list")
        .into_iter()
        .map(|m| m.details.subject)
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn mailboxes_become_folders_and_the_inbox_fills_newest_first() {
    let s = setup().await;
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("an inbox");
    let folders = s.store.list_folders(s.account).expect("folders");
    assert!(folders
        .iter()
        .any(|f| f.id == inbox && f.details.role == Some(FolderRole::Inbox)));
    assert!(folders
        .iter()
        .any(|f| f.details.role == Some(FolderRole::Sent)));

    assert_eq!(j.sync_folder(inbox).await.expect("sync"), 2);
    assert_eq!(subjects(&s.store, inbox), ["Subject m1", "Subject m2"]);
    // Flags come from keywords.
    let seen: Vec<bool> = s
        .store
        .list_messages(inbox, 10, 0)
        .expect("list")
        .into_iter()
        .map(|m| m.details.flags.seen)
        .collect();
    assert!(seen.contains(&true) && seen.contains(&false));
    // A second sync with nothing new changes nothing.
    assert_eq!(j.sync_folder(inbox).await.expect("again"), 0);
    // Refreshing the mailboxes keeps the folder and its sync state.
    assert_eq!(j.refresh_mailboxes().await.expect("refresh"), Some(inbox));
    assert_eq!(s.store.count_folder_messages(inbox).expect("count"), 2);
}

#[tokio::test]
async fn changes_add_update_and_remove_messages() {
    let s = setup().await;
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("inbox");
    j.sync_folder(inbox).await.expect("first sync");

    // New mail arrives.
    s.server
        .deliver(email("m9", "mb-inbox", "2026-11-25T09:00:00Z", false));
    j.sync_folder(inbox).await.expect("sync");
    assert_eq!(
        subjects(&s.store, inbox),
        ["Subject m1", "Subject m2", "Subject m9"]
    );

    // Read elsewhere: m9 gets $seen through Email/set on the server.
    let session = mailcrane_jmap::discover(
        &s.server.base_url(),
        &Auth::Bearer("token".into()),
        &s.config,
    )
    .await
    .expect("session");
    mailcrane_jmap::apply(
        &session,
        &Auth::Bearer("token".into()),
        &s.config,
        &["m9".to_string()],
        &mailcrane_jmap::EmailAction::Keyword {
            keyword: "$seen".into(),
            on: true,
        },
    )
    .await
    .expect("set seen");
    j.sync_folder(inbox).await.expect("sync after read");
    let m9 = s
        .store
        .list_messages(inbox, 10, 0)
        .expect("list")
        .into_iter()
        .find(|m| m.details.subject == "Subject m9")
        .expect("m9");
    assert!(m9.details.flags.seen, "the flag change arrived");

    // Deleted elsewhere.
    mailcrane_jmap::apply(
        &session,
        &Auth::Bearer("token".into()),
        &s.config,
        &["m1".to_string()],
        &mailcrane_jmap::EmailAction::Destroy,
    )
    .await
    .expect("destroy");
    j.sync_folder(inbox).await.expect("sync after delete");
    assert_eq!(subjects(&s.store, inbox), ["Subject m2", "Subject m9"]);
}

#[tokio::test]
async fn cannot_calculate_changes_starts_over() {
    let s = setup().await;
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("inbox");
    j.sync_folder(inbox).await.expect("first sync");
    // A state the server never gave out (as after a server-side reset).
    s.store
        .set_jmap_state(s.account, "Email", "bogus")
        .expect("state");
    j.sync_folder(inbox).await.expect("sync starts over");
    assert_eq!(subjects(&s.store, inbox), ["Subject m1", "Subject m2"]);
    let state = s.store.jmap_state(s.account, "Email").expect("state");
    assert_ne!(state.as_deref(), Some("bogus"));
}

#[tokio::test]
async fn bodies_are_downloaded_once() {
    let s = setup().await;
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("inbox");
    j.sync_folder(inbox).await.expect("sync");
    let message = s.store.list_messages(inbox, 1, 0).expect("list")[0].id;
    j.fetch_body(message).await.expect("body");
    let body = s.store.get_body(message).expect("read").expect("stored");
    assert!(String::from_utf8_lossy(&body).contains("Body m"));
    // Already there: no second download.
    let before = s.server.requests().await.len();
    j.fetch_body(message).await.expect("again");
    assert_eq!(s.server.requests().await.len(), before);
}

#[tokio::test]
async fn queued_actions_reach_the_server() {
    use mailcrane_core::actions::{apply_action, Action};
    let s = setup().await;
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("inbox");
    j.sync_folder(inbox).await.expect("sync");
    let m2 = s
        .store
        .list_messages(inbox, 10, 0)
        .expect("list")
        .into_iter()
        .find(|m| m.details.subject == "Subject m2")
        .expect("m2")
        .id;
    apply_action(&s.store, &[m2], Action::MarkRead, 0, &NoListener).expect("mark read");
    let report = j.replay_actions(0).await.expect("replay");
    assert_eq!((report.done, report.failed, report.remaining), (1, 0, 0));
    assert!(s
        .server
        .email_keywords("m2")
        .expect("m2 on server")
        .contains(&"$seen".to_string()));

    // Delete moves it to Trash on the server.
    apply_action(&s.store, &[m2], Action::Delete, 0, &NoListener).expect("delete");
    let report = j.replay_actions(0).await.expect("replay");
    assert_eq!(report.done, 1);
    assert_eq!(
        s.server.email_mailboxes("m2").expect("still on server"),
        vec!["mb-trash".to_string()]
    );
}

#[tokio::test]
async fn sending_submits_and_files_the_message_in_sent() {
    use mailcrane_core::compose::{send, ComposeDraft, SendOptions};
    use mailcrane_core::secrets::{Secret, SecretKey, SecretStore};
    use mailcrane_testkit::MemorySecretStore;
    let s = setup().await;
    let secrets = MemorySecretStore::new();
    secrets
        .put(
            &SecretKey::account_password(s.account.0),
            &Secret::from_string("pw".into()),
        )
        .expect("password");
    let draft = ComposeDraft {
        account_id: s.account,
        to: vec!["Bob Example <bob@example.net>".into()],
        cc: vec![],
        bcc: vec!["dan@example.org".into()],
        subject: "Over JMAP".into(),
        body_text: "Hello Bob\n".into(),
        in_reply_to: None,
        references: vec![],
        attachments: Vec::new(),
        forward_attachments_of: None,
        rich: None,
    };
    let options = SendOptions {
        timeout: Duration::from_secs(10),
        extra_root_certs: vec![s.server.root_cert_der().to_vec()],
    };
    send(&s.store, &secrets, &draft, &options)
        .await
        .expect("sent");
    let submissions = s.server.submissions();
    assert_eq!(submissions.len(), 1);
    let sub = &submissions[0];
    assert_eq!(sub.mail_from, "alice@example.org");
    // Bcc is in the envelope, never in the message.
    assert!(sub.rcpt_to.contains(&"dan@example.org".to_string()));
    assert!(sub.rcpt_to.contains(&"bob@example.net".to_string()));
    let raw = s
        .server
        .email_raw(&sub.email_id)
        .expect("the message on the server");
    assert!(!String::from_utf8_lossy(&raw)
        .to_ascii_lowercase()
        .contains("bcc:"));
    assert_eq!(
        s.server.email_mailboxes(&sub.email_id).expect("filed"),
        vec!["mb-sent".to_string()]
    );
}

#[tokio::test]
async fn a_sender_without_an_identity_is_refused_clearly() {
    use mailcrane_core::compose::{send, ComposeDraft, SendError, SendOptions};
    use mailcrane_core::secrets::{Secret, SecretKey, SecretStore};
    use mailcrane_testkit::MemorySecretStore;
    let s = setup().await;
    // An account whose address the server has no identity for.
    let other = s
        .store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Someone".into(),
                email: "someone@example.net".into(),
                imap_host: String::new(),
                imap_port: 0,
                imap_security: Security::Tls,
                smtp_host: String::new(),
                smtp_port: 0,
                smtp_security: Security::Tls,
                username: "someone@example.net".into(),
            },
            &s.server.base_url(),
        )
        .expect("account");
    let secrets = MemorySecretStore::new();
    secrets
        .put(
            &SecretKey::account_password(other.0),
            &Secret::from_string("pw".into()),
        )
        .expect("password");
    let draft = ComposeDraft {
        account_id: other,
        to: vec!["bob@example.net".into()],
        cc: vec![],
        bcc: vec![],
        subject: "Hi".into(),
        body_text: "Hi\n".into(),
        in_reply_to: None,
        references: vec![],
        attachments: Vec::new(),
        forward_attachments_of: None,
        rich: None,
    };
    let options = SendOptions {
        timeout: Duration::from_secs(10),
        extra_root_certs: vec![s.server.root_cert_der().to_vec()],
    };
    let result = send(&s.store, &secrets, &draft, &options).await;
    assert!(
        matches!(result, Err(SendError::InvalidSender)),
        "{result:?}"
    );
    assert!(s.server.submissions().is_empty(), "nothing was submitted");
}
