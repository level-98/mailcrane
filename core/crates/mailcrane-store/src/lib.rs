//! SQLite storage and migrations. Search (FTS5) arrives in a later migration.

#![forbid(unsafe_code)]

mod error;
mod migrate;
mod model;
mod repo;

pub use error::StoreError;
pub use model::*;
pub use repo::NewMail;
pub use repo::PushSubscriptionRow;
pub use repo::{is_valid_authserv_id, MAX_AUTHSERV_ID_LEN};
pub use repo::{
    CancelReport, Correspondent, Draft, DraftFields, DraftId, NewOp, NewOutboxMessage, NewRule,
    OpId, OpKind, OpMessage, OutboxId, OutboxMessage, OutboxState, PendingOp, Rule, RuleAction,
    RuleCondition, RuleId, SaveSentCopy, ScheduleMode, SearchFilter, SentCopy, ServerCopy,
    SignatureSetting, ThreadSummary, UnthreadedMessage, ACCOUNT_COLOR_COUNT, MAX_SIGNATURE_BYTES,
};
pub use repo::{JmapRow, Protocol};
pub use repo::{KeywordedMessage, NewSnooze, Snooze, SnoozeId, SnoozeState, Snoozing};
pub use repo::{ServerRules, ServerRulesState};

use rusqlite::Connection;
use std::path::Path;

/// The app's database.
#[derive(Debug)]
pub struct Store {
    pub(crate) conn: Connection,
    /// The path this store was opened with, which names it for
    /// [`urgent_writes`]; `None` in memory.
    urgent_key: Option<String>,
    /// Names this database for caches kept outside it (#434): its path, or
    /// a name of its own for each in-memory store.
    scope: String,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and migrates it to
    /// the newest schema. A file with a newer schema is left untouched and
    /// reported as [`StoreError::NewerSchema`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let urgent_key = Some(path.as_ref().to_string_lossy().into_owned());
        let mut conn = Connection::open(path.as_ref())?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        // Check before any pragma that could write to the file.
        migrate::check_version(&conn, migrate::MIGRATIONS)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate::run(&mut conn, migrate::MIGRATIONS)?;
        // After opening, so the file exists and its canonical path is known.
        let scope = if is_memory_path(path.as_ref()) {
            memory_scope()
        } else {
            Self::scope_of(&path)
        };
        Ok(Store {
            conn,
            urgent_key,
            scope,
        })
    }

    /// Opens a fresh in-memory database at the newest schema.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate::run(&mut conn, migrate::MIGRATIONS)?;
        Ok(Store {
            conn,
            urgent_key: None,
            scope: memory_scope(),
        })
    }

    /// The name of the database at `path` for caches kept outside it, the
    /// same as [`Store::cache_scope`] of a store opened there (#434). The
    /// canonical path, so every spelling of one file (relative, `\` or `/`,
    /// another case on Windows, a symlink) gives one name: a token cached
    /// under one spelling is forgotten under another. A file that doesn't
    /// exist yet keeps the path as given; it has nothing cached.
    pub fn scope_of(path: impl AsRef<Path>) -> String {
        let path = path.as_ref();
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        format!("file:{}", canonical.to_string_lossy())
    }

    /// Names this database for caches kept outside it, such as the
    /// in-memory OAuth tokens (#434). Account IDs start at 1 in every
    /// database, so such a cache keys by this as well as the account.
    pub fn cache_scope(&self) -> &str {
        &self.scope
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
        yield_to_urgent(self.urgent_key.as_deref());
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

/// For each database (by the path it was opened with), how many threads
/// in this process are writing something the user waits for right now (see
/// [`urgent_writes`]).
static URGENT_WRITERS: std::sync::Mutex<Vec<(String, usize)>> = std::sync::Mutex::new(Vec::new());

thread_local! {
    /// The databases this thread is inside [`urgent_writes`] for.
    static URGENT_HERE: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The longest a write transaction waits for urgent writes to finish before
/// it starts anyway: short next to any caller's timeout, so a slow urgent
/// call can delay others' writes a little but never hold them up.
const URGENT_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether `path` opens a private in-memory database rather than a file.
fn is_memory_path(path: &Path) -> bool {
    path.as_os_str().is_empty() || path.as_os_str() == ":memory:"
}

/// A cache scope of its own for each in-memory database (#434).
fn memory_scope() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("memory:{n}")
}

/// Runs `f` with this thread's writes to the database at `db_path` marked
/// urgent (issue #294). While it runs, write transactions of other threads
/// on that same database (opened with the same path) wait, for at most
/// [`URGENT_WAIT`], before they start; other databases are not affected.
/// SQLite's busy handler is not fair, so without this a short write the user
/// is waiting for, such as saving an opened message's body, can lose every
/// race against a sync writing batch after batch, and fail with
/// `SQLITE_BUSY`. A transaction already running finishes first; the urgent
/// write gets the next turn.
pub fn urgent_writes<T>(db_path: &str, f: impl FnOnce() -> T) -> T {
    struct Urgent(String);
    impl Drop for Urgent {
        fn drop(&mut self) {
            URGENT_HERE.with(|here| {
                let mut here = here.borrow_mut();
                if let Some(i) = here.iter().position(|k| *k == self.0) {
                    here.swap_remove(i);
                }
            });
            let mut writers = URGENT_WRITERS.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(i) = writers.iter().position(|(k, _)| *k == self.0) {
                let left = writers.get(i).map_or(0, |(_, n)| n.saturating_sub(1));
                if left == 0 {
                    writers.swap_remove(i);
                } else if let Some(entry) = writers.get_mut(i) {
                    entry.1 = left;
                }
            }
        }
    }
    let nested = URGENT_HERE.with(|here| here.borrow().iter().any(|k| k == db_path));
    if nested {
        return f();
    }
    URGENT_HERE.with(|here| here.borrow_mut().push(db_path.to_owned()));
    {
        let mut writers = URGENT_WRITERS.lock().unwrap_or_else(|p| p.into_inner());
        match writers.iter_mut().find(|(k, _)| k == db_path) {
            Some(entry) => entry.1 = entry.1.saturating_add(1),
            None => writers.push((db_path.to_owned(), 1)),
        }
    }
    let _urgent = Urgent(db_path.to_owned());
    f()
}

/// Whether this thread is inside [`urgent_writes`] for the database at
/// `db_path`, so its write transactions there never wait for other urgent
/// writers. The mark is per thread: work moved to another thread doesn't
/// carry it (#468).
pub fn urgent_here(db_path: &str) -> bool {
    URGENT_HERE.with(|here| here.borrow().iter().any(|k| k == db_path))
}

/// Whether another thread is writing urgently to the database `key`.
fn others_urgent(key: &str) -> bool {
    URGENT_WRITERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .any(|(k, n)| k == key && *n > 0)
}

/// Waits, for at most [`URGENT_WAIT`], while another thread writes urgently
/// to this store's database. Urgent writes themselves never wait here, and
/// in-memory stores never do.
fn yield_to_urgent(key: Option<&str>) {
    let Some(key) = key else {
        return;
    };
    if URGENT_HERE.with(|here| here.borrow().iter().any(|k| k == key)) {
        return;
    }
    let started = std::time::Instant::now();
    while others_urgent(key) && started.elapsed() < URGENT_WAIT {
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
