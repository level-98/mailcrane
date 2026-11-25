//! The FFI sync calls end to end against the fake IMAP server.

use super::*;
use crate::secrets::{install_secret_store, PlatformSecretStore, SecretStoreError};
use mailcrane_store::{NewAccount, NewFolder};
use mailcrane_testkit::FakeImap;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Platform(Mutex<HashMap<String, Vec<u8>>>);

impl PlatformSecretStore for Platform {
    fn put(&self, key: String, value: Vec<u8>) -> Result<(), SecretStoreError> {
        self.0
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?
            .insert(key, value);
        Ok(())
    }
    fn get(&self, key: String) -> Result<Option<Vec<u8>>, SecretStoreError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?
            .get(&key)
            .cloned())
    }
    fn delete(&self, key: String) -> Result<(), SecretStoreError> {
        self.0
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?
            .remove(&key);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Collect(pub(super) Mutex<Vec<CoreEvent>>);

impl CoreEventListener for Collect {
    fn on_event(&self, event: CoreEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

#[test]
fn sync_load_older_and_fetch_body_through_the_ffi() {
    // The fake server needs a runtime of its own that keeps running while the
    // engine blocks this thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let raws: Vec<Vec<u8>> = (1..=300)
        .map(|n| format!("Subject: Message {n}\r\n\r\nBody {n}\r\n").into_bytes())
        .collect();
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws)
            .start()
            .await
            .expect("fake imap")
    });

    let path = std::env::temp_dir().join(format!("mailcrane-ffi-sync-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: "alice@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "alice@example.org".into(),
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
        })
        .expect("folder");
    drop(store);

    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);

    let collect = Arc::new(Collect::default());
    let engine = SyncEngine::new(path.to_string_lossy().into_owned(), collect.clone());
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());

    // Listing folders keeps the existing row (and its id) for INBOX.
    assert_eq!(engine.refresh_folders(account.0), Ok(Some(folder.0)));
    assert_eq!(
        engine.refresh_folders(9999),
        Err(SyncFailure::Failed {
            kind: SyncErrorKind::NotFound
        })
    );

    assert_eq!(engine.sync_folder(folder.0), Ok(200));
    assert_eq!(engine.load_older(folder.0, 200), Ok(100));
    let store = Store::open(&path).expect("store");
    assert_eq!(store.list_uids(folder).expect("uids").len(), 300);
    let message = store
        .list_messages(folder, 1, 0)
        .expect("page")
        .pop()
        .expect("a message");
    drop(store);

    assert_eq!(engine.fetch_body(message.id.0), Ok(()));
    let store = Store::open(&path).expect("store");
    assert!(store.get_body(message.id).expect("body").is_some());

    let events = collect.0.lock().expect("lock").clone();
    assert!(events
        .iter()
        .any(|e| matches!(e, CoreEvent::MessagesChanged { folder_id } if *folder_id == folder.0)));

    // Message actions (M1-14): applied at once, then replayed on the server.
    let uid = message.details.uid;
    assert_eq!(
        engine.apply_action(vec![message.id.0], crate::actions::MessageAction::Flag),
        Ok(Some(account.0))
    );
    assert_eq!(engine.mark_read(message.id.0), Ok(()));
    let store = Store::open(&path).expect("store");
    assert_eq!(store.count_pending_ops(account).expect("count"), 2);
    drop(store);
    assert_eq!(engine.replay_actions(account.0), Ok(0));
    let mut flags = runtime
        .block_on(imap.mailbox().flags("INBOX", uid))
        .expect("on the server");
    flags.sort();
    assert_eq!(flags, ["\\Flagged", "\\Seen"]);
    // Nothing queued: no connection at all.
    assert_eq!(engine.replay_actions(account.0), Ok(0));
    assert_eq!(
        engine.apply_action(vec![9999], crate::actions::MessageAction::Flag),
        Err(crate::actions::ActionFailure::NotFound)
    );
    assert_eq!(
        engine.apply_action(vec![message.id.0], crate::actions::MessageAction::Archive),
        Err(crate::actions::ActionFailure::NoArchiveFolder)
    );

    // Unknown ids are errors, not panics.
    assert_eq!(
        engine.sync_folder(9999),
        Err(SyncFailure::Failed {
            kind: SyncErrorKind::NotFound
        })
    );
    assert_eq!(
        engine.fetch_body(9999),
        Err(SyncFailure::Failed {
            kind: SyncErrorKind::NotFound
        })
    );
    let _ = std::fs::remove_file(&path);
}

