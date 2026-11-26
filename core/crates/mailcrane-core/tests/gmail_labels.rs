//! Gmail labels as folders (M3-02), against the testkit's Gmail-like fake.
//! Never a real mailbox.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::events::{CoreEvent, EventListener};
use mailcrane_core::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{FolderRole, NewAccount, Security, Store};
use mailcrane_testkit::FakeImap;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "app-password-example";

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _: CoreEvent) {}
}

/// One message, as Gmail shows it in every label it has.
fn message() -> Vec<u8> {
    b"From: Bob <bob@example.com>\r\n\
      To: Alice <alice@example.org>\r\n\
      Subject: Receipt for your order\r\n\
      Date: Tue, 24 Nov 2026 10:00:00 +0100\r\n\
      Message-ID: <receipt-1@mail.example.com>\r\n\
      \r\n\
      Thanks for your order.\r\n"
        .to_vec()
}

#[tokio::test]
async fn a_message_under_inbox_and_two_labels_is_counted_once_and_shares_a_thread() {
    // Gmail lists each label as a folder holding the same message.
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![message()])
        .folder("Receipts", vec![message()])
        .folder("Work", vec![message()])
        .gmail_layout()
        .start()
        .await
        .expect("fake imap starts");
    let store = Store::open_in_memory().expect("store");
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
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_secs(10);
    let mut session = Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects");
    let listed = session.list_folders().await.expect("list");
    save_folders(&store, account, &listed).expect("save");
    let folders = store.list_folders(account).expect("folders");
    let id = |name: &str| {
        folders
            .iter()
            .find(|f| f.details.remote_name == name)
            .map(|f| f.id)
            .unwrap_or_else(|| panic!("{name} listed"))
    };
    // All Mail is the archive target; Trash and the others have their roles.
    let role = |name: &str| {
        folders
            .iter()
            .find(|f| f.details.remote_name == name)
            .and_then(|f| f.details.role)
    };
    assert_eq!(role("[Gmail]/All Mail"), Some(FolderRole::All));
    assert_eq!(role("[Gmail]/Trash"), Some(FolderRole::Trash));
    assert_eq!(role("[Gmail]/Sent Mail"), Some(FolderRole::Sent));

    {
        let mut syncer =
            FolderSyncer::new(&store, &mut session, &Quiet, account, SyncConfig::default());
        for name in ["INBOX", "Receipts", "Work"] {
            syncer.sync_folder(id(name)).await.expect("sync");
        }
    }
    mailcrane_core::threading::assign_new(&store, Some(account)).expect("threads");

    // Stored once in each folder (each label shows it)...
    let rows: Vec<_> = ["INBOX", "Receipts", "Work"]
        .iter()
        .map(|name| {
            let list = store.list_messages(id(name), 10, 0).expect("list");
            assert_eq!(list.len(), 1, "{name}");
            list.into_iter().next().expect("one")
        })
        .collect();
    // ...counted once in "All inboxes"...
    assert_eq!(store.count_unified_unread().expect("count"), 1);
    // ...and one conversation.
    let thread = rows[0].details.thread_id;
    assert!(thread.is_some());
    assert!(rows.iter().all(|r| r.details.thread_id == thread));
}
