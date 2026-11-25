//! Syncing folders with the server.

mod convert;
mod incremental;
mod initial;
mod jmap;
mod live;
mod search;

pub use incremental::{Strategy, SyncReport};
pub use initial::{log_failure, FailureLog, FolderSyncer, SyncConfig, SyncError};
pub use jmap::JmapSyncer;
/// One per account: whoever holds it is the only one writing that
/// account's synced mail to the database, so two syncs (live updates and a
/// refresh, say) never fight over the write lock (issue #282). Held across
/// a whole sync, including its network round trips; reads don't need it.
pub type WriteLock = std::sync::Arc<tokio::sync::Mutex<()>>;

pub use live::{
    live_stop, run_live_updates, ImapLogin, LiveAccount, LiveConfig, LiveStop, LiveStopSignal,
};
