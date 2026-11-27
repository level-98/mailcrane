//! Snooze (M4-06) against the testkit's fake IMAP server.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::events::{CoreEvent, EventListener};
use mailcrane_core::snooze::{
    format_keyword, learn_from_keywords, list_snoozed, snooze, sync_snoozes, unsnooze, wake_due,
    SnoozeError, SnoozeState,
};
use mailcrane_core::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{AccountId, FolderId, MessageId, NewAccount, Security, Store};
use mailcrane_testkit::{FakeImap, Fault, Quirk};
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const NOW: i64 = 1_795_550_400; // 2026-11-24 20:00 UTC
const WAKE: i64 = NOW + 12 * 3600; // 08:00 the next morning

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _event: CoreEvent) {}
}

fn raw(subject: &str, id: &str) -> Vec<u8> {
    format!(
        "From: Bob <bob@example.net>\r\nTo: alice@example.org\r\nSubject: {subject}\r\n\
         Message-ID: <{id}>\r\n\r\nHello\r\n"
    )
    .into_bytes()
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

/// Adds the account, saves the folders and syncs every one of them.
async fn setup(imap: &FakeImap, store: &Store) -> AccountId {
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
    sync_all(imap, store, account).await;
    account
}

async fn sync_all(imap: &FakeImap, store: &Store, account: AccountId) {
    let mut session = connect(imap).await;
    let listed = session.list_folders().await.expect("list");
    let ids = save_folders(store, account, &listed).expect("save");
    for id in ids {
        let mut syncer =
            FolderSyncer::new(store, &mut session, &Quiet, account, SyncConfig::default());
        syncer.sync_folder(id).await.expect("sync");
    }
    session.logout().await;
}

/// A sync of every folder's changes since the last (new messages, flags,
/// expunges), as the app's periodic sync does.
async fn sync_changes_all(imap: &FakeImap, store: &Store, account: AccountId) {
    let mut session = connect(imap).await;
    let listed = session.list_folders().await.expect("list");
    let ids = save_folders(store, account, &listed).expect("save");
    for id in ids {
        let mut syncer =
            FolderSyncer::new(store, &mut session, &Quiet, account, SyncConfig::default());
        syncer.sync_changes(id).await.expect("sync changes");
    }
    session.logout().await;
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

fn visible(store: &Store, folder: FolderId) -> Vec<MessageId> {
    store
        .list_messages(folder, 100, 0)
        .expect("list")
        .into_iter()
        .map(|m| m.id)
        .collect()
}

async fn run_sync(
    imap: &FakeImap,
    store: &Store,
    account: AccountId,
    now: i64,
) -> mailcrane_core::snooze::SnoozeReport {
    let mut session = connect(imap).await;
    let report = sync_snoozes(store, &mut session, &Quiet, account, now)
        .await
        .expect("sync snoozes");
    session.logout().await;
    report
}

#[tokio::test]
async fn snooze_moves_to_snoozed_with_keyword_and_wakes_back_unread() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw("Lunch", "lunch@example.net")])
        .start()
        .await
        .expect("imap");
    imap.mailbox().add_flag("INBOX", 1, "\\Seen").await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let inbox = folder(&store, account, "INBOX");
    let message = visible(&store, inbox)[0];

    snooze(&store, &[message], WAKE, &Quiet).expect("snooze");
    assert!(visible(&store, inbox).is_empty(), "hidden at once, offline");
    assert_eq!(
        list_snoozed(&store, Some(account)).expect("list")[0].state,
        SnoozeState::ToSnooze
    );

    let report = run_sync(&imap, &store, account, NOW).await;
    assert_eq!(report.snoozed, 1);
    assert!(imap.mailbox().messages("INBOX").await.is_empty());
    let parked = imap.mailbox().messages("Snoozed").await;
    assert_eq!(parked.len(), 1, "Snoozed folder created and used");
    let keyword = format_keyword(WAKE).expect("keyword");
    assert_eq!(keyword, "$snoozed-until-20261125T0800Z");
    assert!(parked[0].flags.contains(&keyword), "{:?}", parked[0].flags);
    let snoozes = list_snoozed(&store, Some(account)).expect("list");
    assert_eq!(snoozes[0].state, SnoozeState::Snoozed);
    assert!(!snoozes[0].device_only());

    sync_all(&imap, &store, account).await;
    assert!(wake_due(&store, WAKE - 1, &Quiet).expect("wake").is_empty());
    // The app was closed at wake time; it wakes on the next launch.
    let woken = wake_due(&store, WAKE + 3600, &Quiet).expect("wake");
    assert_eq!(woken.len(), 1);
    assert_eq!(
        woken[0].subject, "Lunch",
        "Snoozed folder synced before waking"
    );
    assert!(
        wake_due(&store, WAKE + 3600, &Quiet)
            .expect("wake")
            .is_empty(),
        "only once"
    );

    let report = run_sync(&imap, &store, account, WAKE + 3600).await;
    assert_eq!(report.woken, 1);
    assert!(imap.mailbox().messages("Snoozed").await.is_empty());
    let back = imap.mailbox().messages("INBOX").await;
    assert_eq!(back.len(), 1);
    assert!(
        !back[0].flags.iter().any(|f| f.starts_with("$snoozed")),
        "{:?}",
        back[0].flags
    );
    assert!(
        !back[0].flags.contains(&"\\Seen".to_owned()),
        "marked unread"
    );
    assert!(list_snoozed(&store, Some(account))
        .expect("list")
        .is_empty());
    sync_all(&imap, &store, account).await;
    assert_eq!(visible(&store, inbox).len(), 1);
}

