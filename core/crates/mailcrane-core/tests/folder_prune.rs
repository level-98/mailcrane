//! Folders deleted or renamed on the server by another client leave the
//! store too (#323).

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_imap::{save_folder_list, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{
    AccountId, FolderId, NewAccount, NewRule, RuleAction, RuleCondition, Security, Store,
};
use mailcrane_testkit::{FakeImap, Fault};
use std::time::Duration;

const USER: &str = "ann@example.org";
const PASSWORD: &str = "app-password";

fn raw(n: u32) -> Vec<u8> {
    format!("From: a@example.net\r\nSubject: {n}\r\nMessage-ID: <{n}@example.net>\r\n\r\nHi\r\n")
        .into_bytes()
}

async fn fake(faults: &[Fault]) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw(1)])
        .folder("Projects", Vec::new())
        .folder("Projects/2026", Vec::new())
        .folder("Notes", vec![raw(2), raw(3)])
        .folder("Trash", Vec::new());
    for f in faults {
        builder = builder.fault(f.clone());
    }
    builder.start().await.expect("fake imap starts")
}

async fn connect(imap: &FakeImap) -> Result<Session, mailcrane_imap::ImapError> {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD)).await
}

fn account(store: &Store) -> AccountId {
    store
        .insert_account(&NewAccount {
            display_name: "Ann".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: USER.into(),
        })
        .expect("account")
}

fn id_of(store: &Store, account: AccountId, remote: &str) -> Option<FolderId> {
    store
        .list_folders(account)
        .expect("list")
        .into_iter()
        .find(|f| f.details.remote_name == remote)
        .map(|f| f.id)
}

fn names(store: &Store, account: AccountId) -> Vec<String> {
    let mut n: Vec<String> = store
        .list_folders(account)
        .expect("list")
        .into_iter()
        .map(|f| f.details.remote_name)
        .collect();
    n.sort();
    n
}

