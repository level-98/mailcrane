//! Saving sent messages to the Sent folder (M2-07), against the testkit's
//! fake SMTP and IMAP servers. Never a real mailbox.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::compose::{ComposeDraft, SendOptions};
use mailcrane_core::outbox::*;
use mailcrane_core::secrets::{Secret, SecretKey, SecretStore};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity};
use mailcrane_store::{AccountId, NewAccount, SaveSentCopy, Security, SentCopy, Store};
use mailcrane_testkit::{FakeImap, FakeSmtp, Fault, MemorySecretStore, Quirk};
use std::time::Duration;

const NOW: i64 = 1_800_000_000;
const USER: &str = "ann@example.org";
const PASSWORD: &str = "s3cret-value";

struct Setup {
    smtp: FakeSmtp,
    imap: FakeImap,
    store: Store,
    secrets: MemorySecretStore,
    account: AccountId,
}

async fn setup(imap: FakeImap) -> Setup {
    let smtp = FakeSmtp::builder().start().await.expect("fake smtp");
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Ann".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
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
    let s = Setup {
        smtp,
        imap,
        store,
        secrets,
        account,
    };
    let mut session = connect(&s.imap).await;
    let listed = session.list_folders().await.expect("list");
    save_folders(&s.store, s.account, &listed).expect("save");
    session.logout().await;
    s
}

fn imap_builder() -> mailcrane_testkit::FakeImapBuilder {
    FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![])
}

async fn with_sent() -> FakeImap {
    imap_builder()
        .folder("Sent", vec![])
        .special_use("Sent", "\\Sent")
        .start()
        .await
        .expect("imap")
}

async fn connect(imap: &FakeImap) -> mailcrane_imap::Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    mailcrane_imap::Session::connect(&config, USER, &mailcrane_imap::Secret::new(PASSWORD))
        .await
        .expect("connects")
}

async fn send_one_message(s: &Setup) -> mailcrane_store::OutboxId {
    let draft = ComposeDraft {
        account_id: s.account,
        to: vec!["bob@example.net".into()],
        cc: vec![],
        bcc: vec!["carol@example.com".into()],
        subject: "Sent copy".into(),
        body_text: "Hello".into(),
        in_reply_to: None,
        references: vec![],
        attachments: Vec::new(),
        forward_attachments_of: None,
    };
    let id = enqueue(&s.store, &draft, 0, NOW).expect("queued");
    let options = SendOptions {
        timeout: Duration::from_secs(5),
        extra_root_certs: vec![s.smtp.root_cert_der().to_vec()],
    };
    let report = send_due(&s.store, &s.secrets, s.account, &options, NOW)
        .await
        .expect("send");
    assert_eq!(report.sent, 1);
    id
}

async fn save(s: &Setup, imap: &FakeImap, now: i64) -> SaveReport {
    let mut session = connect(imap).await;
    let report = save_sent_copies(&s.store, &mut session, s.account, now)
        .await
        .expect("save");
    session.logout().await;
    report
}

fn copy_state(s: &Setup, id: mailcrane_store::OutboxId) -> SentCopy {
    s.store
        .get_outbox(id)
        .expect("read")
        .expect("there")
        .sent_copy
}

#[tokio::test]
async fn the_sent_folder_gets_exactly_one_identical_copy_with_seen() {
    let s = setup(with_sent().await).await;
    let id = send_one_message(&s).await;
    assert_eq!(copy_state(&s, id), SentCopy::Pending);
    assert_eq!(save(&s, &s.imap, NOW).await.saved, 1);
    // Running again (or after a crash) doesn't add a second copy.
    save(&s, &s.imap, NOW + 60).await;

    let copies = s.imap.mailbox().messages("Sent").await;
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].flags, ["\\Seen"]);
    let sent = s.smtp.sent().await;
    assert_eq!(
        copies[0].raw, sent[0].data,
        "byte-identical to what was sent"
    );
    assert!(!String::from_utf8_lossy(&copies[0].raw)
        .to_ascii_lowercase()
        .contains("bcc:"));
    assert_eq!(copy_state(&s, id), SentCopy::Saved);
}

