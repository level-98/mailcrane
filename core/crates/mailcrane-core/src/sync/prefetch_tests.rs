//! Background body downloads (M4-25) against the testkit's fake IMAP
//! server. Never a real mailbox.

use super::*;
use crate::events::{CoreEvent, EventListener};
use crate::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{ImapConfig, ImapSecurity, Secret};
use mailcrane_store::{AccountId, BodyState, NewAccount, NewFolder, SearchFilter, Security};
use mailcrane_testkit::{FakeImap, Fault};
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _event: CoreEvent) {}
}

fn conditions(mode: PrefetchMode) -> PrefetchConditions {
    PrefetchConditions {
        mode,
        unmetered: true,
        battery_saver: false,
        free_bytes: 10 * MIN_FREE_BYTES,
    }
}

#[test]
fn the_policy_keeps_to_network_battery_storage_and_setting() {
    // The default: unmetered only.
    let wifi = conditions(PrefetchMode::default());
    assert!(wifi.allows());
    let metered = PrefetchConditions {
        unmetered: false,
        ..wifi
    };
    assert!(!metered.allows());
    // "Always" also downloads on mobile data.
    assert!(PrefetchConditions {
        mode: PrefetchMode::Always,
        ..metered
    }
    .allows());
    // "Never" never does.
    assert!(!conditions(PrefetchMode::Never).allows());
    // Battery Saver and low storage stop it whatever the setting says.
    for mode in [PrefetchMode::UnmeteredOnly, PrefetchMode::Always] {
        let on = conditions(mode);
        assert!(!PrefetchConditions {
            battery_saver: true,
            ..on
        }
        .allows());
        assert!(!PrefetchConditions {
            free_bytes: MIN_FREE_BYTES,
            ..on
        }
        .allows());
        assert!(PrefetchConditions {
            free_bytes: MIN_FREE_BYTES + 1,
            ..on
        }
        .allows());
    }
    // Until the app reports, nothing is downloaded.
    assert!(!PrefetchConditions::default().allows());
    assert_eq!(PrefetchConditions::default(), PrefetchConditions::UNKNOWN);
}

#[test]
fn the_default_limits_are_the_issue_s() {
    let limits = PrefetchLimits::default();
    assert_eq!(limits.max_age_secs, 14 * 24 * 3600);
    assert_eq!(limits.newest, 200);
    assert_eq!(limits.max_size, 1024 * 1024);
    assert_eq!(limits.batch, 20);
}

fn raws(count: usize) -> Vec<Vec<u8>> {
    (1..=count)
        .map(|n| {
            format!(
                "From: bob@example.org\r\nSubject: Message {n}\r\n\r\nPrefetched word{n} here\r\n"
            )
            .into_bytes()
        })
        .collect()
}

async fn connect(imap: &FakeImap) -> Session {
    connect_within(imap, Duration::from_secs(10)).await
}

/// A session whose commands time out after `limit`.
async fn connect_within(imap: &FakeImap, limit: Duration) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = limit;
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

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

async fn synced(imap: &FakeImap) -> (Store, FolderId) {
    let (store, account, folder) = setup();
    let mut session = connect(imap).await;
    FolderSyncer::new(&store, &mut session, &Quiet, account, SyncConfig::default())
        .sync_folder(folder)
        .await
        .expect("syncs");
    session.logout().await;
    (store, folder)
}

/// The folder's stored `UIDVALIDITY`, which a batch's selected folder has.
fn uidvalidity(store: &Store, folder: FolderId) -> u32 {
    store
        .get_folder(folder)
        .expect("get")
        .and_then(|f| f.details.uidvalidity)
        .and_then(|v| u32::try_from(v).ok())
        .expect("synced")
}

fn cached(store: &Store, folder: FolderId) -> usize {
    store
        .list_messages(folder, 1000, 0)
        .expect("list")
        .iter()
        .filter(|m| m.body_state == BodyState::Fetched)
        .count()
}

