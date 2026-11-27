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
        selectable: true,
        virtual_role: None,
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
    assert_eq!(
        (ops[0].step, ops[0].attempts, ops[0].next_attempt_at),
        (1, 3, 140)
    );
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
    let result = f.store.queue_op(
        AccountId(999),
        OpKind::MarkRead,
        f.inbox,
        None,
        &[first],
        100,
    );
    assert!(result.is_err());
    assert!(
        !seen(&f.store, first)?,
        "rolled back with the failed insert"
    );
    Ok(())
}

#[test]
fn an_untried_action_can_be_undone() -> R {
    let f = fixture(Store::open_in_memory()?)?;
    let (a, b) = (f.messages[0], f.messages[1]);
    f.store.queue_op(
        f.account,
        OpKind::Move,
        f.inbox,
        Some(f.archive),
        &[a, b],
        100,
    )?;
    assert_eq!(f.store.cancel_queued(a.message_id)?, Some(f.inbox));
    // a is back; b is still on its way.
    assert_eq!(f.store.list_messages(f.inbox, 10, 0)?.len(), 2);
    let ops = f.store.pending_ops(f.account)?;
    assert_eq!(ops[0].messages, vec![b]);
    assert_eq!(
        f.store.cancel_queued(a.message_id)?,
        None,
        "nothing left to undo"
    );

    // Once tried, it may have reached the server: too late to undo here.
    f.store.defer_op(ops[0].id, 1, 200, "network")?;
    assert_eq!(f.store.cancel_queued(b.message_id)?, None);
    Ok(())
}

#[test]
fn a_folder_reset_drops_only_that_folders_queued_actions() -> R {
    // #517: the reset folder's UIDs name other messages now; other folders'
    // actions, and actions moving mail into the reset folder, are kept.
    let f = fixture(Store::open_in_memory()?)?;
    let in_archive = f.store.upsert_message_header(&header(f.archive, 1))?;
    let flagged = f.messages[0];
    f.store
        .queue_op(f.account, OpKind::Flag, f.inbox, None, &[flagged], 100)?
        .ok_or("queued")?;
    let back = OpMessage {
        message_id: in_archive,
        uid: 1,
    };
    f.store
        .queue_op(
            f.account,
            OpKind::Move,
            f.archive,
            Some(f.inbox),
            &[back],
            100,
        )?
        .ok_or("queued")?;
    let dropped = f
        .store
        .reset_folder_reporting(f.inbox, 2, 1)?
        .ok_or("the folder exists")?;
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].kind, OpKind::Flag);
    assert_eq!(dropped[0].message_ids, [flagged.message_id]);
    let left = f.store.pending_ops(f.account)?;
    assert_eq!(left.len(), 1, "the move from Archive into the inbox stays");
    assert_eq!(left[0].folder_id, f.archive);
    // An unknown folder: nothing changes.
    assert!(f
        .store
        .reset_folder_reporting(FolderId(999), 1, 1)?
        .is_none());
    Ok(())
}

#[test]
fn a_late_revert_or_undo_leaves_a_row_that_took_the_id_alone() -> R {
    // #503: row ids are reused. A flag queued for UID 3; the sync deletes
    // it and stores UID 9, which gets the same row id and is flagged.
    let f = fixture(Store::open_in_memory()?)?;
    let third = f.messages[2];
    let op = f
        .store
        .queue_op(f.account, OpKind::Flag, f.inbox, None, &[third], 100)?
        .ok_or("queued")?;
    f.store.delete_message_row(third.message_id)?;
    let mut newer = header(f.inbox, 9);
    newer.flags.flagged = true;
    let reused = f.store.upsert_message_header(&newer)?;
    assert_eq!(reused, third.message_id, "the id was reused");
    let flagged = |store: &Store| -> Result<bool, StoreError> {
        Ok(store
            .get_message_header(reused)?
            .is_some_and(|h| h.details.flags.flagged))
    };

    // Undo finds no action for the message the row holds now.
    assert_eq!(f.store.cancel_queued(reused)?, None);
    assert!(flagged(&f.store)?);
    // Nor does the server refusing the flag unflag UID 9, or name it.
    assert!(f.store.fail_op(op)?.is_empty());
    assert!(flagged(&f.store)?);
    Ok(())
}

/// Queues `kind` for UID 3, then deletes its row as a sync does and stores
/// UID 9, which takes the same row id. Returns the action, UID 9's row and
/// UID 9 as the message to queue for (#503).
fn reused_after(
    f: &Fixture,
    kind: OpKind,
    flagged: bool,
) -> Result<(OpId, OpMessage), Box<dyn std::error::Error>> {
    let third = f.messages[2];
    let dest = (kind == OpKind::Move).then_some(f.archive);
    let op = f
        .store
        .queue_op(f.account, kind, f.inbox, dest, &[third], 100)?
        .ok_or("queued")?;
    f.store.delete_message_row(third.message_id)?;
    let mut newer = header(f.inbox, 9);
    newer.flags.flagged = flagged;
    let reused = f.store.upsert_message_header(&newer)?;
    assert_eq!(reused, third.message_id, "the id was reused");
    Ok((
        op,
        OpMessage {
            message_id: reused,
            uid: 9,
        },
    ))
}

#[test]
fn an_opposite_action_on_a_reused_row_does_not_cancel_the_old_one() -> R {
    // #503 review: an Unflag for UID 9 isn't the opposite of the Flag
    // queued for UID 3, whatever row id they share.
    let f = fixture(Store::open_in_memory()?)?;
    let (_, nine) = reused_after(&f, OpKind::Flag, true)?;
    let queued = f
        .store
        .queue_op(f.account, OpKind::Unflag, f.inbox, None, &[nine], 101)?;
    assert!(queued.is_some(), "the unflag is queued");
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
fn a_completed_move_never_deletes_a_row_that_took_the_id() -> R {
    // #503 review: UID 9 took the row id and is hidden by its own pending
    // action; completing the move of UID 3 must leave it.
    let f = fixture(Store::open_in_memory()?)?;
    let (op, nine) = reused_after(&f, OpKind::Move, false)?;
    f.store.set_message_hidden(nine.message_id, true)?;
    f.store.complete_op(op, true)?;
    assert!(f.store.get_message_header(nine.message_id)?.is_some());
    Ok(())
}

#[test]
fn a_partly_refused_action_leaves_a_row_that_took_the_id_alone() -> R {
    // #503 review: fail_op_messages reverts only the message it queued.
    let f = fixture(Store::open_in_memory()?)?;
    let (op, nine) = reused_after(&f, OpKind::Flag, true)?;
    assert!(f.store.fail_op_messages(op, &[nine.message_id])?.is_empty());
    assert!(f
        .store
        .get_message_header(nine.message_id)?
        .is_some_and(|h| h.details.flags.flagged));
    Ok(())
}