/// Waits up to `limit` for `done`.
fn wait_for(limit: std::time::Duration, mut done: impl FnMut() -> bool) -> std::time::Duration {
    let started = std::time::Instant::now();
    while !done() {
        assert!(started.elapsed() < limit, "not done within {limit:?}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    started.elapsed()
}

#[test]
fn live_updates_start_once_deliver_and_stop_through_the_ffi() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let raws: Vec<Vec<u8>> = (1..=2)
        .map(|n| format!("Subject: Message {n}\r\n\r\nBody {n}\r\n").into_bytes())
        .collect();
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws)
            .start()
            .await
            .expect("fake imap")
    });

    let path = std::env::temp_dir().join(format!("mailcrane-ffi-live-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: "alice@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "alice@example.org".into(),
        })
        .expect("account");
    let inbox = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "INBOX".into(),
            delimiter: Some("/".into()),
            role: Some(mailcrane_store::FolderRole::Inbox),
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("folder");

    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);

    let collect = Arc::new(Collect::default());
    let engine = SyncEngine::new(path.to_string_lossy().into_owned(), collect.clone());
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());
    let uids = || store.list_uids(inbox).map(|u| u.len()).unwrap_or(0);
    let idles = || {
        runtime
            .block_on(imap.received_commands())
            .iter()
            .filter(|c| *c == "IDLE")
            .count()
    };

    assert_eq!(engine.account_ids(), Ok(vec![account.0]));

    // Idempotent: two starts, one connection.
    assert_eq!(engine.start_live_updates(account.0), Ok(()));
    assert_eq!(engine.start_live_updates(account.0), Ok(()));
    wait_for(std::time::Duration::from_secs(10), || {
        uids() == 2 && idles() >= 1
    });
    assert_eq!(imap.connections_accepted(), 1);

    collect.0.lock().expect("lock").clear();
    runtime.block_on(
        imap.mailbox()
            .deliver("INBOX", b"Subject: Message 3\r\n\r\nBody 3\r\n".to_vec()),
    );
    let took = wait_for(std::time::Duration::from_secs(2), || {
        uids() == 3
            && collect.0.lock().expect("lock").iter().any(
                |e| matches!(e, CoreEvent::MessagesChanged { folder_id } if *folder_id == inbox.0),
            )
    });
    assert!(took < std::time::Duration::from_secs(2), "{took:?}");

    // One writer per account (issue #282): while an app sync holds the
    // account's lock, new mail seen by IDLE waits instead of writing
    // alongside it, and is stored once the lock is free.
    let writing = engine.account_lock(account);
    let guard = writing.lock();
    runtime.block_on(
        imap.mailbox()
            .deliver("INBOX", b"Subject: Message 4\r\n\r\nBody 4\r\n".to_vec()),
    );
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(uids(), 3, "live updates wrote while the account was locked");
    drop(guard);
    wait_for(std::time::Duration::from_secs(5), || uids() == 4);

    // Stop ends IDLE and logs out within 2 seconds.
    let started = std::time::Instant::now();
    engine.stop_live_updates(account.0);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let commands = runtime.block_on(imap.received_commands());
    assert!(
        commands.ends_with(&["DONE".to_string(), "LOGOUT".to_string()]),
        "{commands:?}"
    );
    // Stopping again, or an account that is not running, does nothing.
    engine.stop_live_updates(account.0);
    engine.stop_live_updates(9999);

    // Start after stop opens a new connection, which idles too.
    let idles_before = idles();
    assert_eq!(engine.start_live_updates(account.0), Ok(()));
    wait_for(std::time::Duration::from_secs(10), || {
        idles() > idles_before
    });
    assert_eq!(imap.connections_accepted(), 2);
    engine.stop_live_updates(account.0);

    assert_eq!(
        engine.start_live_updates(9999),
        Err(SyncFailure::Failed {
            kind: SyncErrorKind::NotFound
        })
    );
    drop(engine);
    drop(store);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_stop_that_runs_over_its_limit_keeps_a_new_start_from_opening_a_second_connection() {
    let path = std::env::temp_dir().join(format!(
        "mailcrane-ffi-live-slow-stop-{}.db",
        std::process::id()
    ));
    let mut engine = SyncEngine::new(
        path.to_string_lossy().into_owned(),
        Arc::new(Collect::default()),
    );
    Arc::get_mut(&mut engine)
        .expect("only reference")
        .live_config
        .stop_within = Duration::from_millis(100);

    // A live thread stuck closing its connection: it ignores the stop
    // until released.
    let (release, released) = mpsc::channel::<()>();
    let (done_tx, done) = mpsc::channel();
    let (stop, _signal) = live_stop();
    let thread = std::thread::spawn(move || {
        let _ = released.recv();
        let _ = done_tx.send(());
    });
    engine.live.lock().expect("lock").insert(
        7,
        LiveThread {
            stop,
            done,
            thread,
            stopping: false,
        },
    );

    // The stop gives up waiting, but the thread is remembered as stopping.
    engine.stop_live_updates(7);
    assert!(engine
        .live
        .lock()
        .expect("lock")
        .get(&7)
        .is_some_and(|t| t.stopping));

    // A start while it is still stuck fails instead of starting beside it.
    assert!(engine.start_live_updates(7).is_err());
    assert!(engine
        .live
        .lock()
        .expect("lock")
        .get(&7)
        .is_some_and(|t| t.stopping));

    // Once it finishes, a stop clears it.
    release.send(()).expect("release");
    engine.stop_live_updates(7);
    assert!(engine.live.lock().expect("lock").get(&7).is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn new_mail_is_notified_once_through_background_sync() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let raws = vec![b"Subject: Already here\r\nFrom: Bob <bob@example.net>\r\nMessage-ID: <old@example.net>\r\n\r\nHi\r\n".to_vec()];
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws)
            .start()
            .await
            .expect("fake imap")
    });
    let path = std::env::temp_dir().join(format!("mailcrane-ffi-notify-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Me".into(),
            // Not alice@: the fake server reports every message as from Alice,
            // and mail from the account's own address is never notified.
            email: "me@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "alice@example.org".into(),
        })
        .expect("account");
    drop(store);
    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);
    let engine = SyncEngine::new(
        path.to_string_lossy().into_owned(),
        Arc::new(Collect::default()),
    );
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());

    // Before any sync there is nothing to look at.
    assert_eq!(engine.take_new_mail(), Ok(vec![]));
    // The first background sync finds the folders, syncs the inbox and only
    // records what is already there.
    assert_eq!(engine.background_sync(60), Ok(vec![]));

    // A message arrives: notified once, with what a notification shows.
    let arrived = "Subject: Lunch?\r\nFrom: Carol <carol@example.net>\r\nMessage-ID: <new@example.net>\r\n\r\nNoon?\r\n";
    runtime.block_on(imap.mailbox().deliver("INBOX", arrived.as_bytes().to_vec()));
    let got = engine.background_sync(60).expect("sync");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].sender, "Alice");
    assert_eq!(got[0].subject, "Lunch?");
    assert_eq!(got[0].account_id, account.0);
    assert_eq!(got[0].account_email, "me@example.org");

    // Never again, even after another sync.
    assert_eq!(engine.background_sync(60), Ok(vec![]));
    assert_eq!(engine.take_new_mail(), Ok(vec![]));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_jmap_account_syncs_through_the_same_calls() {
    use mailcrane_testkit::{FakeEmail, FakeJmap};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let server = runtime.block_on(async {
        FakeJmap::builder()
            .user("ana@example.org", "pw")
            .email(FakeEmail {
                id: "m1".into(),
                mailbox_id: "mb-inbox".into(),
                received_at: "2026-11-25T08:00:00Z".into(),
                subject: "Hello over JMAP".into(),
                from: "bob@example.net".into(),
                seen: false,
                raw: "From: bob@example.net\r\nSubject: Hello over JMAP\r\n\r\nJMAP body\r\n"
                    .into(),
            })
            .start()
            .await
            .expect("fake jmap")
    });
    let path = std::env::temp_dir().join(format!("mailcrane-ffi-jmap-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Ana".into(),
                email: "ana@example.org".into(),
                imap_host: String::new(),
                imap_port: 0,
                imap_security: Security::Tls,
                smtp_host: String::new(),
                smtp_port: 0,
                smtp_security: Security::Tls,
                username: "ana@example.org".into(),
            },
            &server.base_url(),
        )
        .expect("account");
    drop(store);
    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);
    let engine = SyncEngine::new(
        path.to_string_lossy().into_owned(),
        Arc::new(Collect::default()),
    );
    *engine.extra_root_cert.lock().expect("lock") = Some(server.root_cert_der().to_vec());

    let inbox = engine
        .refresh_folders(account.0)
        .expect("folders")
        .expect("an inbox");
    assert_eq!(engine.sync_folder(inbox), Ok(1));
    let store = Store::open(&path).expect("store");
    let message = store
        .list_messages(FolderId(inbox), 10, 0)
        .expect("list")
        .pop()
        .expect("the message");
    assert_eq!(message.details.subject, "Hello over JMAP");
    drop(store);
    assert_eq!(engine.fetch_body(message.id.0), Ok(()));
    let store = Store::open(&path).expect("store");
    let body = store.get_body(message.id).expect("read").expect("stored");
    assert!(String::from_utf8_lossy(&body).contains("JMAP body"));
    // Live updates are a no-op for JMAP (push comes later), not an error.
    assert_eq!(engine.start_live_updates(account.0), Ok(()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_local_only_account_never_connects() {
    // A live fake server, and a local-only account (#289) aimed at it with a
    // valid password: every call must still leave the server untouched.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("fixtures@example.org", "pw")
            .folder("INBOX", vec![b"Subject: x\r\n\r\ny\r\n".to_vec()])
            .start()
            .await
            .expect("fake imap")
    });

    let path = std::env::temp_dir().join(format!("mailcrane-ffi-local-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_local_account(&NewAccount {
            display_name: "Fixtures".into(),
            email: "fixtures@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "fixtures@example.org".into(),
        })
        .expect("account");
    let folder = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "Inbox".into(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: Some(1),
            uidnext: Some(2),
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("folder");
    let message = store
        .upsert_message_header(&mailcrane_store::NewMessageHeader {
            folder_id: folder,
            uid: 1,
            message_id_header: None,
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: "x".into(),
            from_name: None,
            from_addr: Some("ann@example.org".into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date: 1_790_000_000,
            internal_date: 1_790_000_000,
            size: 10,
            flags: mailcrane_store::Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
        })
        .expect("message");
    drop(store);

    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);

    let collect = Arc::new(Collect::default());
    let engine = SyncEngine::new(path.to_string_lossy().into_owned(), collect.clone());
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());

    assert_eq!(engine.sync_folder(folder.0), Ok(0));
    assert_eq!(engine.load_older(folder.0, 50), Ok(0));
    assert_eq!(engine.refresh_folders(account.0), Ok(Some(folder.0)));
    assert_eq!(engine.fetch_body(message.0), Ok(()));
    assert_eq!(engine.start_live_updates(account.0), Ok(()));
    // Actions apply locally and are confirmed instead of replayed.
    assert_eq!(engine.mark_read(message.0), Ok(()));
    assert_eq!(engine.replay_actions(account.0), Ok(0));
    assert!(engine.take_new_mail().expect("notifications").is_empty());
    assert!(engine.background_sync(5).expect("background").is_empty());
    std::thread::sleep(std::time::Duration::from_millis(300));
    engine.stop_live_updates(account.0);

    let store = Store::open(&path).expect("store");
    assert_eq!(store.count_pending_ops(account).expect("ops"), 0);
    let header = store
        .get_message_header(message)
        .expect("header")
        .expect("exists");
    assert!(header.details.flags.seen);
    // The backstop itself, for any path that reaches the login.
    assert!(matches!(
        imap_login(&store, account, None),
        Err(events::SyncErrorKind::NotFound)
    ));
    // No error events either: a local account is quiet.
    let errors = collect
        .0
        .lock()
        .expect("events")
        .iter()
        .filter(|e| matches!(e, CoreEvent::SyncError { .. }))
        .count();
    assert_eq!(errors, 0);
    assert_eq!(imap.connections_accepted(), 0, "the server was contacted");
    drop(store);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unreadable_local_marker_fails_loudly_and_never_connects() {
    // An ordinary account aimed at a live server, whose local-only marker
    // can't be read (the column is gone): calls must report a storage
    // failure instead of quietly succeeding, and still never connect.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("bob@example.org", "pw")
            .folder("INBOX", vec![b"Subject: x\r\n\r\ny\r\n".to_vec()])
            .start()
            .await
            .expect("fake imap")
    });

    let path = std::env::temp_dir().join(format!("mailcrane-ffi-marker-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Bob".into(),
            email: "bob@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "bob@example.org".into(),
        })
        .expect("account");
    let folder = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "Inbox".into(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("folder");
    drop(store);
    let conn = rusqlite::Connection::open(&path).expect("raw");
    conn.execute_batch("ALTER TABLE accounts RENAME COLUMN local_only TO gone")
        .expect("rename");
    drop(conn);

    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);
    let collect = Arc::new(Collect::default());
    let engine = SyncEngine::new(path.to_string_lossy().into_owned(), collect.clone());
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());

    fn storage<T>() -> Result<T, SyncFailure> {
        Err(SyncFailure::Failed {
            kind: SyncErrorKind::Storage,
        })
    }
    assert_eq!(engine.sync_folder(folder.0), storage());
    assert_eq!(engine.load_older(folder.0, 10), storage());
    assert_eq!(engine.refresh_folders(account.0), storage());
    assert_eq!(engine.start_live_updates(account.0), storage());
    assert_eq!(engine.replay_actions(account.0), storage());
    assert!(engine.take_new_mail().is_err());
    // Background sync skips the account without connecting and reports it;
    // the notifications it gathers at the end fail the same way.
    assert_eq!(engine.background_sync(5), storage());
    let reported = collect.0.lock().expect("events").iter().any(|e| {
        matches!(
            e,
            CoreEvent::SyncError {
                kind: SyncErrorKind::Storage,
                ..
            }
        )
    });
    assert!(reported, "the failure is reported");
    let store = Store::open(&path).expect("store");
    assert!(matches!(
        imap_login(&store, account, None),
        Err(events::SyncErrorKind::Storage)
    ));
    assert_eq!(imap.connections_accepted(), 0, "the server was contacted");
    drop(store);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_servers_namespace_is_used_for_top_level_folders() {
    // #325: the server says INBOX. is the personal namespace. A folder
    // outside it ("Shared") means the folder list alone wouldn't suggest
    // it, so INBOX.Receipts can only come from NAMESPACE.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("ns@example.org", "pw")
            .namespace("INBOX.", '.')
            .folder("INBOX", Vec::new())
            .folder("INBOX.Projects", Vec::new())
            .folder("Shared", Vec::new())
            .start()
            .await
            .expect("fake imap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ns.db");
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Ns".into(),
            email: "ns@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "ns@example.org".into(),
        })
        .expect("account");
    drop(store);
    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);
    let engine = SyncEngine::new(
        path.to_string_lossy().into_owned(),
        Arc::new(Collect::default()),
    );
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());
    engine.refresh_folders(account.0).expect("refresh");

    let id = engine
        .create_folder(account.0, None, "Receipts".into())
        .expect("create");
    let store = Store::open(&path).expect("store");
    let remote = |id: i64| {
        store
            .get_folder(FolderId(id))
            .expect("get")
            .expect("folder")
            .details
            .remote_name
    };
    assert_eq!(remote(id), "INBOX.Receipts");
    // "Shared" moved to the top lands in the namespace too.
    let shared = store
        .list_folders(account)
        .expect("list")
        .into_iter()
        .find(|f| f.details.remote_name == "Shared")
        .expect("shared")
        .id;
    engine.move_folder(shared.0, None).expect("move");
    assert_eq!(remote(shared.0), "INBOX.Shared");
    let mut names = runtime.block_on(imap.mailbox().folder_names());
    names.sort();
    assert_eq!(
        names,
        ["INBOX", "INBOX.Projects", "INBOX.Receipts", "INBOX.Shared"]
    );
}

