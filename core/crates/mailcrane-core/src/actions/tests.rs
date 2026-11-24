//! Message actions against the testkit's fake IMAP server. Never a real
//! mailbox.

use super::*;
use crate::events::{ActionKind, CoreEvent, EventListener, SyncErrorKind};
use crate::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{AccountId, FolderId, MessageId, NewAccount, Security, Store};
use mailcrane_testkit::{FakeImap, Fault, Quirk};
use std::sync::Mutex;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const NOW: i64 = 1_800_000_000;

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
}

fn raw(n: usize) -> Vec<u8> {
    format!("From: bob@example.org\r\nSubject: Message {n}\r\n\r\nBody {n}\r\n").into_bytes()
}

async fn fake(quirks: &[Quirk], faults: &[Fault]) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", (1..=3).map(raw).collect())
        .folder("Archive", Vec::new())
        .folder("Trash", Vec::new())
        .folder("Work", Vec::new())
        .special_use("Archive", "\\Archive")
        .special_use("Trash", "\\Trash");
    for quirk in quirks {
        builder = builder.quirk(*quirk);
    }
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

struct Setup {
    store: Store,
    account: AccountId,
    inbox: FolderId,
    folders: Vec<(String, FolderId)>,
}

impl Setup {
    fn folder(&self, name: &str) -> FolderId {
        self.folders
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
            .expect("folder")
    }

    /// The local message with this UID in the inbox.
    fn message(&self, uid: u32) -> MessageId {
        self.store
            .list_messages(self.inbox, 100, 0)
            .expect("list")
            .into_iter()
            .find(|m| m.details.uid == uid)
            .map(|m| m.id)
            .expect("message")
    }

    fn visible_uids(&self, folder: FolderId) -> Vec<u32> {
        let mut uids: Vec<u32> = self
            .store
            .list_messages(folder, 100, 0)
            .expect("list")
            .iter()
            .map(|m| m.details.uid)
            .collect();
        uids.sort_unstable();
        uids
    }
}

/// An account whose folders and inbox were synced from `imap`.
async fn setup(imap: &FakeImap, store: Store) -> Setup {
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
    let mut session = connect(imap).await;
    let listed = session.list_folders().await.expect("list");
    let ids = save_folders(&store, account, &listed).expect("save");
    let folders: Vec<(String, FolderId)> =
        listed.iter().map(|f| f.raw_name.clone()).zip(ids).collect();
    let inbox = folders
        .iter()
        .find(|(n, _)| n == "INBOX")
        .map(|(_, id)| *id)
        .expect("inbox");
    {
        let listener = Events::default();
        let mut syncer = FolderSyncer::new(
            &store,
            &mut session,
            &listener,
            account,
            SyncConfig::default(),
        );
        syncer.sync_folder(inbox).await.expect("sync");
    }
    session.logout().await;
    Setup {
        store,
        account,
        inbox,
        folders,
    }
}

async fn replay_now(imap: &FakeImap, s: &Setup, listener: &Events, now: i64) -> ReplayReport {
    let mut session = connect(imap).await;
    let report = replay(&s.store, &mut session, listener, s.account, now)
        .await
        .expect("replay");
    session.logout().await;
    report
}

fn act(s: &Setup, uid: u32, action: Action, listener: &Events) -> Applied {
    apply_action(&s.store, &[s.message(uid)], action, NOW, listener)
        .expect("applies")
        .expect("something to do")
}

async fn server_flags(imap: &FakeImap, folder: &str, uid: u32) -> Vec<String> {
    imap.mailbox()
        .flags(folder, uid)
        .await
        .expect("message on the server")
}

async fn server_uids(imap: &FakeImap, folder: &str) -> Vec<u32> {
    imap.mailbox()
        .messages(folder)
        .await
        .iter()
        .map(|m| m.uid)
        .collect()
}

