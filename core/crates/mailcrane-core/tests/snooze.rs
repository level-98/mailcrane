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
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(live(&imap, "INBOX").await, 0, "no copy left in the Inbox");
    assert_eq!(
        live(&imap, "Snoozed").await,
        1,
        "still snoozed on the server"
    );
}
