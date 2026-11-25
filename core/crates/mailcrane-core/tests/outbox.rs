//! The outbox against the testkit's fake SMTP and IMAP servers. Never a
//! real mailbox.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::compose::{ComposeDraft, SendError, SendOptions};
use mailcrane_core::outbox::*;
use mailcrane_core::secrets::{Secret, SecretKey, SecretStore};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity};
use mailcrane_store::{
    AccountId, NewAccount, OutboxId, OutboxMessage, OutboxState, Security, Store,
};
use mailcrane_testkit::{FakeImap, FakeSmtp, Fault, MemorySecretStore};
use std::time::Duration;

const NOW: i64 = 1_800_000_000;
const USER: &str = "ann@example.org";
const PASSWORD: &str = "s3cret-value";

struct Setup {
    smtp: FakeSmtp,
    store: Store,
    secrets: MemorySecretStore,
    account: AccountId,
}

async fn setup_with(faults: &[Fault], store: Store) -> Setup {
    let mut builder = FakeSmtp::builder();
    for fault in faults {
        builder = builder.fault(fault.clone());
    }
    let smtp = builder.start().await.expect("fake smtp");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Ann Example".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: smtp.addr().port(),
            smtp_security: Security::Tls,
            username: USER.into(),
        })
        .expect("account");
    let secrets = MemorySecretStore::new();
    secrets
        .put(
            &SecretKey::account_password(account.0),
            &Secret::from_string(PASSWORD.into()),
        )
        .expect("password");
    Setup {
        smtp,
        store,
        secrets,
        account,
    }
}

async fn setup(faults: &[Fault]) -> Setup {
    setup_with(faults, Store::open_in_memory().expect("store")).await
}

fn options(s: &Setup) -> SendOptions {
    SendOptions {
        timeout: Duration::from_secs(5),
        extra_root_certs: vec![s.smtp.root_cert_der().to_vec()],
    }
}

fn draft(s: &Setup, subject: &str) -> ComposeDraft {
    ComposeDraft {
        account_id: s.account,
        to: vec!["Bob <bob@example.net>".into()],
        cc: vec![],
        bcc: vec!["carol@example.com".into()],
        subject: subject.into(),
        body_text: "Hello".into(),
        in_reply_to: None,
        references: vec![],
    }
}

async fn send_due_at(s: &Setup, now: i64) -> OutboxReport {
    send_due(&s.store, &s.secrets, s.account, &options(s), now)
        .await
        .expect("send_due")
}

fn state(s: &Setup, id: OutboxId) -> OutboxMessage {
    s.store.get_outbox(id).expect("read").expect("there")
}

#[tokio::test]
async fn queued_while_the_server_is_down_and_delivered_exactly_once() {
    let s = setup(&[]).await;
    s.smtp.set_down(true);
    let id = enqueue(&s.store, &draft(&s, "Offline"), 0, NOW).expect("queued");
    let report = send_due_at(&s, NOW).await;
    assert_eq!((report.sent, report.remaining), (0, 1));
    assert_eq!(report.next_attempt_at, Some(NOW + 30));
    let m = state(&s, id);
    assert_eq!(m.state, OutboxState::Queued);
    // A connection dropped during the TLS handshake reports TLS; either way
    // it's temporary.
    assert!(
        matches!(m.last_error.as_deref(), Some("network" | "tls")),
        "{:?}",
        m.last_error
    );

    s.smtp.set_down(false);
    // Not due yet.
    assert_eq!(send_due_at(&s, NOW + 10).await.sent, 0);
    let report = send_due_at(&s, NOW + 30).await;
    assert_eq!((report.sent, report.remaining), (1, 0));
    assert_eq!(state(&s, id).state, OutboxState::Sent);
    // Nothing left to send: it goes out exactly once.
    send_due_at(&s, NOW + 3_600).await;
    let sent = s.smtp.sent().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].envelope_to,
        vec!["bob@example.net", "carol@example.com"]
    );
    let raw = String::from_utf8_lossy(&sent[0].data).to_ascii_lowercase();
    assert!(!raw.contains("bcc"), "no Bcc on the wire");
    assert!(raw.contains(&state(&s, id).message_id_header.to_ascii_lowercase()));
}

