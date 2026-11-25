//! The FFI sync calls end to end against the fake IMAP server.

use super::*;
use crate::secrets::{install_secret_store, PlatformSecretStore, SecretStoreError};
use mailcrane_store::{NewAccount, NewFolder};
use mailcrane_testkit::FakeImap;
use std::collections::HashMap;

#[derive(Default)]
struct Platform(Mutex<HashMap<String, Vec<u8>>>);

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
struct Collect(Mutex<Vec<CoreEvent>>);

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
