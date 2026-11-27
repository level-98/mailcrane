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
    // No timing window to hit: while checking the NOOP path the close limit
    // is far away, so a slow machine (the Windows CI runner, #357) can
    // oversleep as much as it likes; the sweeper phase below shortens it.
    let checking = PoolConfig {
        check_after: Duration::from_millis(150),
        close_after: Duration::from_secs(600),
        sweep_every: Duration::from_millis(100),
    };
    engine.pool.set_config(checking);

    assert_eq!(engine.fetch_body(list[0].id.0), Ok(()));
    assert_eq!(engine.pool.opened(account), 1);

    // Idle past the check: NOOP first, same connection. Sleeping longer
    // than asked is fine here.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.fetch_body(list[1].id.0), Ok(()));
    assert_eq!(engine.pool.opened(account), 1);
    assert!(runtime
        .block_on(imap.received_commands())
        .iter()
        .any(|c| c == "NOOP"));

    // Idle past the limit: the sweeper closes it, the next open reconnects.
    engine.pool.set_config(PoolConfig {
        close_after: Duration::from_millis(200),
        ..checking
    });
    let started = Instant::now();
    while engine.pool.is_open(account) {
        assert!(started.elapsed() < Duration::from_secs(60), "never closed");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Far away again, so the sweeper can't close the next connection
    // between an open and the count below.
    engine.pool.set_config(checking);
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

#[test]
fn taking_the_write_lock_while_holding_the_pooled_connection_is_caught() {
    // Lock order is WriteLock, then the pooled connection; the reverse
    // could deadlock against a sync or download. The guard fails tests.
    let runtime = runtime_for_fakes();
    let imap = fake(&runtime, 1);
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);
    let account = account_of(&path, folder);
    let store = Store::open(&path).expect("store");
    let cert = engine.extra_root_cert();
    let login = || imap_login(&store, account, cert.as_deref());
    assert!(!super::pool::holding_pooled_connection());

    let seen = std::sync::atomic::AtomicBool::new(false);
    let ok = engine
        .pool
        .run(account, login, Retry::Never, &seen, |_session, seen| {
            seen.store(super::pool::holding_pooled_connection(), Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        });
    assert!(ok.is_ok());
    assert!(seen.load(Ordering::SeqCst), "flagged inside the job");
    assert!(!super::pool::holding_pooled_connection(), "cleared after");

    let gate = engine.account_lock(account);
    let breach = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = engine
            .pool
            .run(account, login, Retry::Never, &gate, |_session, gate| {
                let _guard = gate.lock();
                Box::pin(async { Ok(()) })
            });
    }));
    let message = breach
        .err()
        .and_then(|p| {
            p.downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| p.downcast_ref::<String>().cloned())
        })
        .expect("the breach panics in tests");
    assert!(message.contains("lock order"), "{message}");
    assert!(
        !super::pool::holding_pooled_connection(),
        "cleared on unwind"
    );
    // The right order works.
    let _guard = gate.lock();
}

#[test]
fn an_open_saves_its_body_while_a_sync_holds_the_lock_and_writes() {
    // Opening takes no WriteLock (#294): its body writes rely on IMMEDIATE
    // transactions with busy retries (#301) to slot in between the sync's.
    let runtime = runtime_for_fakes();
    let imap = fake(&runtime, 20);
    let dir = tempfile::tempdir().expect("tempdir");
    let collect = Arc::new(Collect::default());
    let (engine, path, folder) = imap_engine_with(&dir, &imap, collect.clone());
    assert_eq!(engine.sync_folder(folder.0), Ok(20));
    let account = account_of(&path, folder);
    let ids: Vec<MessageId> = messages(&path, folder).iter().map(|m| m.id).collect();
    collect.0.lock().expect("lock").clear();

    let gate = engine.account_lock(account);
    let (started_tx, started) = std::sync::mpsc::channel();
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            // A "sync": holds the account's WriteLock and writes batches of
            // 200 header rows, one transaction each, without pause.
            //
            // What's tested is that the opens never fail. A batch of this
            // stand-in sync may still end in SQLITE_BUSY on a starved
            // machine (the Windows CI runner ran this test for over a
            // minute, #357), as a real sync's could; the real sync reports
            // it and tries again later, so this one counts it and goes on.
            let _syncing = gate.lock();
            let store = Store::open(&path).expect("store");
            let mut uid = 100_000u32;
            let mut batches = 0usize;
            let _ = started_tx.send(());
            while !stop.load(Ordering::SeqCst) {
                let rows: Vec<mailcrane_store::NewMessageHeader> = (0..200)
                    .map(|i| {
                        let n = uid + i;
                        mailcrane_store::NewMessageHeader {
                            folder_id: folder,
                            uid: n,
                            message_id_header: Some(format!("<{n}@example.org>")),
                            in_reply_to: None,
                            references_json: "[]".into(),
                            thread_id: None,
                            subject: "Busy".into(),
                            from_name: None,
                            from_addr: Some("bob@example.org".into()),
                            to_json: "[]".into(),
                            cc_json: "[]".into(),
                            date: 1,
                            internal_date: 1,
                            size: 10,
                            flags: mailcrane_store::Flags::default(),
                            keywords_json: "[]".into(),
                            has_attachments: false,
                            snippet: String::new(),
                        }
                    })
                    .collect();
                match store.upsert_message_headers(&rows) {
                    Ok(_) => {
                        uid += 200;
                        batches += 1;
                    }
                    Err(e) => assert!(e.is_busy(), "sync writes: {e:?}"),
                }
            }
            batches
        });
        started
            .recv_timeout(Duration::from_secs(60))
            .expect("writer started");
        let opened: Vec<_> = ids
            .iter()
            .map(|id| (Instant::now(), engine.fetch_body(id.0)))
            .map(|(t, r)| (t.elapsed(), r))
            .collect();
        // Stop the writer before asserting, so a failure can't hang the test.
        stop.store(true, Ordering::SeqCst);
        let batches = writer.join().expect("writer");
        assert!(batches > 0, "the sync wrote while the bodies were saved");
        for (took, result) in &opened {
            assert_eq!(result, &Ok(()), "after {took:?}");
        }
    });
    let store = Store::open(&path).expect("store");
    for id in ids {
        assert!(store.get_body(id).expect("read").is_some(), "{id:?}");
    }
    let errors: Vec<CoreEvent> = collect
        .0
        .lock()
        .expect("lock")
        .iter()
        .filter(|e| matches!(e, CoreEvent::SyncError { .. }))
        .cloned()
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}
