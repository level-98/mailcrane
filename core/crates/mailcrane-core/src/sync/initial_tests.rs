//! Initial sync against the testkit's fake IMAP server. Never a real mailbox.

use super::{FailureLog, FolderSyncer, SyncConfig, SyncError};
use crate::events::{CoreEvent, EventListener, SyncErrorKind};
use mailcrane_imap::{ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{
    AccountId, BodyState, FolderId, MessageId, NewAccount, NewFolder, Security, Store,
};
use mailcrane_testkit::{FakeImap, Fault};
use std::sync::Mutex;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";

#[derive(Default)]
struct Events(Mutex<Vec<CoreEvent>>);

impl EventListener for Events {
    fn on_event(&self, event: CoreEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

impl Events {
    fn all(&self) -> Vec<CoreEvent> {
        self.0.lock().expect("lock").clone()
    }
    fn messages_changed(&self) -> usize {
        self.all()
            .iter()
            .filter(|e| matches!(e, CoreEvent::MessagesChanged { .. }))
            .count()
    }
}

fn raws(count: usize) -> Vec<Vec<u8>> {
    (1..=count)
        .map(|n| {
            format!("From: alice@example.org\r\nSubject: Message {n}\r\n\r\nBody {n}\r\n")
                .into_bytes()
        })
        .collect()
}

async fn fake(messages: Vec<Vec<u8>>, faults: &[Fault]) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", messages);
    for fault in faults {
        builder = builder.fault(fault.clone());
    }
    builder.start().await.expect("fake imap starts")
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

fn setup() -> (Store, AccountId, FolderId) {
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: USER.into(),
        })
        .expect("account");
    let folder = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "INBOX".into(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
        })
        .expect("folder");
    (store, account, folder)
}

fn range(from: u32, to: u32) -> Vec<u32> {
    (from..=to).collect()
}

#[tokio::test]
async fn first_sync_stores_newest_200_and_load_older_adds_the_next_200() {
    let imap = fake(raws(500), &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    let mut syncer = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    );

    assert_eq!(syncer.sync_folder(folder).await.expect("syncs"), 200);
    assert_eq!(store.list_uids(folder).expect("uids"), range(301, 500));
    // 200 headers in batches of 50: at least one event per batch.
    assert!(events.messages_changed() >= 4);
    let stored = store.get_folder(folder).expect("folder").expect("exists");
    assert_eq!(stored.details.uidvalidity, Some(1));
    assert_eq!(stored.details.uidnext, Some(501));

    // Envelope data is decoded into the row.
    let page = store.list_messages(folder, 1000, 0).expect("page");
    let newest = page.iter().find(|m| m.details.uid == 500).expect("uid 500");
    assert_eq!(newest.details.subject, "Message 500");
    assert_eq!(
        newest.details.from_addr.as_deref(),
        Some("alice@example.org")
    );
    assert_eq!(
        newest.details.size,
        i64::try_from(raws(500)[499].len()).expect("size")
    );

    assert_eq!(syncer.load_older(folder, 200).await.expect("older"), 200);
    let uids = store.list_uids(folder).expect("uids");
    assert_eq!(uids, range(101, 500));
    let unique: std::collections::HashSet<_> = uids.iter().collect();
    assert_eq!(unique.len(), uids.len());
    assert_eq!(
        store.list_messages(folder, 1000, 0).expect("all").len(),
        400
    );
}

#[tokio::test]
async fn a_dropped_connection_keeps_finished_batches_and_a_rerun_completes() {
    let (store, account, folder) = setup();
    let events = Events::default();
    {
        // Commands: CAPABILITY, AUTHENTICATE, CAPABILITY, SELECT, SEARCH,
        // FETCH (batch 1), FETCH (batch 2): the connection drops on the 7th.
        let imap = fake(raws(500), &[Fault::DropAfterCommands(7)]).await;
        let mut session = connect(&imap).await;
        let mut syncer = FolderSyncer::new(
            &store,
            &mut session,
            &events,
            account,
            SyncConfig::default(),
        );
        assert!(syncer.sync_folder(folder).await.is_err());
    }
    // Exactly the first batch (the newest 50) is stored, nothing partial.
    assert_eq!(store.list_uids(folder).expect("uids"), range(451, 500));
    assert!(events.all().iter().any(|e| matches!(
        e,
        CoreEvent::SyncError {
            kind: SyncErrorKind::Network | SyncErrorKind::Timeout,
            ..
        }
    )));

    let imap = fake(raws(500), &[]).await;
    let mut session = connect(&imap).await;
    let mut syncer = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    );
    assert_eq!(syncer.sync_folder(folder).await.expect("completes"), 150);
    assert_eq!(store.list_uids(folder).expect("uids"), range(301, 500));
}

