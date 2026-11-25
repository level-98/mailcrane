//! SQLite storage and migrations. Search (FTS5) arrives in a later migration.

#![forbid(unsafe_code)]

mod error;
mod migrate;
mod model;
mod repo;

pub use error::StoreError;
pub use model::*;
pub use repo::NewMail;
pub use repo::{is_valid_authserv_id, MAX_AUTHSERV_ID_LEN};
pub use repo::{
    CancelReport, Correspondent, Draft, DraftFields, DraftId, NewOp, NewOutboxMessage, NewRule,
    OpId, OpKind, OpMessage, OutboxId, OutboxMessage, OutboxState, PendingOp, Rule, RuleAction,
    RuleCondition, RuleId, SaveSentCopy, ScheduleMode, SearchFilter, SentCopy, ServerCopy,
    SignatureSetting, ThreadSummary, UnthreadedMessage, ACCOUNT_COLOR_COUNT, MAX_SIGNATURE_BYTES,
};
pub use repo::{JmapRow, Protocol};
pub use repo::{KeywordedMessage, NewSnooze, Snooze, SnoozeId, SnoozeState};
pub use repo::{ServerRules, ServerRulesState};

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

    /// How long a statement waits for another connection's write lock
    /// before failing with `SQLITE_BUSY` (5 s by default). For tests.
    pub fn set_busy_timeout(&self, timeout: std::time::Duration) -> Result<(), StoreError> {
        Ok(self.conn.busy_timeout(timeout)?)
    }

    /// Starts a write transaction with `BEGIN IMMEDIATE`: the write lock is
    /// taken up front, waiting under the busy timeout, never by upgrading a
    /// read part-way through (which fails at once in WAL mode). If another
    /// connection still holds the lock after the timeout (`SQLITE_BUSY`),
    /// tries again [`BUSY_RETRIES`] times after a short pause (issue #282).
    pub(crate) fn write_tx(&self) -> Result<rusqlite::Transaction<'_>, StoreError> {
        retry_busy(|| {
            rusqlite::Transaction::new_unchecked(
                &self.conn,
                rusqlite::TransactionBehavior::Immediate,
            )
        })
    }

    /// The schema version of the open database.
    pub fn schema_version(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
}

/// How often a write is retried after `SQLITE_BUSY`, on top of the busy
/// timeout each attempt already waits.
pub(crate) const BUSY_RETRIES: u32 = 2;

/// Runs `f`, retrying [`BUSY_RETRIES`] times with a growing pause while it
/// fails with `SQLITE_BUSY` or `SQLITE_LOCKED`.
pub(crate) fn retry_busy<T>(
    mut f: impl FnMut() -> Result<T, rusqlite::Error>,
) -> Result<T, StoreError> {
    let mut attempt = 0u32;
    loop {
        match f() {
            Err(e) if is_busy(&e) && attempt < BUSY_RETRIES => {
                attempt = attempt.saturating_add(1);
                std::thread::sleep(std::time::Duration::from_millis(
                    100u64.saturating_mul(u64::from(attempt)),
                ));
            }
            other => return other.map_err(StoreError::from),
        }
    }
}

/// Whether another connection holds the lock (`SQLITE_BUSY`, `SQLITE_LOCKED`).
pub(crate) fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}
