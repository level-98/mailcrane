//! One reused IMAP connection per account for work the user waits for
//! (issue #294): opening a message, carrying out message actions and
//! searching the server. Before this, each of those connected, did TLS and
//! logged in again, which costs several round trips.
//!
//! - One connection per account, separate from the sync connection, behind
//!   a lock of its own: calls for one account take turns on it.
//! - Reused while healthy. Checked with `NOOP` first when it has been idle
//!   for more than a minute; closed after five minutes idle, and when the
//!   app says the network changed or it has been in the background a while
//!   ([`SyncEngine::close_connections`]).
//! - On a connection failure the connection is dropped. A job marked
//!   [`Retry::Idempotent`] (fetches, searches) is then run once more on a
//!   new connection; any other job (APPEND, EXPUNGE, STORE, MOVE) is never
//!   retried here, its caller already defers or undoes it.
//! - TLS, credentials, limits and timeouts are exactly those of every other
//!   connection: the same `ImapConfig` and `Session::connect`.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Whether a job may run a second time after the connection failed under
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retry {
    /// Reading only (FETCH, SEARCH, NOOP): safe to run again.
    Idempotent,
    /// Changes something on the server: never run again automatically.
    Never,
}

/// When pooled connections are checked and closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolConfig {
    /// Idle for longer than this, a connection is checked with `NOOP`
    /// before it is used. Default 60 s.
    pub(crate) check_after: Duration,
    /// Idle for longer than this, a connection is closed. Default 5 min.
    pub(crate) close_after: Duration,
    /// How often idle connections are looked for. Default 15 s.
    pub(crate) sweep_every: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            check_after: Duration::from_secs(60),
            close_after: Duration::from_secs(5 * 60),
            sweep_every: Duration::from_secs(15),
        }
    }
}

/// Why the app asks for the connections to be closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloseReason {
    /// The network changed: the old connections are likely dead, so they
    /// are dropped without waiting for a `LOGOUT` answer.
    NetworkChanged,
    /// The app has been in the background for a while: log out politely.
    Background,
}

#[uniffi::export]
impl SyncEngine {
    /// Closes the reused connections of every account (issue #294). Call it
    /// when the network changes, and when the app has been in the
    /// background for more than a minute. The next call that needs one
    /// connects again. Waits for any call using a connection to finish.
    pub fn close_connections(&self, reason: CloseReason) {
        self.pool.close_all(reason);
    }
}

/// A connection with the runtime its socket belongs to.
struct Pooled {
    runtime: tokio::runtime::Runtime,
    session: Session,
    last_used: Instant,
}

impl Pooled {
    /// Closes the connection on a thread with a stack this crate sets
    /// (#462): the apps call [`SyncEngine::close_connections`] on any thread.
    fn close_on_job_thread(self, reason: CloseReason) {
        // `None`: the thread didn't start (the connection is then dropped
        // without LOGOUT) or the close panicked; either way it's gone.
        let _ = crate::worker::on_job_thread(move || self.close(reason));
    }

    fn close(self, reason: CloseReason) {
        let Pooled {
            runtime, session, ..
        } = self;
        match reason {
            // `logout` gives LOGOUT at most 5 s.
            CloseReason::Background => runtime.block_on(session.logout()),
            CloseReason::NetworkChanged => drop(session),
        }
    }
}

/// One account's connection, if open. Its mutex is the connection's lock.
#[derive(Default)]
struct Slot {
    conn: Option<Pooled>,
    /// Connections opened for this account so far (tests).
    #[cfg(test)]
    opened: usize,
}

/// The reused connections of one engine.
pub(crate) struct ConnectionPool {
    slots: Mutex<HashMap<i64, Arc<Mutex<Slot>>>>,
    config: Mutex<PoolConfig>,
    /// The thread closing idle connections, started on first use.
    sweeper: Mutex<Option<JoinHandle<()>>>,
    stopped: Arc<AtomicBool>,
}

