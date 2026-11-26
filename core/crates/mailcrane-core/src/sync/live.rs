//! Live updates while the app is open: new mail appears within seconds,
//! without the user refreshing.
//!
//! [`run_live_updates`] owns one IMAP connection per account, separate from
//! the one user actions use, so nothing waits behind it. It:
//!
//! 1. connects and runs an incremental sync of the inbox;
//! 2. waits with `IDLE` (RFC 2177), re-issued every 10 minutes: a server
//!    may drop a client that idles for 30 (§3), and mobile networks often
//!    forget an idle connection sooner, so re-issuing keeps it alive and
//!    finds a dead one (`DONE` gets no answer) within minutes;
//! 3. when the server reports `EXISTS`, `EXPUNGE` or `FETCH`, sends `DONE`
//!    after a 500 ms debounce, syncs the inbox again and goes back to 2;
//! 4. without the `IDLE` capability, syncs every 5 minutes instead;
//! 5. after a dropped connection or any error, reconnects with exponential
//!    back-off from 2 s to 5 minutes, with jitter; a successful sync resets
//!    the back-off. A refused login is not retried: the loop reports it and
//!    ends, so a wrong password can't get the account locked.
//! 6. when the app says the network changed ([`LiveStop::reconnect`]),
//!    drops the connection (its socket belongs to the old network) and
//!    connects again at once, skipping any back-off wait.
//!
//! Stopping sends `DONE` and `LOGOUT`, and returns within 2 seconds even
//! if the server does not answer.

use super::initial::{log_failure, FolderSyncer, SyncConfig, SyncError};
use super::WriteLock;
use crate::events::{AccountId, CoreEvent, EventListener, FolderId, SyncErrorKind};
use mailcrane_imap::{save_folders, IdleOutcome, ImapAuth, ImapConfig, ImapError, Session};
use mailcrane_store::{FolderRole, Store};
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

/// Timings for live updates. The defaults are the ones to ship; tests
/// shorten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveConfig {
    /// How long one `IDLE` lasts before it is re-issued. Must stay under
    /// 29 minutes (RFC 2177 §3). Default 10 minutes, short enough that a
    /// connection a mobile network dropped silently is noticed soon.
    pub reissue_after: Duration,
    /// How often to sync when the server has no `IDLE`. Default 5 minutes.
    pub poll_every: Duration,
    /// How long to keep collecting changes after the first one, so a
    /// burst needs one sync. Default 500 ms.
    pub debounce: Duration,
    /// First reconnect delay. Default 2 s.
    pub backoff_min: Duration,
    /// Longest reconnect delay. Default 5 minutes.
    pub backoff_max: Duration,
    /// Whether to shorten each reconnect delay by up to a quarter at
    /// random (never below `backoff_min`), so many clients that lost the
    /// same server do not all come back at once. Default on.
    pub jitter: bool,
    /// How long stopping may take, `DONE` and `LOGOUT` included. Default
    /// 2 s.
    pub stop_within: Duration,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            reissue_after: Duration::from_secs(10 * 60),
            poll_every: Duration::from_secs(5 * 60),
            debounce: Duration::from_millis(500),
            backoff_min: Duration::from_secs(2),
            backoff_max: Duration::from_secs(5 * 60),
            jitter: true,
            stop_within: Duration::from_secs(2),
        }
    }
}

/// Where and how to log in. Asked for again before every connection, so a
/// changed password is picked up.
#[derive(Debug, Clone)]
pub struct ImapLogin {
    /// Server, security and limits.
    pub config: ImapConfig,
    /// The account's user name.
    pub username: String,
    /// The password, or an OAuth access token (M3-02).
    pub auth: ImapAuth,
}

/// What [`LiveStop`] has asked for so far.
#[derive(Debug, Clone, Copy, Default)]
struct Asked {
    stop: bool,
    /// How many reconnects were asked for; only changes matter.
    reconnects: u64,
}

/// Tells a running [`run_live_updates`] to stop, or to reconnect now.
/// Dropping it stops it too.
#[derive(Debug)]
pub struct LiveStop(watch::Sender<Asked>);

impl LiveStop {
    /// Asks the live loop to stop. It returns within
    /// [`LiveConfig::stop_within`].
    pub fn stop(&self) {
        self.0.send_modify(|asked| asked.stop = true);
    }

    /// Asks the live loop to drop its connection and connect again now,
    /// without waiting out the back-off: the device moved to another
    /// network, or got one back. The old connection is dropped without
    /// `LOGOUT`, because its socket most likely leads nowhere now.
    pub fn reconnect(&self) {
        self.0
            .send_modify(|asked| asked.reconnects = asked.reconnects.wrapping_add(1));
    }
}