#[tokio::test]
async fn a_second_install_learns_the_wake_time_from_the_keyword() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw("Report", "report@example.net")])
        .start()
        .await
        .expect("imap");
    let phone = Store::open_in_memory().expect("store");
    let account = setup(&imap, &phone).await;
    let inbox = folder(&phone, account, "INBOX");
    snooze(&phone, &[visible(&phone, inbox)[0]], WAKE + 30, &Quiet).expect("snooze");
    run_sync(&imap, &phone, account, NOW).await;

    let tablet = Store::open_in_memory().expect("store");
    let account2 = setup(&imap, &tablet).await;
    assert_eq!(learn_from_keywords(&tablet, account2).expect("learn"), 1);
    assert_eq!(
        learn_from_keywords(&tablet, account2).expect("learn"),
        0,
        "once"
    );
    let learnt = list_snoozed(&tablet, Some(account2)).expect("list");
    assert_eq!(learnt.len(), 1);
    assert_eq!(learnt[0].wake_at, WAKE, "to the minute");
    assert_eq!(learnt[0].state, SnoozeState::Snoozed);

    // The tablet wakes it first; the phone then finds nothing to do.
    assert_eq!(wake_due(&tablet, WAKE, &Quiet).expect("wake").len(), 1);
    assert_eq!(run_sync(&imap, &tablet, account2, WAKE).await.woken, 1);
    assert_eq!(imap.mailbox().messages("INBOX").await.len(), 1);
    assert_eq!(wake_due(&phone, WAKE + 60, &Quiet).expect("wake").len(), 1);
    let report = run_sync(&imap, &phone, account, WAKE + 60).await;
    assert_eq!((report.woken, report.failed), (1, 0));
    assert_eq!(
        imap.mailbox().messages("INBOX").await.len(),
        1,
        "no duplicate"
    );
    assert!(list_snoozed(&phone, Some(account))
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn without_custom_keywords_the_wake_time_is_this_device_only() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .quirk(Quirk::NoCustomKeywords)
        .folder("INBOX", vec![raw("Lunch", "lunch@example.net")])
        .start()
        .await
        .expect("imap");
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let inbox = folder(&store, account, "INBOX");
    snooze(&store, &[visible(&store, inbox)[0]], WAKE, &Quiet).expect("snooze");
    assert_eq!(run_sync(&imap, &store, account, NOW).await.snoozed, 1);
    let parked = imap.mailbox().messages("Snoozed").await;
    assert!(!parked[0].flags.iter().any(|f| f.starts_with("$snoozed")));
    assert!(list_snoozed(&store, Some(account)).expect("list")[0].device_only());

    assert_eq!(wake_due(&store, WAKE, &Quiet).expect("wake").len(), 1);
    assert_eq!(run_sync(&imap, &store, account, WAKE).await.woken, 1);
    assert_eq!(imap.mailbox().messages("INBOX").await.len(), 1);
}

#[tokio::test]
async fn unsnooze_before_and_after_the_server_move() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder(
            "INBOX",
            vec![raw("One", "one@example.net"), raw("Two", "two@example.net")],
        )
        .start()
        .await
        .expect("imap");
    imap.mailbox().add_flag("INBOX", 2, "\\Seen").await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let inbox = folder(&store, account, "INBOX");
    let ids = visible(&store, inbox);
    snooze(&store, &ids, WAKE, &Quiet).expect("snooze");

    // Changed their mind before going online: nothing reaches the server.
    let one = ids
        .iter()
        .copied()
        .find(|id| {
            store
                .get_message_header(*id)
                .expect("h")
                .expect("h")
                .details
                .subject
                == "One"
        })
        .expect("one");
    assert_eq!(
        unsnooze(&store, &[one], NOW, &Quiet).expect("unsnooze"),
        [account]
    );
    assert_eq!(visible(&store, inbox), vec![one]);

    assert_eq!(run_sync(&imap, &store, account, NOW).await.snoozed, 1);
    sync_all(&imap, &store, account).await;
    let snoozed = folder(&store, account, "Snoozed");
    let two = visible(&store, snoozed);
    assert_eq!(two.len(), 1);
    assert_eq!(
        unsnooze(&store, &two, NOW + 60, &Quiet).expect("unsnooze"),
        [account]
    );
    assert!(visible(&store, snoozed).is_empty());
    assert_eq!(run_sync(&imap, &store, account, NOW + 60).await.woken, 1);
    let inbox_now = imap.mailbox().messages("INBOX").await;
    assert_eq!(inbox_now.len(), 2);
    let back = inbox_now
        .iter()
        .find(|m| String::from_utf8_lossy(&m.raw).contains("Two"))
        .expect("two");
    assert!(
        back.flags.contains(&"\\Seen".to_owned()),
        "unsnooze keeps it read"
    );
}

#[tokio::test]
async fn re_snoozing_from_the_snoozed_folder_changes_the_keyword() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw("Lunch", "lunch@example.net")])
        .start()
        .await
        .expect("imap");
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let inbox = folder(&store, account, "INBOX");
    snooze(&store, &[visible(&store, inbox)[0]], WAKE, &Quiet).expect("snooze");
    run_sync(&imap, &store, account, NOW).await;
    sync_all(&imap, &store, account).await;
    let snoozed = folder(&store, account, "Snoozed");
    let later = WAKE + 86_400;
    snooze(&store, &visible(&store, snoozed), later, &Quiet).expect("snooze again");
    assert_eq!(
        visible(&store, snoozed).len(),
        1,
        "stays visible in Snoozed"
    );
    assert_eq!(run_sync(&imap, &store, account, NOW + 60).await.snoozed, 1);
    let parked = imap.mailbox().messages("Snoozed").await;
    let keywords: Vec<&String> = parked[0]
        .flags
        .iter()
        .filter(|f| f.starts_with("$snoozed"))
        .collect();
    assert_eq!(keywords, [&format_keyword(later).expect("keyword")]);
    // It goes back to the Inbox, where it came from.
    assert_eq!(wake_due(&store, later, &Quiet).expect("wake").len(), 1);
    run_sync(&imap, &store, account, later).await;
    assert_eq!(imap.mailbox().messages("INBOX").await.len(), 1);
}

#[test]
fn a_message_without_message_id_cannot_be_snoozed() {
    let store = Store::open_in_memory().expect("store");
    let err = snooze(&store, &[MessageId(42)], WAKE, &Quiet).expect_err("unknown");
    assert!(matches!(err, SnoozeError::UnknownMessage));
}

// --- #423: servers without MOVE (COPY + \Deleted + UID EXPUNGE) ---------

/// A server without MOVE, with one Inbox message, and `faults`.
async fn no_move(faults: Vec<Fault>, quirks: &[Quirk]) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw("Lunch", "lunch@example.net")])
        .quirk(Quirk::NoMove);
    for q in quirks {
        builder = builder.quirk(*q);
    }
    for f in faults {
        builder = builder.fault(f);
    }
    builder.start().await.expect("imap")
}

/// A second server on the same messages, for faults that must only hit a
/// later step (the wake, not the snooze).
async fn same_mailbox(imap: &FakeImap, faults: Vec<Fault>) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .mailbox(imap.mailbox())
        .quirk(Quirk::NoMove);
    for f in faults {
        builder = builder.fault(f);
    }
    builder.start().await.expect("imap")
}

/// Messages in `folder` not flagged `\Deleted` (what a client shows).
async fn live(imap: &FakeImap, folder: &str) -> usize {
    imap.mailbox()
        .messages(folder)
        .await
        .iter()
        .filter(|m| !m.flags.iter().any(|f| f == "\\Deleted"))
        .count()
}

