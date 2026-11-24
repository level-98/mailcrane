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
