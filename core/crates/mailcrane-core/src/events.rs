//! Events the core sends to the apps, so they never have to poll.
//!
//! Events carry ids and error kinds only: never message content, addresses
//! or server text.

pub use mailcrane_store::{AccountId, FolderId};

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