#[tokio::test]
async fn temporary_failures_back_off_then_give_up_after_a_day() {
    let s = setup(&[Fault::TempFailure]).await;
    let id = enqueue(&s.store, &draft(&s, "Busy"), 0, NOW).expect("queued");
    let mut now = NOW;
    let mut waits = Vec::new();
    for _ in 0..7 {
        let report = send_due_at(&s, now).await;
        let next = report.next_attempt_at.expect("still queued");
        waits.push(next - now);
        now = next;
    }
    assert_eq!(waits, [30, 60, 120, 300, 900, 900, 900]);
    assert_eq!(
        state(&s, id).last_error.as_deref(),
        Some("refused_temporary")
    );

    // A day after it was queued, it fails for good.
    let report = send_due_at(&s, NOW + GIVE_UP_AFTER_SECS).await;
    assert_eq!(report.failed, 1);
    let m = state(&s, id);
    assert_eq!(m.state, OutboxState::Failed);
    assert_eq!(m.last_error.as_deref(), Some("refused_temporary"));
    assert!(s.smtp.sent().await.is_empty());
}

#[tokio::test]
async fn permanent_failures_fail_at_once_and_can_be_retried() {
    let s = setup(&[Fault::PermFailure]).await;
    let id = enqueue(&s.store, &draft(&s, "Nope"), 0, NOW).expect("queued");
    let report = send_due_at(&s, NOW).await;
    assert_eq!((report.failed, report.remaining), (1, 0));
    let m = state(&s, id);
    assert_eq!(m.state, OutboxState::Failed);
    assert_eq!(m.last_error.as_deref(), Some("refused"));
    // Not sent again by itself.
    send_due_at(&s, NOW + 3_600).await;
    assert_eq!(state(&s, id).attempts, 1);

    retry(&s.store, id, NOW + 10).expect("retried");
    assert_eq!(state(&s, id).state, OutboxState::Queued);
    assert!(matches!(
        retry(&s.store, OutboxId(999), NOW),
        Err(OutboxError::NotFound)
    ));
}

#[tokio::test]
async fn a_missing_password_fails_the_message() {
    let s = setup(&[]).await;
    s.secrets
        .delete(&SecretKey::account_password(s.account.0))
        .expect("removed");
    let id = enqueue(&s.store, &draft(&s, "x"), 0, NOW).expect("queued");
    assert_eq!(send_due_at(&s, NOW).await.failed, 1);
    assert_eq!(
        state(&s, id).last_error.as_deref(),
        Some("missing_credentials")
    );
}

#[tokio::test]
async fn invalid_messages_are_refused_at_enqueue() {
    let s = setup(&[]).await;
    let mut bad = draft(&s, "x");
    bad.to = vec!["not an address".into()];
    assert!(matches!(
        enqueue(&s.store, &bad, 0, NOW),
        Err(SendError::InvalidRecipients(_))
    ));
    bad.to.clear();
    bad.bcc.clear();
    assert!(matches!(
        enqueue(&s.store, &bad, 0, NOW),
        Err(SendError::NoRecipients)
    ));
    assert!(s.store.list_outbox(s.account).expect("list").is_empty());
}

#[tokio::test]
async fn the_outbox_survives_a_reopen() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("o.db");
    let s = setup_with(&[], Store::open(&path).expect("store")).await;
    let id = enqueue(&s.store, &draft(&s, "Kept"), 0, NOW).expect("queued");
    let account = s.account;
    drop(s.store);
    let store = Store::open(&path).expect("reopen");
    let m = store.get_outbox(id).expect("read").expect("kept");
    assert_eq!(m.state, OutboxState::Queued);
    assert_eq!(m.fields.subject, "Kept");
    assert_eq!(store.outbox_accounts().expect("accounts"), [account]);
}

#[tokio::test]
async fn not_before_holds_a_message_back() {
    let s = setup(&[]).await;
    let id = enqueue(&s.store, &draft(&s, "Later"), NOW + 600, NOW).expect("queued");
    assert_eq!(send_due_at(&s, NOW).await.sent, 0);
    assert_eq!(send_due_at(&s, NOW + 599).await.sent, 0);
    assert_eq!(send_due_at(&s, NOW + 600).await.sent, 1);
    assert_eq!(state(&s, id).state, OutboxState::Sent);
}

