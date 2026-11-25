//! Message actions for the apps (M1-14): read, unread, flag, unflag,
//! archive, delete and move. Applied locally at once; the queue is replayed
//! on the server by [`SyncEngine::replay_actions`] and before every folder
//! sync.

use crate::sync::{CoreEvent, ListenerAdapter, SyncEngine, SyncErrorKind, SyncFailure};
use mailcrane_core::actions::{self, Action, ActionError};
use mailcrane_core::events;
use mailcrane_store::{AccountId, FolderId, MessageId};

/// What the user asked to do with a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MessageAction {
    /// Mark as read.
    MarkRead,
    /// Mark as unread.
    MarkUnread,
    /// Flag (star).
    Flag,
    /// Remove the flag.
    Unflag,
    /// Move to the Archive folder.
    Archive,
    /// Move to Trash; delete for good if already in Trash or there is none.
    Delete,
    /// Move to another folder of the same account.
    Move {
        /// Row id of the destination folder.
        folder_id: i64,
    },
}

impl From<MessageAction> for Action {
    fn from(action: MessageAction) -> Self {
        match action {
            MessageAction::MarkRead => Action::MarkRead,
            MessageAction::MarkUnread => Action::MarkUnread,
            MessageAction::Flag => Action::Flag,
            MessageAction::Unflag => Action::Unflag,
            MessageAction::Archive => Action::Archive,
            MessageAction::Delete => Action::Delete,
            MessageAction::Move { folder_id } => Action::Move {
                to: FolderId(folder_id),
            },
        }
    }
}

/// Which action the server refused, in `CoreEvent::ActionFailed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ActionKind {
    /// Mark as read.
    MarkRead,
    /// Mark as unread.
    MarkUnread,
    /// Flag.
    Flag,
    /// Remove the flag.
    Unflag,
    /// Move, archive or move to Trash.
    Move,
    /// Delete for good.
    Delete,
}

impl From<events::ActionKind> for ActionKind {
    fn from(kind: events::ActionKind) -> Self {
        match kind {
            events::ActionKind::MarkRead => Self::MarkRead,
            events::ActionKind::MarkUnread => Self::MarkUnread,
            events::ActionKind::Flag => Self::Flag,
            events::ActionKind::Unflag => Self::Unflag,
            events::ActionKind::Move => Self::Move,
            events::ActionKind::Delete => Self::Delete,
        }
    }
}

/// An action could not be applied. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ActionFailure {
    /// The account has no Archive folder.
    #[error("the account has no Archive folder")]
    NoArchiveFolder,
    /// A message or the destination folder doesn't exist.
    #[error("not found")]
    NotFound,
    /// The messages are in different folders.
    #[error("the messages are in different folders")]
    MixedFolders,
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

impl From<ActionError> for ActionFailure {
    fn from(error: ActionError) -> Self {
        match error {
            ActionError::NoArchiveFolder => Self::NoArchiveFolder,
            ActionError::UnknownMessage | ActionError::UnknownFolder => Self::NotFound,
            ActionError::MixedFolders => Self::MixedFolders,
            ActionError::Store(_) => Self::Storage,
        }
    }
}

/// Unix seconds now. A clock before 1970 counts as 0; it only delays retries.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[uniffi::export]
impl SyncEngine {
    /// Applies `action` to messages in one folder: the change shows at once
    /// (a `MessagesChanged` event follows) and is queued for the server.
    /// Returns the account id whose queue to replay with
    /// [`SyncEngine::replay_actions`], or `None` if nothing needed to change.
    /// Doesn't wait for a running sync.
    pub fn apply_action(
        &self,
        message_ids: Vec<i64>,
        action: MessageAction,
    ) -> Result<Option<i64>, ActionFailure> {
        let store = self.open_store().map_err(|_| ActionFailure::Storage)?;
        let ids: Vec<MessageId> = message_ids.into_iter().map(MessageId).collect();
        let listener = ListenerAdapter(self.listener.clone());
        let applied = actions::apply_action(&store, &ids, action.into(), now(), &listener)?;
        Ok(applied.filter(|a| a.queued).map(|a| a.account.0))
    }

