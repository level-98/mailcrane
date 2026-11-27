//! JMAP accounts in the store (M3-04).

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn account(email: &str) -> NewAccount {
    NewAccount {
        display_name: "Alice".into(),
        email: email.into(),
        imap_host: String::new(),
        imap_port: 0,
        imap_security: Security::Tls,
        smtp_host: String::new(),
        smtp_port: 0,
        smtp_security: Security::Tls,
        username: "alice".into(),
    }
}

fn folder(account_id: AccountId, name: &str) -> NewFolder {
    NewFolder {
        account_id,
        remote_name: name.into(),
        display_name: name.into(),
        delimiter: Some("/".into()),
        role: None,
        uidvalidity: None,
        uidnext: None,
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
        virtual_role: None,
    }
}

fn header(folder_id: FolderId, seen: bool) -> NewMessageHeader {
    NewMessageHeader {
        folder_id,
        uid: 0,
        message_id_header: Some("m1@example.net".into()),
        in_reply_to: None,
        references_json: "[]".into(),
        thread_id: None,
        subject: "Hello".into(),
        from_name: None,
        from_addr: Some("bob@example.net".into()),
        to_json: "[]".into(),
        cc_json: "[]".into(),
        date: 100,
        internal_date: 100,
        size: 10,
        flags: Flags {
            seen,
            ..Flags::default()
        },
        keywords_json: "[]".into(),
        has_attachments: false,
        snippet: String::new(),
    }
}

#[test]
fn accounts_know_their_protocol() -> R {
    let store = Store::open_in_memory()?;
    let jmap = store.insert_jmap_account(
        &account("a@example.org"),
        "https://jmap.example.org/session",
    )?;
    let imap = store.insert_account(&account("b@example.org"))?;
    assert_eq!(
        store.account_protocol(jmap)?,
        Some(Protocol::Jmap {
            session_url: "https://jmap.example.org/session".into()
        })
    );
    assert_eq!(store.account_protocol(imap)?, Some(Protocol::Imap));
    assert_eq!(store.account_protocol(AccountId(99))?, None);
    Ok(())
}

#[test]
fn emails_get_per_folder_numbers_and_updates_keep_the_row() -> R {
    let store = Store::open_in_memory()?;
    let a = store.insert_jmap_account(&account("a@example.org"), "https://jmap.example.org/s")?;
    let inbox = store.upsert_folder(&folder(a, "Inbox"))?;
    let work = store.upsert_folder(&folder(a, "Work"))?;
    store.set_jmap_mailbox(inbox, "mb-inbox")?;
    store.set_jmap_mailbox(work, "mb-work")?;
    assert_eq!(store.jmap_mailbox(work)?, Some("mb-work".into()));
    assert_eq!(store.jmap_mailboxes(a)?.len(), 2);

    let first = store.upsert_jmap_email(&header(inbox, false), "e1", "b1")?;
    let second = store.upsert_jmap_email(&header(inbox, false), "e2", "b2")?;
    let in_work = store.upsert_jmap_email(&header(work, false), "e1", "b1")?;
    let uids = store.list_uids(inbox)?;
    assert_eq!(uids, vec![1, 2]);
    assert_eq!(store.list_uids(work)?, vec![1]);
    // The folder now counts as synced once.
    assert_eq!(
        store.get_folder(inbox)?.map(|f| f.details.uidvalidity),
        Some(Some(1))
    );

    // Updating flags keeps the same row.
    let again = store.upsert_jmap_email(&header(inbox, true), "e1", "b1")?;
    assert_eq!(again, first);
    assert!(store
        .get_message_header(first)?
        .map(|h| h.details.flags.seen)
        .unwrap_or(false));

    let rows = store.jmap_rows(a, "e1")?;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        store.jmap_row(second)?.map(|r| r.blob_id),
        Some("b2".into())
    );
    assert_eq!(store.delete_message_rows(&[in_work])?, 1);
    assert_eq!(store.jmap_rows(a, "e1")?.len(), 1);
    assert_eq!(store.count_folder_messages(inbox)?, 2);
    Ok(())
}

#[test]
fn state_strings_are_kept_per_type() -> R {
    let store = Store::open_in_memory()?;
    let a = store.insert_jmap_account(&account("a@example.org"), "https://jmap.example.org/s")?;
    assert_eq!(store.jmap_state(a, "Email")?, None);
    store.set_jmap_state(a, "Email", "s1")?;
    store.set_jmap_state(a, "Email", "s2")?;
    store.set_jmap_state(a, "Mailbox", "m1")?;
    assert_eq!(store.jmap_state(a, "Email")?, Some("s2".into()));
    store.clear_jmap_state(a, "Email")?;
    assert_eq!(store.jmap_state(a, "Email")?, None);
    assert_eq!(store.jmap_state(a, "Mailbox")?, Some("m1".into()));
    Ok(())
}
