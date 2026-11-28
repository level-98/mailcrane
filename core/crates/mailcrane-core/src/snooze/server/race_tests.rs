//! The user changes a snooze row while a sync is carrying it out (#431,
//! review of #455): the test hands `wake_one` the row as the sync read it,
//! after the user's change. Nothing may be sent for it, and the change is
//! kept.

use super::*;
use crate::events::{CoreEvent, EventListener};
use crate::snooze::{list_snoozed, snooze, wake_due};
use crate::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{ImapConfig, ImapSecurity, Secret};
use mailcrane_store::{MessageId, NewAccount, Security};
use mailcrane_testkit::FakeImap;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const NOW: i64 = 1_795_550_400;
const WAKE: i64 = NOW + 12 * 3600;
const LATER_WAKE: i64 = WAKE + 72 * 3600;

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _event: CoreEvent) {}
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

async fn sync_all(imap: &FakeImap, store: &Store, account: AccountId) {
    let mut session = connect(imap).await;
    let listed = session.list_folders().await.expect("list");
    let ids = save_folders(store, account, &listed).expect("save");
    for id in ids {
        let mut syncer =
            FolderSyncer::new(store, &mut session, &Quiet, account, SyncConfig::default());
        Box::pin(syncer.sync_changes(id)).await.expect("sync");
    }
    session.logout().await;
}

async fn run_sync(imap: &FakeImap, store: &Store, account: AccountId, now: i64) {
    let mut session = connect(imap).await;
    Box::pin(sync_snoozes(store, &mut session, &Quiet, account, now))
        .await
        .expect("sync snoozes");
    session.logout().await;
}

/// A server with Lunch in the Inbox, and this install's account on it.
async fn lunch() -> (FakeImap, Store, AccountId) {
    let raw = b"From: Bob <bob@example.net>\r\nTo: alice@example.org\r\nSubject: Lunch\r\n\
        Message-ID: <lunch@example.net>\r\n\r\nHello\r\n"
        .to_vec();
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw])
        .start()
        .await
        .expect("imap");
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: USER.into(),
        })
        .expect("account");
    Box::pin(sync_all(&imap, &store, account)).await;
    (imap, store, account)
}

fn folder(store: &Store, account: AccountId, name: &str) -> FolderId {
    store
        .list_folders(account)
        .expect("folders")
        .into_iter()
        .find(|f| f.details.remote_name == name)
        .map(|f| f.id)
        .expect("folder")
}

fn shown(store: &Store, folder: FolderId) -> Vec<MessageId> {
    store
        .list_messages(folder, 10, 0)
        .expect("list")
        .into_iter()
        .map(|m| m.id)
        .collect()
}

fn row(store: &Store, account: AccountId) -> Snooze {
    list_snoozed(store, Some(account)).expect("list")[0].clone()
}

/// Lunch snoozed on the server, then due: a `to_wake` row at step 0.
async fn snoozed_and_due() -> (FakeImap, Store, AccountId) {
    let (imap, store, account) = Box::pin(lunch()).await;
    let inbox = folder(&store, account, "INBOX");
    snooze(&store, &shown(&store, inbox), WAKE, &Quiet).expect("snooze");
    Box::pin(run_sync(&imap, &store, account, NOW)).await;
    Box::pin(sync_all(&imap, &store, account)).await;
    assert_eq!(wake_due(&store, WAKE + 60, &Quiet).expect("wake").len(), 1);
    (imap, store, account)
}

#[tokio::test]
async fn a_wake_replaced_at_step_0_sends_nothing_and_keeps_the_new_snooze() {
    let (imap, store, account) = Box::pin(snoozed_and_due()).await;
    let read = row(&store, account);
    assert_eq!((read.state, read.step), (SnoozeState::ToWake, 0));
    // The user snoozes it again (the message as open on screen, in
    // Snoozed) while the sync is about to wake it.
    let snoozed = folder(&store, account, "Snoozed");
    let message = store
        .find_message_by_header(snoozed, "lunch@example.net")
        .expect("find")
        .expect("in Snoozed");
    snooze(&store, &[message], LATER_WAKE, &Quiet).expect("re-snooze");
    let mut session = connect(&imap).await;
    let mut read = read;
    let outcome = Box::pin(wake_one(&store, &mut session, &mut read)).await;
    assert!(
        matches!(outcome, Ok(ref changed) if changed.is_empty()),
        "nothing done"
    );
    assert!(
        imap.mailbox().messages("INBOX").await.is_empty(),
        "nothing moved"
    );
    let now = row(&store, account);
    assert_eq!(
        (now.state, now.step, now.wake_at),
        (SnoozeState::ToSnooze, 0, LATER_WAKE)
    );
}

/// #544: the user reads the message while a wake's MOVE is in flight, so
/// the step write after the committed MOVE loses. The UID the server named
/// for the copy (COPYUID, RFC 4315 §3) is still recorded, by its own
/// guarded write, so the next pass finds that copy by its UID instead of
/// by the UIDNEXT floor.
#[tokio::test]
async fn the_copy_uid_is_kept_when_the_step_write_after_a_move_loses() {
    let (imap, store, account) = Box::pin(snoozed_and_due()).await;
    let mut s = row(&store, account);
    assert_eq!(
        (s.state, s.step, s.copy_uid),
        (SnoozeState::ToWake, 0, None)
    );
    let snoozed = folder(&store, account, "Snoozed");
    let message = store
        .find_message_by_header(snoozed, "lunch@example.net")
        .expect("find")
        .expect("in Snoozed");

    // The sync's half of move_uids: record step 1, then MOVE.
    let mut session = connect(&imap).await;
    let target = session.select("INBOX").await.expect("select INBOX");
    assert!(start_move(&store, &mut s, target.uidnext).expect("start"));
    session.select("Snoozed").await.expect("select Snoozed");
    let uids = session
        .uid_search_message_id("lunch@example.net")
        .await
        .expect("search");
    let copy = session
        .uid_move_returning(&uids, "INBOX")
        .await
        .expect("move");
    let copy_uid = copy.expect("the server names the copy");
    session.logout().await;

    // Meanwhile the user read it: the step write loses...
    crate::actions::apply_action(
        &store,
        &[message],
        crate::actions::Action::MarkRead,
        NOW,
        &Quiet,
    )
    .expect("read");
    assert!(!advance(&store, &mut s, STEP_FLAGGED_DELETED, copy).expect("advance"));
    // ...but the copy's UID is kept, at step 1.
    let now = row(&store, account);
    assert_eq!(
        (now.step, now.copy_uid),
        (STEP_COPY_STARTED, Some(copy_uid))
    );
    assert!(copy_is_ours(&now));

    // The next pass finishes the wake on that copy.
    Box::pin(run_sync(&imap, &store, account, WAKE + 120)).await;
    assert_eq!(
        imap.mailbox().messages("INBOX").await.len(),
        1,
        "one copy in the Inbox"
    );
    assert!(
        list_snoozed(&store, Some(account))
            .expect("list")
            .iter()
            .all(|r| r.state != SnoozeState::ToWake),
        "the wake finished"
    );
}
