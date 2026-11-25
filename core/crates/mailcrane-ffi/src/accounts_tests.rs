//! Multiple accounts (M4-01): the Unified Inbox, removing an account, and
//! per-account sync locks.

use crate::account::{remove_account, RemoveAccountFailure};
use crate::compose::list_accounts;
use crate::messages::{account_color, list_unified_inbox, unified_unread_count};
use crate::secrets::{install_secret_store, PlatformSecretStore, SecretStoreError};
use crate::settings::{get_account_color, set_account_color, SettingsFailure};
use crate::widget::widget_snapshot;
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
            selectable: true,
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
    let _held = one.lock();
    assert!(two.is_free());
    assert!(!again.is_free());
}

#[test]
fn searching_every_account_names_each_rows_account() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let a = account(&store, "a@example.org");
    let b = account(&store, "b@example.net");
    let ia = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    let ib = folder(&store, b, "INBOX", Some(FolderRole::Inbox));
    message(&store, ia, 1, 100, false);
    message(&store, ib, 1, 200, false);
    drop(store);
    // A chosen colour shows in the results; the other account stays automatic.
    let chosen = (account_color(a.0) + 3) % 8;
    set_account_color(path.clone(), a.0, Some(chosen)).expect("colour");
    let all =
        crate::search::search_all_accounts(path.clone(), "is:unread".into(), 50).expect("all");
    assert_eq!(
        all.iter()
            .map(|m| (m.account_id, m.account_color))
            .collect::<Vec<_>>(),
        [(b.0, account_color(b.0)), (a.0, chosen)]
    );
    let only_b =
        crate::search::search_all_accounts(path, "is:unread account:example.net".into(), 50)
            .expect("b");
    assert_eq!(
        only_b.iter().map(|m| m.account_id).collect::<Vec<_>>(),
        [b.0]
    );
}

#[test]
fn a_chosen_account_color_reaches_every_list_and_can_be_reset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("c.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let a = account(&store, "a@example.org");
    let b = account(&store, "b@example.org");
    let ia = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    message(&store, ia, 1, 100, false);
    drop(store);
    let chosen = (account_color(a.0) + 3) % 8;

    assert_eq!(get_account_color(path.clone(), a.0), Ok(None));
    set_account_color(path.clone(), a.0, Some(chosen)).expect("set");
    assert_eq!(get_account_color(path.clone(), a.0), Ok(Some(chosen)));

    let accounts = list_accounts(path.clone()).expect("accounts");
    assert_eq!(accounts[0].color, chosen);
    assert_eq!(accounts[1].color, account_color(b.0), "b stays automatic");
    let inbox = list_unified_inbox(path.clone(), 0, 10).expect("unified");
    assert_eq!(inbox[0].account_color, chosen);
    let widget = widget_snapshot(path.clone(), 5).expect("widget");
    assert_eq!(widget.messages[0].account_color, chosen);

    set_account_color(path.clone(), a.0, None).expect("reset");
    assert_eq!(get_account_color(path.clone(), a.0), Ok(None));
    let accounts = list_accounts(path).expect("accounts");
    assert_eq!(accounts[0].color, account_color(a.0));
}

#[test]
fn bad_account_colors_and_unknown_accounts_are_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("c.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let a = account(&store, "a@example.org");
    drop(store);
    set_account_color(path.clone(), a.0, Some(4)).expect("set");
    assert_eq!(
        set_account_color(path.clone(), a.0, Some(8)),
        Err(SettingsFailure::InvalidColor)
    );
    assert_eq!(
        set_account_color(path.clone(), a.0, Some(u32::MAX)),
        Err(SettingsFailure::InvalidColor)
    );
    assert_eq!(get_account_color(path.clone(), a.0), Ok(Some(4)));
    assert_eq!(
        set_account_color(path.clone(), 999, Some(1)),
        Err(SettingsFailure::NotFound)
    );
    assert_eq!(get_account_color(path, 999), Err(SettingsFailure::NotFound));
}
