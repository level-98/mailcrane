//! The queue of message actions waiting to be replayed on the server (M1-14).
//!
//! Every function that changes messages does it in one transaction together
//! with the queue, so the mailbox is never half-changed.

use super::narrow;
use crate::{AccountId, FolderId, MessageId, Store, StoreError};
use rusqlite::{params, OptionalExtension, Transaction};

/// Row id of a queued action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpId(pub i64);

/// What a queued action does on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// Set `\Seen`.
    MarkRead,
    /// Clear `\Seen`.
    MarkUnread,
    /// Set `\Flagged`.
    Flag,
    /// Clear `\Flagged`.
    Unflag,
    /// Move to the op's destination folder.
    Move,
    /// Set `\Deleted` and expunge (delete for good).
    Expunge,
}

impl OpKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::MarkRead => "mark_read",
            Self::MarkUnread => "mark_unread",
            Self::Flag => "flag",
            Self::Unflag => "unflag",
            Self::Move => "move",
            Self::Expunge => "expunge",
        }
    }

    fn parse(text: &str) -> Result<Self, StoreError> {
        Ok(match text {
            "mark_read" => Self::MarkRead,
            "mark_unread" => Self::MarkUnread,
            "flag" => Self::Flag,
            "unflag" => Self::Unflag,
            "move" => Self::Move,
            "expunge" => Self::Expunge,
            _ => return Err(StoreError::Corrupt { column: "kind" }),
        })
    }

    /// The flag action that undoes this one, if it is a flag action.
    pub fn opposite(self) -> Option<Self> {
        match self {
            Self::MarkRead => Some(Self::MarkUnread),
            Self::MarkUnread => Some(Self::MarkRead),
            Self::Flag => Some(Self::Unflag),
            Self::Unflag => Some(Self::Flag),
            Self::Move | Self::Expunge => None,
        }
    }
}

/// One message an action applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpMessage {
    /// The local row (which may since have been deleted by a sync).
    pub message_id: MessageId,
    /// Its UID in the op's folder.
    pub uid: u32,
}

/// A queued action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOp {
    pub id: OpId,
    pub account_id: AccountId,
    pub kind: OpKind,
    /// The folder the messages are in.
    pub folder_id: FolderId,
    /// Where they go, for [`OpKind::Move`].
    pub dest_folder_id: Option<FolderId>,
    /// Progress through a multi-command action; 0 at the start.
    pub step: u8,
    /// Failed attempts so far.
    pub attempts: u32,
    /// Unix seconds before which it isn't tried again.
    pub next_attempt_at: i64,
    pub messages: Vec<OpMessage>,
}

/// One action for [`Store::queue_ops`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOp {
    pub account: AccountId,
    pub kind: OpKind,
    /// The folder the messages are in.
    pub folder: FolderId,
    /// Where they go, for [`OpKind::Move`].
    pub dest: Option<FolderId>,
    pub messages: Vec<OpMessage>,
}

/// What [`Store::cancel_queued_many`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CancelReport {
    /// Messages whose action was taken back.
    pub undone: u32,
    /// Messages whose action had already been tried (or had none).
    pub too_late: u32,
    /// The folders of the undone messages, each once.
    pub folders: Vec<FolderId>,
}

