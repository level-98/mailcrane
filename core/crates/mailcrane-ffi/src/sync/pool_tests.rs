//! The reused connection for opens and actions (issue #294), through the
//! FFI, against the fake IMAP server.

use super::pool::{PoolConfig, PoolError, Retry};
use super::prefetch_tests::{imap_engine, imap_engine_with, raws, runtime_for_fakes};
use super::tests::Collect;
use super::*;
use mailcrane_testkit::FakeImap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

fn fake(runtime: &tokio::runtime::Runtime, count: usize) -> FakeImap {
    runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws(count))
            .start()
            .await
            .expect("fake imap")
    })
}

/// The stored messages of `folder`, newest first.
fn messages(path: &str, folder: FolderId) -> Vec<mailcrane_store::MessageHeader> {
    Store::open(path)
        .expect("store")
        .list_messages(folder, 1000, 0)
        .expect("list")
}

fn account_of(path: &str, folder: FolderId) -> AccountId {
    Store::open(path)
        .expect("store")
        .get_folder(folder)
        .expect("read")
        .expect("folder")
        .details
        .account_id
}

fn logins(runtime: &tokio::runtime::Runtime, imap: &FakeImap) -> usize {
    runtime
        .block_on(imap.received_commands())
        .iter()
        .filter(|c| *c == "LOGIN" || *c == "AUTHENTICATE")
        .count()
}

#[test]
fn opening_two_messages_connects_and_logs_in_once() {
    let runtime = runtime_for_fakes();
    let imap = fake(&runtime, 5);
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);
    assert_eq!(engine.sync_folder(folder.0), Ok(5));
    let (connections, logged_in) = (imap.connections_accepted(), logins(&runtime, &imap));
    let selects = runtime.block_on(imap.selected_folders()).len();

    let list = messages(&path, folder);
    assert_eq!(engine.fetch_body(list[0].id.0), Ok(()));
    assert_eq!(engine.fetch_body(list[1].id.0), Ok(()));
    assert_eq!(engine.fetch_body(list[2].id.0), Ok(()));
    // One connection and one login for all three, and one SELECT.
    assert_eq!(imap.connections_accepted(), connections + 1);
    assert_eq!(logins(&runtime, &imap), logged_in + 1);
    assert_eq!(runtime.block_on(imap.selected_folders()).len(), selects + 1);
    let store = Store::open(&path).expect("store");
    for m in &list[..3] {
        assert!(store.get_body(m.id).expect("read").is_some());
    }
}

#[test]
fn opening_a_message_does_not_wait_for_a_running_sync() {
    let runtime = runtime_for_fakes();
    let imap = fake(&runtime, 3);
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);
    assert_eq!(engine.sync_folder(folder.0), Ok(3));
    let account = account_of(&path, folder);
    let message = messages(&path, folder)[0].id.0;

    // A sync holds the account's writer lock for its whole run (#301).
    let gate = engine.account_lock(account);
    let syncing = gate.lock();
    let (done_tx, done) = std::sync::mpsc::channel();
    let opener = engine.clone();
    std::thread::spawn(move || {
        let _ = done_tx.send(opener.fetch_body(message));
    });
    let opened = done
        .recv_timeout(Duration::from_secs(10))
        .expect("the open finished while the sync still ran");
    assert_eq!(opened, Ok(()));
    drop(syncing);
}