#[tokio::test]
async fn batches_save_whole_bodies_without_marking_anything_seen() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", raws(45))
        .start()
        .await
        .expect("fake imap");
    let (store, folder) = synced(&imap).await;
    let limits = PrefetchLimits::default();
    let mut session = connect(&imap).await;
    session.select("INBOX").await.expect("select");
    let mut skip = HashSet::new();
    let mut batches = 0;
    loop {
        let todo = candidates(&store, folder, now(), &limits, &skip).expect("candidates");
        let Some(batch) = todo.get(..todo.len().min(limits.batch)) else {
            break;
        };
        if batch.is_empty() {
            break;
        }
        // Newest first.
        assert!(batch.windows(2).all(|w| w[0].uid > w[1].uid));
        let outcome = imap_batch(
            &store,
            &mut session,
            folder,
            uidvalidity(&store, folder),
            batch,
        )
        .await
        .expect("batch");
        assert_eq!(outcome.saved, batch.len());
        skip.extend(outcome.tried);
        batches += 1;
    }
    session.logout().await;
    assert_eq!(batches, 3, "45 messages in batches of 20");
    assert_eq!(cached(&store, folder), 45);
    // Nothing became \Seen on the server.
    for uid in 1..=45 {
        let flags = imap.mailbox().flags("INBOX", uid).await.expect("flags");
        assert!(!flags.iter().any(|f| f == "\\Seen"), "uid {uid}: {flags:?}");
    }
    // Read and unread state in the store is unchanged too.
    assert_eq!(store.count_unread(folder).expect("unread"), 45);
    // Prefetched bodies are searchable offline, like opened ones.
    let found = store
        .search_messages(&SearchFilter {
            account: None,
            account_name: None,
            folder: Some(folder),
            folder_name: None,
            fts: Some("\"word17\"".into()),
            unread: false,
            flagged: false,
            has_attachment: false,
            after: None,
            before: None,
            limit: 10,
        })
        .expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].details.uid, 17);
    // A second run has nothing left to do.
    assert!(candidates(&store, folder, now(), &limits, &HashSet::new())
        .expect("candidates")
        .is_empty());
}

#[tokio::test]
async fn messages_over_the_size_limit_are_not_downloaded() {
    let mut messages = raws(3);
    let mut big = b"From: bob@example.org\r\nSubject: Big\r\n\r\n".to_vec();
    big.resize(1024 * 1024 + 10, b'x');
    messages.insert(1, big);
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", messages)
        .start()
        .await
        .expect("fake imap");
    let (store, folder) = synced(&imap).await;
    let limits = PrefetchLimits::default();
    let todo = candidates(&store, folder, now(), &limits, &HashSet::new()).expect("candidates");
    let uids: Vec<u32> = todo.iter().map(|c| c.uid).collect();
    // Left out by its stored size, so it is never fetched.
    assert_eq!(uids, [4, 3, 1], "uid 2 is over 1 MiB");
}

#[tokio::test]
async fn a_whole_mebibyte_is_fetched_quickly_and_one_byte_more_is_dropped() {
    // End to end through the fake, with the normal 10 s command timeout.
    // This used to stall now and then (and on the Windows CI runner): the
    // fake left the tail of a big answer in its TLS buffer because it never
    // flushed after writing. It does now.
    let mut big = b"From: bob@example.org\r\nSubject: Big\r\n\r\n".to_vec();
    big.resize(1024 * 1024 + 1, b'x');
    let mut exact = b"From: bob@example.org\r\nSubject: Exact\r\n\r\n".to_vec();
    exact.resize(1024 * 1024, b'y');
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![big, exact])
        .start()
        .await
        .expect("fake imap");
    let (store, folder) = synced(&imap).await;
    let mut session = connect(&imap).await;
    session.select("INBOX").await.expect("select");
    let rows = store.list_messages(folder, 10, 0).expect("list");
    let row = |uid: u32| {
        rows.iter()
            .find(|m| m.details.uid == uid)
            .map(|m| m.id)
            .expect("row")
    };
    let (big_id, exact_id) = (row(1), row(2));
    // As if the stored sizes were wrong: both are asked for. The partial
    // fetch gets one octet over the limit, so the big one is visibly cut
    // off and dropped; the one at the limit is saved whole.
    let started = std::time::Instant::now();
    let outcome = imap_batch(
        &store,
        &mut session,
        folder,
        uidvalidity(&store, folder),
        &[
            Candidate { id: big_id, uid: 1 },
            Candidate {
                id: exact_id,
                uid: 2,
            },
        ],
    )
    .await
    .expect("batch");
    let took = started.elapsed();
    assert_eq!(outcome.saved, 1);
    assert!(store.get_body(big_id).expect("read").is_none());
    let saved = store.get_body(exact_id).expect("read").expect("saved");
    assert_eq!(saved.len(), 1024 * 1024);
    assert!(took < Duration::from_secs(10), "{took:?}");
    session.logout().await;
}

