//! Carrying queued actions out on the server.

use crate::events::{ActionKind, CoreEvent, EventListener, SyncErrorKind};
use crate::sync::SyncError;
use mailcrane_imap::{ChangeError, ImapError, Session, SystemFlag};
use mailcrane_store::{AccountId, DroppedOp, FolderId, OpKind, PendingOp, Store};

/// The first retry waits this long.
const FIRST_RETRY_SECS: i64 = 5;
/// Retries never wait longer than this.
const MAX_RETRY_SECS: i64 = 600;

/// Seconds to wait before the next try after `attempts` failed ones: 5 s,
/// doubling each time, at most 10 minutes.
pub fn backoff_secs(attempts: u32) -> i64 {
    let doublings = attempts.saturating_sub(1).min(16);
    FIRST_RETRY_SECS
        .saturating_mul(1_i64 << doublings)
        .min(MAX_RETRY_SECS)
}

/// What one [`replay`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    /// Actions now done on the server.
    pub done: usize,
    /// Actions the server refused for good (their local changes were
    /// undone), or dropped because the server renumbered their folder.
    pub failed: usize,
    /// Actions still queued (not due yet, or deferred after a failure).
    pub remaining: usize,
}

/// How carrying out one action ended.
enum Outcome {
    /// Done. `expunged` says whether the messages are gone from their
    /// folder on the server, so their hidden local rows can go too.
    Done { expunged: bool },
    /// The server renumbered the folder (RFC 9051 §2.3.1.1): nothing was
    /// applied, and the folder must be reset with these values.
    Renumbered { uidvalidity: u32, uidnext: u32 },
}

/// Replays the account's queued actions on `session`, oldest first, until
/// one fails for now or isn't due yet (later actions never overtake earlier
/// ones). Leaves no folder selected that the caller relies on: select again
/// afterwards. Errors only for a local database failure.
pub async fn replay(
    store: &Store,
    session: &mut Session,
    listener: &dyn EventListener,
    account: AccountId,
    now: i64,
) -> Result<ReplayReport, SyncError> {
    let ops = store.pending_ops(account)?;
    let mut report = ReplayReport::default();
    // Folders reset during this replay: their queued actions are gone.
    let mut reset: Vec<FolderId> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        if reset.contains(&op.folder_id) {
            continue;
        }
        if op.next_attempt_at > now {
            report.remaining = ops.len().saturating_sub(index);
            break;
        }
        match run(store, session, op).await {
            Ok(Outcome::Done { expunged }) => {
                store.complete_op(op.id, expunged)?;
                report.done = report.done.saturating_add(1);
            }
            Ok(Outcome::Renumbered {
                uidvalidity,
                uidnext,
            }) => {
                // Reset the folder here and go on: waiting for its own sync
                // would hold up every later action of the account (#517).
                let dropped = store
                    .reset_folder_reporting(
                        op.folder_id,
                        i64::from(uidvalidity),
                        i64::from(uidnext),
                    )?
                    .unwrap_or_default();
                reset.push(op.folder_id);
                report.failed = report.failed.saturating_add(dropped.len());
                report_folder_reset(listener, account, op.folder_id, dropped);
            }
            Err(error) if error.is_transient() => {
                let attempts = op.attempts.saturating_add(1);
                let next = now.saturating_add(backoff_secs(attempts));
                store.defer_op(op.id, attempts, next, error_code(&error))?;
                tracing::info!(attempts, "message action deferred");
                report.remaining = ops.len().saturating_sub(index);
                break;
            }
            Err(error) => {
                let reverted = store.fail_op(op.id)?;
                tracing::warn!(kind = ?op.kind, "message action refused by the server; undone");
                report.failed = report.failed.saturating_add(1);
                listener.on_event(CoreEvent::MessagesChanged {
                    folder_id: op.folder_id,
                });
                listener.on_event(CoreEvent::ActionFailed {
                    account_id: account,
                    action: action_kind(op.kind),
                    message_ids: reverted,
                    reason: reason(&error),
                });
            }
        }
    }
    Ok(report)
}

/// Tells the app that `folder` was emptied because the server renumbered
/// it, and that the changes queued for it were dropped: their UIDs name
/// other messages now (#517).
pub(crate) fn report_folder_reset(
    listener: &dyn EventListener,
    account: AccountId,
    folder: FolderId,
    dropped: Vec<DroppedOp>,
) {
    listener.on_event(CoreEvent::FolderChanged { folder_id: folder });
    for op in dropped {
        tracing::warn!(
            folder = folder.0,
            kind = ?op.kind,
            "queued action dropped: the server reset the folder"
        );
        listener.on_event(CoreEvent::ActionFailed {
            account_id: account,
            action: action_kind(op.kind),
            message_ids: op.message_ids,
            reason: SyncErrorKind::FolderReset,
        });
    }
}

pub(crate) fn action_kind(kind: OpKind) -> ActionKind {
    match kind {
        OpKind::MarkRead => ActionKind::MarkRead,
        OpKind::MarkUnread => ActionKind::MarkUnread,
        OpKind::Flag => ActionKind::Flag,
        OpKind::Unflag => ActionKind::Unflag,
        OpKind::Move => ActionKind::Move,
        OpKind::Expunge => ActionKind::Delete,
    }
}