#[test]
fn idle_network_change_and_a_dropped_connection_all_reconnect_cleanly() {
    let runtime = runtime_for_fakes();
    let imap = fake(&runtime, 6);
    let dir = tempfile::tempdir().expect("tempdir");
    let collect = Arc::new(Collect::default());
    let (engine, path, folder) = imap_engine_with(&dir, &imap, collect.clone());
    assert_eq!(engine.sync_folder(folder.0), Ok(6));
    let account = account_of(&path, folder);
    let list = messages(&path, folder);
    engine.pool.set_config(PoolConfig {
        check_after: Duration::from_millis(150),
        close_after: Duration::from_millis(600),
        sweep_every: Duration::from_millis(100),
    });

    assert_eq!(engine.fetch_body(list[0].id.0), Ok(()));
    assert_eq!(engine.pool.opened(account), 1);

    // Idle past the check: NOOP first, same connection.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.fetch_body(list[1].id.0), Ok(()));
    assert_eq!(engine.pool.opened(account), 1);
    assert!(runtime
        .block_on(imap.received_commands())
        .iter()
        .any(|c| c == "NOOP"));

    // Idle past the limit: the sweeper closes it, the next open reconnects.
    let started = Instant::now();
    while engine.pool.is_open(account) {
        assert!(started.elapsed() < Duration::from_secs(5), "never closed");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(engine.fetch_body(list[2].id.0), Ok(()));
    assert_eq!(engine.pool.opened(account), 2);

    // The network changed.
    engine.close_connections(CloseReason::NetworkChanged);
    assert!(!engine.pool.is_open(account));
    assert_eq!(engine.fetch_body(list[3].id.0), Ok(()));
    assert_eq!(engine.pool.opened(account), 3);

    // The server drops the connection: the open still works, on a new one,
    // and the user sees no error.
    engine.pool.set_config(PoolConfig::default());
    collect.0.lock().expect("lock").clear();
    imap.drop_connections();
    assert_eq!(engine.fetch_body(list[4].id.0), Ok(()));
    assert_eq!(engine.pool.opened(account), 4);
    let errors: Vec<CoreEvent> = collect
        .0
        .lock()
        .expect("lock")
        .iter()
        .filter(|e| matches!(e, CoreEvent::SyncError { .. }))
        .cloned()
        .collect();
    assert!(errors.is_empty(), "{errors:?}");

    // Backgrounded: logs out politely.
    engine.close_connections(CloseReason::Background);
    assert!(!engine.pool.is_open(account));
    assert!(runtime
        .block_on(imap.received_commands())
        .iter()
        .any(|c| c == "LOGOUT"));
}

#[test]
fn only_idempotent_jobs_are_retried_after_a_connection_failure() {
    let runtime = runtime_for_fakes();
    let imap = fake(&runtime, 1);
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);
    let account = account_of(&path, folder);
    let store = Store::open(&path).expect("store");
    let cert = engine.extra_root_cert();
    let login = || imap_login(&store, account, cert.as_deref());

    for (retry, expected) in [(Retry::Never, 1), (Retry::Idempotent, 2)] {
        let calls = AtomicUsize::new(0);
        let result: Result<(), PoolError> =
            engine
                .pool
                .run(account, login, retry, &calls, |_session, calls| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Err(CoreSyncError::Imap(ImapError::ConnectionClosed)) })
                });
        assert!(matches!(
            result,
            Err(PoolError::Job(CoreSyncError::Imap(
                ImapError::ConnectionClosed
            )))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), expected, "{retry:?}");
    }
    // A refused command is no connection failure: never retried, and the
    // connection stays.
    let calls = AtomicUsize::new(0);
    let opened = engine.pool.opened(account);
    let _ = engine.pool.run(
        account,
        login,
        Retry::Idempotent,
        &calls,
        |session, calls| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                session.noop().await?;
                Err::<(), _>(CoreSyncError::Imap(ImapError::CommandFailed))
            })
        },
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(engine.pool.is_open(account));
    assert_eq!(engine.pool.opened(account), opened + 1);
}

#[test]
fn opens_searches_and_syncs_together_leak_no_connections() {
    let runtime = runtime_for_fakes();
    let imap = fake(&runtime, 40);
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);
    assert_eq!(engine.sync_folder(folder.0), Ok(40));
    let account = account_of(&path, folder);
    let before = imap.connections_accepted();
    let ids: Vec<i64> = messages(&path, folder).iter().map(|m| m.id.0).collect();
    let syncs = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for chunk in ids.chunks(10) {
            let engine = engine.clone();
            scope.spawn(move || {
                for id in chunk {
                    assert_eq!(engine.fetch_body(*id), Ok(()));
                }
            });
        }
        let engine = engine.clone();
        let syncs = &syncs;
        scope.spawn(move || {
            for _ in 0..3 {
                assert!(engine.sync_folder(folder.0).is_ok());
                syncs.fetch_add(1, Ordering::SeqCst);
                assert!(engine.search_server(folder.0, "Message".into()).is_ok());
            }
        });
    });
    // One reused connection for every open and search, plus one per sync
    // (each sync connects and logs out): at most two at once per account.
    assert_eq!(engine.pool.opened(account), 1);
    assert_eq!(
        imap.connections_accepted(),
        before + 1 + syncs.load(Ordering::SeqCst)
    );
    let store = Store::open(&path).expect("store");
    for id in ids {
        assert!(store.get_body(MessageId(id)).expect("read").is_some());
    }
}
