//! The user changes a snooze row while a sync is carrying it out (#431,
//! review of #455): each test hands `snooze_one` / `wake_one` the row as
//! the sync read it, after the user's change. Nothing may be sent for a
//! changed row, and the user's change is kept.

use super::*;
use crate::events::{CoreEvent, EventListener};
use crate::snooze::{list_snoozed, snooze, unsnooze, wake_due};
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

async fn flags(imap: &FakeImap, folder: &str) -> Vec<Vec<String>> {
    imap.mailbox()
        .messages(folder)
        .await
        .into_iter()
        .map(|m| m.flags)
        .collect()
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
    let outcome = Box::pin(wake_one(&store, &mut session, &read)).await;
    assert!(matches!(outcome, Ok(Outcome::Changed)));
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

#[tokio::test]
async fn a_wake_whose_message_left_snoozed_keeps_a_new_snooze() {
    let (imap, store, account) = Box::pin(snoozed_and_due()).await;
    let read = row(&store, account);
    // Another install woke it into the Inbox; the user snoozes it here,
    // from the Inbox, while this sync is about to wake it.
    assert!(
        imap.mailbox()
            .move_messages("Snoozed", |_| true, "INBOX")
            .await
    );
    Box::pin(sync_all(&imap, &store, account)).await;
    let inbox = folder(&store, account, "INBOX");
    let copy = shown(&store, inbox);
    assert_eq!(copy.len(), 1);
    snooze(&store, &copy, LATER_WAKE, &Quiet).expect("re-snooze");
    let mut session = connect(&imap).await;
    let outcome = Box::pin(wake_one(&store, &mut session, &read)).await;
    assert!(matches!(outcome, Ok(Outcome::Changed)));
    let now = row(&store, account);
    assert_eq!(
        (now.state, now.wake_at),
        (SnoozeState::ToSnooze, LATER_WAKE)
    );
    assert!(shown(&store, inbox).is_empty(), "still snoozed, so hidden");
    // The next sync carries the new snooze out.
    Box::pin(run_sync(&imap, &store, account, LATER_WAKE - 3600)).await;
    assert_eq!(row(&store, account).state, SnoozeState::Snoozed);
    assert_eq!(imap.mailbox().messages("Snoozed").await.len(), 1);
}

#[tokio::test]
async fn a_snooze_unsnoozed_before_its_move_sends_nothing() {
    let (imap, store, account) = Box::pin(lunch()).await;
    let inbox = folder(&store, account, "INBOX");
    let ids = shown(&store, inbox);
    snooze(&store, &ids, WAKE, &Quiet).expect("snooze");
    let read = row(&store, account);
    // The user takes it back (an Undo) while the sync is about to move it.
    unsnooze(&store, &ids, NOW, &Quiet).expect("undo");
    let mut session = connect(&imap).await;
    let outcome = Box::pin(snooze_one(&store, &mut session, &read)).await;
    assert!(matches!(outcome, Ok(Outcome::Changed)));
    assert!(list_snoozed(&store, Some(account))
        .expect("list")
        .is_empty());
    assert_eq!(shown(&store, inbox), ids, "shown again");
    let inbox_flags = flags(&imap, "INBOX").await;
    assert!(
        !inbox_flags
            .iter()
            .flatten()
            .any(|f| f.starts_with(KEYWORD_PREFIX)),
        "no keyword without a row: {inbox_flags:?}"
    );
}

#[tokio::test]
async fn a_new_time_picked_before_the_move_is_the_one_used() {
    let (imap, store, account) = Box::pin(lunch()).await;
    let inbox = folder(&store, account, "INBOX");
    let ids = shown(&store, inbox);
    snooze(&store, &ids, WAKE, &Quiet).expect("snooze");
    let read = row(&store, account);
    // The user picks another time while the sync is about to move it.
    snooze(&store, &ids, LATER_WAKE, &Quiet).expect("new time");
    let mut session = connect(&imap).await;
    let outcome = Box::pin(snooze_one(&store, &mut session, &read)).await;
    assert!(matches!(outcome, Ok(Outcome::Changed)));
    // The next sync snoozes it with the time stored now.
    Box::pin(run_sync(&imap, &store, account, NOW)).await;
    let parked = flags(&imap, "Snoozed").await;
    assert_eq!(parked.len(), 1);
    let keywords: Vec<_> = parked[0]
        .iter()
        .filter(|f| f.starts_with(KEYWORD_PREFIX))
        .cloned()
        .collect();
    assert_eq!(keywords, vec![format_keyword(LATER_WAKE).expect("keyword")]);
}

#[tokio::test]
async fn a_keyword_change_in_snoozed_for_a_changed_row_adds_no_keyword() {
    let (imap, store, account) = Box::pin(lunch()).await;
    let inbox = folder(&store, account, "INBOX");
    snooze(&store, &shown(&store, inbox), WAKE, &Quiet).expect("snooze");
    Box::pin(run_sync(&imap, &store, account, NOW)).await;
    Box::pin(sync_all(&imap, &store, account)).await;
    // Snoozed again from Snoozed (a keyword change in place), then the
    // user takes that back while the sync is about to change the keyword.
    let snoozed = folder(&store, account, "Snoozed");
    let parked = shown(&store, snoozed);
    snooze(&store, &parked, LATER_WAKE, &Quiet).expect("re-snooze");
    let read = row(&store, account);
    unsnooze(&store, &parked, NOW + 60, &Quiet).expect("undo");
    let mut session = connect(&imap).await;
    let outcome = Box::pin(snooze_one(&store, &mut session, &read)).await;
    assert!(matches!(outcome, Ok(Outcome::Changed)));
    let new_keyword = format_keyword(LATER_WAKE).expect("keyword");
    assert!(
        !flags(&imap, "Snoozed")
            .await
            .iter()
            .flatten()
            .any(|f| *f == new_keyword),
        "the changed row's keyword was not set"
    );
    // Taken back: the row wakes it (the message is in Snoozed).
    assert_eq!(row(&store, account).state, SnoozeState::ToWake);
}
