//! Issue #282: what the maintainer's phone does with a Gmail account,
//! against the testkit's Gmail-like fake. Never a real mailbox.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::events::{CoreEvent, EventListener};
use mailcrane_core::sync::{FolderSyncer, SyncConfig, SyncError, WriteLock};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{AccountId, FolderId, NewAccount, Security, Store};
use mailcrane_testkit::FakeImap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "app-password-example";

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _: CoreEvent) {}
}

/// Gmail-ish messages: names, Message-IDs, replies, encoded subjects.
fn gmail_messages(count: usize) -> Vec<Vec<u8>> {
    (1..=count)
        .map(|n| {
            let sender = n % 37;
            let topic = n % 23;
            let reply = if n % 3 == 0 {
                format!(
                    "In-Reply-To: <m{}@mail.example.com>\r\nReferences: <m{}@mail.example.com>\r\n",
                    n - 1,
                    n - 1
                )
            } else {
                String::new()
            };
            format!(
                "From: \"Sender {sender}\" <sender{sender}@example.com>\r\n\
                 To: Alice <alice@example.org>\r\n\
                 Subject: =?UTF-8?B?w4l0w6kg?= topic {topic}\r\n\
                 Date: Tue, 24 Nov 2026 10:{:02}:00 +0100\r\n\
                 Message-ID: <m{n}@mail.example.com>\r\n\
                 {reply}\r\n\
                 Body {n}\r\n",
                n % 60
            )
            .into_bytes()
        })
        .collect()
}

