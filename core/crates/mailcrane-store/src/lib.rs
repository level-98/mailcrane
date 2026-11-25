//! SQLite storage and migrations. Search (FTS5) arrives in a later migration.

#![forbid(unsafe_code)]

mod error;
mod migrate;
mod model;
mod repo;

pub use error::StoreError;
pub use model::*;
pub use repo::{
    Correspondent, Draft, DraftFields, DraftId, NewOutboxMessage, OpId, OpKind, OpMessage,
    OutboxId, OutboxMessage, OutboxState, PendingOp, SaveSentCopy, ScheduleMode, SearchFilter,
    SentCopy, ServerCopy, SignatureSetting, ThreadSummary, UnthreadedMessage, MAX_SIGNATURE_BYTES,
};
pub use repo::{KeywordedMessage, NewSnooze, Snooze, SnoozeId, SnoozeState};

use rusqlite::Connection;
use std::path::Path;

/// The app's database.
#[derive(Debug)]
pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and migrates it to
    /// the newest schema. A file with a newer schema is left untouched and
    /// reported as [`StoreError::NewerSchema`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        // Check before any pragma that could write to the file.
        migrate::check_version(&conn, migrate::MIGRATIONS)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate::run(&mut conn, migrate::MIGRATIONS)?;
        Ok(Store { conn })
    }

    /// Opens a fresh in-memory database at the newest schema.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate::run(&mut conn, migrate::MIGRATIONS)?;
        Ok(Store { conn })
    }

    /// The schema version of the open database.
    pub fn schema_version(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
}