#[tokio::test]
async fn changed_uidvalidity_clears_and_refetches_the_folder() {
    let imap = fake(raws(40), &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    let mut syncer = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    );
    syncer.sync_folder(folder).await.expect("syncs");
    assert_eq!(store.list_uids(folder).expect("uids"), range(1, 40));

    // The server renumbers: new UIDVALIDITY, and UIDs start again from 1.
    imap.mailbox().set_uidvalidity(2);
    imap.mailbox().replace_folder("INBOX", raws(5)).await;
    let before = events.all().len();
    syncer.sync_folder(folder).await.expect("resyncs");

    assert_eq!(store.list_uids(folder).expect("uids"), range(1, 5));
    let subjects: Vec<String> = store
        .list_messages(folder, 10, 0)
        .expect("page")
        .into_iter()
        .map(|m| m.details.subject)
        .collect();
    assert_eq!(subjects.len(), 5);
    let stored = store.get_folder(folder).expect("folder").expect("exists");
    assert_eq!(stored.details.uidvalidity, Some(2));
    assert!(events
        .all()
        .iter()
        .skip(before)
        .any(|e| matches!(e, CoreEvent::FolderChanged { .. })));
}

#[tokio::test]
async fn a_message_with_a_malformed_envelope_is_skipped_and_the_rest_stored() {
    let imap = fake(raws(5), &[]).await;
    imap.mailbox().corrupt_envelope("INBOX", 3).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    let mut syncer = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    );
    assert_eq!(syncer.sync_folder(folder).await.expect("syncs"), 4);
    assert_eq!(store.list_uids(folder).expect("uids"), [1, 2, 4, 5]);
    assert!(!events
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::SyncError { .. })));
}

fn message_with_attachment() -> Vec<u8> {
    b"From: alice@example.org\r\nSubject: Report\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
--b1\r\nContent-Type: text/plain\r\n\r\nPlease see the attached report.\r\n\
--b1\r\nContent-Type: application/pdf; name=\"a.pdf\"\r\n\
Content-Disposition: attachment; filename=\"a.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\nSGVsbG8=\r\n--b1--\r\n"
        .to_vec()
}

#[tokio::test]
async fn fetch_body_stores_raw_snippet_and_attachments_without_setting_seen() {
    let mut messages = raws(2);
    messages.push(message_with_attachment());
    let imap = fake(messages.clone(), &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    let mut syncer = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    );
    syncer.sync_folder(folder).await.expect("syncs");

    let header = store
        .list_messages(folder, 10, 0)
        .expect("page")
        .into_iter()
        .find(|m| m.details.uid == 3)
        .expect("message 3");
    assert_eq!(header.body_state, BodyState::None);
    // The list already knows there is an attachment, from BODYSTRUCTURE.
    assert!(header.details.has_attachments);

    syncer.fetch_body(header.id).await.expect("fetches");
    let stored = store
        .get_message_header(header.id)
        .expect("get")
        .expect("exists");
    assert_eq!(stored.body_state, BodyState::Fetched);
    assert!(stored
        .details
        .snippet
        .starts_with("Please see the attached report."));
    assert_eq!(
        store.get_body(header.id).expect("body"),
        messages.get(2).cloned()
    );
    let attachments = store.list_attachments(header.id).expect("attachments");
    assert_eq!(attachments.len(), 1);
    assert_eq!(
        attachments
            .first()
            .and_then(|a| a.details.filename.as_deref()),
        Some("a.pdf")
    );
    // The server's flag state is unchanged: opening did not set \Seen.
    assert_eq!(imap.mailbox().flags("INBOX", 3).await, Some(Vec::new()));

    // A second call uses the cache and does not duplicate attachments.
    syncer.fetch_body(header.id).await.expect("cached");
    assert_eq!(
        store
            .list_attachments(header.id)
            .expect("attachments")
            .len(),
        1
    );
}

#[tokio::test]
async fn unknown_folder_and_message_are_errors_not_panics() {
    let imap = fake(raws(1), &[]).await;
    let (store, account, _) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    let mut syncer = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    );
    assert!(matches!(
        syncer.sync_folder(FolderId(999)).await,
        Err(SyncError::UnknownFolder)
    ));
    assert!(matches!(
        syncer.fetch_body(MessageId(999)).await,
        Err(SyncError::UnknownMessage)
    ));
    assert_eq!(
        events
            .all()
            .iter()
            .filter(|e| matches!(
                e,
                CoreEvent::SyncError {
                    kind: SyncErrorKind::NotFound,
                    ..
                }
            ))
            .count(),
        2
    );
}

#[tokio::test]
async fn a_refused_select_logs_the_command_and_error_but_no_server_text() {
    // The fake answers SELECT of a folder it doesn't have with
    // "NO [NONEXISTENT] no such folder".
    let imap = fake(raws(1), &[]).await;
    let (store, account, _) = setup();
    let missing = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "Private Stuff".into(),
            display_name: "Private Stuff".into(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
        })
        .expect("folder");
    let events = Events::default();
    let mut session = connect(&imap).await;
    let error = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    )
    .sync_folder(missing)
    .await
    .expect_err("SELECT is refused");

    // What report() logs for this failure.
    let log = FailureLog::new("sync", &error, session.last_command());
    assert_eq!(
        log,
        FailureLog {
            stage: "sync",
            kind: SyncErrorKind::Protocol,
            error: "CommandFailed",
            command: "SELECT",
        }
    );
    // No server text, folder name, account or credential in it. (Every
    // field is a `&'static str` or an enum, so none could get in.)
    let text = format!("{log:?}");
    for secret in ["no such folder", "NONEXISTENT", "Private", USER, PASSWORD] {
        assert!(!text.contains(secret), "{text} holds {secret:?}");
    }
}