async fn gmail_fake(count: usize) -> FakeImap {
    FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", gmail_messages(count))
        .gmail_layout()
        .start()
        .await
        .expect("fake imap starts")
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_secs(10);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

/// A database file with the account and its Gmail folders listed.
async fn setup(dir: &Path, imap: &FakeImap) -> (PathBuf, AccountId, FolderId) {
    let path = dir.join("mail.db");
    let store = Store::open(&path).expect("store");
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
    let mut session = connect(imap).await;
    let folders = session.list_folders().await.expect("list");
    save_folders(&store, account, &folders).expect("save");
    let inbox = store
        .list_folders(account)
        .expect("folders")
        .into_iter()
        .find(|f| f.details.remote_name == "INBOX")
        .expect("inbox")
        .id;
    (path, account, inbox)
}

/// What the app does on a refresh, or the live-updates thread after an
/// IDLE wake-up: sync, then thread the new mail. Holds the account's write
/// lock throughout, as both do.
async fn refresh(
    store: &Store,
    session: &mut Session,
    account: AccountId,
    folder: FolderId,
    write_lock: &WriteLock,
) -> Result<(), SyncError> {
    let _writing = write_lock.lock().await;
    let mut syncer = FolderSyncer::new(store, session, &Quiet, account, SyncConfig::default());
    syncer.sync_folder(folder).await?;
    syncer.sync_changes(folder).await?;
    syncer.load_older(folder, 200).await?;
    mailcrane_core::threading::assign_new(store, Some(account))?;
    Ok(())
}

fn describe(error: &SyncError) -> String {
    let log = mailcrane_core::sync::FailureLog::new("sync", error, None);
    format!("{log:?}")
}

/// Two syncers, each with its own IMAP connection, store connection and
/// thread, refreshing the same folder five times at once: the
/// live-updates thread and the app's refresh on the phone. Each store
/// connection waits only `busy_timeout` for the other's write lock.
/// Returns the errors.
fn sync_twice_at_once(
    imap: &FakeImap,
    path: &Path,
    account: AccountId,
    folder: FolderId,
    locks: [WriteLock; 2],
    busy_timeout: Duration,
) -> Vec<String> {
    let threads: Vec<_> = locks
        .into_iter()
        .map(|write_lock| {
            let (port, cert, path) = (
                imap.addr().port(),
                imap.root_cert_der().to_vec(),
                path.to_path_buf(),
            );
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                runtime.block_on(async move {
                    let store = Store::open(&path).expect("store");
                    store.set_busy_timeout(busy_timeout).expect("timeout");
                    let mut config = ImapConfig::new("localhost", port, ImapSecurity::ImplicitTls)
                        .with_extra_root_cert(cert);
                    config.command_timeout = Duration::from_secs(10);
                    let mut session = Session::connect(&config, USER, &Secret::new(PASSWORD))
                        .await
                        .expect("connects");
                    let mut errors = Vec::new();
                    for _ in 0..5 {
                        if let Err(e) =
                            refresh(&store, &mut session, account, folder, &write_lock).await
                        {
                            errors.push(describe(&e));
                        }
                    }
                    errors
                })
            })
        })
        .collect();
    threads
        .into_iter()
        .flat_map(|t| t.join().expect("thread"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_updates_and_a_full_sync_of_a_big_folder_at_once_never_hit_a_busy_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let imap = gmail_fake(3_000).await;
    let (path, account, inbox) = setup(dir.path(), &imap).await;

    // One write lock for the account, shared by both, as the FFI shares it
    // with the live-updates thread. A 1 ms busy timeout makes any write
    // that overlaps the other's fail at once, so passing means no overlap.
    let lock = WriteLock::default();
    let errors = sync_twice_at_once(
        &imap,
        &path,
        account,
        inbox,
        [lock.clone(), lock],
        Duration::from_millis(1),
    );
    assert!(errors.is_empty(), "{errors:#?}");

    // Every message stored once: the newest 200, then ten loads of 200.
    let store = Store::open(&path).expect("store");
    let uids = store.list_uids(inbox).expect("uids");
    assert_eq!(uids.len(), 2_200);
    assert_eq!(uids.first(), Some(&801));
    assert_eq!(uids.last(), Some(&3_000));
}

#[tokio::test]
async fn a_v15_database_with_partial_gmail_rows_syncs_with_v16_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let imap = gmail_fake(1_000).await;
    let (path, account, inbox) = setup(dir.path(), &imap).await;

    // What three earlier builds left behind: some headers from a sync that
    // stopped part-way (a stale UIDVALIDITY, UIDs the server no longer
    // has, and ones it still has), threads, then the file put back at v15.
    {
        let store = Store::open(&path).expect("store");
        let mut session = connect(&imap).await;
        let mut syncer =
            FolderSyncer::new(&store, &mut session, &Quiet, account, SyncConfig::default());
        syncer.sync_folder(inbox).await.expect("partial sync");
        mailcrane_core::threading::assign_new(&store, Some(account)).expect("threads");
    }
    {
        let conn = rusqlite::Connection::open(&path).expect("open");
        conn.execute_batch(
            "UPDATE folders SET uidvalidity = 1, uidnext = 5 WHERE remote_name = 'INBOX';
             UPDATE messages SET uid = uid + 100000 WHERE uid % 7 = 0;
             ALTER TABLE folders DROP COLUMN selectable;
             ALTER TABLE accounts DROP COLUMN local_only;
             ALTER TABLE accounts DROP COLUMN provider_authserv_id;
             DROP TABLE server_rules_deployed;
             DROP TABLE server_rules;
             PRAGMA user_version = 15;",
        )
        .expect("back to v15 with partial rows");
    }

    let store = Store::open(&path).expect("migrates to the newest schema");
    let newest = Store::open_in_memory()
        .and_then(|s| s.schema_version())
        .expect("newest");
    assert_eq!(store.schema_version().expect("version"), newest);
    let mut session = connect(&imap).await;
    // Folders listed again, as the app does on start.
    let folders = session.list_folders().await.expect("list");
    save_folders(&store, account, &folders).expect("save");
    let lock = WriteLock::default();
    for _ in 0..2 {
        refresh(&store, &mut session, account, inbox, &lock)
            .await
            .unwrap_or_else(|e| panic!("{}", describe(&e)));
    }
    let uids = store.list_uids(inbox).expect("uids");
    // The stale UIDVALIDITY cleared the old rows: the newest 200, then two
    // loads of 200.
    assert_eq!(uids.len(), 600);
    assert!(uids.iter().all(|u| *u <= 1_000), "stale UIDs were cleared");
}