#[tokio::test]
async fn flag_actions_apply_locally_then_reach_the_server() {
    let imap = fake(&[], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();

    act(&s, 1, Action::MarkRead, &events);
    act(&s, 2, Action::Flag, &events);
    let header = |uid| {
        s.store
            .get_message_header(s.message(uid))
            .expect("read")
            .expect("exists")
            .details
            .flags
    };
    assert!(header(1).seen, "applied locally at once");
    assert!(header(2).flagged);
    assert!(events
        .all()
        .contains(&CoreEvent::MessagesChanged { folder_id: s.inbox }));
    assert!(
        server_flags(&imap, "INBOX", 1).await.is_empty(),
        "not yet sent"
    );

    let report = replay_now(&imap, &s, &events, NOW).await;
    assert_eq!((report.done, report.failed, report.remaining), (2, 0, 0));
    assert_eq!(server_flags(&imap, "INBOX", 1).await, ["\\Seen"]);
    assert_eq!(server_flags(&imap, "INBOX", 2).await, ["\\Flagged"]);

    act(&s, 1, Action::MarkUnread, &events);
    act(&s, 2, Action::Unflag, &events);
    replay_now(&imap, &s, &events, NOW).await;
    assert!(server_flags(&imap, "INBOX", 1).await.is_empty());
    assert!(server_flags(&imap, "INBOX", 2).await.is_empty());
    assert_eq!(s.store.count_pending_ops(s.account).expect("count"), 0);
}

#[tokio::test]
async fn archive_move_and_delete_reach_the_server() {
    let imap = fake(&[], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();

    act(&s, 1, Action::Archive, &events);
    act(
        &s,
        2,
        Action::Move {
            to: s.folder("Work"),
        },
        &events,
    );
    act(&s, 3, Action::Delete, &events);
    assert!(s.visible_uids(s.inbox).is_empty(), "hidden at once");

    let report = replay_now(&imap, &s, &events, NOW).await;
    assert_eq!(report.done, 3);
    assert!(server_uids(&imap, "INBOX").await.is_empty());
    assert_eq!(server_uids(&imap, "Archive").await.len(), 1);
    assert_eq!(server_uids(&imap, "Work").await.len(), 1);
    assert_eq!(server_uids(&imap, "Trash").await.len(), 1);
    assert!(s.store.list_uids(s.inbox).expect("uids").is_empty());
    let received = imap.received_commands().await;
    assert!(received.iter().any(|c| c == "UID MOVE"));
    assert!(!received.iter().any(|c| c == "EXPUNGE"));
}

#[tokio::test]
async fn delete_in_trash_deletes_for_good_with_uid_expunge() {
    let imap = fake(&[], &[]).await;
    imap.mailbox().deliver("Trash", raw(9)).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let trash = s.folder("Trash");
    {
        let mut session = connect(&imap).await;
        let listener = Events::default();
        let mut syncer = FolderSyncer::new(
            &s.store,
            &mut session,
            &listener,
            s.account,
            SyncConfig::default(),
        );
        syncer.sync_folder(trash).await.expect("sync trash");
    }
    let id = s.store.list_messages(trash, 10, 0).expect("list")[0].id;
    let events = Events::default();
    apply_action(&s.store, &[id], Action::Delete, NOW, &events)
        .expect("applies")
        .expect("done");
    replay_now(&imap, &s, &events, NOW).await;
    assert!(server_uids(&imap, "Trash").await.is_empty());
    let received = imap.received_commands().await;
    assert!(received.iter().any(|c| c == "UID EXPUNGE"));
    assert!(!received.iter().any(|c| c == "EXPUNGE"));
}

#[tokio::test]
async fn archive_without_an_archive_folder_is_an_error_and_changes_nothing() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw(1)])
        .start()
        .await
        .expect("starts");
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let result = apply_action(
        &s.store,
        &[s.message(1)],
        Action::Archive,
        NOW,
        &Events::default(),
    );
    assert!(matches!(result, Err(ActionError::NoArchiveFolder)));
    assert_eq!(s.visible_uids(s.inbox), [1]);
    assert_eq!(s.store.count_pending_ops(s.account).expect("count"), 0);
}

