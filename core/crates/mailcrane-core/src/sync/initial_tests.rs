//! Initial sync against the testkit's fake IMAP server. Never a real mailbox.

use super::{FailureLog, FolderSyncer, SyncConfig, SyncError};
use crate::events::{CoreEvent, EventListener, SyncErrorKind};
use mailcrane_imap::{ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{
    AccountId, BodyState, FolderId, FolderRole, MessageId, NewAccount, NewFolder, Security, Store,
    VirtualRole,
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
            selectable: true,
            virtual_role: None,
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
async fn a_folder_too_big_for_one_search_line_syncs_newest_first() {
    // Issue #282: Gmail's `* SEARCH` answer for a big inbox was one line
    // longer than the 64 KiB limit (LineTooLong on UID SEARCH). 200 000
    // UIDs in one answer would be about 1.3 MB.
    const COUNT: u32 = 200_000;
    let imap = fake(raws(usize::try_from(COUNT).expect("fits")), &[]).await;
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

    // First sync: the newest 200, one window.
    assert_eq!(syncer.sync_folder(folder).await.expect("first sync"), 200);
    let mut uids = store.list_uids(folder).expect("uids");
    uids.sort_unstable();
    assert_eq!(uids, range(COUNT - 199, COUNT));

    // Refresh: changes and new mail, still within the line limit.
    syncer.sync_changes(folder).await.expect("refresh");
    // Older mail: the next 200 below the oldest stored.
    assert_eq!(
        syncer.load_older(folder, 200).await.expect("load older"),
        200
    );
    let mut uids = store.list_uids(folder).expect("uids");
    uids.sort_unstable();
    assert_eq!(uids, range(COUNT - 399, COUNT));
    assert!(
        !events
            .all()
            .iter()
            .any(|e| matches!(e, CoreEvent::SyncError { .. })),
        "{:?}",
        events.all()
    );
    // Each search asked for at most one window.
    let searches = imap
        .received_commands()
        .await
        .iter()
        .filter(|c| *c == "UID SEARCH")
        .count();
    assert!(searches <= 4, "{searches} searches");
}

#[tokio::test]
async fn gmails_starred_and_important_are_virtual_folders_with_no_role() {
    // #390: listed with \Flagged and \Important, "[Gmail]/Starred" and
    // "[Gmail]/Important" are virtual folders, shown like the rest of
    // "[Gmail]" instead of by their raw paths.
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", raws(1))
        .gmail_layout()
        .start()
        .await
        .expect("fake imap starts");
    let (store, account, _) = setup();
    let mut session = connect(&imap).await;
    let listed = session.list_folders().await.expect("list");
    mailcrane_imap::save_folders(&store, account, &listed).expect("save");

    let folders = store.list_folders(account).expect("folders");
    let details = |name: &str| {
        folders
            .iter()
            .find(|f| f.details.remote_name == name)
            .unwrap_or_else(|| panic!("{name} listed"))
            .details
            .clone()
    };
    let role = |name: &str| details(name).role;
    // Virtual folders: a virtual role and no role, so nothing moves into them.
    assert_eq!(
        details("[Gmail]/Starred").virtual_role,
        Some(VirtualRole::Starred)
    );
    assert_eq!(
        details("[Gmail]/Important").virtual_role,
        Some(VirtualRole::Important)
    );
    assert_eq!(role("[Gmail]/Starred"), None);
    assert_eq!(role("[Gmail]/Important"), None);
    assert!(folders
        .iter()
        .filter(|f| f.details.role.is_some())
        .all(|f| f.details.virtual_role.is_none()));
    // The others are unchanged.
    assert_eq!(role("INBOX"), Some(FolderRole::Inbox));
    assert_eq!(role("[Gmail]/All Mail"), Some(FolderRole::All));
    assert_eq!(role("[Gmail]/Sent Mail"), Some(FolderRole::Sent));
    assert_eq!(role("[Gmail]/Drafts"), Some(FolderRole::Drafts));
    assert_eq!(role("[Gmail]/Spam"), Some(FolderRole::Spam));
    assert_eq!(role("[Gmail]/Trash"), Some(FolderRole::Trash));
    assert_eq!(role("[Gmail]"), None);
}

#[tokio::test]
async fn important_counts_only_on_gmail_but_flagged_counts_anywhere() {
    // #390 review: \Important isn't in RFC 6154, so on a server without the
    // Gmail quirk (no X-GM-EXT-1) a folder carrying it stays an ordinary
    // folder. \Flagged is RFC 6154 and means starred everywhere.
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", raws(1))
        .folder("Important", Vec::new())
        .special_use("Important", "\\Important")
        .folder("Flagged", Vec::new())
        .special_use("Flagged", "\\Flagged")
        .start()
        .await
        .expect("fake imap starts");
    let (store, account, _) = setup();
    let mut session = connect(&imap).await;
    assert!(!session
        .quirks()
        .contains(&mailcrane_imap::ServerQuirk::GmailLabels));
    let listed = session.list_folders().await.expect("list");
    mailcrane_imap::save_folders(&store, account, &listed).expect("save");
    let folders = store.list_folders(account).expect("folders");
    let virtual_role = |name: &str| {
        folders
            .iter()
            .find(|f| f.details.remote_name == name)
            .unwrap_or_else(|| panic!("{name} listed"))
            .details
            .virtual_role
    };
    assert_eq!(virtual_role("Important"), None);
    assert_eq!(virtual_role("Flagged"), Some(VirtualRole::Starred));
}

#[tokio::test]
async fn gmail_layout_syncs_every_folder_and_never_selects_the_noselect_parent() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", raws(3))
        .gmail_layout()
        .start()
        .await
        .expect("fake imap starts");
    let (store, account, _) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    let listed = session.list_folders().await.expect("list");
    mailcrane_imap::save_folders(&store, account, &listed).expect("save");

    let folders = store.list_folders(account).expect("folders");
    let parent = folders
        .iter()
        .find(|f| f.details.remote_name == "[Gmail]")
        .expect("[Gmail] stays in the folder tree");
    assert!(!parent.details.selectable);
    let sent = folders
        .iter()
        .find(|f| f.details.remote_name == "[Gmail]/Sent Mail")
        .expect("child with a space in its name");
    assert!(sent.details.selectable);

    let mut syncer = FolderSyncer::new(
        &store,
        &mut session,
        &events,
        account,
        SyncConfig::default(),
    );
    for folder in &folders {
        let name = &folder.details.remote_name;
        syncer
            .sync_folder(folder.id)
            .await
            .unwrap_or_else(|e| panic!("sync {name}: {e:?}"));
        syncer
            .sync_changes(folder.id)
            .await
            .unwrap_or_else(|e| panic!("refresh {name}: {e:?}"));
        syncer
            .load_older(folder.id, 10)
            .await
            .unwrap_or_else(|e| panic!("load older {name}: {e:?}"));
    }
    assert_eq!(
        syncer.sync_changes(parent.id).await.expect("skip").strategy,
        super::super::Strategy::NotSelectable
    );
    assert!(events
        .all()
        .iter()
        .all(|e| !matches!(e, CoreEvent::SyncError { .. })));

    let selected = imap.selected_folders().await;
    assert!(!selected.iter().any(|n| n == "[Gmail]"), "{selected:?}");
    assert!(
        selected.iter().any(|n| n == "[Gmail]/Sent Mail"),
        "{selected:?}"
    );
    let inbox = folders
        .iter()
        .find(|f| f.details.remote_name == "INBOX")
        .expect("inbox");
    assert_eq!(store.list_uids(inbox.id).expect("uids").len(), 3);
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
            selectable: true,
            virtual_role: None,
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
            store: "none",
            field: "none",
            sqlite: "none",
            sqlite_code: 0,
        }
    );
    // No server text, folder name, account or credential in it. (Every
    // field is a `&'static str` or an enum, so none could get in.)
    let text = format!("{log:?}");
    for secret in ["no such folder", "NONEXISTENT", "Private", USER, PASSWORD] {
        assert!(!text.contains(secret), "{text} holds {secret:?}");
    }
}