#[tokio::test]
async fn messages_older_than_the_age_limit_are_not_downloaded() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", raws(5))
        .start()
        .await
        .expect("fake imap");
    let (store, folder) = synced(&imap).await;
    let limits = PrefetchLimits::default();
    // The fake's messages arrived now: 15 days later they are too old.
    let later = now() + 15 * 24 * 3600;
    assert!(candidates(&store, folder, later, &limits, &HashSet::new())
        .expect("candidates")
        .is_empty());
    let soon = now() + 13 * 24 * 3600;
    assert_eq!(
        candidates(&store, folder, soon, &limits, &HashSet::new())
            .expect("candidates")
            .len(),
        5
    );
    // Only the newest `newest` messages count.
    let few = PrefetchLimits {
        newest: 2,
        ..limits
    };
    let uids: Vec<u32> = candidates(&store, folder, now(), &few, &HashSet::new())
        .expect("candidates")
        .iter()
        .map(|c| c.uid)
        .collect();
    assert_eq!(uids, [5, 4]);
}

#[tokio::test]
async fn an_interrupted_batch_saves_nothing_and_the_next_run_carries_on() {
    let messages = raws(30);
    let clean = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", messages.clone())
        .start()
        .await
        .expect("fake imap");
    let (store, folder) = synced(&clean).await;
    let limits = PrefetchLimits::default();

    // How many commands a connection sends up to and including SELECT, on
    // a fresh server, so the next server can drop the one after it.
    let probe = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", messages.clone())
        .start()
        .await
        .expect("fake imap");
    let mut session = connect(&probe).await;
    session.select("INBOX").await.expect("select");
    let before_fetch = probe.received_commands().await.len();
    session.logout().await;

    // The connection drops in the middle of the first batch's FETCH.
    let flaky = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", messages)
        .fault(Fault::DropAfterCommands(before_fetch + 1))
        .start()
        .await
        .expect("fake imap");
    let mut session = connect(&flaky).await;
    session.select("INBOX").await.expect("select");
    let todo = candidates(&store, folder, now(), &limits, &HashSet::new()).expect("candidates");
    let batch = &todo[..limits.batch];
    assert!(imap_batch(
        &store,
        &mut session,
        folder,
        uidvalidity(&store, folder),
        batch
    )
    .await
    .is_err());
    assert_eq!(cached(&store, folder), 0, "no partial body is kept");

    // The next run (on a working connection) downloads everything.
    let mut session = connect(&clean).await;
    session.select("INBOX").await.expect("select");
    let mut skip = HashSet::new();
    loop {
        let todo = candidates(&store, folder, now(), &limits, &skip).expect("candidates");
        if todo.is_empty() {
            break;
        }
        let batch = &todo[..todo.len().min(limits.batch)];
        let outcome = imap_batch(
            &store,
            &mut session,
            folder,
            uidvalidity(&store, folder),
            batch,
        )
        .await
        .expect("batch");
        skip.extend(outcome.tried);
    }
    session.logout().await;
    assert_eq!(cached(&store, folder), 30);
}
