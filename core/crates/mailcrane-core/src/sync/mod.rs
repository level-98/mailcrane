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

// For the debug-only fixture import (#289), which stores messages the way
// sync does.
#[cfg(feature = "fixtures")]
pub(crate) use convert::{json_string, json_strings};
#[cfg(feature = "fixtures")]
pub(crate) use initial::save_body;
