//! Syncing folders with the server.

mod convert;
mod incremental;
mod initial;
mod jmap;
mod live;
mod local;
pub mod prefetch;
pub mod push;
mod search;

pub use incremental::{Strategy, SyncReport};
pub use initial::{log_failure, FailureLog, FolderSyncer, SyncConfig, SyncError};
pub use jmap::JmapSyncer;
pub use local::push_local_changes;
/// One per account: whoever holds it is the only one writing that
/// account's synced mail to the database, so two syncs (live updates and a
/// refresh, say) never fight over the write lock (issue #282). Held across
/// a whole sync, including its network round trips; reads don't need it.
pub type WriteLock = std::sync::Arc<tokio::sync::Mutex<()>>;

pub use live::{
    live_stop, run_live_updates, ImapLogin, LiveAccount, LiveConfig, LiveStop, LiveStopSignal,
};

// For the debug-only fixture import (#289), which stores messages the way
// sync does.
#[cfg(feature = "fixtures")]
pub(crate) use convert::{json_string, json_strings};
#[cfg(feature = "fixtures")]
pub(crate) use initial::save_body;