/// The receiving side of [`LiveStop`].
#[derive(Debug, Clone)]
pub struct LiveStopSignal {
    rx: watch::Receiver<Asked>,
    /// The reconnect count last acted on.
    seen: u64,
}

impl LiveStopSignal {
    fn is_stopped(&self) -> bool {
        self.rx.borrow().stop
    }

    /// Finishes once a stop was asked for, or the [`LiveStop`] is gone.
    async fn stopped(&mut self) {
        loop {
            if self.rx.borrow_and_update().stop {
                return;
            }
            if self.rx.changed().await.is_err() {
                return;
            }
        }
    }

    /// Finishes once a reconnect was asked for since the last time this
    /// finished. Never finishes once the [`LiveStop`] is gone (that is a
    /// stop, which [`Self::stopped`] reports).
    async fn reconnect_asked(&mut self) {
        loop {
            let reconnects = self.rx.borrow_and_update().reconnects;
            if reconnects != self.seen {
                self.seen = reconnects;
                return;
            }
            if self.rx.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

/// A stop handle and the signal to pass to [`run_live_updates`].
pub fn live_stop() -> (LiveStop, LiveStopSignal) {
    let (tx, rx) = watch::channel(Asked::default());
    (LiveStop(tx), LiveStopSignal { rx, seen: 0 })
}

/// Keeps the account's inbox up to date until stopped; see the module
/// docs. `login` gives the connection details, or the kind of error to
/// report if they are missing (for example no saved password). Failures
/// are sent to `listener` as [`CoreEvent::SyncError`] and retried with
/// back-off; this only returns once stopped.
///
/// The account's [`WriteLock`] (in [`LiveAccount`]): each sync after a change
/// holds it, so live updates never write to the database while a sync
/// started by the app does (issue #282). Waiting in IDLE doesn't hold it.
pub async fn run_live_updates(
    store: &Store,
    account: LiveAccount,
    listener: &dyn EventListener,
    login: impl Fn() -> Result<ImapLogin, SyncErrorKind>,
    sync_config: SyncConfig,
    config: LiveConfig,
    stop: LiveStopSignal,
) {
    let LiveAccount {
        id: account,
        write_lock,
    } = account;
    let mut source = ImapSource {
        store,
        account,
        listener,
        login,
        sync_config,
        write_lock,
    };
    run(&mut source, listener, account, config, stop).await;
}

/// The account live updates run for.
pub struct LiveAccount {
    /// Row id of the account.
    pub id: AccountId,
    /// The account's write lock, shared with the app's syncs.
    pub write_lock: WriteLock,
}

/// Opens connections. A trait so the loop's timing can be tested without
/// a network.
trait Source {
    type Conn: Conn;
    async fn connect(&mut self) -> Result<Self::Conn, SyncErrorKind>;
}

/// One live connection.
trait Conn {
    fn supports_idle(&self) -> bool;
    /// Incremental sync of the inbox; leaves it selected.
    async fn sync(&mut self) -> Result<(), SyncErrorKind>;
    async fn idle(
        &mut self,
        max: Duration,
        debounce: Duration,
        stop: impl Future<Output = ()>,
    ) -> Result<IdleOutcome, SyncErrorKind>;
    /// `LOGOUT` and close.
    async fn close(&mut self);
}

/// How a connection's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Served {
    Stopped,
    Failed,
    /// A reconnect was asked for: connect again now.
    Reconnect,
}

async fn run<S: Source>(
    source: &mut S,
    listener: &dyn EventListener,
    account: AccountId,
    config: LiveConfig,
    mut stop: LiveStopSignal,
) {
    let mut backoff = Backoff::new(&config, seed(account));
    // Its own receiver, so waiting for a reconnect and for a stop can
    // happen side by side.
    let mut wake = stop.clone();
    while !stop.is_stopped() {
        let connected = tokio::select! {
            biased;
            () = stop.stopped() => return,
            // A connect on the old network may hang until its timeout.
            () = wake.reconnect_asked() => {
                backoff.reset();
                continue;
            }
            connected = source.connect() => connected,
        };
        match connected {
            Err(kind) => {
                tracing::warn!(account = account.0, ?kind, "live updates cannot connect");
                listener.on_event(CoreEvent::SyncError {
                    account_id: account,
                    kind,
                });
                // Retrying a refused login every few minutes can get the
                // account locked by the provider. Stop until the app starts
                // live updates again (next foreground, or a fixed password).
                if kind == SyncErrorKind::Auth {
                    return;
                }
            }
            Ok(mut conn) => {
                // Once a stop is asked for, everything left (DONE, LOGOUT)
                // gets `stop_within`; then the connection is just dropped.
                let mut hard = stop.clone();
                let served = tokio::select! {
                    biased;
                    served = async {
                        let served = serve(&mut conn, &config, &mut backoff, &mut stop).await;
                        if served == Served::Stopped {
                            conn.close().await;
                        }
                        served
                    } => served,
                    () = async {
                        hard.stopped().await;
                        tokio::time::sleep(config.stop_within).await;
                    } => Served::Stopped,
                    () = wake.reconnect_asked() => Served::Reconnect,
                };
                match served {
                    Served::Stopped => return,
                    Served::Reconnect => {
                        tracing::debug!(
                            account = account.0,
                            "live updates reconnect: network changed"
                        );
                        drop(conn);
                        backoff.reset();
                        continue;
                    }
                    Served::Failed => {}
                }
            }
        }
        let delay = backoff.next_delay();
        tracing::debug!(account = account.0, ?delay, "live updates reconnect after");
        tokio::select! {
            biased;
            () = stop.stopped() => return,
            // The network is back: no reason to wait out the back-off.
            () = wake.reconnect_asked() => backoff.reset(),
            () = tokio::time::sleep(delay) => {}
        }
    }
}

/// Syncs, then waits for the next change, until stopped or failed.
async fn serve<C: Conn>(
    conn: &mut C,
    config: &LiveConfig,
    backoff: &mut Backoff,
    stop: &mut LiveStopSignal,
) -> Served {
    loop {
        let synced = tokio::select! {
            biased;
            () = stop.stopped() => return Served::Stopped,
            synced = conn.sync() => synced,
        };
        if let Err(kind) = synced {
            tracing::warn!(?kind, "live sync failed");
            return Served::Failed;
        }
        backoff.reset();
        loop {
            if conn.supports_idle() {
                match conn
                    .idle(config.reissue_after, config.debounce, stop.stopped())
                    .await
                {
                    Ok(IdleOutcome::Changed) => break,
                    // RFC 2177 §3: re-issue before the server's timeout.
                    Ok(IdleOutcome::TimedOut) => {}
                    Ok(IdleOutcome::Stopped) => return Served::Stopped,
                    Err(kind) => {
                        tracing::info!(?kind, "IDLE connection lost");
                        return Served::Failed;
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    () = stop.stopped() => return Served::Stopped,
                    () = tokio::time::sleep(config.poll_every) => break,
                }
            }
        }
    }
}

/// Exponential back-off: `min`, `2 × min`, `4 × min` … up to `max`.
struct Backoff {
    min: Duration,
    max: Duration,
    jitter: bool,
    attempt: u32,
    rng: u64,
}

impl Backoff {
    fn new(config: &LiveConfig, seed: u64) -> Self {
        Self {
            min: config.backoff_min,
            max: config.backoff_max.max(config.backoff_min),
            jitter: config.jitter,
            attempt: 0,
            rng: seed | 1,
        }
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }

    fn next_delay(&mut self) -> Duration {
        let factor = 1u32.checked_shl(self.attempt).unwrap_or(u32::MAX);
        let nominal = self.min.saturating_mul(factor).min(self.max);
        self.attempt = self.attempt.saturating_add(1).min(31);
        if !self.jitter {
            return nominal;
        }
        // xorshift64: good enough to spread clients out, and no dependency.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let quarter = u128::from(u64::try_from((nominal / 4).as_nanos()).unwrap_or(u64::MAX));
        let cut = quarter * u128::from(self.rng % 1_000_000) / 1_000_000;
        let cut = Duration::from_nanos(u64::try_from(cut).unwrap_or(0));
        nominal.saturating_sub(cut).max(self.min)
    }
}

fn seed(account: AccountId) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    u64::from(now) ^ (account.0 as u64).rotate_left(32)
}

/// The real source: IMAP sessions syncing the account's inbox.
struct ImapSource<'a, L> {
    store: &'a Store,
    account: AccountId,
    listener: &'a dyn EventListener,
    login: L,
    sync_config: SyncConfig,
    write_lock: WriteLock,
}

impl<'a, L: Fn() -> Result<ImapLogin, SyncErrorKind>> Source for ImapSource<'a, L> {
    type Conn = ImapConn<'a>;