#[tokio::test]
async fn folders_gone_from_the_server_are_removed_and_standard_ones_rematched() {
    let imap = fake(&[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = account(&store);
    let mut session = connect(&imap).await.expect("connects");
    save_folder_list(
        &store,
        account,
        &session.list_folders().await.expect("list"),
    )
    .expect("save");
    let notes = id_of(&store, account, "Notes").expect("notes");
    let trash = id_of(&store, account, "Trash").expect("trash");
    // Pretend Notes was synced: a message stored in it.
    store
        .upsert_message_header(&mailcrane_store::NewMessageHeader {
            folder_id: notes,
            uid: 1,
            message_id_header: Some("<2@example.net>".into()),
            subject: "2".into(),
            from_name: None,
            from_addr: Some("a@example.net".into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date: 1_790_000_000,
            internal_date: 1_790_000_000,
            size: 10,
            flags: Default::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
        })
        .expect("message");
    let rule = store
        .insert_rule(&NewRule {
            account_id: Some(account),
            name: "To notes".into(),
            enabled: true,
            match_all: true,
            stop_processing: false,
            conditions: vec![RuleCondition::Subject("note".into())],
            actions: vec![RuleAction::Move(notes), RuleAction::Flag],
        })
        .expect("rule");

    // Another client deletes Notes, renames Projects (with its subfolder)
    // and renames Trash to a name that still means Trash.
    let mailbox = imap.mailbox();
    mailbox.delete_folder("Notes").await;
    assert!(mailbox.rename_folder("Projects", "Work").await);
    assert!(mailbox.rename_folder("Trash", "Deleted Items").await);

    let saved = save_folder_list(
        &store,
        account,
        &session.list_folders().await.expect("list"),
    )
    .expect("save");
    session.logout().await;

    assert_eq!(
        names(&store, account),
        ["Deleted Items", "INBOX", "Work", "Work/2026"]
    );
    // Notes and the two Projects folders are reported as removed; Trash kept
    // its row under the new name.
    assert_eq!(saved.removed.len(), 3);
    assert!(saved.removed.contains(&notes));
    assert_eq!(id_of(&store, account, "Deleted Items"), Some(trash));
    assert!(store.get_folder(notes).expect("get").is_none());
    assert!(store
        .list_messages(notes, 10, 0)
        .expect("messages")
        .is_empty());

    // The rule is turned off and says why; its move went with the folder.
    let r = store.get_rule(rule).expect("get").expect("rule");
    assert!(!r.rule.enabled);
    assert_eq!(r.missing_folder.as_deref(), Some("Notes"));
    assert_eq!(r.rule.actions, vec![RuleAction::Flag]);
    // Turning it back on clears the note.
    store.set_rule_enabled(rule, true).expect("enable");
    assert_eq!(
        store
            .get_rule(rule)
            .expect("get")
            .expect("rule")
            .missing_folder,
        None
    );
}

#[tokio::test]
async fn an_empty_or_failed_list_removes_nothing() {
    let imap = fake(&[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = account(&store);
    let mut session = connect(&imap).await.expect("connects");
    save_folder_list(
        &store,
        account,
        &session.list_folders().await.expect("list"),
    )
    .expect("save");
    session.logout().await;
    let before = names(&store, account);

    let saved = save_folder_list(&store, account, &[]).expect("save");
    assert!(saved.removed.is_empty());
    assert_eq!(names(&store, account), before);

    // A connection dropped during LIST: nothing to save, nothing changes.
    let mut hit = false;
    for after in 1..=6 {
        let flaky = fake(&[Fault::DropAfterCommands(after)]).await;
        let Ok(mut session) = connect(&flaky).await else {
            continue;
        };
        match session.list_folders().await {
            Ok(_) => {}
            Err(_) => {
                hit = true;
                assert_eq!(names(&store, account), before);
            }
        }
    }
    assert!(hit, "some fault dropped the LIST");
}

#[tokio::test]
async fn the_inbox_is_never_removed() {
    let imap = fake(&[]).await;
    let store = Store::open_in_memory().expect("store");
    let account = account(&store);
    let mut session = connect(&imap).await.expect("connects");
    let listed = session.list_folders().await.expect("list");
    save_folder_list(&store, account, &listed).expect("save");
    session.logout().await;
    // A list without INBOX (a broken server) keeps it.
    let without: Vec<_> = listed
        .into_iter()
        .filter(|f| !f.raw_name.eq_ignore_ascii_case("INBOX"))
        .collect();
    save_folder_list(&store, account, &without).expect("save");
    assert!(id_of(&store, account, "INBOX").is_some());
}

#[tokio::test]
async fn a_renamed_virtual_folder_keeps_its_row() {
    // #461: Gmail renames "[Gmail]/Starred" when its interface language
    // changes. A virtual folder has no standard role, so it's matched by
    // its virtual role instead, and keeps its row (and its messages)
    // rather than being removed and fetched again.
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw(1)])
        .folder("Starred", Vec::new())
        // Both names mean starred mail (RFC 6154 \Flagged, on any server).
        .special_use("Starred", "\\Flagged")
        .special_use("Com estrela", "\\Flagged")
        .start()
        .await
        .expect("fake imap starts");
    let store = Store::open_in_memory().expect("store");
    let account = account(&store);
    let mut session = connect(&imap).await.expect("connects");
    save_folder_list(
        &store,
        account,
        &session.list_folders().await.expect("list"),
    )
    .expect("save");
    let starred = id_of(&store, account, "Starred").expect("starred");

    assert!(imap.mailbox().rename_folder("Starred", "Com estrela").await);
    let saved = save_folder_list(
        &store,
        account,
        &session.list_folders().await.expect("list"),
    )
    .expect("save");
    session.logout().await;

    assert!(saved.removed.is_empty(), "nothing removed");
    assert_eq!(id_of(&store, account, "Com estrela"), Some(starred));
    assert_eq!(names(&store, account), ["Com estrela", "INBOX"]);
    assert_eq!(
        store
            .get_folder(starred)
            .expect("get")
            .expect("folder")
            .details
            .virtual_role,
        Some(mailcrane_store::VirtualRole::Starred)
    );
}

#[tokio::test]
async fn ambiguous_virtual_folders_are_not_paired_by_order() {
    // #482 review: every \\Flagged folder has the same virtual role, so two
    // gone and two new ones can't be told apart: they're removed and added,
    // never paired by LIST order.
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw(1)])
        .folder("Pins", Vec::new())
        .folder("Marked", Vec::new())
        .special_use("Pins", "\\Flagged")
        .special_use("Marked", "\\Flagged")
        .special_use("Pins 2", "\\Flagged")
        .special_use("Marked 2", "\\Flagged")
        .start()
        .await
        .expect("fake imap starts");
    let store = Store::open_in_memory().expect("store");
    let account = account(&store);
    let mut session = connect(&imap).await.expect("connects");
    save_folder_list(
        &store,
        account,
        &session.list_folders().await.expect("list"),
    )
    .expect("save");
    let pins = id_of(&store, account, "Pins").expect("pins");
    let marked = id_of(&store, account, "Marked").expect("marked");
    assert!(imap.mailbox().rename_folder("Pins", "Pins 2").await);
    assert!(imap.mailbox().rename_folder("Marked", "Marked 2").await);
    let saved = save_folder_list(
        &store,
        account,
        &session.list_folders().await.expect("list"),
    )
    .expect("save");
    session.logout().await;
    assert!(saved.removed.contains(&pins) && saved.removed.contains(&marked));
    assert_eq!(names(&store, account), ["INBOX", "Marked 2", "Pins 2"]);
    assert_ne!(id_of(&store, account, "Pins 2"), Some(pins));
    assert_ne!(id_of(&store, account, "Pins 2"), Some(marked));
}
