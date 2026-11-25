//! Incremental sync against the testkit's fake IMAP server, with each
//! strategy forced by a server quirk. Never a real mailbox.

use super::{Strategy, SyncReport};
use crate::events::{CoreEvent, EventListener};
use crate::sync::convert::remote_flags;
use crate::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{AccountId, FolderId, NewAccount, NewFolder, RemoteFlags, Security, Store};
use mailcrane_testkit::{FakeImap, Fault, Quirk};
use proptest::prelude::{any, prop, prop_assert, prop_oneof, proptest, Just, ProptestConfig};
use proptest::strategy::Strategy as _;
use std::sync::Mutex;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const INBOX: &str = "INBOX";

/// Each strategy, the quirk that forces it, and what it reports.
const STRATEGIES: [(Option<Quirk>, Strategy); 3] = [
    (None, Strategy::Qresync),
    (Some(Quirk::NoQresync), Strategy::Condstore),
    (Some(Quirk::NoCondstore), Strategy::Compare),
];

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
    format!("From: alice@example.org\r\nSubject: Message {n}\r\n\r\nBody {n}\r\n").into_bytes()
}

fn raws(count: usize) -> Vec<Vec<u8>> {
    (1..=count).map(raw).collect()
}

async fn fake(count: usize, quirk: Option<Quirk>, faults: &[Fault]) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder(INBOX, raws(count));
    if let Some(quirk) = quirk {
        builder = builder.quirk(quirk);
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
            remote_name: INBOX.into(),
            display_name: INBOX.into(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("folder");
    (store, account, folder)
}

/// UIDs and flags as the store has them, by UID.
fn store_state(store: &Store, folder: FolderId) -> Vec<RemoteFlags> {
    let mut rows: Vec<RemoteFlags> = store
        .list_messages(folder, 10_000, 0)
        .expect("messages")
        .into_iter()
        .map(|m| RemoteFlags {
            uid: m.details.uid,
            flags: m.details.flags,
            keywords_json: m.details.keywords_json,
        })
        .collect();
    rows.sort_by_key(|r| r.uid);
    rows
}

/// UIDs and flags as the server has them, in the store's terms.
async fn server_state(imap: &FakeImap) -> Vec<RemoteFlags> {
    imap.mailbox()
        .messages(INBOX)
        .await
        .iter()
        .map(|m| remote_flags(m.uid, &m.flags))
        .collect()
}

fn stored_modseq(store: &Store, folder: FolderId) -> Option<i64> {
    store
        .get_folder(folder)
        .expect("folder")
        .expect("exists")
        .details
        .highestmodseq
}

/// First sync, then one incremental sync so the folder has a stored
/// `HIGHESTMODSEQ` where the server supports it.
async fn synced(
    store: &Store,
    session: &mut Session,
    events: &Events,
    account: AccountId,
    folder: FolderId,
) {
    let mut syncer = FolderSyncer::new(store, session, events, account, SyncConfig::default());
    syncer.sync_folder(folder).await.expect("first sync");
    let report = syncer.sync_changes(folder).await.expect("settles");
    assert_eq!(report.strategy, Strategy::Compare);
}

async fn sync_changes(
    store: &Store,
    session: &mut Session,
    events: &Events,
    account: AccountId,
    folder: FolderId,
) -> SyncReport {
    FolderSyncer::new(store, session, events, account, SyncConfig::default())
        .sync_changes(folder)
        .await
        .expect("syncs changes")
}

#[tokio::test]
async fn each_strategy_picks_up_new_flagged_unflagged_and_expunged_messages() {
    for (quirk, strategy) in STRATEGIES {
        let imap = fake(10, quirk, &[]).await;
        let mailbox = imap.mailbox();
        let (store, account, folder) = setup();
        let events = Events::default();
        let mut session = connect(&imap).await;
        synced(&store, &mut session, &events, account, folder).await;
        mailbox.add_flag(INBOX, 2, "\\Seen").await;

        // Another client delivers, flags and expunges.
        mailbox.deliver(INBOX, raw(11)).await;
        mailbox.deliver(INBOX, raw(12)).await;
        mailbox.add_flag(INBOX, 3, "\\Flagged").await;
        mailbox.add_flag(INBOX, 4, "$Label").await;
        mailbox.remove_flag(INBOX, 2, "\\Seen").await;
        mailbox.expunge(INBOX, 5).await;
        mailbox.expunge(INBOX, 10).await;
        // Flagged after the first sync, then expunged: only the expunge counts.
        mailbox.add_flag(INBOX, 6, "\\Seen").await;
        mailbox.expunge(INBOX, 6).await;

        let report = sync_changes(&store, &mut session, &events, account, folder).await;
        assert_eq!(report.strategy, strategy, "{quirk:?}");
        assert_eq!(report.new, 2, "{quirk:?}");
        assert_eq!(
            store_state(&store, folder),
            server_state(&imap).await,
            "{quirk:?}"
        );
        assert_eq!(
            store.list_uids(folder).expect("uids"),
            [1, 2, 3, 4, 7, 8, 9, 11, 12],
            "{quirk:?}"
        );
        assert!(events
            .all()
            .iter()
            .any(|e| matches!(e, CoreEvent::MessagesChanged { .. })));

        // Unflag elsewhere, and a sync with nothing new.
        mailbox.remove_flag(INBOX, 3, "\\Flagged").await;
        let report = sync_changes(&store, &mut session, &events, account, folder).await;
        assert_eq!((report.strategy, report.new), (strategy, 0), "{quirk:?}");
        assert_eq!(
            store_state(&store, folder),
            server_state(&imap).await,
            "{quirk:?}"
        );
        assert_eq!(
            stored_modseq(&store, folder),
            mailbox
                .highest_modseq(INBOX)
                .await
                .filter(|_| quirk != Some(Quirk::NoCondstore))
                .and_then(|m| i64::try_from(m).ok()),
            "{quirk:?}"
        );
    }
}

#[tokio::test]
async fn each_strategy_clears_and_refetches_on_a_uidvalidity_change() {
    for (quirk, _) in STRATEGIES {
        let imap = fake(10, quirk, &[]).await;
        let (store, account, folder) = setup();
        let events = Events::default();
        let mut session = connect(&imap).await;
        synced(&store, &mut session, &events, account, folder).await;

        imap.mailbox().set_uidvalidity(2);
        imap.mailbox().replace_folder(INBOX, raws(3)).await;
        let before = events.all().len();
        let report = sync_changes(&store, &mut session, &events, account, folder).await;

        assert_eq!(
            (report.strategy, report.new),
            (Strategy::Initial, 3),
            "{quirk:?}"
        );
        assert_eq!(store.list_uids(folder).expect("uids"), [1, 2, 3]);
        let row = store.get_folder(folder).expect("folder").expect("exists");
        assert_eq!(row.details.uidvalidity, Some(2));
        assert!(events
            .all()
            .iter()
            .skip(before)
            .any(|e| matches!(e, CoreEvent::FolderChanged { .. })));
    }
}

#[tokio::test]
async fn highestmodseq_is_stored_only_after_the_changes_are() {
    // On the faulty server, which shares the mailbox: CAPABILITY,
    // AUTHENTICATE, CAPABILITY, then ENABLE, SELECT (QRESYNC), UID SEARCH
    // for new UIDs, and the 7th command, the UID FETCH of the new headers,
    // is dropped.
    let imap = fake(10, None, &[]).await;
    let faulty = FakeImap::builder()
        .user(USER, PASSWORD)
        .mailbox(imap.mailbox())
        .fault(Fault::DropAfterCommands(7))
        .start()
        .await
        .expect("faulty imap starts");
    let (store, account, folder) = setup();
    let events = Events::default();
    {
        let mut session = connect(&imap).await;
        let mut syncer = FolderSyncer::new(
            &store,
            &mut session,
            &events,
            account,
            SyncConfig::default(),
        );
        syncer.sync_folder(folder).await.expect("first sync");
    }
    {
        let mut session = connect(&imap).await;
        let report = sync_changes(&store, &mut session, &events, account, folder).await;
        assert_eq!(report.strategy, Strategy::Compare);
    }
    let before = stored_modseq(&store, folder);
    assert!(before.is_some());

    imap.mailbox().add_flag(INBOX, 1, "\\Seen").await;
    imap.mailbox().deliver(INBOX, raw(11)).await;
    {
        let mut session = connect(&faulty).await;
        let mut syncer = FolderSyncer::new(
            &store,
            &mut session,
            &events,
            account,
            SyncConfig::default(),
        );
        assert!(syncer.sync_changes(folder).await.is_err());
    }
    assert_eq!(
        faulty.received_commands().await.last().map(String::as_str),
        Some("UID FETCH")
    );
    // The flag change was committed, the new message was not, and the
    // stored HIGHESTMODSEQ still points before both.
    let state = store_state(&store, folder);
    assert!(state.iter().any(|r| r.uid == 1 && r.flags.seen));
    assert!(!state.iter().any(|r| r.uid == 11));
    assert_eq!(stored_modseq(&store, folder), before);

    // The next sync asks for the same changes again and completes.
    let mut session = connect(&imap).await;
    let report = sync_changes(&store, &mut session, &events, account, folder).await;
    assert_eq!((report.strategy, report.new), (Strategy::Qresync, 1));
    assert_eq!(store_state(&store, folder), server_state(&imap).await);
    assert_eq!(
        stored_modseq(&store, folder),
        imap.mailbox()
            .highest_modseq(INBOX)
            .await
            .and_then(|m| i64::try_from(m).ok())
    );
}

#[tokio::test]
async fn a_qresync_sync_without_changes_sends_one_select_and_no_fetch() {
    let imap = fake(200, None, &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    synced(&store, &mut session, &events, account, folder).await;

    let start = imap.received_commands().await.len();
    let report = sync_changes(&store, &mut session, &events, account, folder).await;
    assert_eq!(report.strategy, Strategy::Qresync);
    let first: Vec<String> = imap.received_commands().await.split_off(start);
    assert_eq!(first, ["ENABLE", "SELECT"]);

    // QRESYNC stays enabled for the session.
    let start = imap.received_commands().await.len();
    let report = sync_changes(&store, &mut session, &events, account, folder).await;
    assert_eq!(
        report,
        SyncReport {
            strategy: Strategy::Qresync,
            new: 0,
            changed: 0
        }
    );
    let second: Vec<String> = imap.received_commands().await.split_off(start);
    assert_eq!(second, ["SELECT"]);
}

#[tokio::test]
async fn condstore_without_changes_skips_the_flag_fetch() {
    let imap = fake(20, Some(Quirk::NoQresync), &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    synced(&store, &mut session, &events, account, folder).await;

    let start = imap.received_commands().await.len();
    let report = sync_changes(&store, &mut session, &events, account, folder).await;
    assert_eq!(report.strategy, Strategy::Condstore);
    let commands: Vec<String> = imap.received_commands().await.split_off(start);
    assert_eq!(commands, ["SELECT", "UID SEARCH"]);
}

#[tokio::test]
async fn nomodseq_falls_back_to_comparing_and_forgets_the_modseq() {
    let imap = fake(10, None, &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    synced(&store, &mut session, &events, account, folder).await;

    imap.mailbox().set_nomodseq(INBOX, true).await;
    imap.mailbox().add_flag(INBOX, 2, "\\Answered").await;
    imap.mailbox().expunge(INBOX, 3).await;
    let report = sync_changes(&store, &mut session, &events, account, folder).await;

    assert_eq!(report.strategy, Strategy::Compare);
    assert_eq!(store_state(&store, folder), server_state(&imap).await);
    assert_eq!(stored_modseq(&store, folder), None);
}

#[tokio::test]
async fn a_reset_highestmodseq_falls_back_to_comparing() {
    for quirk in [None, Some(Quirk::NoQresync)] {
        let imap = fake(10, quirk, &[]).await;
        let (store, account, folder) = setup();
        let events = Events::default();
        let mut session = connect(&imap).await;
        synced(&store, &mut session, &events, account, folder).await;

        // The server loses its mod-sequences, then a flag changes and a
        // message goes: HIGHESTMODSEQ is now lower than the stored one.
        imap.mailbox().reset_modseq(INBOX).await;
        imap.mailbox().add_flag(INBOX, 2, "\\Seen").await;
        imap.mailbox().expunge(INBOX, 4).await;
        let report = sync_changes(&store, &mut session, &events, account, folder).await;

        assert_eq!(report.strategy, Strategy::Compare, "{quirk:?}");
        assert_eq!(store_state(&store, folder), server_state(&imap).await);
        // The new, lower HIGHESTMODSEQ is the one to use from now on.
        assert_eq!(
            stored_modseq(&store, folder),
            imap.mailbox()
                .highest_modseq(INBOX)
                .await
                .and_then(|m| i64::try_from(m).ok())
        );
    }
}

#[tokio::test]
async fn a_folder_never_synced_gets_a_first_sync() {
    let imap = fake(5, None, &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    let report = sync_changes(&store, &mut session, &events, account, folder).await;
    assert_eq!((report.strategy, report.new), (Strategy::Initial, 5));
    assert_eq!(store.list_uids(folder).expect("uids"), [1, 2, 3, 4, 5]);
}

/// A change another client makes on the server. Indexes pick a message
/// among those that exist at the time, modulo their number.
#[derive(Debug, Clone)]
enum Op {
    Deliver,
    Flag(usize, usize),
    Unflag(usize, usize),
    Expunge(usize),
}

const FLAGS: [&str; 4] = ["\\Seen", "\\Flagged", "\\Answered", "$Label"];

fn op() -> impl proptest::strategy::Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Deliver),
        (any::<usize>(), 0..FLAGS.len()).prop_map(|(i, f)| Op::Flag(i, f)),
        (any::<usize>(), 0..FLAGS.len()).prop_map(|(i, f)| Op::Unflag(i, f)),
        any::<usize>().prop_map(Op::Expunge),
    ]
}

async fn apply_op(imap: &FakeImap, op: &Op, next: &mut usize) {
    let mailbox = imap.mailbox();
    let uids: Vec<u32> = mailbox
        .messages(INBOX)
        .await
        .iter()
        .map(|m| m.uid)
        .collect();
    let pick = |i: usize| uids.get(i.checked_rem(uids.len())?).copied();
    match op {
        Op::Deliver => {
            *next += 1;
            mailbox.deliver(INBOX, raw(*next)).await;
        }
        Op::Flag(i, f) => {
            if let Some(uid) = pick(*i) {
                mailbox.add_flag(INBOX, uid, FLAGS[*f]).await;
            }
        }
        Op::Unflag(i, f) => {
            if let Some(uid) = pick(*i) {
                mailbox.remove_flag(INBOX, uid, FLAGS[*f]).await;
            }
        }
        Op::Expunge(i) => {
            if let Some(uid) = pick(*i) {
                mailbox.expunge(INBOX, uid).await;
            }
        }
    }
}

/// Runs the rounds of changes against one strategy and returns the final
/// store state, after checking it matches the server after every round.
async fn run_strategy(quirk: Option<Quirk>, rounds: &[Vec<Op>]) -> Vec<RemoteFlags> {
    const START: usize = 6;
    let imap = fake(START, quirk, &[]).await;
    let (store, account, folder) = setup();
    let events = Events::default();
    let mut session = connect(&imap).await;
    synced(&store, &mut session, &events, account, folder).await;
    let mut next = START;
    for ops in rounds {
        for op in ops {
            apply_op(&imap, op, &mut next).await;
        }
        sync_changes(&store, &mut session, &events, account, folder).await;
        assert_eq!(
            store_state(&store, folder),
            server_state(&imap).await,
            "{quirk:?} after {ops:?}"
        );
    }
    store_state(&store, folder)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn every_strategy_leaves_the_store_matching_the_server(
        rounds in prop::collection::vec(prop::collection::vec(op(), 0..8), 1..4)
    ) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let finals: Vec<Vec<RemoteFlags>> = runtime.block_on(async {
            let mut finals = Vec::new();
            for (quirk, _) in STRATEGIES {
                finals.push(run_strategy(quirk, &rounds).await);
            }
            finals
        });
        // The same changes give the same store, whatever the strategy.
        prop_assert!(finals.windows(2).all(|w| w[0] == w[1]));
    }
}