/// [`Store::queue_op`]'s work inside an open transaction.
fn queue_in_tx(
    tx: &Transaction<'_>,
    account: AccountId,
    kind: OpKind,
    folder: FolderId,
    dest: Option<FolderId>,
    messages: &[OpMessage],
    now: i64,
) -> Result<Option<OpId>, StoreError> {
    // Read in this transaction, before anything changes, so no wake can
    // start in between (lead's decision on G6). Only the copies the wake
    // moves count: the one in Snoozed and the one in its destination (the
    // original folder, or the Inbox). Other copies of the same Message-ID,
    // such as Gmail's All Mail or a Sent copy, can still be moved.
    // ('snoozed' is mailcrane-core's SNOOZED_FOLDER_NAME, matched the same
    // way, ignoring case.)
    for m in messages {
        let waking: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM snoozes s JOIN messages msg \
                 ON msg.message_id_header = s.message_id_header \
                 JOIN folders f ON f.id = msg.folder_id \
                 WHERE msg.id = ?1 AND s.account_id = ?2 AND f.account_id = ?2 \
                 AND s.state = 'to_wake' AND s.step > 0 \
                 AND (f.id = s.original_folder_id OR f.role = 'inbox' \
                      OR lower(f.remote_name) = 'snoozed'))",
            params![m.message_id.0, account.0],
            |r| r.get(0),
        )?;
        if waking && matches!(kind, OpKind::Move | OpKind::Expunge) {
            return Err(StoreError::Waking);
        }
    }
    let mut kept: Vec<OpMessage> = Vec::new();
    for m in messages {
        apply_local(tx, kind, folder, *m)?;
        if kind == OpKind::MarkRead {
            // Read after a wake was due: the read wins over the wake's
            // "mark unread" (#459).
            tx.execute(
                "UPDATE snoozes SET mark_unread = 0 WHERE account_id = ?2 \
                 AND state = 'to_wake' AND message_id_header = \
                 (SELECT message_id_header FROM messages WHERE id = ?1)",
                params![m.message_id.0, account.0],
            )?;
        }
        if let Some(opposite) = kind.opposite() {
            // The newest queued action on this message, if untried. Only one
            // queued for this UID of this folder: an older action may name a
            // row id that another message took since (#503).
            let latest: Option<(i64, String)> = tx
                .query_row(
                    "SELECT o.id, o.kind FROM pending_ops o \
                         JOIN pending_op_messages pm ON pm.op_id = o.id \
                         WHERE pm.message_id = ?1 AND pm.uid = ?2 AND o.folder_id = ?3 \
                         AND o.attempts = 0 AND o.step = 0 \
                         ORDER BY o.id DESC LIMIT 1",
                    params![m.message_id.0, m.uid, folder.0],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((op, latest_kind)) = latest {
                if OpKind::parse(&latest_kind)? == opposite {
                    tx.execute(
                        "DELETE FROM pending_op_messages WHERE op_id = ?1 AND message_id = ?2",
                        params![op, m.message_id.0],
                    )?;
                    tx.execute(
                        "DELETE FROM pending_ops WHERE id = ?1 AND NOT EXISTS \
                             (SELECT 1 FROM pending_op_messages WHERE op_id = ?1)",
                        params![op],
                    )?;
                    continue;
                }
            }
        }
        kept.push(*m);
    }
    Ok(if kept.is_empty() {
        None
    } else {
        tx.execute(
            "INSERT INTO pending_ops (account_id, kind, folder_id, dest_folder_id, \
                 next_attempt_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![account.0, kind.as_str(), folder.0, dest.map(|d| d.0), now],
        )?;
        let op = tx.last_insert_rowid();
        let mut insert = tx.prepare(
            "INSERT OR IGNORE INTO pending_op_messages (op_id, message_id, uid) \
                 VALUES (?1, ?2, ?3)",
        )?;
        for m in &kept {
            insert.execute(params![op, m.message_id.0, m.uid])?;
        }
        Some(OpId(op))
    })
}

