//! The queue of message actions (M1-14).

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
        uidnext: Some(10),
        highestmodseq: None,
        is_subscribed: true,
    }
}

fn header(folder_id: FolderId, uid: u32) -> NewMessageHeader {
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
        date: 1_700_000_000 + i64::from(uid),
        internal_date: 1_700_000_000,
        size: 10,
        flags: Flags::default(),
        keywords_json: "[]".into(),
        has_attachments: false,
        snippet: String::new(),
    }
}

struct Fixture {
    store: Store,
    account: AccountId,
    inbox: FolderId,
    archive: FolderId,
    messages: Vec<OpMessage>,
}

fn fixture(store: Store) -> Result<Fixture, StoreError> {
    let account = store.insert_account(&account())?;
    let inbox = store.upsert_folder(&folder(account, "INBOX"))?;
    let archive = store.upsert_folder(&folder(account, "Archive"))?;
    let mut messages = Vec::new();
    for uid in [1, 2, 3] {
        let message_id = store.upsert_message_header(&header(inbox, uid))?;
        messages.push(OpMessage { message_id, uid });
    }
    Ok(Fixture {
        store,
        account,
        inbox,
        archive,
        messages,
    })
}

fn seen(store: &Store, m: OpMessage) -> Result<bool, StoreError> {
    Ok(store
        .get_message_header(m.message_id)?
        .map(|h| h.details.flags.seen)
        .unwrap_or(false))
}

#[test]
fn flag_action_applies_locally_and_queues() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let first = f.messages[0];
    let op = f
        .store
        .queue_op(f.account, OpKind::MarkRead, f.inbox, None, &[first], 100)?;
    assert!(op.is_some());
    assert!(seen(&f.store, first)?);
    assert_eq!(f.store.count_unread(f.inbox)?, 2);

    let ops = f.store.pending_ops(f.account)?;
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].kind, OpKind::MarkRead);
    assert_eq!(ops[0].messages, vec![first]);
    assert_eq!(ops[0].next_attempt_at, 100);
    assert_eq!((ops[0].attempts, ops[0].step), (0, 0));
    Ok(())
}

#[test]
fn opposite_actions_cancel_out_before_replay() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let first = f.messages[0];
    f.store
        .queue_op(f.account, OpKind::MarkRead, f.inbox, None, &[first], 100)?;
    let again = f
        .store
        .queue_op(f.account, OpKind::MarkUnread, f.inbox, None, &[first], 101)?;
    assert_eq!(again, None);
    assert!(!seen(&f.store, first)?);
    assert!(f.store.pending_ops(f.account)?.is_empty());
    Ok(())
}

#[test]
fn an_action_already_tried_is_not_cancelled() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let first = f.messages[0];
    let op = f
        .store
        .queue_op(f.account, OpKind::Flag, f.inbox, None, &[first], 100)?
        .ok_or("queued")?;
    // It may have reached the server before the connection dropped.
    f.store.defer_op(op, 1, 105, "network")?;
    f.store
        .queue_op(f.account, OpKind::Unflag, f.inbox, None, &[first], 101)?;
    let kinds: Vec<OpKind> = f
        .store
        .pending_ops(f.account)?
        .iter()
        .map(|o| o.kind)
        .collect();
    assert_eq!(kinds, [OpKind::Flag, OpKind::Unflag]);
    Ok(())
}

#[test]
fn only_the_cancelled_message_leaves_a_shared_action() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let (a, b) = (f.messages[0], f.messages[1]);
    f.store
        .queue_op(f.account, OpKind::MarkRead, f.inbox, None, &[a, b], 100)?;
    f.store
        .queue_op(f.account, OpKind::MarkUnread, f.inbox, None, &[a], 101)?;
    let ops = f.store.pending_ops(f.account)?;
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].messages, vec![b]);
    Ok(())
}

#[test]
fn move_hides_then_completes_or_fails() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let (a, b) = (f.messages[0], f.messages[1]);
    let moved = f
        .store
        .queue_op(f.account, OpKind::Move, f.inbox, Some(f.archive), &[a], 100)?
        .ok_or("queued")?;
    let kept = f
        .store
        .queue_op(f.account, OpKind::Move, f.inbox, Some(f.archive), &[b], 100)?
        .ok_or("queued")?;
    assert_eq!(f.store.list_messages(f.inbox, 10, 0)?.len(), 1);
    assert_eq!(f.store.count_unread(f.inbox)?, 1);
    // A sync still knows the hidden UIDs, so it won't fetch them again.
    assert_eq!(f.store.list_uids(f.inbox)?, [1, 2, 3]);

    f.store.complete_op(moved, true)?;
    assert!(f.store.get_message_header(a.message_id)?.is_none());

    let reverted = f.store.fail_op(kept)?;
    assert_eq!(reverted, [b.message_id]);
    assert_eq!(f.store.list_messages(f.inbox, 10, 0)?.len(), 2);
    assert!(f.store.pending_ops(f.account)?.is_empty());
    Ok(())
}

#[test]
fn failed_flag_action_is_undone() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let first = f.messages[0];
    let op = f
        .store
        .queue_op(f.account, OpKind::MarkRead, f.inbox, None, &[first], 100)?
        .ok_or("queued")?;
    f.store.fail_op(op)?;
    assert!(!seen(&f.store, first)?);
    // Failing an action that is gone already does nothing.
    assert!(f.store.fail_op(op)?.is_empty());
    Ok(())
}

#[test]
fn deferring_and_steps_are_saved_and_survive_a_reopen() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("q.db");
    let f = fixture(Store::open(&path)?)?;
    let op = f
        .store
        .queue_op(
            f.account,
            OpKind::Move,
            f.inbox,
            Some(f.archive),
            &f.messages,
            100,
        )?
        .ok_or("queued")?;
    f.store.set_op_step(op, 1)?;
    f.store.defer_op(op, 3, 140, "timeout")?;
    let account = f.account;
    drop(f);

    let store = Store::open(&path)?;
    let ops = store.pending_ops(account)?;
    assert_eq!(ops.len(), 1);
    assert_eq!((ops[0].step, ops[0].attempts, ops[0].next_attempt_at), (1, 3, 140));
    assert_eq!(ops[0].messages.len(), 3);
    assert_eq!(store.count_pending_ops(account)?, 1);
    Ok(())
}

#[test]
fn a_sync_does_not_undo_a_queued_flag_change() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let (a, b) = (f.messages[0], f.messages[1]);
    f.store
        .queue_op(f.account, OpKind::MarkRead, f.inbox, None, &[a], 100)?;
    // The server still says both are unread; b changed elsewhere to read.
    let stale = |uid, seen| RemoteFlags {
        uid,
        flags: Flags {
            seen,
            ..Flags::default()
        },
        keywords_json: "[]".into(),
    };
    f.store
        .apply_remote_changes(f.inbox, &[], &[stale(a.uid, false), stale(b.uid, true)])?;
    assert!(seen(&f.store, a)?, "queued change kept");
    assert!(seen(&f.store, b)?, "other messages still update");
    Ok(())
}

#[test]
fn queueing_for_an_unknown_account_changes_nothing() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let first = f.messages[0];
    let result = f
        .store
        .queue_op(AccountId(999), OpKind::MarkRead, f.inbox, None, &[first], 100);
    assert!(result.is_err());
    assert!(!seen(&f.store, first)?, "rolled back with the failed insert");
    Ok(())
}
