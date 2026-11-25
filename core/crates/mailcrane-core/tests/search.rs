//! Server search (M4-03) against the testkit's fake IMAP server.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::events::{CoreEvent, EventListener};
use mailcrane_core::search::{local_search, parse_query};
use mailcrane_core::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{AccountId, FolderId, NewAccount, Security, Store};
use mailcrane_testkit::FakeImap;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _event: CoreEvent) {}
}

fn raw(subject: &str, body: &str) -> Vec<u8> {
    format!("From: Bob <bob@example.net>\r\nTo: alice@example.org\r\nSubject: {subject}\r\n\r\n{body}\r\n")
        .into_bytes()
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

async fn setup(imap: &FakeImap, store: &Store) -> (AccountId, FolderId) {
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
    let listed = session.list_folders().await.expect("list");
    let ids = save_folders(store, account, &listed).expect("save");
    let inbox = listed
        .iter()
        .zip(ids)
        .find(|(f, _)| f.raw_name == "INBOX")
        .map(|(_, id)| id)
        .expect("inbox");
    let mut syncer = FolderSyncer::new(store, &mut session, &Quiet, account, SyncConfig::default());
    syncer.sync_folder(inbox).await.expect("sync");
    session.logout().await;
    (account, inbox)
}

#[tokio::test]
async fn server_search_finds_and_stores_a_message_not_stored_locally() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder(
            "INBOX",
            vec![raw("Lunch", "Noon?"), raw("Report", "Numbers")],
        )
        .start()
        .await
        .expect("imap");
    let store = Store::open_in_memory().expect("store");
    let (account, inbox) = setup(&imap, &store).await;
    // Arrives after the last sync: only the server has it.
    imap.mailbox()
        .deliver("INBOX", raw("Holiday photos", "From the zebrafish trip"))
        .await;
    let query = parse_query("zebrafish");
    let before = local_search(&store, account, None, &query, 50).expect("local");
    assert!(before.is_empty(), "not stored yet");

    let mut session = connect(&imap).await;
    let stored = {
        let mut syncer =
            FolderSyncer::new(&store, &mut session, &Quiet, account, SyncConfig::default());
        syncer
            .search_server(inbox, &query)
            .await
            .expect("server search")
    };
    assert_eq!(stored, 1);
    // Its header is stored now; searching again stores nothing new.
    let again = {
        let mut syncer =
            FolderSyncer::new(&store, &mut session, &Quiet, account, SyncConfig::default());
        syncer
            .search_server(inbox, &query)
            .await
            .expect("server search")
    };
    session.logout().await;
    assert_eq!(again, 0, "no duplicates");
    let subject_hits =
        local_search(&store, account, None, &parse_query("holiday"), 50).expect("local");
    assert_eq!(subject_hits.len(), 1);
    assert_eq!(subject_hits[0].details.subject, "Holiday photos");
    assert_eq!(store.list_uids(inbox).expect("uids").len(), 3);
}

#[tokio::test]
async fn offline_local_search_still_works() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw("Lunch", "Noon?")])
        .start()
        .await
        .expect("imap");
    let store = Store::open_in_memory().expect("store");
    let (account, _) = setup(&imap, &store).await;
    imap.shutdown().await;
    let hits = local_search(&store, account, None, &parse_query("lunch"), 50).expect("local");
    assert_eq!(hits.len(), 1);
}