    async fn connect(&mut self) -> Result<ImapConn<'a>, SyncErrorKind> {
        let login = (self.login)()?;
        let mut session = Session::connect_auth(&login.config, &login.username, &login.auth)
            .await
            .map_err(|e| {
                let e = SyncError::from(e);
                log_failure("live connect", &e, None);
                e.kind()
            })?;
        let inbox = match find_inbox(self.store, self.account)? {
            Some(inbox) => inbox,
            None => {
                // Folders were never listed for this account: do it now.
                let _writing = self.write_lock.lock().await;
                let folders = match session.list_folders().await {
                    Ok(folders) => folders,
                    Err(e) => {
                        let e = SyncError::from(e);
                        log_failure("live list folders", &e, session.last_command());
                        return Err(e.kind());
                    }
                };
                save_folders(self.store, self.account, &folders).map_err(|e| {
                    let e = SyncError::from(e);
                    log_failure("live save folders", &e, None);
                    e.kind()
                })?;
                find_inbox(self.store, self.account)?.ok_or(SyncErrorKind::NotFound)?
            }
        };
        Ok(ImapConn {
            session: Some(session),
            store: self.store,
            listener: self.listener,
            account: self.account,
            sync_config: self.sync_config,
            inbox,
            idle_refused: false,
            write_lock: self.write_lock.clone(),
        })
    }
}

