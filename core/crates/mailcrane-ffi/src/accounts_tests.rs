//! Multiple accounts (M4-01): the Unified Inbox, removing an account, and
//! per-account sync locks.

use crate::account::{remove_account, RemoveAccountFailure};
use crate::messages::{account_color, list_unified_inbox, unified_unread_count};
use crate::secrets::{install_secret_store, PlatformSecretStore, SecretStoreError};
use mailcrane_store::{
    AccountId, Flags, FolderId, FolderRole, NewAccount, NewFolder, NewMessageHeader, Security,
    Store,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

fn account(store: &Store, email: &str) -> AccountId {
    store
        .insert_account(&NewAccount {
            display_name: "A".into(),
            email: email.into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: email.into(),
        })
        .expect("account")
}

fn folder(store: &Store, account: AccountId, name: &str, role: Option<FolderRole>) -> FolderId {
    store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: name.into(),
            display_name: name.into(),
            delimiter: None,
            role,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
        })
        .expect("folder")
}

fn message(store: &Store, folder: FolderId, uid: u32, date: i64, seen: bool) {
    store
        .upsert_message_header(&NewMessageHeader {
            folder_id: folder,
            uid,
            message_id_header: None,
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: format!("m{uid}@{date}"),
            from_name: None,
            from_addr: Some("x@example.net".into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date,
            internal_date: date,
            size: 1,
            flags: Flags {
                seen,
                ..Flags::default()
            },
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
        })
        .expect("message");
}

#[test]
fn the_unified_inbox_merges_every_inbox_and_pages_stably() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("m.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let a = account(&store, "a@example.org");
    let b = account(&store, "b@example.org");
    let ia = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    let ib = folder(&store, b, "INBOX", Some(FolderRole::Inbox));
    let archive = folder(&store, a, "Archive", Some(FolderRole::Archive));
    // Interleaved dates, a tie across accounts, and mail outside inboxes.
    message(&store, ia, 1, 100, false);
    message(&store, ib, 1, 200, true);
    message(&store, ia, 2, 300, false);
    message(&store, ib, 2, 300, false);
    message(&store, archive, 1, 999, false);
    drop(store);
    let all = list_unified_inbox(path.clone(), 0, 50).expect("unified");
    let dates: Vec<i64> = all.iter().map(|m| m.date).collect();
    assert_eq!(
        dates,
        [300, 300, 200, 100],
        "archive left out, newest first"
    );
    assert_eq!(all[2].account_id, b.0);
    assert_eq!(all[2].account_color, account_color(b.0));
    assert_eq!(all[3].account_id, a.0);
    // Pages of 1 give the same rows, without duplicates or gaps.
    let mut paged = Vec::new();
    for offset in 0..5 {
        paged.extend(list_unified_inbox(path.clone(), offset, 1).expect("page"));
    }
    assert_eq!(
        paged.iter().map(|m| m.id).collect::<Vec<_>>(),
        all.iter().map(|m| m.id).collect::<Vec<_>>()
    );
    assert_eq!(unified_unread_count(path).expect("count"), 3);
    assert_ne!(account_color(a.0), account_color(b.0));
}

#[test]
fn removing_an_account_deletes_its_secrets_and_data() {
    let platform = Arc::new(Platform::default());
    install_secret_store(platform.clone());
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("m.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let a = account(&store, "a@example.org");
    let b = account(&store, "b@example.org");
    let ia = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    let ib = folder(&store, b, "INBOX", Some(FolderRole::Inbox));
    message(&store, ia, 1, 100, false);
    message(&store, ib, 1, 200, false);
    drop(store);
    platform
        .put(format!("account/{}/password", a.0), b"pw".to_vec())
        .expect("put");
    platform
        .put(format!("account/{}/password", b.0), b"other".to_vec())
        .expect("put");

    remove_account(path.clone(), a.0).expect("remove");

    let store = Store::open(&path).expect("store");
    assert!(store.get_account(a).expect("get").is_none());
    assert!(store.list_folders(a).expect("folders").is_empty());
    assert_eq!(
        list_unified_inbox(path.clone(), 0, 50)
            .expect("unified")
            .len(),
        1
    );
    assert_eq!(
        platform
            .get(format!("account/{}/password", a.0))
            .expect("get"),
        None
    );
    assert!(platform
        .get(format!("account/{}/password", b.0))
        .expect("get")
        .is_some());
    assert_eq!(
        remove_account(path, a.0),
        Err(RemoveAccountFailure::NotFound)
    );
}

#[test]
fn each_account_has_its_own_sync_lock() {
    struct Quiet;
    impl crate::sync::CoreEventListener for Quiet {
        fn on_event(&self, _event: crate::sync::CoreEvent) {}
    }
    let engine = crate::sync::SyncEngine::new("unused.db".into(), Arc::new(Quiet));
    let one = engine.account_lock(AccountId(1));
    let again = engine.account_lock(AccountId(1));
    let two = engine.account_lock(AccountId(2));
    assert!(Arc::ptr_eq(&one, &again));
    assert!(!Arc::ptr_eq(&one, &two));
    // Holding account 1's lock doesn't block account 2.
    let _held = one.lock().expect("lock 1");
    assert!(two.try_lock().is_ok());
    assert!(again.try_lock().is_err());
}
