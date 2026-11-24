//! Syncing folders with the server.

mod convert;
mod incremental;
mod initial;
mod live;

pub use incremental::{Strategy, SyncReport};
pub use initial::{FolderSyncer, SyncConfig, SyncError};
pub use live::{live_stop, run_live_updates, ImapLogin, LiveConfig, LiveStop, LiveStopSignal};
