//! Message actions (M1-14): read, unread, flag, unflag, archive, delete and
//! move.
//!
//! An action changes the local store at once (and tells the apps), and is
//! queued in the same transaction. [`replay`] later carries the queue out on
//! the server, in order, retrying with back-off; if the server refuses an
//! action for good, the local change is undone and
//! [`CoreEvent::ActionFailed`] is sent.

mod replay;

pub use replay::{backoff_secs, replay, ReplayReport};

use crate::events::{CoreEvent, EventListener};
use mailcrane_store::{
    AccountId, FolderId, FolderRole, MessageId, OpKind, OpMessage, Store, StoreError,
};

/// What the user asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Mark as read (`\Seen`).
    MarkRead,
    /// Mark as unread.
    MarkUnread,
    /// Flag (star) it (`\Flagged`).
    Flag,
    /// Remove the flag.
    Unflag,
    /// Move to the account's Archive folder.
    Archive,
    /// Move to Trash, or delete for good if already in Trash or there is no
    /// Trash folder.
    Delete,
    /// Move to another folder of the same account.
    Move {
        /// The destination.
        to: FolderId,
    },
}

/// Why an action could not be applied. Nothing was changed.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// One of the messages is not in the database.
    #[error("unknown message")]
    UnknownMessage,
    /// The destination folder is not in the database, or belongs to another
    /// account.
    #[error("unknown folder")]
    UnknownFolder,
    /// The account has no Archive folder, so there is nowhere to archive to.
    #[error("the account has no Archive folder")]
    NoArchiveFolder,
    /// The messages are in different folders. Act on one folder at a time.
    #[error("the messages are in different folders")]
    MixedFolders,
    /// The local database failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// What [`apply_action`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The messages' account, whose queue should be replayed.
    pub account: AccountId,
    /// Whether anything was queued (false if nothing needed to change, or
    /// the action cancelled out an earlier one).
    pub queued: bool,
}

/// Applies `action` to `messages`, which must all be in one folder: changes
/// the store, queues the action for the server and sends `MessagesChanged`.
/// Messages already in the wanted state are skipped. On error nothing is
/// changed.
pub fn apply_action(
    store: &Store,
    messages: &[MessageId],
    action: Action,
    now: i64,
    listener: &dyn EventListener,
) -> Result<Option<Applied>, ActionError> {
    let mut headers = Vec::with_capacity(messages.len());
    for id in messages {
        let header = store
            .get_message_header(*id)?
            .ok_or(ActionError::UnknownMessage)?;
        headers.push(header);
    }
    let Some(first) = headers.first() else {
        return Ok(None);
    };
    let folder_id = first.details.folder_id;
    if headers.iter().any(|h| h.details.folder_id != folder_id) {
        return Err(ActionError::MixedFolders);
    }
    let folder = store
        .get_folder(folder_id)?
        .ok_or(ActionError::UnknownFolder)?;
    let account = folder.details.account_id;
    let role_folder = |role: FolderRole| -> Result<Option<FolderId>, ActionError> {
        Ok(store
            .list_folders(account)?
            .into_iter()
            .find(|f| f.details.role == Some(role))
            .map(|f| f.id))
    };

    let (kind, dest) = match action {
        Action::MarkRead => (OpKind::MarkRead, None),
        Action::MarkUnread => (OpKind::MarkUnread, None),
        Action::Flag => (OpKind::Flag, None),
        Action::Unflag => (OpKind::Unflag, None),
        Action::Archive => {
            let archive = role_folder(FolderRole::Archive)?.ok_or(ActionError::NoArchiveFolder)?;
            (OpKind::Move, Some(archive))
        }
        Action::Delete => match role_folder(FolderRole::Trash)? {
            Some(trash) if trash != folder_id => (OpKind::Move, Some(trash)),
            // Already in Trash, or no Trash: delete for good.
            _ => (OpKind::Expunge, None),
        },
        Action::Move { to } => {
            let target = store.get_folder(to)?.ok_or(ActionError::UnknownFolder)?;
            if target.details.account_id != account {
                return Err(ActionError::UnknownFolder);
            }
            (OpKind::Move, Some(to))
        }
    };

    let needed: Vec<OpMessage> = headers
        .iter()
        .filter(|h| {
            let f = h.details.flags;
            match kind {
                OpKind::MarkRead => !f.seen,
                OpKind::MarkUnread => f.seen,
                OpKind::Flag => !f.flagged,
                OpKind::Unflag => f.flagged,
                // Moving a message into the folder it is in does nothing.
                OpKind::Move => dest != Some(folder_id),
                OpKind::Expunge => true,
            }
        })
        .map(|h| OpMessage {
            message_id: h.id,
            uid: h.details.uid,
        })
        .collect();
    if needed.is_empty() {
        return Ok(Some(Applied {
            account,
            queued: false,
        }));
    }
    let op = store.queue_op(account, kind, folder_id, dest, &needed, now)?;
    listener.on_event(CoreEvent::MessagesChanged { folder_id });
    tracing::debug!(?kind, count = needed.len(), queued = op.is_some(), "message action applied");
    Ok(Some(Applied {
        account,
        queued: op.is_some(),
    }))
}

#[cfg(test)]
mod tests;