    /// Undoes the newest action on `message_id` if it hasn't reached the
    /// server yet (the snackbar's Undo). Returns false if it's too late.
    pub fn undo_action(&self, message_id: i64) -> Result<bool, ActionFailure> {
        let store = self.open_store().map_err(|_| ActionFailure::Storage)?;
        let listener = ListenerAdapter(self.listener.clone());
        Ok(actions::undo(&store, MessageId(message_id), &listener)?)
    }

    /// Carries the account's queued actions out on the server, oldest
    /// first, then uploads its changed drafts and copies of sent messages
    /// for the Sent folder, and carries out snoozes. Ones that fail for now are
    /// retried later with back-off (5 s up to 10 minutes, on the next call or
    /// sync after that); actions the server refuses for good are undone, with
    /// an `ActionFailed` event. Returns how many actions and drafts are still
    /// waiting (drafts of an account without a Drafts folder don't count:
    /// waiting won't help them). Blocking; waits for a running sync.
    pub fn replay_actions(&self, account_id: i64) -> Result<u32, SyncFailure> {
        let account = AccountId(account_id);
        let storage = |_| crate::sync::failed(events::SyncErrorKind::Storage);
        let (ops, drafts) = {
            let store = self.open_store()?;
            let snoozes = store
                .list_snoozes(Some(account))
                .map_err(storage)?
                .iter()
                .filter(|s| s.state != mailcrane_store::SnoozeState::Snoozed)
                .count();
            let ops = store.count_pending_ops(account).map_err(storage)?
                + store.count_pending_sent_copies(account).map_err(storage)?
                + u64::try_from(snoozes).unwrap_or(u64::MAX);
            let drafts = store
                .list_drafts(account)
                .map_err(storage)?
                .iter()
                .filter(|d| d.dirty)
                .count();
            (ops, drafts)
        };
        if ops == 0 && drafts == 0 {
            return Ok(0);
        }
        // JMAP accounts (M3-04): actions only. Drafts stay on this device and
        // snoozes are local until JMAP gets its own way to share them.
        if let Some(r) = self.run_jmap(account, |s| {
            Box::pin(async move { s.replay_actions(now()).await.map(|r| r.remaining) })
        }) {
            return r.map(|n| u32::try_from(n).unwrap_or(u32::MAX));
        }
        let remaining = self.with_session(account, |store, session, listener| {
            Box::pin(async move {
                let replayed = actions::replay(store, session, listener, account, now()).await?;
                let uploaded =
                    mailcrane_core::drafts::upload_drafts(store, session, account, now()).await?;
                let drafts_left = if uploaded.no_drafts_folder {
                    0
                } else {
                    uploaded.remaining
                };
                let copies =
                    mailcrane_core::outbox::save_sent_copies(store, session, account, now())
                        .await?;
                let snoozes =
                    mailcrane_core::snooze::sync_snoozes(store, session, listener, account, now())
                        .await?;
                Ok(replayed
                    .remaining
                    .saturating_add(drafts_left)
                    .saturating_add(copies.remaining)
                    .saturating_add(snoozes.remaining))
            })
        })?;
        Ok(u32::try_from(remaining).unwrap_or(u32::MAX))
    }
}

impl From<events::CoreEvent> for CoreEvent {
    fn from(event: events::CoreEvent) -> Self {
        match event {
            events::CoreEvent::FolderChanged { folder_id } => Self::FolderChanged {
                folder_id: folder_id.0,
            },
            events::CoreEvent::MessagesChanged { folder_id } => Self::MessagesChanged {
                folder_id: folder_id.0,
            },
            events::CoreEvent::SyncError { account_id, kind } => Self::SyncError {
                account_id: account_id.0,
                kind: kind.into(),
            },
            events::CoreEvent::ActionFailed {
                account_id,
                action,
                message_ids,
                reason,
            } => Self::ActionFailed {
                account_id: account_id.0,
                action: action.into(),
                message_ids: message_ids.into_iter().map(|m| m.0).collect(),
                reason: SyncErrorKind::from(reason),
            },
        }
    }
}

#[cfg(test)]
mod tests;