/// The stored inbox: the folder with the inbox role, or named `INBOX`
/// (case-insensitive, RFC 9051 §5.1).
fn find_inbox(store: &Store, account: AccountId) -> Result<Option<FolderId>, SyncErrorKind> {
    let folders = store
        .list_folders(account)
        .map_err(|_| SyncErrorKind::Storage)?;
    Ok(folders
        .iter()
        .find(|f| f.details.role == Some(FolderRole::Inbox))
        .or_else(|| {
            folders
                .iter()
                .find(|f| f.details.remote_name.eq_ignore_ascii_case("INBOX"))
        })
        .map(|f| f.id))
}

impl ImapConn<'_> {
    /// Incremental sync of the inbox; leaves it selected. Reports its own
    /// failures as events.
    async fn sync_inbox(&mut self) -> Result<(), SyncErrorKind> {
        let session = self.session.as_mut().ok_or(SyncErrorKind::Network)?;
        FolderSyncer::new(
            self.store,
            session,
            self.listener,
            self.account,
            self.sync_config,
        )
        .sync_changes(self.inbox)
        .await
        .map(|_| ())
        .map_err(|e| e.kind())
    }
}

/// Unix seconds now. A clock before 1970 counts as 0.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

struct ImapConn<'a> {
    /// `None` once closed.
    session: Option<Session>,
    store: &'a Store,
    listener: &'a dyn EventListener,
    account: AccountId,
    sync_config: SyncConfig,
    inbox: FolderId,
    /// The server advertised `IDLE` but refused it: poll instead.
    idle_refused: bool,
    /// Held while syncing, never while idle.
    write_lock: WriteLock,
}

impl Conn for ImapConn<'_> {
    fn supports_idle(&self) -> bool {
        !self.idle_refused && self.session.as_ref().is_some_and(Session::supports_idle)
    }

    async fn sync(&mut self) -> Result<(), SyncErrorKind> {
        // One writer per account: a sync the app started finishes first.
        let lock = self.write_lock.clone();
        let _writing = lock.lock().await;
        self.sync_inbox().await?;
        // What a sync from the app does next, so mail that arrives live is
        // treated the same: rules on new inbox mail (M4-09), the actions
        // they queue, and threading (M4-02). A failure in these only leaves
        // the work for the next sync.
        let now = unix_now();
        if crate::rules::apply_new(self.store, self.account, now, self.listener).is_ok() {
            let session = self.session.as_mut().ok_or(SyncErrorKind::Network)?;
            let replayed =
                crate::actions::replay(self.store, session, self.listener, self.account, now).await;
            // Carrying out actions may select other folders, and IDLE
            // watches the selected one: select the inbox again, catching
            // anything that arrived meanwhile.
            if replayed.is_ok_and(|r| r.done > 0 || r.failed > 0) {
                self.sync_inbox().await?;
            }
        }
        let _ = crate::threading::assign_new(self.store, Some(self.account));
        Ok(())
    }

    async fn idle(
        &mut self,
        max: Duration,
        debounce: Duration,
        stop: impl Future<Output = ()>,
    ) -> Result<IdleOutcome, SyncErrorKind> {
        let session = self.session.as_mut().ok_or(SyncErrorKind::Network)?;
        match session.idle(max, debounce, stop).await {
            Ok(outcome) => Ok(outcome),
            Err(ImapError::CommandFailed) => {
                tracing::info!("the server refused IDLE; polling instead");
                self.idle_refused = true;
                Ok(IdleOutcome::TimedOut)
            }
            Err(e) => {
                let e = SyncError::from(e);
                log_failure("idle", &e, session.last_command());
                Err(e.kind())
            }
        }
    }

    async fn close(&mut self) {
        if let Some(session) = self.session.take() {
            session.logout().await;
        }
    }
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