#[test]
fn a_database_failure_logs_the_sqlite_code_but_no_values() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE m (folder_id INTEGER, uid INTEGER, subject TEXT, UNIQUE (folder_id, uid));
         INSERT INTO m VALUES (1, 42, 'Private subject');",
    )
    .expect("schema");
    let error = conn
        .execute_batch("INSERT INTO m VALUES (1, 42, 'Private subject');")
        .expect_err("duplicate");
    let error = SyncError::Store(mailcrane_store::StoreError::from(error));

    let log = FailureLog::new("sync", &error, Some("UID FETCH"));
    assert_eq!(
        log,
        FailureLog {
            stage: "sync",
            kind: SyncErrorKind::Storage,
            error: "Store",
            command: "UID FETCH",
            store: "Sqlite",
            field: "none",
            sqlite: "SQLITE_CONSTRAINT_UNIQUE",
            sqlite_code: rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE,
        }
    );
    let text = format!("{log:?}");
    for secret in ["Private", "42", "folder_id", "m.uid"] {
        assert!(!text.contains(secret), "{text} holds {secret:?}");
    }

    // The same through the IMAP error (save_folders).
    let wrapped = SyncError::Imap(mailcrane_imap::ImapError::Store(
        mailcrane_store::StoreError::Invalid { field: "uid" },
    ));
    let log = FailureLog::new("list folders", &wrapped, Some("LIST"));
    assert_eq!(
        (log.store, log.field, log.sqlite),
        ("Invalid", "uid", "none")
    );
}
