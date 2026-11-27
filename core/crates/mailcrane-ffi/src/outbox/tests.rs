use super::*;
use mailcrane_core::secrets::{Secret, SecretKey};
use mailcrane_store::{NewAccount, Security};
use mailcrane_testkit::{FakeSmtp, Fault, MemorySecretStore};

/// Only a hang guard: a busy machine (a Gradle build, disk or memory
/// pressure) can stall the fake servers for seconds, and a short limit
/// would fail the test for that alone (#409, as in #362).
const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(60);

struct Setup {
    _dir: tempfile::TempDir,
    db_path: String,
    smtp: FakeSmtp,
    secrets: MemorySecretStore,
    account: i64,
}

async fn setup(faults: &[Fault]) -> Setup {
    let mut builder = FakeSmtp::builder();
    for f in faults {
        builder = builder.fault(f.clone());
    }
    let smtp = builder.start().await.expect("fake smtp");
    let dir = tempfile::tempdir().expect("dir");
    let db_path = dir.path().join("o.db").to_string_lossy().into_owned();
    let account = Store::open(&db_path)
        .expect("store")
        .insert_account(&NewAccount {
            display_name: "Ann".into(),
            email: "ann@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: smtp.addr().port(),
            smtp_security: Security::Tls,
            username: "ann@example.org".into(),
        })
        .expect("account")
        .0;
    let secrets = MemorySecretStore::new();
    secrets
        .put(
            &SecretKey::account_password(account),
            &Secret::from_string("pw".into()),
        )
        .expect("password");
    Setup {
        _dir: dir,
        db_path,
        smtp,
        secrets,
        account,
    }
}

fn options(s: &Setup) -> core::SendOptions {
    core::SendOptions {
        timeout: HANG_GUARD,
        extra_root_certs: vec![s.smtp.root_cert_der().to_vec()],
    }
}

fn draft(s: &Setup, subject: &str) -> ComposeDraft {
    ComposeDraft {
        account_id: s.account,
        to: vec!["Bob <bob@example.net>".into()],
        cc: vec![],
        bcc: vec![],
        subject: subject.into(),
        body_text: "Hi".into(),
        in_reply_to: None,
        references: vec![],
        attachments: Vec::new(),
        forward_attachments_of: None,
        rich: None,
    }
}

#[tokio::test]
async fn send_goes_out_at_once_when_it_can() {
    let s = setup(&[]).await;
    let result = send_via_outbox_with(&s.db_path, &s.secrets, draft(&s, "Now"), &options(&s))
        .await
        .expect("sent");
    assert_eq!(
        result,
        SendResult {
            rejected: vec![],
            queued: false
        }
    );
    assert_eq!(s.smtp.sent().await.len(), 1);
    assert!(list_outbox(s.db_path.clone(), s.account)
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn offline_send_is_queued_and_delivered_later() {
    let s = setup(&[]).await;
    s.smtp.set_down(true);
    let result = send_via_outbox_with(&s.db_path, &s.secrets, draft(&s, "Later"), &options(&s))
        .await
        .expect("queued");
    assert!(result.queued);
    let list = list_outbox(s.db_path.clone(), s.account).expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, OutboxEntryState::Queued);
    assert_eq!(list[0].subject, "Later");
    assert_eq!(outbox_accounts(s.db_path.clone()), Ok(vec![s.account]));

    s.smtp.set_down(false);
    // Make it due now.
    let store = Store::open(&s.db_path).expect("store");
    store
        .outbox_retry_at(OutboxId(list[0].id), 1, 0, "network")
        .expect("due");
    drop(store);
    let status = send_outbox_with(&s.db_path, &s.secrets, AccountId(s.account), &options(&s))
        .await
        .expect("sent");
    assert_eq!(status.remaining, 0);
    assert_eq!(s.smtp.sent().await.len(), 1);
    assert!(outbox_accounts(s.db_path.clone())
        .expect("accounts")
        .is_empty());
}

#[tokio::test]
async fn a_permanent_refusal_is_reported_and_leaves_the_outbox_empty() {
    let s = setup(&[Fault::PermFailure]).await;
    let result = send_via_outbox_with(&s.db_path, &s.secrets, draft(&s, "No"), &options(&s)).await;
    assert_eq!(result, Err(SendFailure::Refused { temporary: false }));
    assert!(list_outbox(s.db_path.clone(), s.account)
        .expect("list")
        .is_empty());

    let mut bad = draft(&s, "x");
    bad.to = vec!["nope".into()];
    assert!(matches!(
        send_via_outbox_with(&s.db_path, &s.secrets, bad, &options(&s)).await,
        Err(SendFailure::InvalidRecipients { .. })
    ));
}

#[tokio::test]
async fn outbox_entries_can_be_edited_or_deleted() {
    let s = setup(&[]).await;
    s.smtp.set_down(true);
    for subject in ["Edit me", "Delete me"] {
        send_via_outbox_with(&s.db_path, &s.secrets, draft(&s, subject), &options(&s))
            .await
            .expect("queued");
    }
    let list = list_outbox(s.db_path.clone(), s.account).expect("list");
    let draft_id = edit_outbox(s.db_path.clone(), list[0].id).expect("edited");
    let back = crate::drafts::get_draft(s.db_path.clone(), draft_id).expect("draft");
    assert_eq!(back.subject, "Edit me");
    assert_eq!(delete_outbox(s.db_path.clone(), list[1].id), Ok(()));
    assert_eq!(
        delete_outbox(s.db_path.clone(), list[1].id),
        Err(OutboxFailure::NotFound)
    );
    assert!(list_outbox(s.db_path.clone(), s.account)
        .expect("list")
        .is_empty());
}
