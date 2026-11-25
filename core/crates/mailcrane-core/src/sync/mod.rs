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
pub use live::{live_stop, run_live_updates, ImapLogin, LiveConfig, LiveStop, LiveStopSignal};