#[tokio::test]
async fn offline_actions_survive_a_reopen_and_replay_later() {
    let imap = fake(&[], &[]).await;
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("m.db");
    let s = setup(&imap, Store::open(&path).expect("store")).await;
    let events = Events::default();
    act(&s, 1, Action::MarkRead, &events);
    act(&s, 2, Action::Archive, &events);

    // The connection drops at SELECT (login takes the first three
    // commands): the first try fails for now and backs off.
    let down = fake(&[], &[Fault::DropAfterCommands(4)]).await;
    let mut session = connect(&down).await;
    let report = replay(&s.store, &mut session, &events, s.account, NOW)
        .await
        .expect("replay");
    assert_eq!(report.done, 0);
    assert_eq!(report.remaining, 2);
    let (account, inbox) = (s.account, s.inbox);
    drop(s);

    let store = Store::open(&path).expect("reopen");
    let ops = store.pending_ops(account).expect("ops");
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].attempts, 1);
    assert_eq!(ops[0].next_attempt_at, NOW + 5);
    assert_eq!(store.list_messages(inbox, 10, 0).expect("list").len(), 2);

    // Not due yet: nothing is sent.
    let mut session = connect(&imap).await;
    let early = replay(&store, &mut session, &events, account, NOW + 1)
        .await
        .expect("replay");
    assert_eq!((early.done, early.remaining), (0, 2));
    let later = replay(&store, &mut session, &events, account, NOW + 5)
        .await
        .expect("replay");
    session.logout().await;
    assert_eq!((later.done, later.remaining), (2, 0));
    assert_eq!(server_flags(&imap, "INBOX", 1).await, ["\\Seen"]);
    assert_eq!(server_uids(&imap, "Archive").await.len(), 1);
}

