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
        selectable: true,
        virtual_role: None,
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

#[test]
fn upsert_message_headers_is_all_or_nothing() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    // The second header names a folder that does not exist.
    let bad = s.upsert_message_headers(&[header(f, 1, 100), header(FolderId(999), 2, 100)]);
    assert!(bad.is_err());
    assert!(s.list_uids(f)?.is_empty());
    let ids = s.upsert_message_headers(&[header(f, 2, 100), header(f, 1, 100)])?;
    assert_eq!(ids.len(), 2);
    assert_eq!(s.list_uids(f)?, [1, 2]);
    assert_eq!(s.oldest_uid(f)?, Some(1));
    assert_eq!(s.oldest_uid(FolderId(999))?, None);
    Ok(())
}

#[test]
fn reset_folder_clears_messages_and_sets_uid_state() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let m = s.upsert_message_header(&header(f, 1, 100))?;
    s.store_body(m, b"x", 5)?;
    s.insert_attachments(m, &[attachment()])?;

    assert!(s.reset_folder(f, 7, 9)?);
    assert!(s.list_uids(f)?.is_empty());
    assert!(s.get_body(m)?.is_none());
    assert!(s.list_attachments(m)?.is_empty());
    let got = s.get_folder(f)?.ok_or("missing")?;
    assert_eq!(
        (got.details.uidvalidity, got.details.uidnext),
        (Some(7), Some(9))
    );

    // An unknown folder changes nothing.
    let other = s.upsert_folder(&folder(acc, "Sent"))?;
    s.upsert_message_header(&header(other, 1, 100))?;
    assert!(!s.reset_folder(FolderId(999), 1, 1)?);
    assert_eq!(s.list_uids(other)?, [1]);
    Ok(())
}

#[test]
fn store_parsed_body_replaces_attachments_and_is_atomic() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let m = s.upsert_message_header(&header(f, 1, 100))?;

    assert!(s.store_parsed_body(m, b"raw", 5, "snip", &[attachment()])?);
    assert!(s.store_parsed_body(m, b"raw", 6, "snip", &[attachment()])?);
    let got = s.get_message_header(m)?.ok_or("missing")?;
    assert_eq!(got.body_state, BodyState::Fetched);
    assert_eq!(got.details.snippet, "snip");
    assert!(got.details.has_attachments);
    assert_eq!(s.list_attachments(m)?.len(), 1);

    // An unknown message stores nothing.
    assert!(!s.store_parsed_body(MessageId(999), b"raw", 5, "x", &[])?);
    Ok(())
}

#[test]
fn remote_changes_delete_and_reflag_only_stored_messages() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    s.upsert_message_headers(&[header(f, 1, 10), header(f, 2, 20), header(f, 3, 30)])?;
    assert_eq!(s.newest_uid(f)?, Some(3));

    let seen = RemoteFlags {
        uid: 2,
        flags: Flags {
            seen: true,
            ..Flags::default()
        },
        keywords_json: "[\"$x\"]".into(),
    };
    let unchanged = RemoteFlags {
        uid: 3,
        flags: Flags::default(),
        keywords_json: "[]".into(),
    };
    let unknown = RemoteFlags {
        uid: 99,
        ..unchanged.clone()
    };
    // UID 1 and 98 are gone; 98 was never stored.
    let changed = s.apply_remote_changes(f, &[1, 98], &[seen, unchanged, unknown])?;
    assert_eq!(changed, 2);
    assert_eq!(s.list_uids(f)?, [2, 3]);
    let two = s
        .list_messages(f, 10, 0)?
        .into_iter()
        .find(|m| m.details.uid == 2)
        .ok_or("uid 2")?;
    assert!(two.details.flags.seen);
    assert_eq!(two.details.keywords_json, "[\"$x\"]");

    assert!(s.set_folder_sync_state(f, 4, Some(77))?);
    let stored = s.get_folder(f)?.ok_or("folder")?;
    assert_eq!(
        (stored.details.uidnext, stored.details.highestmodseq),
        (Some(4), Some(77))
    );
    assert!(!s.set_folder_sync_state(FolderId(999), 1, None)?);
    Ok(())
}