#[tokio::test]
async fn gmail_saves_sent_mail_itself_so_nothing_is_appended() {
    let imap = imap_builder()
        .folder("Sent", vec![])
        .special_use("Sent", "\\Sent")
        .quirk(Quirk::GmailLabels)
        .start()
        .await
        .expect("imap");
    let s = setup(imap).await;
    let id = send_one_message(&s).await;
    assert_eq!(save(&s, &s.imap, NOW).await.skipped, 1);
    assert!(s.imap.mailbox().messages("Sent").await.is_empty());
    assert!(!s
        .imap
        .received_commands()
        .await
        .iter()
        .any(|c| c == "APPEND"));
    assert_eq!(copy_state(&s, id), SentCopy::Skipped);
}

#[tokio::test]
async fn always_and_never_override_detection() {
    let gmail = imap_builder()
        .folder("Sent", vec![])
        .special_use("Sent", "\\Sent")
        .quirk(Quirk::GmailLabels)
        .start()
        .await
        .expect("imap");
    let s = setup(gmail).await;
    s.store
        .set_save_sent_copy(s.account, SaveSentCopy::Always)
        .expect("setting");
    send_one_message(&s).await;
    assert_eq!(save(&s, &s.imap, NOW).await.saved, 1);
    assert_eq!(s.imap.mailbox().messages("Sent").await.len(), 1);

    let plain = setup(with_sent().await).await;
    plain
        .store
        .set_save_sent_copy(plain.account, SaveSentCopy::Never)
        .expect("setting");
    send_one_message(&plain).await;
    assert_eq!(save(&plain, &plain.imap, NOW).await.skipped, 1);
    assert!(plain.imap.mailbox().messages("Sent").await.is_empty());
}

#[tokio::test]
async fn without_a_sent_folder_one_is_created_and_used() {
    let imap = imap_builder().start().await.expect("imap");
    let s = setup(imap).await;
    send_one_message(&s).await;
    assert_eq!(save(&s, &s.imap, NOW).await.saved, 1);
    assert!(s
        .imap
        .mailbox()
        .folder_names()
        .await
        .contains(&SENT_FOLDER_NAME.to_string()));
    assert_eq!(s.imap.mailbox().messages(SENT_FOLDER_NAME).await.len(), 1);
}

#[tokio::test]
async fn a_failed_copy_retries_and_never_resends_the_message() {
    let s = setup(with_sent().await).await;
    let id = send_one_message(&s).await;

    // The connection drops at SELECT: the copy waits, the message stays sent.
    let flaky = FakeImap::builder()
        .mailbox(s.imap.mailbox().clone())
        .user(USER, PASSWORD)
        .special_use("Sent", "\\Sent")
        .fault(Fault::DropAfterCommands(4))
        .start()
        .await
        .expect("imap");
    let report = save(&s, &flaky, NOW).await;
    assert_eq!((report.saved, report.remaining), (0, 1));
    assert_eq!(copy_state(&s, id), SentCopy::Pending);
    assert_eq!(save(&s, &s.imap, NOW + 1).await.saved, 0, "not due yet");
    assert_eq!(save(&s, &s.imap, NOW + 5).await.saved, 1);
    assert_eq!(s.imap.mailbox().messages("Sent").await.len(), 1);

    // A permanent refusal: shown as "sent, not saved", retried by hand.
    let other = send_one_message(&s).await;
    s.imap.mailbox().delete_folder("Sent").await;
    s.store
        .set_save_sent_copy(s.account, SaveSentCopy::Always)
        .expect("setting");
    // The folder is gone but still known locally; APPEND is refused.
    let report = save(&s, &s.imap, NOW + 10).await;
    assert_eq!(report.failed, 1);
    assert_eq!(copy_state(&s, other), SentCopy::Failed);
    let attention = s.store.list_outbox(s.account).expect("list");
    assert_eq!(attention.len(), 1);
    assert_eq!(attention[0].id, other);
    retry(&s.store, other, NOW + 20).expect("retried");
    assert_eq!(copy_state(&s, other), SentCopy::Pending);

    // Through all of that, each message went over SMTP exactly once.
    assert_eq!(s.smtp.sent().await.len(), 2);
}