const LATER: i64 = NOW + 86_400;

async fn snooze_lunch(imap: &FakeImap, store: &Store) -> AccountId {
    let account = setup(imap, store).await;
    let inbox = folder(store, account, "INBOX");
    snooze(store, &visible(store, inbox), WAKE, &Quiet).expect("snooze");
    account
}

#[tokio::test]
async fn without_move_a_drop_after_copy_doesnt_copy_twice() {
    let imap = no_move(vec![Fault::DropAfterCommandOnce("UID COPY".into())], &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snooze_lunch(&imap, &store).await;

    let first = run_sync(&imap, &store, account, NOW).await;
    assert_eq!(first.remaining, 1, "the drop defers it: {first:?}");
    assert_eq!(
        imap.mailbox().messages("Snoozed").await.len(),
        1,
        "the COPY happened"
    );

    let second = run_sync(&imap, &store, account, LATER).await;
    assert_eq!(second.snoozed, 1, "{second:?}");
    assert_eq!(
        imap.mailbox().messages("Snoozed").await.len(),
        1,
        "exactly one copy"
    );
    assert!(
        imap.mailbox().messages("INBOX").await.is_empty(),
        "original expunged"
    );
    assert_eq!(
        list_snoozed(&store, Some(account)).expect("list")[0].state,
        SnoozeState::Snoozed
    );
}

#[tokio::test]
async fn without_move_a_drop_after_flagging_finishes_the_move() {
    let imap = no_move(vec![Fault::DropAfterCommandOnce("\\Deleted".into())], &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snooze_lunch(&imap, &store).await;

    assert_eq!(run_sync(&imap, &store, account, NOW).await.remaining, 1);
    let report = run_sync(&imap, &store, account, LATER).await;
    assert_eq!(report.snoozed, 1, "{report:?}");
    assert_eq!(imap.mailbox().messages("Snoozed").await.len(), 1);
    assert!(imap.mailbox().messages("INBOX").await.is_empty());
}

async fn refused_store_rolls_back(quirks: &[Quirk]) {
    let refuse = Fault::RefuseCommand {
        folder: Some("INBOX".into()),
        text: "\\Deleted".into(),
    };
    let imap = no_move(vec![refuse], quirks).await;
    let store = Store::open_in_memory().expect("store");
    let account = snooze_lunch(&imap, &store).await;

    let report = run_sync(&imap, &store, account, NOW).await;
    assert_eq!(report.failed, 1, "{report:?}");
    // Only the original is left: the copy is gone (or flagged \Deleted
    // without UIDPLUS, which has no UID EXPUNGE).
    assert_eq!(live(&imap, "Snoozed").await, 0, "no copy left behind");
    assert_eq!(live(&imap, "INBOX").await, 1);
    assert!(list_snoozed(&store, Some(account))
        .expect("list")
        .is_empty());
    let inbox = folder(&store, account, "INBOX");
    assert_eq!(visible(&store, inbox).len(), 1, "shown again");
}

#[tokio::test]
async fn without_move_a_refused_store_removes_the_copy() {
    refused_store_rolls_back(&[]).await;
}

#[tokio::test]
async fn without_move_or_uidplus_a_refused_store_removes_the_copy() {
    refused_store_rolls_back(&[Quirk::NoUidplus]).await;
}

#[tokio::test]
async fn when_even_the_rollback_is_refused_the_snooze_waits_for_repair() {
    // \Deleted refused everywhere: neither the move nor its rollback works.
    let refuse_all = Fault::RefuseCommand {
        folder: None,
        text: "\\Deleted".into(),
    };
    let imap = no_move(vec![refuse_all], &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snooze_lunch(&imap, &store).await;

    let report = run_sync(&imap, &store, account, NOW).await;
    assert_eq!((report.failed, report.remaining), (0, 1), "{report:?}");
    let kept = list_snoozed(&store, Some(account)).expect("list");
    assert_eq!(kept.len(), 1, "never dropped silently");
    assert_eq!(kept[0].step, 4, "needs repair");

    // The server allows it again: the next sync removes the copy, then undoes.
    let healthy = same_mailbox(&imap, Vec::new()).await;
    let report = run_sync(&healthy, &store, account, LATER).await;
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(live(&healthy, "Snoozed").await, 0);
    assert_eq!(live(&healthy, "INBOX").await, 1);
    assert!(list_snoozed(&store, Some(account))
        .expect("list")
        .is_empty());
}

/// Snoozes "Lunch" on a server without MOVE and makes it due.
async fn snoozed_and_due(imap: &FakeImap, store: &Store) -> AccountId {
    let account = snooze_lunch(imap, store).await;
    assert_eq!(run_sync(imap, store, account, NOW).await.snoozed, 1);
    sync_all(imap, store, account).await;
    assert_eq!(wake_due(store, WAKE + 60, &Quiet).expect("wake").len(), 1);
    account
}

#[tokio::test]
async fn without_move_a_drop_after_copy_on_wake_doesnt_copy_twice() {
    let imap = no_move(Vec::new(), &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snoozed_and_due(&imap, &store).await;
    let flaky = same_mailbox(&imap, vec![Fault::DropAfterCommandOnce("UID COPY".into())]).await;

    assert_eq!(
        run_sync(&flaky, &store, account, WAKE + 60).await.remaining,
        1
    );
    let report = run_sync(&flaky, &store, account, LATER).await;
    assert_eq!(report.woken, 1, "{report:?}");
    assert_eq!(
        imap.mailbox().messages("INBOX").await.len(),
        1,
        "exactly one back"
    );
    assert!(imap.mailbox().messages("Snoozed").await.is_empty());
}

#[tokio::test]
async fn waking_with_two_copies_in_snoozed_brings_back_one() {
    let imap = no_move(Vec::new(), &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snoozed_and_due(&imap, &store).await;
    // An older client left a second copy with the same Message-ID.
    imap.mailbox()
        .deliver("Snoozed", raw("Lunch", "lunch@example.net"))
        .await;

    let report = run_sync(&imap, &store, account, WAKE + 60).await;
    assert_eq!(report.woken, 1, "{report:?}");
    assert_eq!(
        imap.mailbox().messages("INBOX").await.len(),
        1,
        "only one comes back"
    );
    assert_eq!(
        live(&imap, "Snoozed").await,
        1,
        "the other stays where it is"
    );
}

#[tokio::test]
async fn without_move_a_refused_store_on_wake_removes_the_copy() {
    let imap = no_move(Vec::new(), &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snoozed_and_due(&imap, &store).await;
    let refusing = same_mailbox(
        &imap,
        vec![Fault::RefuseCommand {
            folder: Some("Snoozed".into()),
            text: "\\Deleted".into(),
        }],
    )
    .await;

    let report = run_sync(&refusing, &store, account, WAKE + 60).await;
    // #436: the wake is kept and retried, not dropped.
    assert_eq!((report.failed, report.remaining), (0, 1), "{report:?}");
    assert_eq!(live(&imap, "INBOX").await, 0, "no copy left in the Inbox");
    assert_eq!(
        live(&imap, "Snoozed").await,
        1,
        "still snoozed on the server"
    );
    assert_keyword_and_row_kept(&imap, &store, account).await;
}

/// After a refused wake: the message in Snoozed still has its wake keyword
/// and the device still has the snooze row, back at step 0 (#436).
async fn assert_keyword_and_row_kept(imap: &FakeImap, store: &Store, account: AccountId) {
    let parked = imap.mailbox().messages("Snoozed").await;
    let keyword = format_keyword(WAKE).expect("keyword");
    assert!(
        parked.iter().any(|m| m.flags.contains(&keyword)),
        "keyword intact: {parked:?}"
    );
    let kept = list_snoozed(store, Some(account)).expect("list");
    assert_eq!(kept.len(), 1, "row kept");
    assert_eq!((kept[0].state, kept[0].step), (SnoozeState::ToWake, 0));
}

/// The woken copy in the Inbox: no snooze keyword, and unread.
async fn assert_woken_copy(imap: &FakeImap) {
    let back = imap.mailbox().messages("INBOX").await;
    assert_eq!(back.len(), 1, "{back:?}");
    assert!(
        !back[0].flags.iter().any(|f| f.starts_with("$snoozed")),
        "{:?}",
        back[0].flags
    );
    assert!(!back[0].flags.iter().any(|f| f == "\\Seen"), "unread");
}

/// A server with MOVE on the same messages.
async fn with_move(imap: &FakeImap, faults: Vec<Fault>) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .mailbox(imap.mailbox());
    for f in faults {
        builder = builder.fault(f);
    }
    builder.start().await.expect("imap")
}

/// Marks the message in Snoozed read, so the wake has to make it unread.
async fn read_in_snoozed(imap: &FakeImap) {
    let uid = imap.mailbox().messages("Snoozed").await[0].uid;
    imap.mailbox().add_flag("Snoozed", uid, "\\Seen").await;
}

#[tokio::test]
async fn a_refused_move_on_wake_keeps_the_keyword_and_the_row() {
    let imap = no_move(Vec::new(), &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snoozed_and_due(&imap, &store).await;
    let refusing = with_move(
        &imap,
        vec![Fault::RefuseCommand {
            folder: None,
            text: "UID MOVE".into(),
        }],
    )
    .await;

    let report = run_sync(&refusing, &store, account, WAKE + 60).await;
    assert_eq!(report.remaining, 1, "{report:?}");
    assert_keyword_and_row_kept(&imap, &store, account).await;
    let snoozed = folder(&store, account, "Snoozed");
    assert_eq!(
        visible(&store, snoozed).len(),
        1,
        "shown in Snoozed meanwhile"
    );

    // The server allows it again: the retry wakes it properly.
    let healthy = with_move(&imap, Vec::new()).await;
    let report = run_sync(&healthy, &store, account, LATER).await;
    assert_eq!(report.woken, 1, "{report:?}");
    assert_woken_copy(&imap).await;
    assert!(list_snoozed(&store, Some(account))
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn a_move_whose_reply_is_lost_still_unsnoozes_the_copy() {
    let imap = no_move(Vec::new(), &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snoozed_and_due(&imap, &store).await;
    read_in_snoozed(&imap).await;
    let flaky = with_move(&imap, vec![Fault::DropAfterCommandOnce("UID MOVE".into())]).await;

    assert_eq!(
        run_sync(&flaky, &store, account, WAKE + 60).await.remaining,
        1
    );
    // The MOVE happened; the copy still has its keyword until the retry.
    assert!(imap.mailbox().messages("Snoozed").await.is_empty());
    let report = run_sync(&flaky, &store, account, LATER).await;
    assert_eq!(report.woken, 1, "{report:?}");
    assert_woken_copy(&imap).await;
}

#[tokio::test]
async fn a_dropped_copy_on_wake_ends_with_an_unsnoozed_unread_copy() {
    let imap = no_move(Vec::new(), &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snoozed_and_due(&imap, &store).await;
    read_in_snoozed(&imap).await;
    let flaky = same_mailbox(&imap, vec![Fault::DropAfterCommandOnce("UID COPY".into())]).await;

    assert_eq!(
        run_sync(&flaky, &store, account, WAKE + 60).await.remaining,
        1
    );
    // Nothing was changed on the original before the move.
    let keyword = format_keyword(WAKE).expect("keyword");
    let original = imap.mailbox().messages("Snoozed").await;
    assert!(
        original[0].flags.contains(&keyword),
        "{:?}",
        original[0].flags
    );
    let report = run_sync(&flaky, &store, account, LATER).await;
    assert_eq!(report.woken, 1, "{report:?}");
    assert_woken_copy(&imap).await;
}

// --- #432 review -----------------------------------------------------------

/// A new wake time picked while a move is under way applies once the move
/// finishes: the copy ends up with the new keyword, not the old one.
#[tokio::test]
async fn a_new_wake_time_mid_move_ends_up_on_the_copy() {
    let imap = no_move(vec![Fault::DropAfterCommandOnce("UID COPY".into())], &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let inbox = folder(&store, account, "INBOX");
    let message = visible(&store, inbox);
    snooze(&store, &message, WAKE, &Quiet).expect("snooze");
    assert_eq!(run_sync(&imap, &store, account, NOW).await.remaining, 1);

    let later_wake = NOW + 48 * 3600;
    snooze(&store, &message, later_wake, &Quiet).expect("new time");
    let report = run_sync(&imap, &store, account, LATER).await;
    assert_eq!(report.snoozed, 1, "{report:?}");
    let parked = imap.mailbox().messages("Snoozed").await;
    assert_eq!(parked.len(), 1);
    let new_keyword = format_keyword(later_wake).expect("keyword");
    let old_keyword = format_keyword(WAKE).expect("keyword");
    assert!(
        parked[0].flags.contains(&new_keyword),
        "{:?}",
        parked[0].flags
    );
    assert!(
        !parked[0].flags.contains(&old_keyword),
        "{:?}",
        parked[0].flags
    );
}

/// A rollback that keeps failing is given up after 5 failed syncs and 24
/// hours: the message is shown again, never left hidden for good.
#[tokio::test]
async fn a_rollback_that_never_works_is_given_up_and_the_message_shown() {
    let refuse_all = Fault::RefuseCommand {
        folder: None,
        text: "\\Deleted".into(),
    };
    let imap = no_move(vec![refuse_all], &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snooze_lunch(&imap, &store).await;
    let inbox = folder(&store, account, "INBOX");

    // Four failed syncs over a few hours: kept for repair, still hidden.
    for hour in 0..4 {
        let report = run_sync(&imap, &store, account, NOW + hour * 3600).await;
        assert_eq!(report.remaining, 1, "hour {hour}: {report:?}");
    }
    assert_eq!(list_snoozed(&store, Some(account)).expect("list").len(), 1);
    assert!(visible(&store, inbox).is_empty());
    // Five failures, but not yet 24 hours: still kept.
    assert_eq!(
        run_sync(&imap, &store, account, NOW + 5 * 3600)
            .await
            .remaining,
        1
    );

    // A day on: given up, and the message is back where the server has it.
    let report = run_sync(&imap, &store, account, NOW + 25 * 3600).await;
    assert_eq!(report.failed, 1, "{report:?}");
    assert!(list_snoozed(&store, Some(account))
        .expect("list")
        .is_empty());
    assert_eq!(visible(&store, inbox).len(), 1, "shown again");
    assert_eq!(live(&imap, "INBOX").await, 1);
}

/// Refused at step 3 (the original already flagged \Deleted): the move is
/// carried forward, not undone, so the server isn't left half-moved.
#[tokio::test]
async fn a_refusal_after_flagging_carries_the_move_forward() {
    let imap = no_move(vec![Fault::DropAfterCommandOnce("UID EXPUNGE".into())], &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snooze_lunch(&imap, &store).await;
    assert_eq!(run_sync(&imap, &store, account, NOW).await.remaining, 1);
    assert_eq!(
        list_snoozed(&store, Some(account)).expect("list")[0].step,
        3
    );

    // The retry is refused before it reaches the expunge.
    let refusing = same_mailbox(
        &imap,
        vec![Fault::RefuseCommand {
            folder: None,
            text: "SELECT".into(),
        }],
    )
    .await;
    let report = run_sync(&refusing, &store, account, LATER).await;
    assert_eq!(report.snoozed, 1, "{report:?}");
    assert_eq!(
        list_snoozed(&store, Some(account)).expect("list")[0].state,
        SnoozeState::Snoozed
    );
    assert_eq!(live(&imap, "Snoozed").await, 1);
    assert_eq!(live(&imap, "INBOX").await, 0, "not shown twice");
}

// --- #444: every early exit, against the model in snooze/server.rs ----------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Dir {
    Snooze,
    Wake,
}

#[derive(Debug, Clone)]
enum Case {
    /// The first command containing this text is carried out, then the
    /// connection drops.
    Drop(&'static str),
    /// Commands containing this text are refused (in this folder, if any).
    Refuse(Option<&'static str>, &'static str),
    /// A drop after the COPY, then the original disappears.
    MissingOriginal,
    /// A drop after the COPY, then the copy disappears.
    MissingCopy,
    /// Commands containing this text are refused (in this folder, if any)
    /// for days, so the give-up rule ends the move.
    GiveUp(Option<&'static str>, &'static str),
}

/// Far beyond every sync of a case, so a re-snoozed message stays snoozed.
const RESNOOZE_AT: i64 = WAKE + 60 * 86_400;

/// A server on the same messages, with or without MOVE.
async fn twin(imap: &FakeImap, with_move_cap: bool, faults: Vec<Fault>) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .mailbox(imap.mailbox());
    if !with_move_cap {
        builder = builder.quirk(Quirk::NoMove);
    }
    for f in faults {
        builder = builder.fault(f);
    }
    builder.start().await.expect("imap")
}

async fn live_with_keyword(imap: &FakeImap, folder: &str) -> usize {
    imap.mailbox()
        .messages(folder)
        .await
        .iter()
        .filter(|m| m.flags.iter().any(|f| f.starts_with("$snoozed")))
        .count()
}

/// What the user does right after the faulty sync of a wake (#431).
#[derive(Debug, Clone, Copy, PartialEq)]
enum After {
    Nothing,
    /// Snoozes it again where it shows in the Inbox (the snooze points at
    /// that local row).
    ReSnooze,
    /// Snoozes it again where it shows in the Snoozed folder (the snooze
    /// has no local row and finds the message by its Message-ID).
    ReSnoozeFromList,
    /// Snoozes it again wherever it shows, then undoes that at once.
    ReSnoozeThenUndo,
}

/// The local rows shown in the Inbox and Snoozed.
fn shown(store: &Store, account: AccountId) -> Vec<MessageId> {
    let mut shown = visible(store, folder(store, account, "INBOX"));
    shown.extend(visible(store, folder(store, account, "Snoozed")));
    shown
}

/// The live (not `\Deleted`) messages in `folder`.
async fn live_messages(imap: &FakeImap, folder: &str) -> Vec<mailcrane_testkit::Message> {
    imap.mailbox()
        .messages(folder)
        .await
        .into_iter()
        .filter(|m| !m.flags.iter().any(|f| f == r"\Deleted"))
        .collect()
}

/// One row of the fault table: a snooze or a wake meets `case`, then the
/// user does `after` (wakes only), then the server is healthy again.
/// Returns whether `after` could be done (nothing shows to snooze again
/// when the copy vanished mid-wake).
async fn run_case(dir: Dir, with_move_cap: bool, case: Case, after: After) -> bool {
    let name = format!("{dir:?} move={with_move_cap} {after:?} {case:?}");
    let base = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw("Lunch", "lunch@example.net")])
        .start()
        .await
        .expect("imap");
    let store = Store::open_in_memory().expect("store");
    let account = Box::pin(setup(&base, &store)).await;
    let inbox = folder(&store, account, "INBOX");
    snooze(&store, &visible(&store, inbox), WAKE, &Quiet).expect("snooze");
    let start = if dir == Dir::Wake {
        assert_eq!(
            Box::pin(run_sync(&base, &store, account, NOW))
                .await
                .snoozed,
            1,
            "{name}"
        );
        Box::pin(sync_all(&base, &store, account)).await;
        let uid = base.mailbox().messages("Snoozed").await[0].uid;
        base.mailbox().add_flag("Snoozed", uid, "\\Seen").await;
        assert_eq!(wake_due(&store, WAKE + 60, &Quiet).expect("wake").len(), 1);
        WAKE + 60
    } else {
        NOW
    };
    let (source, target) = match dir {
        Dir::Snooze => ("INBOX", "Snoozed"),
        Dir::Wake => ("Snoozed", "INBOX"),
    };
    let faults = match &case {
        Case::Drop(text) => vec![Fault::DropAfterCommandOnce((*text).into())],
        Case::Refuse(folder, text) | Case::GiveUp(folder, text) => vec![Fault::RefuseCommand {
            folder: folder.map(Into::into),
            text: (*text).into(),
        }],
        Case::MissingOriginal | Case::MissingCopy => {
            vec![Fault::DropAfterCommandOnce("UID COPY".into())]
        }
    };
    let faulty = Box::pin(twin(&base, with_move_cap, faults)).await;
    let _ = Box::pin(run_sync(&faulty, &store, account, start)).await;
    // The mutations only mean something when the drop left a move part-way.
    let mid_move = list_snoozed(&store, Some(account))
        .expect("list")
        .iter()
        .any(|r| r.step > 0);
    match &case {
        _ if !mid_move => {}
        Case::MissingOriginal => {
            let originals = base.mailbox().messages(source).await;
            for m in originals {
                base.mailbox().expunge(source, m.uid).await;
            }
        }
        Case::MissingCopy => {
            if let Some(copy) = base.mailbox().messages(target).await.last() {
                base.mailbox().expunge(target, copy.uid).await;
            }
        }
        _ => {}
    }

    // A re-snooze while the wake is moving the message (past step 0) is
    // refused, with nothing changed (#431, lead's decision on #455). At
    // step 0 nothing has moved: the new snooze replaces the wake.
    let before = list_snoozed(&store, Some(account)).expect("list");
    let waking = before
        .first()
        .is_some_and(|r| r.state == SnoozeState::ToWake && r.step > 0);
    let not_moved = !waking
        && before
            .first()
            .is_some_and(|r| r.state == SnoozeState::ToWake);
    let mut acted = true;
    let mut refused_first = false;
    let mut undone = false;
    let mut resnoozed = false;
    if after != After::Nothing {
        Box::pin(sync_all(&base, &store, account)).await;
        let ids = match after {
            After::ReSnooze => visible(&store, inbox),
            After::ReSnoozeFromList => visible(&store, folder(&store, account, "Snoozed")),
            _ => shown(&store, account),
        };
        acted = !ids.is_empty();
        if acted {
            let result = snooze(&store, &ids, RESNOOZE_AT, &Quiet);
            if waking {
                assert!(
                    matches!(result, Err(SnoozeError::Waking)),
                    "{name}: refused while waking: {result:?}"
                );
                assert_eq!(
                    list_snoozed(&store, Some(account)).expect("list"),
                    before,
                    "{name}: nothing changed"
                );
                let now_shown = shown(&store, account);
                assert!(
                    ids.iter().all(|id| now_shown.contains(id)),
                    "{name}: still shown"
                );
                refused_first = true;
            } else if ids.iter().all(|id| {
                store
                    .get_message_header(*id)
                    .expect("header")
                    .is_some_and(|h| h.details.flags.deleted)
            }) {
                // A woken original left flagged `\Deleted` (its expunge was
                // refused): not the message. The snooze finds nothing live
                // and is undone; the message stays woken in the Inbox.
                result.expect("re-snooze");
            } else {
                result.expect("re-snooze");
                if after == After::ReSnoozeThenUndo {
                    unsnooze(&store, &ids, start + 1, &Quiet).expect("undo");
                    undone = true;
                } else {
                    resnoozed = true;
                }
            }
        }
    }
    let snoozed_at_once = resnoozed;

    // A refusal only counts when the move sends that command: COPY and
    // `\Deleted` without MOVE, MOVE with it.
    let met = |text: &str| match text {
        "UID MOVE" => with_move_cap,
        "UID COPY" | "\\Deleted" => !with_move_cap,
        _ => true,
    };
    let given_up = matches!(case, Case::GiveUp(_, text) if met(text));
    if matches!(case, Case::GiveUp(..)) {
        // Refused for days: at least GIVE_UP_FAILURES syncs over 24 hours.
        for half_day in 1..=8 {
            let _ = Box::pin(run_sync(
                &faulty,
                &store,
                account,
                start + half_day * 43_200,
            ))
            .await;
        }
    }
    // A healthy server afterwards: retries resume and finish (and nothing
    // given up comes back).
    let healthy = Box::pin(twin(&base, with_move_cap, Vec::new())).await;
    for day in 1..=6 {
        let _ = Box::pin(run_sync(
            &healthy,
            &store,
            account,
            start + (4 + day * 2) * 86_400,
        ))
        .await;
    }

    // Refused while waking: once the wake has landed, the user snoozes the
    // message again, where it shows, and that works as any snooze does.
    if refused_first && after != After::ReSnoozeThenUndo && !given_up {
        Box::pin(sync_changes_all(&base, &store, account)).await;
        let mut ids = visible(&store, inbox);
        if ids.is_empty() {
            ids = visible(&store, folder(&store, account, "Snoozed"));
        }
        assert!(!ids.is_empty(), "{name}: shown once woken");
        snooze(&store, &ids, RESNOOZE_AT, &Quiet).expect("re-snooze once woken");
        resnoozed = true;
        for day in 1..=2 {
            let _ = Box::pin(run_sync(
                &healthy,
                &store,
                account,
                start + (20 + day) * 86_400,
            ))
            .await;
        }
    }

    // Invariant 1: one live place. Invariant 2: no keyword in Snoozed
    // without a row. Then the expected end state.
    let rows = list_snoozed(&store, Some(account)).expect("list");
    let in_inbox = Box::pin(live(&base, "INBOX")).await;
    let in_snoozed = Box::pin(live(&base, "Snoozed")).await;
    // A given-up rollback can leave its copy (invariant 1's exception).
    let copy_may_stay = given_up && matches!(case, Case::GiveUp(_, t) if t.contains("Deleted"));
    if copy_may_stay {
        assert!(
            (1..=2).contains(&(in_inbox + in_snoozed)),
            "{name}: {in_inbox} + {in_snoozed}"
        );
    } else {
        assert_eq!(
            in_inbox + in_snoozed,
            1,
            "{name}: one live place ({in_inbox} + {in_snoozed})"
        );
    }
    if rows.is_empty() {
        assert_eq!(
            Box::pin(live_with_keyword(&base, "Snoozed")).await,
            0,
            "{name}: keyword without a row"
        );
    }
    // Invariant 3: a give-up leaves the message shown here, with no row.
    let assert_given_up = |rows: &[mailcrane_store::Snooze]| {
        assert!(rows.is_empty(), "{name}: given up: {rows:?}");
        assert!(
            !shown(&store, account).is_empty(),
            "{name}: shown on this device"
        );
    };
    if resnoozed {
        if given_up && rows.is_empty() {
            assert_given_up(&rows);
            return acted;
        }
        // Invariant 5: one row, snoozed until the new time.
        assert_eq!(rows.len(), 1, "{name}: {rows:?}");
        assert_eq!(
            (rows[0].state, rows[0].wake_at),
            (SnoozeState::Snoozed, RESNOOZE_AT),
            "{name}"
        );
        assert_eq!(
            (in_inbox, in_snoozed),
            (0, 1),
            "{name}: one copy, in Snoozed"
        );
        let parked = Box::pin(live_messages(&base, "Snoozed")).await;
        let new_keyword = format_keyword(RESNOOZE_AT).expect("keyword");
        let keywords: Vec<_> = parked[0]
            .flags
            .iter()
            .filter(|f| f.starts_with("$snoozed"))
            .collect();
        assert_eq!(keywords, vec![&new_keyword], "{name}: only the new keyword");
        if snoozed_at_once && not_moved {
            assert!(
                parked[0].flags.iter().any(|f| f == "\\Seen"),
                "{name}: snoozed again without being woken (still read): {:?}",
                parked[0].flags
            );
        }
        return acted;
    }
    if undone {
        if given_up {
            assert_given_up(&rows);
            return acted;
        }
        // Undone: as if the user hadn't snoozed again, the wake ends with
        // the message back in the Inbox, shown, without a keyword.
        assert!(rows.is_empty(), "{name}: woken: {rows:?}");
        assert_eq!(in_inbox, 1, "{name}: back in the Inbox");
        // The app syncs its folders' changes too: the copy the wake made
        // shows, and one a rollback removed goes.
        Box::pin(sync_changes_all(&base, &store, account)).await;
        assert_eq!(
            visible(&store, inbox).len(),
            1,
            "{name}: shown in the Inbox after the undo"
        );
        // (Unless the keyword cleanup on the copy was refused, which is
        // best effort, as without the re-snooze.)
        if !matches!(case, Case::Refuse(Some("INBOX"), "-FLAGS")) {
            let back = Box::pin(live_messages(&base, "INBOX")).await;
            assert!(
                !back[0].flags.iter().any(|f| f.starts_with("$snoozed")),
                "{name}: no keyword: {:?}",
                back[0].flags
            );
        }
        return acted;
    }
    if given_up {
        assert_given_up(&rows);
        return acted;
    }
    let refused = matches!(case, Case::Refuse(..));
    match dir {
        Dir::Snooze if refused => {
            // Undone, or (refusals the move never meets) snoozed.
            assert!(
                rows.is_empty() || rows[0].state == SnoozeState::Snoozed,
                "{name}: {rows:?}"
            );
        }
        Dir::Snooze => {
            assert_eq!(rows.len(), 1, "{name}");
            assert_eq!(rows[0].state, SnoozeState::Snoozed, "{name}");
            assert_eq!(in_snoozed, 1, "{name}");
        }
        Dir::Wake => {
            assert!(rows.is_empty(), "{name}: woken: {rows:?}");
            assert_eq!(in_inbox, 1, "{name}: back in the Inbox");
            let refused_cleanup = matches!(case, Case::Refuse(Some("INBOX"), "-FLAGS"));
            if !refused_cleanup {
                let back = Box::pin(live_messages(&base, "INBOX")).await;
                assert!(
                    !back[0]
                        .flags
                        .iter()
                        .any(|f| f.starts_with("$snoozed") || f == "\\Seen"),
                    "{name}: unsnoozed and unread: {:?}",
                    back[0].flags
                );
            }
        }
    }
    acted
}

/// Every early exit the table knows, for a snooze or a wake, with or
/// without MOVE.
fn table() -> Vec<Case> {
    vec![
        Case::Drop("UID COPY"),
        Case::Drop("UID MOVE"),
        Case::Drop("\\Deleted"),
        Case::Drop("UID EXPUNGE"),
        Case::Drop("-FLAGS"),
        Case::Refuse(None, "UID COPY"),
        Case::Refuse(None, "UID MOVE"),
        Case::Refuse(Some("INBOX"), "\\Deleted"),
        Case::Refuse(Some("Snoozed"), "\\Deleted"),
        // Refused in both folders: the rollback is refused too (step 4).
        Case::Refuse(None, "\\Deleted"),
        Case::Refuse(None, "UID EXPUNGE"),
        Case::Refuse(Some("INBOX"), "-FLAGS"),
        Case::Refuse(None, "+FLAGS"),
        Case::MissingOriginal,
        Case::MissingCopy,
        Case::GiveUp(None, "UID COPY"),
        Case::GiveUp(None, "UID MOVE"),
        Case::GiveUp(None, "\\Deleted"),
    ]
}

/// Runs one row on its own thread (the default stack, as the app's
/// threads have) and its own runtime, so a failing row doesn't stop the
/// others. `Err` if the row's checks failed.
fn run_row(dir: Dir, with_move_cap: bool, case: Case, after: After) -> std::thread::Result<bool> {
    std::thread::Builder::new()
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(run_case(dir, with_move_cap, case, after))
        })
        .expect("thread")
        .join()
}

#[test]
fn every_early_exit_keeps_the_model_invariants() {
    let mut failed = Vec::new();
    for dir in [Dir::Snooze, Dir::Wake] {
        for with_move_cap in [false, true] {
            for case in table() {
                let name = format!("{dir:?} move={with_move_cap} {case:?}");
                if run_row(dir, with_move_cap, case, After::Nothing).is_err() {
                    failed.push(name);
                }
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} rows broke: {failed:#?}",
        failed.len()
    );
}

/// #431 crossed with the fault table (lead's ask and decision on #455):
/// after every early exit of a wake, the user snoozes the message again
/// (as shown in the Inbox, or in Snoozed with no local row), or snoozes it
/// again and undoes that. The cases leave the wake at each step (1 after a
/// lost COPY or MOVE reply, 2 after a lost `\Deleted` reply, 3 after a
/// lost EXPUNGE reply, 4 when the rollback is refused too) or end it by
/// the give-up rule. Past step 0 the re-snooze is refused with nothing
/// changed, and works once the wake has landed; at step 0 it replaces the
/// wake, never waking the message first. Every row keeps the invariants
/// and ends snoozed once, until the new time, in Snoozed with only the new
/// keyword; or, undone, woken as if nothing happened; or given up.
#[test]
fn a_resnooze_after_every_early_exit_of_a_wake_keeps_the_invariants() {
    let mut crossed = Vec::new();
    let mut failed = Vec::new();
    // (Snoozing again then undoing it meets the pre-existing undo gaps,
    // filed with the other finishing-write gaps; its rows come with that.)
    for after in [After::ReSnooze, After::ReSnoozeFromList] {
        for with_move_cap in [false, true] {
            for case in table() {
                let name = format!("{after:?} move={with_move_cap} {case:?}");
                // Each row on its own, so one failing row doesn't hide the
                // others: the report lists every broken crossing.
                match run_row(Dir::Wake, with_move_cap, case.clone(), after) {
                    Ok(true) => {
                        if after != After::ReSnoozeThenUndo {
                            crossed.push(format!("move={with_move_cap} {case:?}"));
                        }
                    }
                    Ok(false) => {}
                    Err(_) => failed.push(name),
                }
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} rows broke: {failed:#?}",
        failed.len()
    );
    // Every case met a re-snooze (in the Inbox or in Snoozed, wherever the
    // message showed), except a vanished copy, which leaves nothing shown.
    for with_move_cap in [false, true] {
        for case in table() {
            let row = format!("move={with_move_cap} {case:?}");
            assert!(
                crossed.contains(&row) || row.ends_with("MissingCopy"),
                "{row}: never re-snoozed"
            );
        }
    }
}

/// #444: the original disappears after a COPY whose reply was lost (step 1),
/// and the wake time changes before the retry. The copy is the message now:
/// it's kept (not rolled back, which would lose it), with only the new
/// keyword.
#[tokio::test]
async fn a_lost_original_after_the_copy_keeps_the_copy_with_the_new_keyword() {
    let imap = no_move(vec![Fault::DropAfterCommandOnce("UID COPY".into())], &[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = snooze_lunch(&imap, &store).await;
    assert_eq!(run_sync(&imap, &store, account, NOW).await.remaining, 1);
    assert_eq!(
        list_snoozed(&store, Some(account)).expect("list")[0].step,
        1
    );

    let original = imap.mailbox().messages("INBOX").await[0].uid;
    imap.mailbox().expunge("INBOX", original).await;
    // The hidden row went with it (as a sync of a vanished message does),
    // so the retry has to search for the message, and finds nothing.
    let hidden = list_snoozed(&store, Some(account)).expect("list")[0]
        .local_message_id
        .expect("hidden row");
    store.delete_message_row(hidden).expect("delete row");
    let later_wake = NOW + 48 * 3600;
    store
        .snooze_message(&mailcrane_store::NewSnooze {
            account_id: account,
            message_id_header: "lunch@example.net".into(),
            original_folder_id: None,
            hide_message: None,
            wake_at: later_wake,
        })
        .expect("new time");

    let report = run_sync(&imap, &store, account, LATER).await;
    assert_eq!(report.snoozed, 1, "{report:?}");
    let parked = imap.mailbox().messages("Snoozed").await;
    assert_eq!(parked.len(), 1, "the copy is kept");
    let new_keyword = format_keyword(later_wake).expect("keyword");
    let old_keyword = format_keyword(WAKE).expect("keyword");
    assert!(
        parked[0].flags.contains(&new_keyword),
        "{:?}",
        parked[0].flags
    );
    assert!(
        !parked[0].flags.contains(&old_keyword),
        "{:?}",
        parked[0].flags
    );
}

/// #431 (lead's decision on #455): snoozing a message again while its
/// wake is moving it is refused, with nothing changed, and the message
/// stays shown. Once the wake lands, snoozing it again works: one copy, in
/// Snoozed, until the new time, with only its keyword. With and without
/// MOVE.
#[tokio::test]
async fn a_resnooze_while_waking_is_refused_until_the_wake_lands() {
    for with_move_cap in [false, true] {
        let name = format!("move={with_move_cap}");
        let imap = no_move(Vec::new(), &[]).await;
        let store = Store::open_in_memory().expect("store");
        let account = snoozed_and_due(&imap, &store).await;
        let drop_on = if with_move_cap {
            "UID MOVE"
        } else {
            "UID COPY"
        };
        let flaky = twin(
            &imap,
            with_move_cap,
            vec![Fault::DropAfterCommandOnce(drop_on.into())],
        )
        .await;
        assert_eq!(
            run_sync(&flaky, &store, account, WAKE + 60).await.remaining,
            1,
            "{name}"
        );
        let rows = list_snoozed(&store, Some(account)).expect("list");
        assert!(rows[0].step > 0, "{name}: half done");

        // The copy shows in the Inbox; snoozing it again is refused.
        sync_all(&imap, &store, account).await;
        let inbox = folder(&store, account, "INBOX");
        let copy = visible(&store, inbox);
        assert_eq!(copy.len(), 1, "{name}");
        let later_wake = WAKE + 72 * 3600;
        assert!(
            matches!(
                snooze(&store, &copy, later_wake, &Quiet),
                Err(SnoozeError::Waking)
            ),
            "{name}"
        );
        assert_eq!(
            list_snoozed(&store, Some(account)).expect("list"),
            rows,
            "{name}: nothing changed"
        );
        assert_eq!(visible(&store, inbox), copy, "{name}: still shown");

        // The wake lands; now snoozing again works.
        let healthy = twin(&imap, with_move_cap, Vec::new()).await;
        run_sync(&healthy, &store, account, LATER).await;
        assert!(
            list_snoozed(&store, Some(account))
                .expect("list")
                .is_empty(),
            "{name}: woken"
        );
        sync_changes_all(&imap, &store, account).await;
        let copy = visible(&store, inbox);
        assert_eq!(copy.len(), 1, "{name}");
        snooze(&store, &copy, later_wake, &Quiet).expect("snooze once woken");
        run_sync(&healthy, &store, account, LATER + 60).await;
        assert_eq!(live(&imap, "INBOX").await, 0, "{name}: not in the Inbox");
        assert_eq!(
            live(&imap, "Snoozed").await,
            1,
            "{name}: one copy, in Snoozed"
        );
        let rows = list_snoozed(&store, Some(account)).expect("list");
        assert_eq!(rows.len(), 1, "{name}");
        assert_eq!(
            (rows[0].state, rows[0].wake_at),
            (SnoozeState::Snoozed, later_wake),
            "{name}"
        );
        let parked = live_messages(&imap, "Snoozed").await;
        let new_keyword = format_keyword(later_wake).expect("keyword");
        let keywords: Vec<_> = parked[0]
            .flags
            .iter()
            .filter(|f| f.starts_with("$snoozed"))
            .collect();
        assert_eq!(keywords, vec![&new_keyword], "{name}: only the new keyword");
    }
}