fn reason(error: &ChangeError) -> SyncErrorKind {
    match error {
        // A refused SELECT means the folder is gone.
        ChangeError::Imap(ImapError::CommandFailed) => SyncErrorKind::NotFound,
        ChangeError::Refused { .. } => SyncErrorKind::Protocol,
        ChangeError::Imap(e) => SyncError::Imap(clone_kind(e)).kind(),
    }
}

/// `ImapError` isn't `Clone`; rebuild the variants whose kind matters.
fn clone_kind(e: &ImapError) -> ImapError {
    match e {
        ImapError::CannotConnect => ImapError::CannotConnect,
        ImapError::ConnectionClosed => ImapError::ConnectionClosed,
        ImapError::ServerBye => ImapError::ServerBye,
        ImapError::TlsFailure => ImapError::TlsFailure,
        ImapError::AuthFailed => ImapError::AuthFailed,
        ImapError::Timeout => ImapError::Timeout,
        _ => ImapError::Protocol,
    }
}

/// A short code for `last_error`; never server text.
fn error_code(error: &ChangeError) -> &'static str {
    match error {
        ChangeError::Refused { .. } => "refused",
        ChangeError::Imap(ImapError::Timeout) => "timeout",
        ChangeError::Imap(
            ImapError::CannotConnect | ImapError::ConnectionClosed | ImapError::ServerBye,
        ) => "network",
        ChangeError::Imap(_) => "protocol",
    }
}

/// Progress steps of a move without `MOVE` and of an expunge.
const STEP_COPIED: u8 = 1;
const STEP_FLAGGED_DELETED: u8 = 2;

async fn run(store: &Store, session: &mut Session, op: &PendingOp) -> Result<Outcome, ChangeError> {
    let folder = store
        .get_folder(op.folder_id)
        .map_err(|e| ChangeError::Imap(ImapError::Store(e)))?
        // The folder was removed locally (and so were its messages).
        .ok_or(ChangeError::Refused { transient: false })?;
    let uids: Vec<u32> = op.messages.iter().map(|m| m.uid).collect();
    let info = session.select(&folder.details.remote_name).await?;
    // The server renumbered the folder (RFC 9051 §2.3.1.1): the stored UIDs
    // name other messages now, so apply nothing (#517).
    if folder
        .details
        .uidvalidity
        .is_some_and(|v| v != i64::from(info.uidvalidity))
    {
        return Ok(Outcome::Renumbered {
            uidvalidity: info.uidvalidity,
            uidnext: info.uidnext,
        });
    }
    match op.kind {
        OpKind::MarkRead | OpKind::MarkUnread | OpKind::Flag | OpKind::Unflag => {
            let (flag, add) = match op.kind {
                OpKind::MarkRead => (SystemFlag::Seen, true),
                OpKind::MarkUnread => (SystemFlag::Seen, false),
                OpKind::Flag => (SystemFlag::Flagged, true),
                _ => (SystemFlag::Flagged, false),
            };
            session.uid_store_flag(&uids, flag, add).await?;
            Ok(Outcome::Done { expunged: false })
        }
        OpKind::Move => {
            let dest = op
                .dest_folder_id
                .and_then(|id| store.get_folder(id).ok().flatten())
                .ok_or(ChangeError::Refused { transient: false })?;
            let to = dest.details.remote_name;
            if session.supports_move() {
                // RFC 6851: atomic, and repeating it after a dropped
                // connection moves nothing twice.
                session.uid_move(&uids, &to).await?;
                return Ok(Outcome::Done { expunged: true });
            }
            // RFC 6851 §3.3 fallback: COPY, STORE \Deleted, UID EXPUNGE. The
            // step is saved after each command, so a replay never copies
            // twice.
            if op.step < STEP_COPIED {
                session.uid_copy(&uids, &to).await?;
                set_step(store, op, STEP_COPIED)?;
            }
            delete_for_good(store, session, op, &uids).await
        }
        OpKind::Expunge => delete_for_good(store, session, op, &uids).await,
    }
}

/// `STORE +FLAGS (\Deleted)`, then `UID EXPUNGE` of just these UIDs when the
/// server has it. Without it the messages stay flagged `\Deleted` (and
/// hidden locally): a plain `EXPUNGE` would also remove messages other
/// clients marked `\Deleted`.
async fn delete_for_good(
    store: &Store,
    session: &mut Session,
    op: &PendingOp,
    uids: &[u32],
) -> Result<Outcome, ChangeError> {
    if op.step < STEP_FLAGGED_DELETED {
        session
            .uid_store_flag(uids, SystemFlag::Deleted, true)
            .await?;
        set_step(store, op, STEP_FLAGGED_DELETED)?;
    }
    if session.supports_uid_expunge() {
        session.uid_expunge(uids).await?;
        Ok(Outcome::Done { expunged: true })
    } else {
        Ok(Outcome::Done { expunged: false })
    }
}

fn set_step(store: &Store, op: &PendingOp, step: u8) -> Result<(), ChangeError> {
    store
        .set_op_step(op.id, step)
        .map_err(|e| ChangeError::Imap(ImapError::Store(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_five_seconds_up_to_ten_minutes() {
        let waits: Vec<i64> = (1..=9).map(backoff_secs).collect();
        assert_eq!(waits, [5, 10, 20, 40, 80, 160, 320, 600, 600]);
        assert_eq!(backoff_secs(0), 5);
        assert_eq!(backoff_secs(u32::MAX), 600);
    }
}
