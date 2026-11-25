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

fn apply_local(tx: &Transaction<'_>, kind: OpKind, id: MessageId) -> Result<(), StoreError> {
    let sql = match kind {
        OpKind::MarkRead => "UPDATE messages SET seen = 1 WHERE id = ?1",
        OpKind::MarkUnread => "UPDATE messages SET seen = 0 WHERE id = ?1",
        OpKind::Flag => "UPDATE messages SET flagged = 1 WHERE id = ?1",
        OpKind::Unflag => "UPDATE messages SET flagged = 0 WHERE id = ?1",
        OpKind::Move | OpKind::Expunge => "UPDATE messages SET hidden = 1 WHERE id = ?1",
    };
    tx.execute(sql, params![id.0])?;
    Ok(())
}

fn revert_local(tx: &Transaction<'_>, kind: OpKind, id: MessageId) -> Result<(), StoreError> {
    match kind.opposite() {
        Some(opposite) => apply_local(tx, opposite, id),
        None => {
            tx.execute(
                "UPDATE messages SET hidden = 0 WHERE id = ?1",
                params![id.0],
            )?;
            Ok(())
        }
    }
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
        let tx = self.conn.unchecked_transaction()?;
        let mut kept: Vec<OpMessage> = Vec::new();
        for m in messages {
            apply_local(&tx, kind, m.message_id)?;
            if let Some(opposite) = kind.opposite() {
                // The newest queued action on this message, if untried.
                let latest: Option<(i64, String)> = tx
                    .query_row(
                        "SELECT o.id, o.kind FROM pending_ops o \
                         JOIN pending_op_messages pm ON pm.op_id = o.id \
                         WHERE pm.message_id = ?1 AND o.attempts = 0 AND o.step = 0 \
                         ORDER BY o.id DESC LIMIT 1",
                        params![m.message_id.0],
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
        let id = if kept.is_empty() {
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
        };
        tx.commit()?;
        Ok(id)
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
        let tx = self.conn.unchecked_transaction()?;
        if remove_messages {
            tx.execute(
                "DELETE FROM messages WHERE hidden = 1 AND id IN \
                 (SELECT message_id FROM pending_op_messages WHERE op_id = ?1)",
                params![op.0],
            )?;
        }
        delete_op(&tx, op)?;
        tx.commit()?;
        Ok(())
    }

    /// The server refused the action for good: undoes the local change on
    /// the messages that still exist and drops the action. Returns the
    /// local ids it was applied to.
    pub fn fail_op(&self, op: OpId) -> Result<Vec<MessageId>, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let kind: Option<String> = tx
            .query_row(
                "SELECT kind FROM pending_ops WHERE id = ?1",
                params![op.0],
                |r| r.get(0),
            )
            .optional()?;
        let Some(kind) = kind else {
            return Ok(Vec::new());
        };
        let kind = OpKind::parse(&kind)?;
        let messages = load_messages(&tx, op)?;
        for m in &messages {
            revert_local(&tx, kind, m.message_id)?;
        }
        delete_op(&tx, op)?;
        tx.commit()?;
        Ok(messages.into_iter().map(|m| m.message_id).collect())
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
        let tx = self.conn.unchecked_transaction()?;
        let latest: Option<(i64, String, i64)> = tx
            .query_row(
                "SELECT o.id, o.kind, o.folder_id FROM pending_ops o \
                 JOIN pending_op_messages pm ON pm.op_id = o.id \
                 WHERE pm.message_id = ?1 ORDER BY o.id DESC LIMIT 1",
                params![message.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((op, kind, folder)) = latest else {
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
        revert_local(&tx, OpKind::parse(&kind)?, message)?;
        tx.execute(
            "DELETE FROM pending_op_messages WHERE op_id = ?1 AND message_id = ?2",
            params![op, message.0],
        )?;
        tx.execute(
            "DELETE FROM pending_ops WHERE id = ?1 AND NOT EXISTS \
             (SELECT 1 FROM pending_op_messages WHERE op_id = ?1)",
            params![op],
        )?;
        tx.commit()?;
        Ok(Some(FolderId(folder)))
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