#[test]
fn a_jmap_account_manages_folders_through_mailbox_set() {
    // M4-26: create, rename, move and delete on a JMAP account, each made
    // on the server first, with the same rules as IMAP.
    use crate::folders::{can_manage_folders, list_folders};
    use crate::sync::folder_admin::{FolderFailure, FolderNameProblem};
    use mailcrane_testkit::{FakeEmail, FakeJmap};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let server = runtime.block_on(async {
        FakeJmap::builder()
            .user("jo@example.org", "pw")
            .mailbox("mb-inbox", "Inbox", Some("inbox"), None)
            .mailbox("mb-trash", "Trash", Some("trash"), None)
            .mailbox("mb-projects", "Projects", None, None)
            .mailbox("mb-mail", "Mail", None, None)
            .email(FakeEmail {
                id: "m1".into(),
                mailbox_id: "mb-mail".into(),
                received_at: "2026-11-25T08:00:00Z".into(),
                subject: "Kept".into(),
                from: "bob@example.net".into(),
                seen: true,
                raw: "From: bob@example.net\r\nSubject: Kept\r\n\r\nHi\r\n".into(),
            })
            .start()
            .await
            .expect("fake jmap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("jmap-folders.db");
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Jo".into(),
                email: "jo@example.org".into(),
                imap_host: String::new(),
                imap_port: 0,
                imap_security: Security::Tls,
                smtp_host: String::new(),
                smtp_port: 0,
                smtp_security: Security::Tls,
                username: "jo@example.org".into(),
            },
            &server.base_url(),
        )
        .expect("account");
    drop(store);
    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);
    let db = path.to_string_lossy().into_owned();
    let engine = SyncEngine::new(db.clone(), Arc::new(Collect::default()));
    *engine.extra_root_cert.lock().expect("lock") = Some(server.root_cert_der().to_vec());
    engine.refresh_folders(account.0).expect("folders");

    assert_eq!(can_manage_folders(db.clone(), account.0), Ok(true));
    let id_of = |remote: &str| {
        Store::open(&path)
            .expect("store")
            .list_folders(account)
            .expect("list")
            .into_iter()
            .find(|f| f.details.remote_name == remote)
            .map(|f| f.id.0)
    };
    let changeable = |remote: &str| {
        let id = id_of(remote).expect("folder");
        list_folders(db.clone(), account.0)
            .expect("list")
            .into_iter()
            .find(|f| f.id == id)
            .map(|f| f.can_change)
    };
    assert_eq!(changeable("Inbox"), Some(false));
    assert_eq!(changeable("Projects"), Some(true));
    let server_parent = |name: &str| {
        server
            .mailboxes()
            .into_iter()
            .find(|m| m.name == name)
            .map(|m| m.parent_id)
    };

    // Create at the top and under a folder.
    let receipts = engine
        .create_folder(account.0, None, "Receipts".into())
        .expect("create");
    assert_eq!(server_parent("Receipts"), Some(None));
    let projects = id_of("Projects").expect("projects");
    engine
        .create_folder(account.0, Some(projects), "2026".into())
        .expect("create");
    assert_eq!(server_parent("2026"), Some(Some("mb-projects".into())));
    assert!(id_of("Projects/2026").is_some());

    // Rename: the subfolder's path follows; the server keeps the parent.
    engine.rename_folder(projects, "Work".into()).expect("rename");
    assert!(server_parent("Work").is_some() && server_parent("Projects").is_none());
    assert_eq!(id_of("Work"), Some(projects));
    assert!(id_of("Work/2026").is_some());

    // Move under Work.
    engine
        .move_folder(receipts, Some(projects))
        .expect("move");
    assert_eq!(server_parent("Receipts"), Some(Some("mb-projects".into())));
    assert_eq!(id_of("Work/Receipts"), Some(receipts));

    // Refused before contacting the server.
    assert_eq!(
        engine.create_folder(account.0, None, "a/b".into()),
        Err(FolderFailure::InvalidName {
            problem: FolderNameProblem::Separator {
                separator: "/".into()
            }
        })
    );
    let inbox = id_of("Inbox").expect("inbox");
    assert_eq!(engine.rename_folder(inbox, "Post".into()), Err(FolderFailure::SystemFolder));
    assert_eq!(engine.delete_folder(projects), Err(FolderFailure::HasSubfolders));

    // The server refuses a folder that still holds mail: nothing changes here.
    let mail = id_of("Mail").expect("mail");
    let before = Store::open(&path).expect("store").list_folders(account).expect("list");
    assert_eq!(engine.delete_folder(mail), Err(FolderFailure::HasMessages));
    assert_eq!(
        Store::open(&path).expect("store").list_folders(account).expect("list"),
        before
    );
    assert!(server_parent("Mail").is_some());

    // An empty folder goes, on the server and here.
    let year = id_of("Work/2026").expect("2026");
    engine.delete_folder(year).expect("delete");
    assert!(server_parent("2026").is_none());
    assert!(id_of("Work/2026").is_none());
}
