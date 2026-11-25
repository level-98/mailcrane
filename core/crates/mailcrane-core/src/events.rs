//! Events the core sends to the apps, so they never have to poll.
//!
//! Events carry ids and error kinds only: never message content, addresses
//! or server text.

pub use mailcrane_store::{AccountId, FolderId, MessageId};

/// Why a sync failed, in terms an app can act on. Carries no server text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncErrorKind {
    /// The server could not be reached, or the connection dropped.
    Network,
    /// TLS failed, including an untrusted certificate.
    Tls,
    /// The server refused the credentials.
    Auth,
    /// The server sent something unexpected, or refused a command.
    Protocol,
    /// The server took too long.
    Timeout,
    /// The local database failed.
    Storage,
    /// The folder or message does not exist (any more).
    NotFound,
    /// The folder's `UIDVALIDITY` changed; sync the folder again.
    FolderReset,
    /// Another sync was writing to the local database and kept it busy.
    /// Local and passing: nothing is wrong with the server or the account,
    /// and the next sync works. Apps shouldn't show it as an error.
    Busy,
}

/// Which action the server refused, in [`CoreEvent::ActionFailed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Something changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEvent {
    /// The folder itself changed, for example its cache was cleared because
    /// the server renumbered its messages.
    FolderChanged {
        /// The folder.
        folder_id: FolderId,
    },
    /// Messages in the folder were added or changed.
    MessagesChanged {
        /// The folder.
        folder_id: FolderId,
    },
    /// A sync failed. Everything that completed before the failure is kept.
    SyncError {
        /// The account being synced.
        account_id: AccountId,
        /// What kind of failure.
        kind: SyncErrorKind,
    },
    /// The server refused a message action for good. Its local change was
    /// undone, so the messages are back as they were; tell the user.
    ActionFailed {
        /// The messages' account.
        account_id: AccountId,
        /// What was refused.
        action: ActionKind,
        /// The messages it was applied to.
        message_ids: Vec<MessageId>,
        /// Why, as far as can be told without server text.
        reason: SyncErrorKind,
    },
}

/// Receives events. The apps register one; it may be called from any thread
/// and must not block.
pub trait EventListener: Send + Sync {
    /// Called for every event.
    fn on_event(&self, event: CoreEvent);
}

/// A listener that ignores everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoListener;

impl EventListener for NoListener {
    fn on_event(&self, _event: CoreEvent) {}
}