impl ConnectionPool {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            slots: Mutex::new(HashMap::new()),
            config: Mutex::new(PoolConfig::default()),
            sweeper: Mutex::new(None),
            stopped: Arc::new(AtomicBool::new(false)),
        })
    }

    fn config(&self) -> PoolConfig {
        *self.config.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn set_config(&self, config: PoolConfig) {
        *self.config.lock().unwrap_or_else(|p| p.into_inner()) = config;
    }

    fn slot(&self, account: AccountId) -> Arc<Mutex<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots.entry(account.0).or_default().clone()
    }

    /// Connections opened for `account` so far (tests).
    #[cfg(test)]
    pub(crate) fn opened(&self, account: AccountId) -> usize {
        self.slot(account)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .opened
    }

    /// Whether `account` has an open connection now (tests).
    #[cfg(test)]
    pub(crate) fn is_open(&self, account: AccountId) -> bool {
        self.slot(account)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .conn
            .is_some()
    }

    /// Runs `job` on `account`'s connection, with `ctx` lent alongside it,
    /// opening a connection with `login` if
    /// there is none or the one there no longer works. Calls for one
    /// account run one at a time. A connection failure drops the
    /// connection; with [`Retry::Idempotent`] the job then runs once more
    /// on a new one. Any other error leaves the connection for the next
    /// call.
    pub(crate) fn run<C, T>(
        self: &Arc<Self>,
        account: AccountId,
        login: impl Fn() -> Result<ImapLogin, events::SyncErrorKind>,
        retry: Retry,
        ctx: &C,
        mut job: impl for<'a> FnMut(&'a mut Session, &'a C) -> SyncFuture<'a, T>,
    ) -> Result<T, PoolError> {
        self.start_sweeper();
        let config = self.config();
        let slot = self.slot(account);
        let mut slot = slot.lock().unwrap_or_else(|p| p.into_inner());
        // Lock order: the account's WriteLock, if needed, is taken before
        // this point, never while the connection is held (see
        // `check_lock_order`).
        let _holding = Holding::start();
        let mut attempts = 0u8;
        loop {
            attempts = attempts.saturating_add(1);
            let mut conn = match take_healthy(&mut slot, &config) {
                Some(conn) => conn,
                None => {
                    let conn = open(&login)?;
                    #[cfg(test)]
                    {
                        slot.opened = slot.opened.saturating_add(1);
                    }
                    conn
                }
            };
            match conn.runtime.block_on(job(&mut conn.session, ctx)) {
                Ok(value) => {
                    conn.last_used = Instant::now();
                    slot.conn = Some(conn);
                    return Ok(value);
                }
                Err(error) if breaks_connection(&error) => {
                    log_failure("pooled job", &error, conn.session.last_command());
                    // Dead or in an unknown state: never reused.
                    drop(conn);
                    if retry == Retry::Idempotent && attempts < 2 && worth_retrying(&error) {
                        continue;
                    }
                    return Err(PoolError::Job(error));
                }
                Err(error) => {
                    conn.last_used = Instant::now();
                    slot.conn = Some(conn);
                    return Err(PoolError::Job(error));
                }
            }
        }
    }

    /// Closes every account's connection, waiting for calls using one.
    pub(crate) fn close_all(&self, reason: CloseReason) {
        let slots: Vec<Arc<Mutex<Slot>>> = self
            .slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect();
        for slot in slots {
            let conn = slot.lock().unwrap_or_else(|p| p.into_inner()).conn.take();
            if let Some(conn) = conn {
                conn.close_on_job_thread(reason);
            }
        }
    }

    /// Closes connections idle for longer than `close_after`. Connections
    /// in use are skipped; they are fresh anyway.
    fn sweep(&self) {
        let close_after = self.config().close_after;
        let slots: Vec<Arc<Mutex<Slot>>> = self
            .slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect();
        for slot in slots {
            let Ok(mut slot) = slot.try_lock() else {
                continue;
            };
            let idle = slot
                .conn
                .as_ref()
                .is_some_and(|c| c.last_used.elapsed() > close_after);
            if idle {
                if let Some(conn) = slot.conn.take() {
                    drop(slot);
                    conn.close(CloseReason::Background);
                }
            }
        }
    }

    fn start_sweeper(self: &Arc<Self>) {
        let mut sweeper = self.sweeper.lock().unwrap_or_else(|p| p.into_inner());
        if sweeper.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        // The thread holds the pool weakly, so dropping the engine ends it.
        let pool = Arc::downgrade(self);
        let stopped = self.stopped.clone();
        let spawned = crate::worker::job_thread("mailcrane-pool-sweeper".into(), move || {
            let mut since = Instant::now();
            loop {
                std::thread::sleep(SWEEP_POLL);
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let Some(pool) = pool.upgrade() else {
                    return;
                };
                if since.elapsed() >= pool.config().sweep_every {
                    pool.sweep();
                    since = Instant::now();
                }
            }
        });
        *sweeper = spawned.ok();
    }

    /// Stops the sweeper and closes every connection.
    pub(crate) fn shut_down(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.close_all(CloseReason::NetworkChanged);
    }
}