/// [`Store::cancel_queued`]'s work inside an open transaction.
fn cancel_in_tx(tx: &Transaction<'_>, message: MessageId) -> Result<Option<FolderId>, StoreError> {
    // Only an action queued for the message the row holds now: row ids are
    // reused, so an older action may name an id that is another message
    // now (#503).
    let latest: Option<(i64, String, i64, u32)> = tx
        .query_row(
            "SELECT o.id, o.kind, o.folder_id, pm.uid FROM pending_ops o \
                 JOIN pending_op_messages pm ON pm.op_id = o.id \
                 JOIN messages m ON m.id = pm.message_id \
                   AND m.folder_id = o.folder_id AND m.uid = pm.uid \
                 WHERE pm.message_id = ?1 ORDER BY o.id DESC LIMIT 1",
            params![message.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((op, kind, folder, uid)) = latest else {
        return Ok(None);
    };
    let untried: bool = tx.query_row(
        "SELECT attempts = 0 AND step = 0 FROM pending_ops WHERE id = ?1",
        params![op],
        |r| r.get(0),
    )?;
    if !untried {
        return Ok(None);
    }
    let target = OpMessage {
        message_id: message,
        uid,
    };
    revert_local(tx, OpKind::parse(&kind)?, FolderId(folder), target)?;
    tx.execute(
        "DELETE FROM pending_op_messages WHERE op_id = ?1 AND message_id = ?2",
        params![op, message.0],
    )?;
    tx.execute(
        "DELETE FROM pending_ops WHERE id = ?1 AND NOT EXISTS \
             (SELECT 1 FROM pending_op_messages WHERE op_id = ?1)",
        params![op],
    )?;
    Ok(Some(FolderId(folder)))
}

/// The rows a queued action may change: `m`'s row only while it is still
/// that UID of the action's folder. Row ids are reused once a row is
/// deleted, so an undo or a revert that runs later could otherwise change
/// another message that took the id (#503).
const SAME_MESSAGE: &str = "WHERE id = ?1 AND folder_id = ?2 AND uid = ?3";

/// Applies `kind` to `m`'s row. Returns whether the row still holds `m`.
fn apply_local(
    tx: &Transaction<'_>,
    kind: OpKind,
    folder: FolderId,
    m: OpMessage,
) -> Result<bool, StoreError> {
    let set = match kind {
        OpKind::MarkRead => "seen = 1",
        OpKind::MarkUnread => "seen = 0",
        OpKind::Flag => "flagged = 1",
        OpKind::Unflag => "flagged = 0",
        OpKind::Move | OpKind::Expunge => "hidden = 1",
    };
    let n = tx.execute(
        &format!("UPDATE messages SET {set} {SAME_MESSAGE}"),
        params![m.message_id.0, folder.0, m.uid],
    )?;
    Ok(n > 0)
}

/// Undoes `kind` on `m`'s row. Returns whether the row still holds `m`.
fn revert_local(
    tx: &Transaction<'_>,
    kind: OpKind,
    folder: FolderId,
    m: OpMessage,
) -> Result<bool, StoreError> {
    match kind.opposite() {
        Some(opposite) => apply_local(tx, opposite, folder, m),
        None => {
            let n = tx.execute(
                &format!("UPDATE messages SET hidden = 0 {SAME_MESSAGE}"),
                params![m.message_id.0, folder.0, m.uid],
            )?;
            Ok(n > 0)
        }
    }
}

/// The folder an action's messages are in, if the action is still queued.
fn op_folder(tx: &Transaction<'_>, op: OpId) -> Result<Option<(OpKind, FolderId)>, StoreError> {
    let row: Option<(String, i64)> = tx
        .query_row(
            "SELECT kind, folder_id FROM pending_ops WHERE id = ?1",
            params![op.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(kind, folder)| Ok((OpKind::parse(&kind)?, FolderId(folder))))
        .transpose()
}

/// A queued action dropped because its folder was refreshed from the server
/// (#517, #541): its `UIDVALIDITY` changed, or a JMAP server couldn't say
/// what changed. Its UIDs or rows may name other messages now, so it can't
/// be applied as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedOp {
    /// What the action was.
    pub kind: OpKind,
    /// The local rows it was applied to (gone with the folder's reset).
    pub message_ids: Vec<MessageId>,
    /// How far a multi-command action had got on the server (see
    /// [`PendingOp::step`]); 0 if it hadn't started.
    pub step: u8,
    /// Each message's `Message-ID` (without angle brackets), in the order of
    /// [`DroppedOp::message_ids`], to find it again after the reset (#541).
    pub message_id_headers: Vec<Option<String>>,
}

impl DroppedOp {
    /// Whether the action was partly carried out on the server (#541): a
    /// move without `MOVE` that already copied, or a move or expunge that
    /// already set `\Deleted`. Dropping it would leave a copy in both
    /// folders, or a message marked deleted.
    pub fn half_done(&self) -> bool {
        self.step > 0 && matches!(self.kind, OpKind::Move | OpKind::Expunge)
    }
}

/// Removes every queued action on messages of `folder`, inside `tx`, and
/// returns them (#517). Actions moving mail *into* the folder are kept:
/// they're keyed by their own folder's UIDs.
pub(crate) fn take_folder_ops(
    tx: &Transaction<'_>,
    folder: FolderId,
) -> Result<Vec<DroppedOp>, StoreError> {
    let ops: Vec<(i64, String, i64)> = {
        let mut stmt =
            tx.prepare("SELECT id, kind, step FROM pending_ops WHERE folder_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![folder.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut dropped = Vec::with_capacity(ops.len());
    for (id, kind, step) in ops {
        let kind = OpKind::parse(&kind)?;
        let messages = load_messages(tx, OpId(id))?;
        let mut message_id_headers = Vec::with_capacity(messages.len());
        for m in &messages {
            // Only from the row that is still that message (#503).
            let header: Option<String> = tx
                .query_row(
                    &format!("SELECT message_id_header FROM messages {SAME_MESSAGE}"),
                    params![m.message_id.0, folder.0, m.uid],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            message_id_headers.push(header.filter(|h| !h.is_empty()));
        }
        delete_op(tx, OpId(id))?;
        dropped.push(DroppedOp {
            kind,
            message_ids: messages.iter().map(|m| m.message_id).collect(),
            step: narrow(step, "step")?,
            message_id_headers,
        });
    }
    Ok(dropped)
}

fn load_messages(tx: &rusqlite::Connection, op: OpId) -> Result<Vec<OpMessage>, StoreError> {
    let mut stmt = tx
        .prepare("SELECT message_id, uid FROM pending_op_messages WHERE op_id = ?1 ORDER BY uid")?;
    let mut rows = stmt.query(params![op.0])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(OpMessage {
            message_id: MessageId(r.get(0)?),
            uid: narrow(r.get(1)?, "uid")?,
        });
    }
    Ok(out)
}

fn delete_op(tx: &Transaction<'_>, op: OpId) -> Result<(), StoreError> {
    tx.execute("DELETE FROM pending_ops WHERE id = ?1", params![op.0])?;
    Ok(())
}

impl Store {
    /// Applies `kind` locally to `messages` (all in `folder`) and queues it,
    /// in one transaction. Flag actions set the flag; move and expunge hide
    /// the messages. A flag action on a message whose opposite is still
    /// queued and untried cancels that one instead of queueing anything for
    /// the message. Returns the new op, or `None` if every message cancelled
    /// out. On error nothing is changed.
    pub fn queue_op(
        &self,
        account: AccountId,
        kind: OpKind,
        folder: FolderId,
        dest: Option<FolderId>,
        messages: &[OpMessage],
        now: i64,
    ) -> Result<Option<OpId>, StoreError> {
        let tx = self.write_tx()?;
        let id = queue_in_tx(&tx, account, kind, folder, dest, messages, now)?;
        tx.commit()?;
        Ok(id)
    }

    /// Several [`Store::queue_op`]s (e.g. one per folder for a multi-select
    /// action, M4-14) in one transaction: all are applied and queued, or on
    /// error none is. Returns each op in order, `None` where every message
    /// cancelled out.
    pub fn queue_ops(&self, ops: &[NewOp], now: i64) -> Result<Vec<Option<OpId>>, StoreError> {
        let tx = self.write_tx()?;
        let mut ids = Vec::with_capacity(ops.len());
        for op in ops {
            ids.push(queue_in_tx(
                &tx,
                op.account,
                op.kind,
                op.folder,
                op.dest,
                &op.messages,
                now,
            )?);
        }
        tx.commit()?;
        Ok(ids)
    }

    /// The account's queued actions in the order they were made, including
    /// ones not due yet (the caller replays in order and stops at the first
    /// that isn't due, so later actions never overtake earlier ones).
    pub fn pending_ops(&self, account: AccountId) -> Result<Vec<PendingOp>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, folder_id, dest_folder_id, step, attempts, next_attempt_at \
             FROM pending_ops WHERE account_id = ?1 ORDER BY id",
        )?;
        let mut rows = stmt.query(params![account.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let id = OpId(r.get(0)?);
            let dest: Option<i64> = r.get(3)?;
            out.push(PendingOp {
                id,
                account_id: account,
                kind: OpKind::parse(&r.get::<_, String>(1)?)?,
                folder_id: FolderId(r.get(2)?),
                dest_folder_id: dest.map(FolderId),
                step: narrow(r.get(4)?, "step")?,
                attempts: narrow(r.get(5)?, "attempts")?,
                next_attempt_at: r.get(6)?,
                messages: Vec::new(),
            });
        }
        drop(rows);
        drop(stmt);
        for op in &mut out {
            op.messages = load_messages(&self.conn, op.id)?;
        }
        Ok(out)
    }

    /// The action succeeded on the server: drops it, and with
    /// `remove_messages` also deletes the hidden local rows (a move or
    /// expunge that is now done; the destination's next sync brings moved
    /// messages back with their new UIDs).
    pub fn complete_op(&self, op: OpId, remove_messages: bool) -> Result<(), StoreError> {
        let tx = self.write_tx()?;
        if remove_messages {
            // Only rows that are still those messages (#503).
            tx.execute(
                "DELETE FROM messages WHERE hidden = 1 AND EXISTS \
                 (SELECT 1 FROM pending_op_messages pm JOIN pending_ops o ON o.id = pm.op_id \
                  WHERE pm.op_id = ?1 AND pm.message_id = messages.id \
                    AND pm.uid = messages.uid AND o.folder_id = messages.folder_id)",
                params![op.0],
            )?;
        }
        delete_op(&tx, op)?;
        tx.commit()?;
        Ok(())
    }

    /// The server refused the action for good: undoes the local change on
    /// the messages that still exist and drops the action. Returns the
    /// local ids it undid: not a row that holds another message now (#503).
    pub fn fail_op(&self, op: OpId) -> Result<Vec<MessageId>, StoreError> {
        let tx = self.write_tx()?;
        let Some((kind, folder)) = op_folder(&tx, op)? else {
            return Ok(Vec::new());
        };
        let mut reverted = Vec::new();
        for m in load_messages(&tx, op)? {
            if revert_local(&tx, kind, folder, m)? {
                reverted.push(m.message_id);
            }
        }
        delete_op(&tx, op)?;
        tx.commit()?;
        Ok(reverted)
    }

    /// The server refused the action for some of its messages only (a JMAP
    /// `Email/set` can refuse ids one by one): undoes the local change on
    /// those messages and takes them out of the action, so the rest can be
    /// completed with [`Store::complete_op`]. Returns the ids it undid
    /// (not a row that holds another message now, #503).
    pub fn fail_op_messages(
        &self,
        op: OpId,
        refused: &[MessageId],
    ) -> Result<Vec<MessageId>, StoreError> {
        let tx = self.write_tx()?;
        let Some((kind, folder)) = op_folder(&tx, op)? else {
            return Ok(Vec::new());
        };
        let mut undone = Vec::new();
        for m in load_messages(&tx, op)? {
            if refused.contains(&m.message_id) {
                let still_there = revert_local(&tx, kind, folder, m)?;
                tx.execute(
                    "DELETE FROM pending_op_messages WHERE op_id = ?1 AND message_id = ?2",
                    params![op.0, m.message_id.0],
                )?;
                if still_there {
                    undone.push(m.message_id);
                }
            }
        }
        tx.commit()?;
        Ok(undone)
    }

    /// Takes `messages` out of the action without touching their rows (#541):
    /// a JMAP replay found their rows hold other emails now (row ids are
    /// reused), so neither completing nor failing the action may change
    /// them. Returns the ids it took out.
    pub fn drop_op_messages(
        &self,
        op: OpId,
        messages: &[MessageId],
    ) -> Result<Vec<MessageId>, StoreError> {
        let tx = self.write_tx()?;
        let mut dropped = Vec::new();
        for m in messages {
            let n = tx.execute(
                "DELETE FROM pending_op_messages WHERE op_id = ?1 AND message_id = ?2",
                params![op.0, m.0],
            )?;
            if n > 0 {
                dropped.push(*m);
            }
        }
        tx.commit()?;
        Ok(dropped)
    }

    /// The action failed for now: records the attempt and when to try next.
    /// `error` is a short code such as `network`, never server text.
    pub fn defer_op(
        &self,
        op: OpId,
        attempts: u32,
        next_attempt_at: i64,
        error: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE pending_ops SET attempts = ?2, next_attempt_at = ?3, last_error = ?4 \
             WHERE id = ?1",
            params![op.0, attempts, next_attempt_at, error],
        )?;
        Ok(())
    }

    /// Records that a multi-command action got past `step`, so a replay
    /// continues from there.
    pub fn set_op_step(&self, op: OpId, step: u8) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE pending_ops SET step = ?2 WHERE id = ?1",
            params![op.0, step],
        )?;
        Ok(())
    }

    /// Undo (M4-04): takes `message` out of its newest queued action if
    /// that action hasn't been tried yet, and undoes its local change, in
    /// one transaction. Returns the folder the message is in, or `None` if
    /// there was nothing to undo (already sent to the server, or no action).
    pub fn cancel_queued(&self, message: MessageId) -> Result<Option<FolderId>, StoreError> {
        let tx = self.write_tx()?;
        let folder = cancel_in_tx(&tx, message)?;
        tx.commit()?;
        Ok(folder)
    }

    /// Undo for a multi-select action (M4-14): [`Store::cancel_queued`] for
    /// each of `messages`, in one transaction. Messages whose action has
    /// already been tried stay as they are and count as too late.
    pub fn cancel_queued_many(&self, messages: &[MessageId]) -> Result<CancelReport, StoreError> {
        let tx = self.write_tx()?;
        let mut report = CancelReport::default();
        for message in messages {
            match cancel_in_tx(&tx, *message)? {
                Some(folder) => {
                    report.undone = report.undone.saturating_add(1);
                    if !report.folders.contains(&folder) {
                        report.folders.push(folder);
                    }
                }
                None => report.too_late = report.too_late.saturating_add(1),
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// How many actions are queued for the account.
    pub fn count_pending_ops(&self, account: AccountId) -> Result<u64, StoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM pending_ops WHERE account_id = ?1",
            params![account.0],
            |r| r.get(0),
        )?;
        narrow(n, "count")
    }
}
