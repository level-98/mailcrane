//! SQLite storage and migrations. Search (FTS5) arrives in a later migration.

#![forbid(unsafe_code)]

mod error;
mod migrate;
mod model;
mod repo;

pub use error::StoreError;
pub use model::*;
pub use repo::NewMail;
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
        yield_to_urgent();
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

/// Threads in this process whose writes the user is waiting for right now
/// (see [`urgent_writes`]).
static URGENT_WRITERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

thread_local! {
    /// Whether this thread is inside [`urgent_writes`].
    static URGENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The longest a write transaction waits for urgent writes to finish before
/// it starts anyway, so nothing can be held up for good.
const URGENT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Runs `f` with this thread's writes marked urgent (issue #294): while it
/// runs, write transactions of other threads in this process wait, for at
/// most [`URGENT_WAIT`], before they start. SQLite's busy handler is not
/// fair, so without this a short write the user is waiting for, such as
/// saving an opened message's body, can lose every race against a sync
/// writing batch after batch, and fail with `SQLITE_BUSY`. A transaction
/// already running finishes first; the urgent write gets the next turn.
pub fn urgent_writes<T>(f: impl FnOnce() -> T) -> T {
    struct Urgent {
        nested: bool,
    }
    impl Drop for Urgent {
        fn drop(&mut self) {
            if !self.nested {
                URGENT.with(|u| u.set(false));
                step_urgent(|n| n.checked_sub(1));
            }
        }
    }
    let nested = URGENT.with(|u| u.replace(true));
    if !nested {
        step_urgent(|n| n.checked_add(1));
    }
    let _urgent = Urgent { nested };
    f()
}

/// Changes [`URGENT_WRITERS`] by `change`, leaving it as it is where that
/// would overflow.
fn step_urgent(change: impl Fn(usize) -> Option<usize>) {
    use std::sync::atomic::Ordering::SeqCst;
    let mut current = URGENT_WRITERS.load(SeqCst);
    while let Some(next) = change(current) {
        match URGENT_WRITERS.compare_exchange_weak(current, next, SeqCst, SeqCst) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

/// Waits, for at most [`URGENT_WAIT`], while another thread's urgent writes
/// run. Urgent writes themselves never wait here.
fn yield_to_urgent() {
    if URGENT.with(std::cell::Cell::get) {
        return;
    }
    let started = std::time::Instant::now();
    while URGENT_WRITERS.load(std::sync::atomic::Ordering::SeqCst) > 0
        && started.elapsed() < URGENT_WAIT
    {
        std::thread::sleep(std::time::Duration::from_millis(2));
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