thread_local! {
    /// How many pooled connections this thread holds right now (nested
    /// `run` calls would count twice; there are none).
    static HOLDING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Marks the current thread as holding a pooled connection while it lives.
struct Holding;

impl Holding {
    fn start() -> Self {
        HOLDING.with(|h| h.set(h.get().saturating_add(1)));
        Holding
    }
}

impl Drop for Holding {
    fn drop(&mut self) {
        HOLDING.with(|h| h.set(h.get().saturating_sub(1)));
    }
}

/// Whether the current thread holds a pooled connection: inside
/// [`ConnectionPool::run`], including its job.
pub(crate) fn holding_pooled_connection() -> bool {
    HOLDING.with(|h| h.get() > 0)
}

/// The lock order is the account's WriteLock first, then its pooled
/// connection. A call that held the connection and then waited for the
/// WriteLock could wait forever for a sync or download that holds the
/// WriteLock and waits for the connection. Called before every WriteLock
/// acquisition: a breach fails tests at once and is logged in the app.
pub(crate) fn check_lock_order() {
    if holding_pooled_connection() {
        // Test builds only: the app never panics, it logs (below).
        #[cfg(test)]
        #[allow(
            clippy::panic,
            reason = "compiled into tests only, to fail them at once"
        )]
        {
            panic!("lock order: WriteLock taken while holding the pooled connection");
        }
        #[cfg(not(test))]
        tracing::error!(
            stage = "lock order",
            "WriteLock taken while holding the pooled connection"
        );
    }
}

/// How often the sweeper wakes to check whether it should stop or sweep.
const SWEEP_POLL: Duration = Duration::from_millis(200);

/// Why a pooled call failed.
#[derive(Debug)]
pub(crate) enum PoolError {
    /// No connection could be opened.
    Connect(events::SyncErrorKind),
    /// The job failed.
    Job(CoreSyncError),
}

impl PoolError {
    pub(crate) fn kind(&self) -> events::SyncErrorKind {
        match self {
            PoolError::Connect(kind) => *kind,
            PoolError::Job(error) => error.kind(),
        }
    }
}

/// The slot's connection if it can be used: not idle for too long, and,
/// after a minute's idleness, answering `NOOP`. A connection that fails
/// the check is dropped.
fn take_healthy(slot: &mut Slot, config: &PoolConfig) -> Option<Pooled> {
    let mut conn = slot.conn.take()?;
    let idle = conn.last_used.elapsed();
    if idle > config.close_after {
        conn.close(CloseReason::Background);
        return None;
    }
    if idle > config.check_after {
        match conn.runtime.block_on(conn.session.noop()) {
            Ok(()) => conn.last_used = Instant::now(),
            Err(_) => return None,
        }
    }
    Some(conn)
}

/// Opens a connection with its own runtime.
fn open(
    login: &impl Fn() -> Result<ImapLogin, events::SyncErrorKind>,
) -> Result<Pooled, PoolError> {
    let login = login().map_err(PoolError::Connect)?;
    let runtime = runtime().map_err(PoolError::Connect)?;
    let session = runtime
        .block_on(Session::connect_auth(
            &login.config,
            &login.username,
            &login.auth,
        ))
        .map_err(|e| {
            let e = CoreSyncError::from(e);
            log_failure("connect", &e, None);
            PoolError::Connect(e.kind())
        })?;
    Ok(Pooled {
        runtime,
        session,
        last_used: Instant::now(),
    })
}

/// Whether the connection can't be trusted after `error`: it closed, timed
/// out mid-command, or the server sent something unreadable or too big.
fn breaks_connection(error: &CoreSyncError) -> bool {
    matches!(
        error,
        CoreSyncError::Imap(
            ImapError::ConnectionClosed
                | ImapError::ServerBye
                | ImapError::Timeout
                | ImapError::Protocol
                | ImapError::LineTooLong
                | ImapError::ResponseTooLarge
        )
    )
}

/// Whether a new connection may do better: the old one dropped or timed
/// out. A malformed or oversized answer would only come again.
fn worth_retrying(error: &CoreSyncError) -> bool {
    matches!(
        error,
        CoreSyncError::Imap(
            ImapError::ConnectionClosed | ImapError::ServerBye | ImapError::Timeout
        )
    )
}