#[test]
fn prefetch_candidates_keep_to_the_newest_recent_small_uncached() -> R {
    let s = Store::open_in_memory()?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let other = s.upsert_folder(&folder(acc, "Archive"))?;
    // uid n arrives at time 100 + n.
    let mut ids = Vec::new();
    for uid in 1..=6 {
        ids.push(s.upsert_message_header(&header(f, uid, 100 + i64::from(uid)))?);
    }
    s.upsert_message_header(&header(other, 7, 200))?;
    let mut big = header(f, 8, 50);
    big.size = 2_000;
    s.upsert_message_header(&big)?;
    // uid 6 already has its body.
    assert!(s.store_body(ids[5], b"raw", 1)?);

    // Newest first, cached and other folders left out.
    let all = s.prefetch_candidates(f, 0, 200, 1_000)?;
    let uids: Vec<u32> = all.iter().map(|(_, uid)| *uid).collect();
    assert_eq!(uids, [5, 4, 3, 2, 1]);
    assert_eq!(all.first().map(|(id, _)| *id), Some(ids[4]));
    // Too old.
    let uids: Vec<u32> = s
        .prefetch_candidates(f, 103, 200, 1_000)?
        .iter()
        .map(|(_, uid)| *uid)
        .collect();
    assert_eq!(uids, [5, 4, 3]);
    // Only the newest 3 messages count, the cached one among them.
    let uids: Vec<u32> = s
        .prefetch_candidates(f, 0, 3, 1_000)?
        .iter()
        .map(|(_, uid)| *uid)
        .collect();
    assert_eq!(uids, [5, 4]);
    // The big one only when the size limit allows it.
    let uids: Vec<u32> = s
        .prefetch_candidates(f, 0, 200, 5_000)?
        .iter()
        .map(|(_, uid)| *uid)
        .collect();
    assert_eq!(uids, [5, 4, 3, 2, 1, 8]);
    Ok(())
}

#[test]
fn write_transactions_wait_for_urgent_writes_to_finish() -> R {
    // Issue #294: a write the user waits for gets the next turn.
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("u.db");
    let s = Store::open(&path)?;
    let acc = s.insert_account(&account())?;
    let f = s.upsert_folder(&folder(acc, "INBOX"))?;
    let (inside_tx, inside) = std::sync::mpsc::channel();
    let (release_tx, release) = std::sync::mpsc::channel::<()>();
    let urgent_path = path.clone();
    let key = path.to_string_lossy().into_owned();
    let urgent = std::thread::spawn(move || {
        urgent_writes(&key, || {
            let store = Store::open(&urgent_path).expect("store");
            let _ = inside_tx.send(());
            let _ = release.recv_timeout(std::time::Duration::from_secs(10));
            // Its own writes are never held up.
            store
                .upsert_message_header(&header(f, 1, 10))
                .expect("urgent write");
            std::time::Instant::now()
        })
    });
    inside.recv()?;
    let started = std::time::Instant::now();
    let release_after = std::time::Duration::from_millis(300);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(release_after);
        let _ = release_tx.send(());
    });
    // Another thread's write transaction (as a sync's batches are) waits
    // until the urgent one is done.
    s.upsert_message_headers(&[header(f, 2, 20)])?;
    let ordinary_done = std::time::Instant::now();
    let urgent_done = urgent.join().map_err(|_| "urgent thread")?;
    releaser.join().map_err(|_| "releaser")?;
    assert!(started.elapsed() >= release_after);
    assert!(urgent_done <= ordinary_done, "the urgent write went first");
    // Nothing is held up once no urgent write runs.
    let quick = std::time::Instant::now();
    s.upsert_message_headers(&[header(f, 3, 30)])?;
    assert!(quick.elapsed() < std::time::Duration::from_secs(1));
    Ok(())
}

#[test]
fn urgent_writes_on_one_database_never_delay_another() -> R {
    let dir = tempfile::tempdir()?;
    let other = Store::open(dir.path().join("other.db"))?;
    let acc = other.insert_account(&account())?;
    let f = other.upsert_folder(&folder(acc, "INBOX"))?;
    let urgent_key = dir.path().join("urgent.db").to_string_lossy().into_owned();
    let (inside_tx, inside) = std::sync::mpsc::channel();
    let (done_tx, done) = std::sync::mpsc::channel::<()>();
    let urgent = std::thread::spawn(move || {
        urgent_writes(&urgent_key, || {
            let _ = inside_tx.send(());
            let _ = done.recv_timeout(std::time::Duration::from_secs(10));
        })
    });
    inside.recv()?;
    let started = std::time::Instant::now();
    other.upsert_message_headers(&[header(f, 1, 10)])?;
    let took = started.elapsed();
    let _ = done_tx.send(());
    urgent.join().map_err(|_| "urgent thread")?;
    assert!(took < std::time::Duration::from_millis(200), "{took:?}");
    Ok(())
}