#[tokio::test]
async fn a_claimed_message_is_never_sent_twice() {
    let s = setup(&[]).await;
    let id = enqueue(&s.store, &draft(&s, "Once"), 0, NOW).expect("queued");
    assert!(s.store.claim_outbox(id).expect("claim"));
    assert!(
        !s.store.claim_outbox(id).expect("claim"),
        "second claim refused"
    );
    // Another sender finds nothing to send.
    assert_eq!(
        send_one(&s.store, &s.secrets, id, &options(&s), NOW)
            .await
            .expect("send"),
        None
    );
    assert!(s.smtp.sent().await.is_empty());
    // And it can't be deleted while being sent.
    assert!(matches!(delete(&s.store, id), Err(OutboxError::NotFound)));
}

async fn imap_with_sent(message_id: Option<&str>) -> FakeImap {
    let mut sent = Vec::new();
    if let Some(id) = message_id {
        sent.push(
            format!("From: ann@example.org\r\nSubject: x\r\nMessage-ID: <{id}>\r\n\r\nx\r\n")
                .into_bytes(),
        );
    }
    FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![])
        .folder("Sent", sent)
        .special_use("Sent", "\\Sent")
        .start()
        .await
        .expect("fake imap")
}

async fn recover(s: &Setup, imap: &FakeImap) -> usize {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    let mut session =
        mailcrane_imap::Session::connect(&config, USER, &mailcrane_imap::Secret::new(PASSWORD))
            .await
            .expect("connects");
    let listed = session.list_folders().await.expect("list");
    save_folders(&s.store, s.account, &listed).expect("save");
    let found = recover_interrupted(&s.store, &mut session, s.account, NOW + 100)
        .await
        .expect("recover");
    session.logout().await;
    found
}

#[tokio::test]
async fn an_interrupted_send_found_in_sent_is_not_sent_again() {
    let s = setup(&[]).await;
    let id = enqueue(&s.store, &draft(&s, "Crash"), 0, NOW).expect("queued");
    // The app stopped after claiming the message (it may have gone out).
    s.store
        .set_outbox_state(id, OutboxState::Sending)
        .expect("state");
    let message_id = state(&s, id).message_id_header;
    let imap = imap_with_sent(Some(&message_id)).await;
    assert_eq!(recover(&s, &imap).await, 1);
    assert_eq!(state(&s, id).state, OutboxState::Sent);
    send_due_at(&s, NOW + 200).await;
    assert!(s.smtp.sent().await.is_empty(), "not resent");
}

#[tokio::test]
async fn an_interrupted_send_not_in_sent_is_sent_again() {
    let s = setup(&[]).await;
    let id = enqueue(&s.store, &draft(&s, "Crash"), 0, NOW).expect("queued");
    s.store
        .set_outbox_state(id, OutboxState::Sending)
        .expect("state");
    let imap = imap_with_sent(None).await;
    assert_eq!(recover(&s, &imap).await, 0);
    assert_eq!(state(&s, id).state, OutboxState::Queued);
    assert_eq!(send_due_at(&s, NOW + 200).await.sent, 1);
    assert_eq!(s.smtp.sent().await.len(), 1);
}

#[tokio::test]
async fn a_failed_message_can_be_edited_as_a_draft_or_deleted() {
    let s = setup(&[Fault::PermFailure]).await;
    let id = enqueue(&s.store, &draft(&s, "Fix me"), 0, NOW).expect("queued");
    send_due_at(&s, NOW).await;
    let draft_id = edit(&s.store, id, NOW).expect("edited");
    let d = s.store.get_draft(draft_id).expect("read").expect("draft");
    assert_eq!(d.fields.subject, "Fix me");
    assert_eq!(d.fields.bcc, ["carol@example.com"]);
    assert!(s.store.get_outbox(id).expect("read").is_none());

    let other = enqueue(&s.store, &draft(&s, "Bin me"), 0, NOW).expect("queued");
    assert_eq!(
        delete(&s.store, other).expect("deleted").fields.subject,
        "Bin me"
    );
    assert!(s.store.list_outbox(s.account).expect("list").is_empty());
}