#[tokio::test]
async fn without_move_it_copies_flags_and_uid_expunges() {
    let imap = fake(&[Quirk::NoMove], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    act(&s, 1, Action::Archive, &events);
    replay_now(&imap, &s, &events, NOW).await;
    assert_eq!(server_uids(&imap, "INBOX").await, [2, 3]);
    assert_eq!(server_uids(&imap, "Archive").await.len(), 1);
    let received = imap.received_commands().await;
    for want in ["UID COPY", "UID STORE", "UID EXPUNGE"] {
        assert!(received.iter().any(|c| c == want), "sent {want}");
    }
    assert!(!received.iter().any(|c| c == "UID MOVE"));
    assert!(!received.iter().any(|c| c == "EXPUNGE"));
}

#[tokio::test]
async fn without_move_or_uidplus_it_never_sends_a_bare_expunge() {
    let imap = fake(&[Quirk::NoMove, Quirk::NoUidplus], &[]).await;
    // Another client's message marked \Deleted must survive.
    imap.mailbox().add_flag("INBOX", 3, "\\Deleted").await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    act(&s, 1, Action::Archive, &events);
    let report = replay_now(&imap, &s, &events, NOW).await;
    assert_eq!(report.done, 1);
    let received = imap.received_commands().await;
    assert!(!received.iter().any(|c| c == "EXPUNGE"), "{received:?}");
    assert!(!received.iter().any(|c| c == "UID EXPUNGE"));
    // The original stays flagged \Deleted on the server, hidden locally.
    assert_eq!(server_flags(&imap, "INBOX", 1).await, ["\\Deleted"]);
    assert!(server_uids(&imap, "INBOX").await.contains(&3));
    assert_eq!(s.visible_uids(s.inbox), [2, 3]);
    assert_eq!(s.store.list_uids(s.inbox).expect("uids"), [1, 2, 3]);
}

#[tokio::test]
async fn a_permanent_no_undoes_the_change_and_fires_action_failed() {
    let imap = fake(&[], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    let id = s.message(1);
    act(
        &s,
        1,
        Action::Move {
            to: s.folder("Work"),
        },
        &events,
    );
    // Someone deleted the folder on the server meanwhile.
    imap.mailbox().delete_folder("Work").await;
    let report = replay_now(&imap, &s, &events, NOW).await;
    assert_eq!(report.failed, 1);
    assert_eq!(s.visible_uids(s.inbox), [1, 2, 3], "move undone");
    assert!(events.all().contains(&CoreEvent::ActionFailed {
        account_id: s.account,
        action: ActionKind::Move,
        message_ids: vec![id],
        reason: SyncErrorKind::Protocol,
    }));
    assert_eq!(s.store.count_pending_ops(s.account).expect("count"), 0);
}

#[tokio::test]
async fn replaying_a_move_after_a_dropped_connection_makes_no_duplicate() {
    let imap = fake(&[], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    act(&s, 1, Action::Archive, &events);
    // The server applies the MOVE, but the connection drops before the
    // client hears back: emulate it by moving on the server first.
    imap.mailbox()
        .move_messages("INBOX", |uid| uid == 1, "Archive")
        .await;
    let report = replay_now(&imap, &s, &events, NOW).await;
    assert_eq!(report.done, 1);
    assert_eq!(server_uids(&imap, "Archive").await.len(), 1, "no duplicate");
    assert_eq!(server_uids(&imap, "INBOX").await, [2, 3]);
}

#[tokio::test]
async fn a_copy_fallback_interrupted_after_copy_does_not_copy_again() {
    let imap = fake(&[Quirk::NoMove], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    act(&s, 1, Action::Archive, &events);
    // Answer login (three commands), SELECT and UID COPY, then drop at the
    // UID STORE.
    let flaky = FakeImap::builder()
        .mailbox(imap.mailbox().clone())
        .user(USER, PASSWORD)
        .quirk(Quirk::NoMove)
        .fault(Fault::DropAfterCommands(6))
        .start()
        .await
        .expect("starts");
    let mut session = connect(&flaky).await;
    let first = replay(&s.store, &mut session, &events, s.account, NOW)
        .await
        .expect("replay");
    assert_eq!(first.remaining, 1);
    assert_eq!(server_uids(&imap, "Archive").await.len(), 1, "copied once");
    let ops = s.store.pending_ops(s.account).expect("ops");
    assert_eq!(ops[0].step, 1, "progress saved");

    let second = replay_now(&imap, &s, &events, NOW + 60).await;
    assert_eq!(second.done, 1);
    assert_eq!(
        server_uids(&imap, "Archive").await.len(),
        1,
        "not copied twice"
    );
    assert_eq!(server_uids(&imap, "INBOX").await, [2, 3]);
}

#[tokio::test]
async fn read_then_unread_before_replay_sends_nothing() {
    let imap = fake(&[], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    act(&s, 1, Action::MarkRead, &events);
    let second = act(&s, 1, Action::MarkUnread, &events);
    assert!(!second.queued);
    let before = imap.received_commands().await.len();
    let report = replay_now(&imap, &s, &events, NOW).await;
    assert_eq!(report.done, 0);
    let received = imap.received_commands().await;
    assert!(
        !received[before..].iter().any(|c| c == "UID STORE"),
        "{received:?}"
    );
}

#[tokio::test]
async fn messages_already_in_the_wanted_state_are_skipped() {
    let imap = fake(&[], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    let first = act(&s, 1, Action::MarkUnread, &events);
    assert!(!first.queued, "already unread");
    assert_eq!(s.store.count_pending_ops(s.account).expect("count"), 0);
}

#[tokio::test]
async fn bad_requests_change_nothing() {
    let imap = fake(&[], &[]).await;
    let s = setup(&imap, Store::open_in_memory().expect("store")).await;
    let events = Events::default();
    let unknown = apply_action(&s.store, &[MessageId(999)], Action::Flag, NOW, &events);
    assert!(matches!(unknown, Err(ActionError::UnknownMessage)));
    let bad_folder = apply_action(
        &s.store,
        &[s.message(1)],
        Action::Move { to: FolderId(999) },
        NOW,
        &events,
    );
    assert!(matches!(bad_folder, Err(ActionError::UnknownFolder)));
    assert_eq!(
        apply_action(&s.store, &[], Action::Flag, NOW, &events).expect("ok"),
        None
    );
    assert_eq!(s.store.count_pending_ops(s.account).expect("count"), 0);
    assert!(events.all().is_empty());
}
