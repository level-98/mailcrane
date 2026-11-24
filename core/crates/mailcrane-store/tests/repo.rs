//! Repository behaviour.

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn account() -> NewAccount {
    NewAccount {
        display_name: "Alice".into(),
        email: "alice@example.org".into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 587,
        smtp_security: Security::Starttls,
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
        uidvalidity: Some(1),
        uidnext: Some(2),
        highestmodseq: None,
        is_subscribed: true,
    }
}

#[test]
fn upsert_folders_is_all_or_nothing() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let ids = s.upsert_folders(&[folder(acc, "INBOX"), folder(acc, "Sent")])?;
    assert_eq!(ids.len(), 2);
    // The second folder belongs to an account that does not exist, so the
    // foreign key fails and the first one must not be saved either.
    let err = s.upsert_folders(&[folder(acc, "Drafts"), folder(AccountId(9999), "X")]);
    assert!(err.is_err());
    let names: Vec<String> = s
        .list_folders(acc)?
        .into_iter()
        .map(|f| f.details.remote_name)
        .collect();
    assert_eq!(names, ["INBOX", "Sent"]);
    Ok(())
}

fn header(folder_id: FolderId, uid: u32, date: i64) -> NewMessageHeader {
    NewMessageHeader {
        folder_id,
        uid,
        message_id_header: Some(format!("<{uid}@example.org>")),
        in_reply_to: None,
        references_json: "[]".into(),
        thread_id: None,
        subject: "Hello".into(),
        from_name: Some("Bob".into()),
        from_addr: Some("bob@example.org".into()),
        to_json: "[]".into(),
        cc_json: "[]".into(),
        date,
        internal_date: date,
        size: 10,
        flags: Flags::default(),
        keywords_json: "[]".into(),
        has_attachments: false,
        snippet: String::new(),
    }
}

fn attachment() -> NewAttachment {
    NewAttachment {
        part_id: "2".into(),
        filename: Some("a.txt".into()),
        mime_type: "text/plain".into(),
        size: 3,
        content_id: None,
        is_inline: false,
    }
}

#[test]
fn account_roundtrip_and_delete() -> R {
    let s = Store::open_in_memory()?;
    let id = s.insert_account(&account())?;
    let got = s.get_account(id)?.ok_or("missing")?;
    assert_eq!(got.details, account());
    assert_eq!(s.list_accounts()?.len(), 1);
    assert!(s.delete_account(id)?);
    assert!(s.get_account(id)?.is_none());
    assert!(!s.delete_account(id)?);
    Ok(())
}

#[test]
fn deleting_account_cascades() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let m = s.upsert_message_header(&header(f, 1, 100))?;
    s.store_body(m, b"Subject: x\r\n\r\nhi", 5)?;
    s.insert_attachments(m, &[attachment()])?;

    assert!(s.delete_account(acc)?);
    assert!(s.list_folders(acc)?.is_empty());
    assert!(s.get_message_header(m)?.is_none());
    assert!(s.get_body(m)?.is_none());
    assert!(s.list_attachments(m)?.is_empty());
    Ok(())
}

#[test]
fn folder_upsert_keeps_id() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let a = s.upsert_folder(&folder(acc, "INBOX"))?;
    let mut f = folder(acc, "INBOX");
    f.role = Some(FolderRole::Inbox);
    f.uidnext = Some(50);
    let b = s.upsert_folder(&f)?;
    assert_eq!(a, b);
    let list = s.list_folders(acc)?;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].details.role, Some(FolderRole::Inbox));
    assert_eq!(list[0].details.uidnext, Some(50));
    Ok(())
}

#[test]
fn message_upsert_flags_unread_and_body() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let a = s.upsert_message_header(&header(f, 1, 10))?;
    let b = s.upsert_message_header(&header(f, 2, 20))?;
    assert_eq!(s.count_unread(f)?, 2);

    let seen = Flags {
        seen: true,
        ..Flags::default()
    };
    assert!(s.set_flags(a, seen)?);
    assert_eq!(s.count_unread(f)?, 1);
    assert!(!s.set_flags(MessageId(9999), seen)?);

    assert!(s.store_body(b, b"raw", 7)?);
    assert_eq!(s.get_body(b)?.as_deref(), Some(&b"raw"[..]));
    // Re-upserting a header keeps the body state.
    s.upsert_message_header(&header(f, 2, 20))?;
    let h = s.get_message_header(b)?.ok_or("missing")?;
    assert_eq!(h.body_state, BodyState::Fetched);
    assert!(!s.store_body(MessageId(9999), b"x", 1)?);
    assert!(s.get_body(a)?.is_none());
    Ok(())
}

#[test]
fn listing_orders_and_paginates_with_equal_dates() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let mut ids = Vec::new();
    for uid in 1..=7u32 {
        // Mostly equal dates, one newer and one older.
        let date = match uid {
            3 => 200,
            6 => 50,
            _ => 100,
        };
        ids.push(s.upsert_message_header(&header(f, uid, date))?);
    }
    let all = s.list_messages(f, 100, 0)?;
    let keys: Vec<(i64, i64)> = all.iter().map(|m| (m.details.date, m.id.0)).collect();
    let mut sorted = keys.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(keys, sorted);

    let mut paged = Vec::new();
    let mut offset = 0;
    loop {
        let page = s.list_messages(f, 3, offset)?;
        if page.is_empty() {
            break;
        }
        offset += 3;
        paged.extend(page.into_iter().map(|m| m.id));
    }
    assert_eq!(paged, all.iter().map(|m| m.id).collect::<Vec<_>>());
    assert_eq!(paged.len(), 7);
    Ok(())
}

#[test]
fn attachments_roundtrip() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let m = s.upsert_message_header(&header(f, 1, 1))?;
    s.insert_attachments(m, &[attachment(), attachment()])?;
    let list = s.list_attachments(m)?;
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].details, attachment());
    Ok(())
}

#[test]
fn attachments_for_missing_message_insert_nothing_and_error() -> R {
    let s = Store::open_in_memory()?;
    let r = s.insert_attachments(MessageId(42), &[attachment()]);
    assert!(r.is_err());
    Ok(())
}
