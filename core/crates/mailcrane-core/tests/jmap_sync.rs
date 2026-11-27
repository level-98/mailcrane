//! Syncing a JMAP account into the store (M3-04), against the fake server.

#![allow(clippy::expect_used)] // test helpers may panic

use mailcrane_core::events::NoListener;
use mailcrane_core::sync::{JmapSyncer, SyncConfig};
use mailcrane_jmap::{Auth, ClientConfig};
use mailcrane_store::{AccountId, FolderId, FolderRole, NewAccount, Security, Store, VirtualRole};
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
    syncer_with(s, &NoListener).await
}

/// A syncer whose events go to `listener`.
async fn syncer_with<'a>(
    s: &'a Setup,
    listener: &'a dyn mailcrane_core::events::EventListener,
) -> JmapSyncer<'a> {
    JmapSyncer::connect(
        &s.store,
        listener,
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

#[tokio::test]
async fn a_partly_refused_move_keeps_the_refused_message_where_it_was() {
    use mailcrane_core::actions::{apply_action, Action};
    let s = setup().await;
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("inbox");
    j.sync_folder(inbox).await.expect("sync");
    let ids: Vec<_> = s
        .store
        .list_messages(inbox, 10, 0)
        .expect("list")
        .into_iter()
        .map(|m| (m.details.subject.clone(), m.id))
        .collect();
    let id_of = |subject: &str| {
        ids.iter()
            .find(|(s, _)| s == subject)
            .map(|(_, id)| *id)
            .expect("message")
    };
    // The server won't let m1 be changed.
    s.server.refuse_changes("m1");
    apply_action(
        &s.store,
        &[id_of("Subject m1"), id_of("Subject m2")],
        Action::Delete,
        0,
        &NoListener,
    )
    .expect("delete both");
    let report = j.replay_actions(0).await.expect("replay");
    assert_eq!((report.done, report.failed, report.remaining), (1, 1, 0));
    // m2 moved to Trash on the server; m1 stayed in the inbox there and here.
    assert_eq!(
        s.server.email_mailboxes("m2").expect("m2"),
        vec!["mb-trash".to_string()]
    );
    assert_eq!(
        s.server.email_mailboxes("m1").expect("m1"),
        vec!["mb-inbox".to_string()]
    );
    let left = subjects(&s.store, inbox);
    assert_eq!(
        left,
        ["Subject m1"],
        "the refused message is still local, in the inbox"
    );
}

#[tokio::test]
async fn a_partly_refused_flag_change_is_undone_for_the_refused_message() {
    use mailcrane_core::actions::{apply_action, Action};
    let s = setup().await;
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("inbox");
    j.sync_folder(inbox).await.expect("sync");
    let all: Vec<_> = s.store.list_messages(inbox, 10, 0).expect("list");
    s.server.refuse_changes("m2");
    let both: Vec<_> = all.iter().map(|m| m.id).collect();
    apply_action(&s.store, &both, Action::Flag, 0, &NoListener).expect("flag both");
    j.replay_actions(0).await.expect("replay");
    for m in s.store.list_messages(inbox, 10, 0).expect("list") {
        let flagged_on_server = m.details.subject == "Subject m1";
        assert_eq!(
            m.details.flags.flagged, flagged_on_server,
            "{}",
            m.details.subject
        );
    }
}

/// Alice's JMAP account on `server`, with her password stored.
struct Sender {
    server: FakeJmap,
    store: Store,
    account: AccountId,
    secrets: mailcrane_testkit::MemorySecretStore,
}

/// A fake server with only `mailboxes` (id, role; empty for the default
/// Inbox, Drafts, Sent and Trash) and `faults`, and Alice's account on it.
async fn sender(mailboxes: &[(&str, &str)], faults: &[mailcrane_testkit::Fault]) -> Sender {
    use mailcrane_core::secrets::{Secret, SecretKey, SecretStore};
    let mut builder = FakeJmap::builder();
    for (id, role) in mailboxes {
        builder = builder.mailbox(id, id, Some(role), None);
    }
    for fault in faults {
        builder = builder.fault(fault.clone());
    }
    let server = builder.start().await.expect("server");
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Alice".into(),
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
    let secrets = mailcrane_testkit::MemorySecretStore::new();
    secrets
        .put(
            &SecretKey::account_password(account.0),
            &Secret::from_string("pw".into()),
        )
        .expect("password");
    Sender {
        server,
        store,
        account,
        secrets,
    }
}

impl Sender {
    fn draft(&self, subject: &str) -> mailcrane_core::compose::ComposeDraft {
        mailcrane_core::compose::ComposeDraft {
            account_id: self.account,
            to: vec!["bob@example.net".into()],
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            body_text: "Hello Bob
"
            .into(),
            in_reply_to: None,
            references: vec![],
            attachments: Vec::new(),
            forward_attachments_of: None,
            rich: None,
        }
    }
    fn options(&self) -> mailcrane_core::compose::SendOptions {
        mailcrane_core::compose::SendOptions {
            timeout: Duration::from_secs(10),
            extra_root_certs: vec![self.server.root_cert_der().to_vec()],
        }
    }
    /// Sends queued message `id` through the outbox at time `now`.
    async fn send_queued(
        &self,
        id: mailcrane_store::OutboxId,
        now: i64,
    ) -> mailcrane_core::outbox::SendOutcome {
        mailcrane_core::outbox::send_one(&self.store, &self.secrets, id, &self.options(), now)
            .await
            .expect("store")
            .expect("was queued")
    }
    fn outbox(&self, id: mailcrane_store::OutboxId) -> mailcrane_store::OutboxMessage {
        self.store
            .get_outbox(id)
            .expect("read")
            .expect("still in the outbox")
    }
}

/// Sends one message from Alice over a fake server with only `mailboxes`
/// (id, role), and returns the server.
async fn send_with_mailboxes(mailboxes: &[(&str, &str)]) -> FakeJmap {
    let s = sender(mailboxes, &[]).await;
    mailcrane_core::compose::send(
        &s.store,
        &s.secrets,
        &s.draft("No Sent mailbox"),
        &s.options(),
    )
    .await
    .expect("sent without a Sent mailbox");
    s.server
}

#[tokio::test]
async fn sending_works_without_a_sent_mailbox() {
    // RFC 8621 doesn't require a `sent` role (#276).
    let server = send_with_mailboxes(&[("mb-inbox", "inbox"), ("mb-drafts", "drafts")]).await;
    let submissions = server.submissions();
    assert_eq!(submissions.len(), 1, "submitted exactly once");
    assert_eq!(submissions[0].rcpt_to, vec!["bob@example.net".to_string()]);
    // The imported copy was destroyed on success: nothing is left in Drafts
    // looking like an unsent draft.
    assert_eq!(server.email_mailboxes(&submissions[0].email_id), None);
    assert!(server.emails_in("mb-drafts").is_empty());
    assert_eq!(server.email_count(), 0);
}

#[tokio::test]
async fn sending_works_with_neither_drafts_nor_sent() {
    let server = send_with_mailboxes(&[("mb-inbox", "inbox")]).await;
    assert_eq!(server.submissions().len(), 1, "submitted exactly once");
    assert_eq!(server.email_count(), 0, "the temporary copy is gone");
}

const NOW: i64 = 1_790_000_000;

#[tokio::test]
async fn a_refused_submission_leaves_no_copy_and_the_retry_sends_once() {
    // #277: the import succeeds, the submission is refused; then the
    // outbox retry is accepted.
    use mailcrane_core::outbox::{enqueue, retry, SendOutcome};
    use mailcrane_store::{OutboxState, SentCopy};
    let s = sender(&[], &[mailcrane_testkit::Fault::RefuseSubmissions(1)]).await;
    let id = enqueue(&s.store, &s.draft("Refused once"), 0, NOW).expect("queued");

    let first = s.send_queued(id, NOW).await;
    assert!(matches!(first, SendOutcome::Failed { .. }), "{first:?}");
    assert_eq!(s.outbox(id).state, OutboxState::Failed);
    assert!(s.server.submissions().is_empty());
    // The imported copy was destroyed again: the mailbox is unchanged.
    assert!(
        s.server.emails_in("mb-drafts").is_empty(),
        "no copy in Drafts"
    );
    assert_eq!(s.server.email_count(), 0);

    retry(&s.store, id, NOW + 10).expect("retried");
    let second = s.send_queued(id, NOW + 10).await;
    assert!(matches!(second, SendOutcome::Sent { .. }), "{second:?}");
    let submissions = s.server.submissions();
    assert_eq!(submissions.len(), 1, "exactly one submission");
    assert!(
        s.server.emails_in("mb-drafts").is_empty(),
        "no leftover copies"
    );
    assert_eq!(
        s.server.emails_in("mb-sent"),
        vec![submissions[0].email_id.clone()],
        "one copy, in Sent"
    );
    assert_eq!(s.outbox(id).sent_copy, SentCopy::Saved);
}

#[tokio::test]
async fn without_a_sent_mailbox_the_outbox_keeps_no_copy() {
    // Review nit on #286: the server kept no Sent copy, so the outbox must
    // not record one as saved.
    use mailcrane_core::outbox::{enqueue, SendOutcome};
    use mailcrane_store::SentCopy;
    let s = sender(&[("mb-inbox", "inbox"), ("mb-drafts", "drafts")], &[]).await;
    let id = enqueue(&s.store, &s.draft("No Sent"), 0, NOW).expect("queued");
    let outcome = s.send_queued(id, NOW).await;
    assert!(matches!(outcome, SendOutcome::Sent { .. }), "{outcome:?}");
    assert_eq!(s.outbox(id).sent_copy, SentCopy::Skipped);
}

/// #448: an email the server leaves out of its Email/set reply (in neither
/// `updated` nor `notUpdated`, against RFC 8620 §5.3) is unconfirmed. The
/// replay fetches its state and applies it, whichever way the server went.
#[tokio::test]
async fn an_email_left_out_of_the_set_reply_is_reconciled() {
    use mailcrane_core::actions::{apply_action, Action};
    for applied in [false, true] {
        let s = setup().await;
        let mut j = syncer(&s).await;
        let inbox = j
            .refresh_mailboxes()
            .await
            .expect("mailboxes")
            .expect("inbox");
        j.sync_folder(inbox).await.expect("sync");
        s.server.omit_from_set_reply("m2", applied);
        let both: Vec<_> = s
            .store
            .list_messages(inbox, 10, 0)
            .expect("list")
            .iter()
            .map(|m| m.id)
            .collect();
        apply_action(&s.store, &both, Action::Flag, 0, &NoListener).expect("flag both");
        let report = j.replay_actions(0).await.expect("replay");
        assert_eq!((report.done, report.remaining), (1, 0), "applied={applied}");
        for m in s.store.list_messages(inbox, 10, 0).expect("list") {
            let flagged_on_server = m.details.subject == "Subject m1" || applied;
            assert_eq!(
                m.details.flags.flagged, flagged_on_server,
                "applied={applied}: {} matches the server",
                m.details.subject
            );
        }
    }
}

/// The same for a delete (a move to Trash): left out and not applied, the
/// email is back in the Inbox locally; applied, it's gone from it.
#[tokio::test]
async fn a_delete_left_out_of_the_set_reply_is_reconciled() {
    use mailcrane_core::actions::{apply_action, Action};
    for applied in [false, true] {
        let s = setup().await;
        let mut j = syncer(&s).await;
        let inbox = j
            .refresh_mailboxes()
            .await
            .expect("mailboxes")
            .expect("inbox");
        j.sync_folder(inbox).await.expect("sync");
        s.server.omit_from_set_reply("m2", applied);
        let ids: Vec<_> = s
            .store
            .list_messages(inbox, 10, 0)
            .expect("list")
            .into_iter()
            .filter(|m| m.details.subject == "Subject m1" || m.details.subject == "Subject m2")
            .map(|m| m.id)
            .collect();
        apply_action(&s.store, &ids, Action::Delete, 0, &NoListener).expect("delete both");
        j.replay_actions(0).await.expect("replay");
        let on_server = s.server.email_mailboxes("m2").expect("m2");
        let left = subjects(&s.store, inbox);
        assert_eq!(
            left.contains(&"Subject m2".to_string()),
            on_server == vec!["mb-inbox".to_string()],
            "applied={applied}: local Inbox {left:?} vs server {on_server:?}"
        );
        assert!(
            !left.contains(&"Subject m1".to_string()),
            "applied={applied}"
        );
    }
}

/// Records every event, for the tests that check what the user is told.
#[derive(Default)]
struct Events(std::sync::Mutex<Vec<mailcrane_core::events::CoreEvent>>);

impl mailcrane_core::events::EventListener for Events {
    fn on_event(&self, event: mailcrane_core::events::CoreEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

impl Events {
    /// The message ids of every `ActionFailed` event so far.
    fn failed_ids(&self) -> Vec<mailcrane_store::MessageId> {
        self.0
            .lock()
            .expect("lock")
            .iter()
            .filter_map(|e| match e {
                mailcrane_core::events::CoreEvent::ActionFailed { message_ids, .. } => {
                    Some(message_ids.clone())
                }
                _ => None,
            })
            .flatten()
            .collect()
    }
}

/// #451 review: a change the server left out of its reply and didn't
/// carry out is a refusal the user hears about: `ActionFailed` for that
/// message, counted once in the report. Carried out, nothing is reported.
#[tokio::test]
async fn an_unapplied_omission_is_reported_as_failed() {
    use mailcrane_core::actions::{apply_action, Action};
    for applied in [false, true] {
        let s = setup().await;
        let events = Events::default();
        let mut j = syncer_with(&s, &events).await;
        let inbox = j
            .refresh_mailboxes()
            .await
            .expect("mailboxes")
            .expect("inbox");
        j.sync_folder(inbox).await.expect("sync");
        let all = s.store.list_messages(inbox, 10, 0).expect("list");
        let m2 = all
            .iter()
            .find(|m| m.details.subject == "Subject m2")
            .map(|m| m.id)
            .expect("m2");
        s.server.omit_from_set_reply("m2", applied);
        let ids: Vec<_> = all.iter().map(|m| m.id).collect();
        apply_action(&s.store, &ids, Action::Flag, 0, &NoListener).expect("flag");
        let report = j.replay_actions(0).await.expect("replay");
        if applied {
            assert_eq!(report.failed, 0, "nothing to report");
            assert!(events.failed_ids().is_empty(), "no ActionFailed");
        } else {
            assert_eq!(report.failed, 1, "the op counts as failed once");
            assert_eq!(
                events.failed_ids(),
                vec![m2],
                "the user hears about m2 only"
            );
        }
    }
}

/// #451 review: the follow-up Email/get is split to the server's
/// `maxObjectsInGet` (RFC 8620 §2). With a limit of 1 and two emails left
/// out, both are confirmed; one oversized call would be refused, and the
/// changes the server did make would be undone and reported as failed.
#[tokio::test]
async fn the_reconcile_fetch_respects_max_objects_in_get() {
    use mailcrane_core::actions::{apply_action, Action};
    let s = setup().await;
    s.server.set_max_objects_in_get(1);
    let mut j = syncer(&s).await;
    let inbox = j
        .refresh_mailboxes()
        .await
        .expect("mailboxes")
        .expect("inbox");
    j.sync_folder(inbox).await.expect("sync");
    let all = s.store.list_messages(inbox, 10, 0).expect("list");
    for m in &all {
        let email = m.details.subject.trim_start_matches("Subject ").to_owned();
        s.server.omit_from_set_reply(&email, true);
    }
    assert!(all.len() >= 2, "needs two emails to split");
    let ids: Vec<_> = all.iter().map(|m| m.id).collect();
    apply_action(&s.store, &ids, Action::Flag, 0, &NoListener).expect("flag");
    let report = j.replay_actions(0).await.expect("replay");
    assert_eq!((report.done, report.failed), (1, 0));
    for m in s.store.list_messages(inbox, 10, 0).expect("list") {
        assert!(
            m.details.flags.flagged,
            "{} stays flagged",
            m.details.subject
        );
    }
}

/// #451 review: reconciling one op leaves alone an email that a later
/// queued op changes, so the later op's local result survives until its
/// own replay settles it (here m2 marked read, queued after a flag the
/// server left out and didn't apply).
#[tokio::test]
async fn the_reconcile_keeps_a_later_queued_change() {
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
        .map(|m| m.id)
        .expect("m2");
    s.server.omit_from_set_reply("m2", false);
    apply_action(&s.store, &[m2], Action::Flag, 0, &NoListener).expect("flag");
    // Queued after, and not due yet in this replay.
    apply_action(&s.store, &[m2], Action::MarkRead, 1_000, &NoListener).expect("read");
    j.replay_actions(0).await.expect("replay");
    let after = s
        .store
        .list_messages(inbox, 10, 0)
        .expect("list")
        .into_iter()
        .find(|m| m.id == m2)
        .expect("m2 still listed");
    assert!(after.details.flags.seen, "the queued mark-read survives");
    assert_eq!(
        s.store.pending_ops(s.account).expect("ops").len(),
        1,
        "it's still queued"
    );
}

/// #451, lead: an omitted email with a later queued op is never counted
/// done by this op while unconfirmed (#448 again). Not applied, this op's
/// flag is undone for it and the user is told, while the later op's local
/// mark-read stays. Applied, the flag is confirmed and nothing is reported.
#[tokio::test]
async fn an_omission_with_a_later_op_is_confirmed_or_undone() {
    use mailcrane_core::actions::{apply_action, Action};
    for applied in [false, true] {
        let s = setup().await;
        let events = Events::default();
        let mut j = syncer_with(&s, &events).await;
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
            .map(|m| m.id)
            .expect("m2");
        s.server.omit_from_set_reply("m2", applied);
        apply_action(&s.store, &[m2], Action::Flag, 0, &NoListener).expect("flag");
        apply_action(&s.store, &[m2], Action::MarkRead, 1_000, &NoListener).expect("read");
        let report = j.replay_actions(0).await.expect("replay");
        let after = s
            .store
            .list_messages(inbox, 10, 0)
            .expect("list")
            .into_iter()
            .find(|m| m.id == m2)
            .expect("m2 still listed");
        assert_eq!(
            after.details.flags.flagged, applied,
            "applied={applied}: local flag matches the server"
        );
        assert!(
            after.details.flags.seen,
            "applied={applied}: the queued mark-read survives"
        );
        if applied {
            assert_eq!(report.failed, 0, "confirmed");
            assert!(events.failed_ids().is_empty());
        } else {
            assert_eq!(report.failed, 1, "undone, not done");
            assert_eq!(events.failed_ids(), vec![m2], "the user hears about m2");
        }
    }
}

/// #451 review: an unconfirmed email is detached from its op, with its
/// local change undone, before the op is completed. So completing the op
/// (which deletes a delete's hidden rows) can't take the email with it,
/// and a local failure between the steps leaves it shown, never lost.
#[tokio::test]
async fn a_detached_email_survives_completing_its_op() {
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
        .map(|m| m.id)
        .expect("m2");
    apply_action(&s.store, &[m2], Action::Delete, 0, &NoListener).expect("delete");
    assert!(!subjects(&s.store, inbox).contains(&"Subject m2".to_string()));
    let op = s.store.pending_ops(s.account).expect("ops")[0].id;
    // The first step of the reconcile, then completing the op, as a replay
    // interrupted before applying the server's state would leave it.
    s.store.fail_op_messages(op, &[m2]).expect("detach");
    s.store.complete_op(op, true).expect("complete");
    assert!(
        subjects(&s.store, inbox).contains(&"Subject m2".to_string()),
        "m2 is back in the Inbox, not deleted with the op"
    );
}

#[tokio::test]
async fn a_flagged_mailbox_is_virtual_and_an_important_one_ordinary() {
    // #461: RFC 8621 mailbox roles come from the same IANA registry as the
    // IMAP attributes, so "flagged" is a virtual folder, as `\Flagged` is on
    // IMAP: no folder role, so nothing moves, archives or deletes into it.
    // "important" (RFC 8457 §3) may be a real folder; IMAP counts it only on
    // Gmail, which has no JMAP, so it stays ordinary (#482 review).
    let server = FakeJmap::builder()
        .mailbox("mb-inbox", "Inbox", Some("inbox"), None)
        .mailbox("mb-flagged", "Starred", Some("flagged"), None)
        .mailbox("mb-important", "Important", Some("important"), None)
        .mailbox("mb-work", "Work", None, None)
        .start()
        .await
        .expect("server");
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Alice".into(),
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
    let s = Setup {
        server,
        store,
        account,
        config,
    };
    let mut j = syncer(&s).await;
    j.refresh_mailboxes().await.expect("mailboxes");
    let folders = s.store.list_folders(s.account).expect("folders");
    let by_name = |name: &str| {
        folders
            .iter()
            .find(|f| f.details.display_name == name)
            .expect("mailbox stored")
            .details
            .clone()
    };
    let starred = by_name("Starred");
    assert_eq!(starred.virtual_role, Some(VirtualRole::Starred));
    assert_eq!(starred.role, None);
    let important = by_name("Important");
    assert_eq!(important.virtual_role, None, "an ordinary folder");
    assert_eq!(important.role, None);
    assert_eq!(by_name("Work").virtual_role, None);
    assert_eq!(by_name("Inbox").role, Some(FolderRole::Inbox));
}
