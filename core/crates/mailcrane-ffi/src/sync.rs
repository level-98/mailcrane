//! Syncing, as the apps call it: `sync_folder`, `load_older`, `fetch_body`,
//! live updates while the app is open (`start_live_updates`,
//! `stop_live_updates`), and a listener that receives events instead of the
//! apps polling.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use mailcrane_core::events::{self, EventListener};
use mailcrane_core::secrets::SecretKey;
use mailcrane_core::sync::{
    live_stop, log_failure, run_live_updates, FolderSyncer, ImapLogin, LiveAccount, LiveConfig,
    LiveStop, SyncConfig, SyncError as CoreSyncError,
};
use mailcrane_imap::{
    save_folder_list, ImapAuth, ImapConfig, ImapError, ImapSecurity, Secret, Session,
};
use mailcrane_store::{AccountId, FolderId, FolderRole, MessageId, Security, Store};

/// Why a sync failed. Never carries server text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncErrorKind {
    /// The server could not be reached, or the connection dropped.
    Network,
    /// TLS failed, including an untrusted certificate.
    Tls,
    /// The server refused the credentials.
    Auth,
    /// The server sent something unexpected, or refused a command.
    Protocol,
    /// The server took too long.
    Timeout,
    /// The local database failed.
    Storage,
    /// The folder or message does not exist (any more).
    NotFound,
    /// The folder's `UIDVALIDITY` changed; sync the folder again.
    FolderReset,
    /// Another sync was writing to the local database and kept it busy.
    /// Local and passing: nothing is wrong with the server or the account,
    /// and the next sync works. Apps shouldn't show it as an error.
    Busy,
}

impl From<events::SyncErrorKind> for SyncErrorKind {
    fn from(kind: events::SyncErrorKind) -> Self {
        match kind {
            events::SyncErrorKind::Network => Self::Network,
            events::SyncErrorKind::Tls => Self::Tls,
            events::SyncErrorKind::Auth => Self::Auth,
            events::SyncErrorKind::Protocol => Self::Protocol,
            events::SyncErrorKind::Timeout => Self::Timeout,
            events::SyncErrorKind::Storage => Self::Storage,
            events::SyncErrorKind::NotFound => Self::NotFound,
            events::SyncErrorKind::FolderReset => Self::FolderReset,
            events::SyncErrorKind::Busy => Self::Busy,
        }
    }
}

/// Something changed. Re-read the data from the core.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreEvent {
    /// The folder itself changed (for example its cache was cleared).
    FolderChanged {
        /// Row id of the folder.
        folder_id: i64,
    },
    /// Messages in the folder were added or changed.
    MessagesChanged {
        /// Row id of the folder.
        folder_id: i64,
    },
    /// A sync failed. What finished before the failure is kept.
    SyncError {
        /// Row id of the account.
        account_id: i64,
        /// What kind of failure.
        kind: SyncErrorKind,
    },
    /// The server refused a message action for good. Its local change was
    /// undone (a `MessagesChanged` event comes with it); tell the user.
    ActionFailed {
        /// Row id of the account.
        account_id: i64,
        /// What was refused.
        action: crate::actions::ActionKind,
        /// Row ids of the messages it was applied to.
        message_ids: Vec<i64>,
        /// Why, without server text.
        reason: SyncErrorKind,
    },
}

/// A new message to notify about (M4-05). Holds only what a notification
/// shows; the app decides whether to show sender and subject at all.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewMailNotification {
    pub message_id: i64,
    pub account_id: i64,
    /// The account's address, to name its notification channel.
    pub account_email: String,
    /// The sender's display name, else their address, else empty.
    pub sender: String,
    pub subject: String,
    /// When it arrived on the server, unix seconds UTC.
    pub received_at: i64,
}

impl NewMailNotification {
    fn from_store(m: mailcrane_store::NewMail, account: &mailcrane_store::Account) -> Self {
        let sender = m
            .from_name
            .filter(|n| !n.trim().is_empty())
            .or(m.from_addr)
            .unwrap_or_default();
        Self {
            message_id: m.message_id.0,
            account_id: account.id.0,
            account_email: account.details.email.clone(),
            sender: crate::messages::display_line(&sender),
            subject: crate::messages::display_line(&m.subject),
            received_at: m.internal_date,
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Receives events, implemented in Kotlin and Swift. It may be called from
/// any thread and must not block.
#[uniffi::export(with_foreign)]
pub trait CoreEventListener: Send + Sync {
    /// Called for every event.
    fn on_event(&self, event: CoreEvent);
}

pub(crate) struct ListenerAdapter(pub(crate) Arc<dyn CoreEventListener>);

impl EventListener for ListenerAdapter {
    fn on_event(&self, event: events::CoreEvent) {
        self.0.on_event(event.into());
    }
}

/// A sync call failed. The message never includes server text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SyncFailure {
    /// The sync failed; `kind` says how.
    #[error("sync failed: {kind:?}")]
    Failed {
        /// What kind of failure.
        kind: SyncErrorKind,
    },
}

pub(crate) fn failed(kind: events::SyncErrorKind) -> SyncFailure {
    SyncFailure::Failed { kind: kind.into() }
}

impl From<CoreSyncError> for SyncFailure {
    fn from(error: CoreSyncError) -> Self {
        failed(error.kind())
    }
}

/// A running live-updates thread for one account.
struct LiveThread {
    stop: LiveStop,
    /// Receives once the thread's loop has returned.
    done: mpsc::Receiver<()>,
    thread: JoinHandle<()>,
    /// A stop was asked for but the thread had not finished in time. It is
    /// kept here so a new start waits for it instead of opening a second
    /// connection beside it.
    stopping: bool,
}

/// Syncs folders of the accounts in one database. Calls are blocking: call
/// them from a background thread or dispatcher. Calls for one account run
/// one at a time; different accounts run side by side, so a slow or hung
/// account doesn't hold up the others (M4-01). Live updates run on threads
/// of their own, with their own connections, and never wait for these calls.
#[derive(uniffi::Object)]
pub struct SyncEngine {
    db_path: String,
    pub(crate) listener: Arc<dyn CoreEventListener>,
    pub(crate) config: SyncConfig,
    live_config: LiveConfig,
    /// One lock per account, serialising that account's calls. Shared
    /// with live-update threads, which start background downloads.
    busy: Arc<Mutex<HashMap<i64, Arc<AccountGate>>>>,
    /// Background body downloads (M4-25).
    pub(crate) prefetch: Arc<Prefetcher>,
    /// One reused connection per account for opens and actions (#294).
    pub(crate) pool: Arc<ConnectionPool>,
    /// Live updates, by account id.
    live: Mutex<HashMap<i64, LiveThread>>,
    /// The platform's resolver, for the `_sieve._tcp` SRV record (M4-23).
    dns: Mutex<Option<Arc<dyn crate::autoconfig::PlatformDns>>>,
    #[cfg(test)]
    extra_root_cert: Mutex<Option<Vec<u8>>>,
    /// For each pooled attempt of an IMAP open, whether its job thread was
    /// marked urgent when the job started (#473: it mustn't be).
    #[cfg(test)]
    pub(crate) urgent_at_open: Mutex<Vec<bool>>,
}

#[uniffi::export]
impl SyncEngine {
    /// An engine for the database at `db_path`. Events go to `listener`.
    #[uniffi::constructor]
    pub fn new(db_path: String, listener: Arc<dyn CoreEventListener>) -> Arc<Self> {
        Arc::new(Self {
            db_path,
            listener,
            config: SyncConfig::default(),
            live_config: LiveConfig::default(),
            busy: Arc::new(Mutex::new(HashMap::new())),
            prefetch: Prefetcher::new(),
            pool: ConnectionPool::new(),
            live: Mutex::new(HashMap::new()),
            dns: Mutex::new(None),
            #[cfg(test)]
            extra_root_cert: Mutex::new(None),
            #[cfg(test)]
            urgent_at_open: Mutex::new(Vec::new()),
        })
    }

    /// Gives the engine the platform's DNS resolver, used to find the
    /// account's ManageSieve server through its `_sieve._tcp` SRV record
    /// (server-side rules, M4-23). Without one, only the IMAP host on port
    /// 4190 is tried.
    pub fn set_platform_dns(&self, dns: Option<Arc<dyn crate::autoconfig::PlatformDns>>) {
        *self.dns.lock().unwrap_or_else(|p| p.into_inner()) = dns;
    }

    /// Fetches headers for the newest messages of the folder (200 at first,
    /// in batches of 50, newest first). Returns how many were stored. Queued
    /// message actions of the account due by now are replayed first, so the
    /// server's flags already include them. After a successful sync of an
    /// inbox, its recent bodies are downloaded in the background when the
    /// device conditions allow it (M4-25, see `set_device_conditions`).
    pub fn sync_folder(&self, folder_id: i64) -> Result<u32, SyncFailure> {
        let result = self.sync_folder_now(folder_id);
        if result.is_ok() {
            self.prefetch_after_sync(FolderId(folder_id));
        }
        result
    }

    /// Fetches the next `count` messages older than the oldest stored one.
    /// Returns how many were stored.
    pub fn load_older(&self, folder_id: i64, count: u32) -> Result<u32, SyncFailure> {
        let account = self.folder_account(FolderId(folder_id))?;
        if self.is_local(account)? {
            return Ok(0);
        }
        let wanted = usize::try_from(count).unwrap_or(usize::MAX);
        if let Some(r) = self.run_jmap(account, |s| {
            Box::pin(async move { s.load_older(FolderId(folder_id), wanted).await })
        }) {
            return r.map(|n| u32::try_from(n).unwrap_or(u32::MAX));
        }
        let stored = self.run(FolderId(folder_id), |syncer| {
            Box::pin(async move {
                syncer
                    .load_older(
                        FolderId(folder_id),
                        usize::try_from(count).unwrap_or(usize::MAX),
                    )
                    .await
            })
        })?;
        Ok(u32::try_from(stored).unwrap_or(u32::MAX))
    }

    /// Downloads the message body once and keeps it for offline reading.
    /// Does not mark the message as read on the server.
    pub fn fetch_body(&self, message_id: i64) -> Result<(), SyncFailure> {
        let store = self.open_store()?;
        let header = store
            .get_message_header(MessageId(message_id))
            .map_err(|_| failed(events::SyncErrorKind::Storage))?
            .ok_or_else(|| failed(events::SyncErrorKind::NotFound))?;
        drop(store);
        // Already downloaded (opened before, or in the background, M4-25):
        // no connection at all.
        if header.body_state == mailcrane_store::BodyState::Fetched {
            return Ok(());
        }
        let folder = header.details.folder_id;
        let account = self.folder_account(folder)?;
        // Local-only messages are stored whole when they are imported.
        if self.is_local(account)? {
            return Ok(());
        }
        // From here on a background download gives way (M4-25).
        let gate = self.account_lock(account);
        let _waiting = gate.announce();
        // Opening never waits for a sync: it takes no writer lock, on IMAP
        // (issue #294) and JMAP alike (#472). Its only writes are the body's
        // own short transactions, which the core marks urgent just around the
        // save (`save_opened_body`), so they take the next turn for SQLite's
        // write lock without holding it up while connecting or downloading
        // (#473).
        if let Some(r) = self.run_jmap_open(account, |s| {
            Box::pin(async move { s.fetch_body(MessageId(message_id)).await })
        }) {
            return r;
        }
        // On the account's reused connection.
        let config = self.config;
        self.pooled(account, Retry::Idempotent, |store, session, listener| {
            // Tests: whether the thread starting the job (after waiting for
            // the connection, before downloading) was marked urgent; it
            // mustn't be (#473).
            #[cfg(test)]
            self.urgent_at_open
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(mailcrane_store::urgent_here(&self.db_path));
            Box::pin(async move {
                FolderSyncer::new(store, session, listener, account, config)
                    .fetch_body(MessageId(message_id))
                    .await
            })
        })
    }

    /// Marks the message as read, locally at once and queued for the
    /// server (M1-14); sends `MessagesChanged` for its folder when something
    /// changed. Doesn't wait for a running sync. The queue is replayed by
    /// `replay_actions` or the next sync.
    pub fn mark_read(&self, message_id: i64) -> Result<(), SyncFailure> {
        match self.apply_action(vec![message_id], crate::actions::MessageAction::MarkRead) {
            Ok(_) => Ok(()),
            Err(crate::actions::ActionFailure::NotFound) => {
                Err(failed(events::SyncErrorKind::NotFound))
            }
            Err(_) => Err(failed(events::SyncErrorKind::Storage)),
        }
    }

    /// Lists the account's folders on the server and saves them, keeping
    /// each folder's sync state. Returns the inbox's folder id, or `None` if
    /// the server has no inbox. On failure the saved folders are unchanged.
    pub fn refresh_folders(&self, account_id: i64) -> Result<Option<i64>, SyncFailure> {
        // Never on the caller's stack (#462).
        on_job_thread(|| self.refresh_folders_here(account_id))
    }
}

impl SyncEngine {
    /// [`Self::refresh_folders`], on the current thread.
    fn refresh_folders_here(&self, account_id: i64) -> Result<Option<i64>, SyncFailure> {
        let account = AccountId(account_id);
        if self.is_local(account)? {
            let store = self.open_store()?;
            return Ok(inbox_of(&store, account)?.map(|f| f.0));
        }
        if let Some(r) = self.run_jmap(account, |s| {
            Box::pin(async move { s.refresh_mailboxes().await })
        }) {
            return r.map(|inbox| inbox.map(|f| f.0));
        }
        let lock = self.account_lock(account);
        let _guard = lock.lock();
        let store = self.open_store()?;
        let report = |kind| self.report(account, kind);
        let runtime = runtime().map_err(report)?;
        runtime.block_on(async {
            let mut session = self.connect(&store, account).await.map_err(report)?;
            let listed = session.list_folders().await;
            let command = session.last_command();
            session.logout().await;
            let folders = listed.map_err(|e| {
                let e = CoreSyncError::from(e);
                log_failure("list folders", &e, command);
                report(e.kind())
            })?;
            let saved = save_folder_list(&store, account, &folders).map_err(|e| {
                let e = CoreSyncError::from(e);
                log_failure("save folders", &e, None);
                report(e.kind())
            })?;
            // Folders gone from the server (#323): the apps re-read the list
            // and leave a removed folder that was open.
            for folder in saved.removed {
                self.listener
                    .on_event(events::CoreEvent::FolderChanged { folder_id: folder }.into());
            }
            Ok::<(), SyncFailure>(())
        })?;
        Ok(inbox_of(&store, account)?.map(|f| f.0))
    }
}

#[uniffi::export]
impl SyncEngine {
    /// The ids of every account in the database, for starting and stopping
    /// live updates.
    pub fn account_ids(&self) -> Result<Vec<i64>, SyncFailure> {
        let store = self.open_store()?;
        let accounts = store
            .list_accounts()
            .map_err(|_| failed(events::SyncErrorKind::Storage))?;
        Ok(accounts.into_iter().map(|a| a.id.0).collect())
    }

    /// Starts keeping the account's inbox up to date while the app is open:
    /// IMAP IDLE on a connection of its own, or polling every 5 minutes if
    /// the server has no IDLE. Changes arrive as `MessagesChanged` events;
    /// failures as `SyncError` events, retried with back-off. Returns at
    /// once. Calling it again while it runs does nothing.
    pub fn start_live_updates(&self, account_id: i64) -> Result<(), SyncFailure> {
        // JMAP push (EventSource) is a later issue; JMAP accounts are kept
        // up to date by the app's syncs and the periodic check until then.
        if self.is_jmap(AccountId(account_id)) || self.is_local(AccountId(account_id))? {
            return Ok(());
        }
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(running) = live.get(&account_id) {
            if !running.thread.is_finished() {
                if !running.stopping {
                    return Ok(());
                }
                // A stop that ran over its limit: let that thread finish
                // first, so the account never has two connections.
                if let Err(mpsc::RecvTimeoutError::Timeout) =
                    running.done.recv_timeout(self.stop_limit())
                {
                    return Err(failed(events::SyncErrorKind::Timeout));
                }
            }
        }
        let store = self.open_store()?;
        let account = AccountId(account_id);
        store
            .get_account(account)
            .map_err(|_| failed(events::SyncErrorKind::Storage))?
            .ok_or_else(|| failed(events::SyncErrorKind::NotFound))?;
        drop(store);

        let (stop, signal) = live_stop();
        let (done_tx, done) = mpsc::channel();
        let db_path = self.db_path.clone();
        // Inbox changes found by live updates start background downloads.
        let listener = PrefetchOnChange {
            inner: ListenerAdapter(self.listener.clone()),
            prefetcher: self.prefetch.clone(),
            locks: self.busy.clone(),
            pool: self.pool.clone(),
            db_path: self.db_path.clone(),
            extra_root_cert: self.extra_root_cert(),
            account,
            sync_config: self.config,
        };
        let (sync_config, live_config) = (self.config, self.live_config);
        let extra_root_cert = self.extra_root_cert();
        let write_lock = self.account_lock(account).write_lock();
        let thread = crate::worker::job_thread(format!("mailcrane-live-{account_id}"), move || {
            let report = |kind| {
                listener.on_event(events::CoreEvent::SyncError {
                    account_id: account,
                    kind,
                })
            };
            let runtime = match runtime() {
                Ok(runtime) => runtime,
                Err(kind) => return report(kind),
            };
            let store = match Store::open(&db_path) {
                Ok(store) => store,
                Err(_) => return report(events::SyncErrorKind::Storage),
            };
            let login = || imap_login(&store, account, extra_root_cert.as_deref());
            runtime.block_on(run_live_updates(
                &store,
                LiveAccount {
                    id: account,
                    write_lock,
                },
                &listener,
                login,
                sync_config,
                live_config,
                signal,
            ));
            let _ = done_tx.send(());
        })
        .map_err(|_| failed(events::SyncErrorKind::Network))?;
        // A finished thread for this account (it could not open the
        // database, say) is replaced.
        live.insert(
            account_id,
            LiveThread {
                stop,
                done,
                thread,
                stopping: false,
            },
        );
        Ok(())
    }

    /// Stops live updates for the account: ends IDLE and logs out, within
    /// about 2 seconds. Does nothing if they are not running.
    pub fn stop_live_updates(&self, account_id: i64) {
        // The lock is held while waiting, so a start for this account can't
        // slip in beside a thread that is still closing its connection.
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        let Some(running) = live.get_mut(&account_id) else {
            return;
        };
        running.stop.stop();
        running.stopping = true;
        // The loop gives DONE and LOGOUT `stop_within`; allow a little more
        // for the thread to finish, but never block forever.
        let waited = running.done.recv_timeout(self.stop_limit());
        if let Err(mpsc::RecvTimeoutError::Timeout) = waited {
            // Kept, marked as stopping: the next start waits for it.
            return;
        }
        if let Some(finished) = live.remove(&account_id) {
            let _ = finished.thread.join();
        }
    }

    /// Tells every running live update to drop its connection and connect
    /// again now, skipping any back-off wait (M4-05). Call it when the
    /// device's default network changes or comes back: an IDLE connection
    /// on the old network would otherwise sit unnoticed until it is
    /// re-issued. Returns at once; does nothing for accounts without live
    /// updates or ones being stopped.
    pub fn reconnect_live_updates(&self) {
        let live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        for running in live.values() {
            if !running.stopping && !running.thread.is_finished() {
                running.stop.reconnect();
            }
        }
    }

    /// New mail to notify about (M4-05), from what is stored now; no network.
    /// Each message is returned at most once, ever: the store remembers it,
    /// also across resyncs. The first call for an account returns nothing
    /// and only records what is already in its inbox.
    pub fn take_new_mail(&self) -> Result<Vec<NewMailNotification>, SyncFailure> {
        let store = self.open_store()?;
        let accounts = store
            .list_accounts()
            .map_err(|_| failed(events::SyncErrorKind::Storage))?;
        let now = unix_now();
        let mut out = Vec::new();
        // A local-only account gets no notifications: its mail is imported,
        // not new.
        for account in &accounts {
            if store
                .is_local_account(account.id)
                .map_err(|_| failed(events::SyncErrorKind::Storage))?
            {
                continue;
            }
            let found = mailcrane_core::notify::take_new_mail(&store, account, now)
                .map_err(|_| failed(events::SyncErrorKind::Storage))?;
            out.extend(
                found
                    .into_iter()
                    .map(|m| NewMailNotification::from_store(m, account)),
            );
        }
        Ok(out)
    }

    /// Syncs every account's inbox, stopping before `deadline_seconds` have
    /// passed, then returns the new mail to notify about ([`take_new_mail`]).
    /// For the periodic background check. An account that fails is skipped
    /// (its error goes to the listener as usual); the others still sync.
    /// Blocking.
    ///
    /// [`take_new_mail`]: Self::take_new_mail
    pub fn background_sync(
        &self,
        deadline_seconds: u32,
    ) -> Result<Vec<NewMailNotification>, SyncFailure> {
        let started = std::time::Instant::now();
        let deadline = Duration::from_secs(u64::from(deadline_seconds));
        let store = self.open_store()?;
        let accounts = store
            .list_accounts()
            .map_err(|_| failed(events::SyncErrorKind::Storage))?;
        for account in &accounts {
            if started.elapsed() >= deadline {
                break;
            }
            match store.is_local_account(account.id) {
                Ok(false) => {}
                Ok(true) => continue,
                // Never connect without knowing; report it like any other
                // account failure and carry on with the others.
                Err(_) => {
                    let _ = self.report(account.id, events::SyncErrorKind::Storage);
                    continue;
                }
            }
            let inbox = match inbox_of(&store, account.id) {
                Ok(Some(folder)) => Some(folder.0),
                _ => self.refresh_folders(account.id.0).ok().flatten(),
            };
            if let Some(folder) = inbox {
                // Errors are reported to the listener; carry on with the others.
                let _ = self.sync_folder(folder);
            }
        }
        drop(store);
        // Give the background downloads the syncs started a little time, so
        // a message opened from a notification is already there (M4-25).
        let left = deadline.saturating_sub(started.elapsed());
        self.prefetch.wait(left.min(BACKGROUND_PREFETCH_WAIT));
        self.take_new_mail()
    }

    /// How long a stop waits for the live thread to finish.
    fn stop_limit(&self) -> Duration {
        self.live_config.stop_within + Duration::from_millis(500)
    }
}

impl Drop for SyncEngine {
    fn drop(&mut self) {
        // Dropping each `LiveStop` tells its thread to stop.
        self.live.lock().unwrap_or_else(|p| p.into_inner()).clear();
        // Background downloads stop after their current batch.
        self.prefetch.stop();
        // Reused connections are dropped with the engine.
        self.pool.shut_down();
    }
}

pub(crate) type SyncFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, CoreSyncError>> + 'a>>;

impl SyncEngine {
    /// Whether the account is local-only (#289): it never syncs and never
    /// connects. A marker that can't be read is a storage failure, returned
    /// to the caller, so it neither connects nor hides the error.
    pub(crate) fn is_local(&self, account: AccountId) -> Result<bool, SyncFailure> {
        self.open_store()?
            .is_local_account(account)
            .map_err(|_| failed(events::SyncErrorKind::Storage))
    }

    /// [`SyncEngine::sync_folder`] without the background downloads.
    fn sync_folder_now(&self, folder_id: i64) -> Result<u32, SyncFailure> {
        let account = self.folder_account(FolderId(folder_id))?;
        // A local-only account (#289) has no server: nothing to fetch.
        if self.is_local(account)? {
            return Ok(0);
        }
        if let Some(r) = self.run_jmap(account, |s| {
            Box::pin(async move {
                // Queued actions first, so the server's flags already include them.
                let _ = s.replay_actions(crate::actions::now()).await;
                s.sync_folder(FolderId(folder_id)).await
            })
        }) {
            return r.map(|n| u32::try_from(n).unwrap_or(u32::MAX));
        }
        let stored = self.run_replaying(FolderId(folder_id), true, |syncer| {
            Box::pin(async move { syncer.sync_folder(FolderId(folder_id)).await })
        })?;
        Ok(u32::try_from(stored).unwrap_or(u32::MAX))
    }

    /// Starts background downloads (M4-25) if `folder` is its account's
    /// inbox. Never fails: anything that goes wrong only means no download.
    fn prefetch_after_sync(&self, folder: FolderId) {
        let Ok(store) = self.open_store() else {
            return;
        };
        let Ok(Some(row)) = store.get_folder(folder) else {
            return;
        };
        let account = row.details.account_id;
        // A local-only account (#289) has nothing to download.
        if !matches!(store.is_local_account(account), Ok(false)) {
            return;
        }
        if inbox_of(&store, account).ok().flatten() != Some(folder) {
            return;
        }
        drop(store);
        self.prefetch.start(PrefetchJob {
            db_path: self.db_path.clone(),
            listener: self.listener.clone(),
            extra_root_cert: self.extra_root_cert(),
            gate: self.account_lock(account),
            pool: self.pool.clone(),
            account,
            folder,
            sync_config: self.config,
        });
    }

    /// The lock serialising `account`'s calls.
    pub(crate) fn account_lock(&self, account: AccountId) -> Arc<AccountGate> {
        gate_for(&self.busy, account)
    }

    pub(crate) fn open_store(&self) -> Result<Store, SyncFailure> {
        Store::open(&self.db_path).map_err(|_| failed(events::SyncErrorKind::Storage))
    }

    /// Connects for the folder's account and runs `job` with a syncer.
    pub(crate) fn run<T: Send>(
        &self,
        folder: FolderId,
        job: impl for<'a> FnOnce(&'a mut FolderSyncer<'_>) -> SyncFuture<'a, T> + Send,
    ) -> Result<T, SyncFailure> {
        self.run_replaying(folder, false, job)
    }

    /// [`Self::run`], replaying the account's queued actions first when
    /// `replay` is set. A replay that fails only defers the actions; the
    /// sync goes ahead.
    fn run_replaying<T: Send>(
        &self,
        folder: FolderId,
        replay: bool,
        job: impl for<'a> FnOnce(&'a mut FolderSyncer<'_>) -> SyncFuture<'a, T> + Send,
    ) -> Result<T, SyncFailure> {
        // Never on the caller's stack (#462).
        on_job_thread(|| self.run_replaying_here(folder, replay, job))
    }

    /// [`Self::run_replaying`], on the current thread.
    fn run_replaying_here<T>(
        &self,
        folder: FolderId,
        replay: bool,
        job: impl for<'a> FnOnce(&'a mut FolderSyncer<'_>) -> SyncFuture<'a, T>,
    ) -> Result<T, SyncFailure> {
        let store = self.open_store()?;
        let account = store
            .get_folder(folder)
            .map_err(|_| failed(events::SyncErrorKind::Storage))?
            .ok_or_else(|| failed(events::SyncErrorKind::NotFound))?
            .details
            .account_id;
        // A poisoned lock only means an earlier call panicked; nothing in it
        // is left half-done.
        let lock = self.account_lock(account);
        let _guard = lock.lock();
        let listener = ListenerAdapter(self.listener.clone());
        let report = |kind| self.report(account, kind);
        let runtime = runtime().map_err(report)?;
        runtime.block_on(async {
            let mut session = self.connect(&store, account).await.map_err(report)?;
            if replay {
                // Only a local database failure comes back as an error, and
                // the sync reports that itself; actions that failed against
                // the server are already deferred or undone.
                let _ = mailcrane_core::actions::replay(
                    &store,
                    &mut session,
                    &listener,
                    account,
                    crate::actions::now(),
                )
                .await;
                // Drafts too (M2-05); a failed upload is deferred, not lost.
                let _ = mailcrane_core::drafts::upload_drafts(
                    &store,
                    &mut session,
                    account,
                    crate::actions::now(),
                )
                .await;
                // And copies of sent messages for the Sent folder (M2-07).
                let _ = mailcrane_core::outbox::save_sent_copies(
                    &store,
                    &mut session,
                    account,
                    crate::actions::now(),
                )
                .await;
                // And snoozes to carry out (M4-06).
                let _ = mailcrane_core::snooze::sync_snoozes(
                    &store,
                    &mut session,
                    &listener,
                    account,
                    crate::actions::now(),
                )
                .await;
            }
            // The syncer borrows the session; end that borrow before logout.
            let result = {
                let mut syncer =
                    FolderSyncer::new(&store, &mut session, &listener, account, self.config);
                job(&mut syncer).await
            };
            // Server-side rules (M4-23): bring the account's ManageSieve
            // script up to date when it's time (usually it isn't, and nothing
            // connects). Before local rules run, so they skip what the server
            // runs. A failure leaves the server and the device as they were.
            if result.is_ok() && replay {
                if let Some(login) = sieve_login(&store, account, self.extra_root_cert().as_deref())
                {
                    let dns = crate::autoconfig::ForeignDns(
                        self.dns.lock().unwrap_or_else(|p| p.into_inner()).clone(),
                    );
                    let _ = mailcrane_core::rules::server::sync(
                        &store,
                        account,
                        &login,
                        &dns,
                        crate::actions::now(),
                    )
                    .await;
                }
            }
            // Rules on new inbox mail (M4-09), then their actions go out in
            // this session. A failure only leaves them for the next sync.
            if result.is_ok() {
                let now = crate::actions::now();
                if mailcrane_core::rules::apply_new(&store, account, now, &listener).is_ok() {
                    let _ = mailcrane_core::actions::replay(
                        &store,
                        &mut session,
                        &listener,
                        account,
                        now,
                    )
                    .await;
                }
            }
            session.logout().await;
            // New headers join their conversations (M4-02). A failure here
            // only leaves them unthreaded until the next sync.
            if result.is_ok() {
                let _ = mailcrane_core::threading::assign_new(&store, Some(account));
            }
            result.map_err(SyncFailure::from)
        })
    }

    /// Runs `job` with the store, the account's reused connection and the
    /// listener, holding the account's writer lock: for work that changes
    /// many rows (action replay, drafts, sent copies, snoozes, outbox
    /// recovery), so it never writes beside a sync. Never retried on a new
    /// connection: the jobs change things on the server and defer or undo
    /// what failed themselves.
    pub(crate) fn with_session<T: Send>(
        &self,
        account: AccountId,
        job: impl for<'a> FnOnce(&'a Store, &'a mut Session, &'a dyn EventListener) -> SyncFuture<'a, T>
            + Send,
    ) -> Result<T, SyncFailure> {
        // Lock order: WriteLock, then the pooled connection (`pooled`).
        let lock = self.account_lock(account);
        let _guard = lock.lock();
        let mut job = Some(job);
        self.pooled(account, Retry::Never, move |store, session, listener| {
            match job.take() {
                Some(job) => job(store, session, listener),
                // Unreachable with `Retry::Never`; fail rather than panic.
                None => Box::pin(async { Err(CoreSyncError::Imap(ImapError::ConnectionClosed)) }),
            }
        })
    }

    /// Runs `job` on the account's reused connection (issue #294) with the
    /// store and a listener. `SyncError` events from inside the job are held
    /// back, since a failed attempt may be retried; the final failure is
    /// reported once. Takes no writer lock itself. Runs on a thread with a
    /// stack this crate sets (#462), never on the caller's.
    pub(crate) fn pooled<T: Send>(
        &self,
        account: AccountId,
        retry: Retry,
        mut job: impl for<'a> FnMut(&'a Store, &'a mut Session, &'a dyn EventListener) -> SyncFuture<'a, T>
            + Send,
    ) -> Result<T, SyncFailure> {
        on_job_thread(|| {
            let ctx = (
                self.open_store()?,
                HoldErrors(ListenerAdapter(self.listener.clone())),
            );
            let extra_root_cert = self.extra_root_cert();
            let login = || imap_login(&ctx.0, account, extra_root_cert.as_deref());
            self.pool
                .run(account, login, retry, &ctx, |session, (store, listener)| {
                    job(store, session, listener)
                })
                .map_err(|e| self.report(account, e.kind()))
        })
    }

    /// Sends a `SyncError` event and returns the matching failure.
    fn report(&self, account: AccountId, kind: events::SyncErrorKind) -> SyncFailure {
        self.listener.on_event(
            events::CoreEvent::SyncError {
                account_id: account,
                kind,
            }
            .into(),
        );
        failed(kind)
    }

    /// The test root certificate, if one was set.
    fn extra_root_cert(&self) -> Option<Vec<u8>> {
        #[cfg(test)]
        {
            self.extra_root_cert
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    async fn connect(
        &self,
        store: &Store,
        account: AccountId,
    ) -> Result<Session, events::SyncErrorKind> {
        // An OAuth account (M3-02) gets a fresh access token first.
        if let Ok(secrets) = crate::secrets::secret_store() {
            match mailcrane_core::account::credentials::refresh_if_needed(
                store,
                secrets.as_ref(),
                account,
                &[],
                OAUTH_TIMEOUT,
            )
            .await
            {
                Err(mailcrane_core::account::credentials::CredentialError::ReauthRequired) => {
                    return Err(events::SyncErrorKind::Auth)
                }
                Err(mailcrane_core::account::credentials::CredentialError::Storage) => {
                    return Err(events::SyncErrorKind::Storage)
                }
                // Not reachable now: try the cached token.
                Err(mailcrane_core::account::credentials::CredentialError::Missing) | Ok(()) => {}
            }
        }
        let login = imap_login(store, account, self.extra_root_cert().as_deref())?;
        Session::connect_auth(&login.config, &login.username, &login.auth)
            .await
            .map_err(|e: ImapError| {
                let e = CoreSyncError::from(e);
                log_failure("connect", &e, None);
                e.kind()
            })
    }
}

/// Runs `work` on a thread with a stack this crate sets (#462), never on the
/// caller's, whose size the app chooses. A thread that couldn't start, or
/// work that panicked (in the app; tests let the panic through), comes back
/// as `Network`, the kind `runtime` gives too. Only the call's result says
/// so: no `SyncError` event is sent, since this runs before the job knows
/// its account. `worker::on_job_thread` logs which of the two it was.
pub(crate) fn on_job_thread<T: Send>(
    work: impl FnOnce() -> Result<T, SyncFailure> + Send,
) -> Result<T, SyncFailure> {
    crate::worker::on_job_thread(work).unwrap_or(Err(failed(events::SyncErrorKind::Network)))
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, events::SyncErrorKind> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| events::SyncErrorKind::Network)
}

/// The account's inbox: the folder with the inbox role, otherwise the one
/// named `INBOX` in any case (RFC 9051 §5.1).
fn inbox_of(store: &Store, account: AccountId) -> Result<Option<FolderId>, SyncFailure> {
    let folders = store
        .list_folders(account)
        .map_err(|_| failed(events::SyncErrorKind::Storage))?;
    let by_role = folders
        .iter()
        .find(|f| f.details.role == Some(FolderRole::Inbox));
    let by_name = || {
        folders
            .iter()
            .find(|f| f.details.remote_name.eq_ignore_ascii_case("INBOX"))
    };
    Ok(by_role.or_else(by_name).map(|f| f.id))
}

/// The account's IMAP server and credentials, from the store and the
/// platform's secret store. `extra_root_cert` is only ever set by tests.
fn imap_login(
    store: &Store,
    account: AccountId,
    extra_root_cert: Option<&[u8]>,
) -> Result<ImapLogin, events::SyncErrorKind> {
    let row = store
        .get_account(account)
        .map_err(|_| events::SyncErrorKind::Storage)?
        .ok_or(events::SyncErrorKind::NotFound)?;
    // Never connect for a local-only account (#289), whatever path got here.
    // An unreadable marker is a storage failure: no connection either.
    if store
        .is_local_account(account)
        .map_err(|_| events::SyncErrorKind::Storage)?
    {
        return Err(events::SyncErrorKind::NotFound);
    }
    let secrets = crate::secrets::secret_store().map_err(|_| events::SyncErrorKind::Auth)?;
    // Outside an async runtime (the connection pool), an OAuth account's
    // token is refreshed here; inside one, the async paths did it first.
    if tokio::runtime::Handle::try_current().is_err() {
        if let Ok(rt) = runtime() {
            let _ = rt.block_on(mailcrane_core::account::credentials::refresh_if_needed(
                store,
                secrets.as_ref(),
                account,
                &[],
                OAUTH_TIMEOUT,
            ));
        }
    }
    use mailcrane_core::account::credentials::{credential, Credential, CredentialError};
    let auth = match credential(store, secrets.as_ref(), account) {
        Ok(Credential::Password(p)) => ImapAuth::Password(Secret::new(p)),
        Ok(Credential::OAuth(token)) => ImapAuth::XOAuth2(Secret::new(token.as_str())),
        Err(CredentialError::Storage) => return Err(events::SyncErrorKind::Storage),
        Err(_) => return Err(events::SyncErrorKind::Auth),
    };
    let security = match row.details.imap_security {
        Security::Tls => ImapSecurity::ImplicitTls,
        Security::Starttls => ImapSecurity::StartTls,
    };
    let mut config = ImapConfig::new(row.details.imap_host, row.details.imap_port, security);
    if let Some(der) = extra_root_cert {
        config = config.with_extra_root_cert(der.to_vec());
    }
    Ok(ImapLogin {
        config,
        username: row.details.username,
        auth,
    })
}

/// How long refreshing an OAuth access token may take (M3-02).
const OAUTH_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the background check waits for background downloads before
/// returning its notifications.
const BACKGROUND_PREFETCH_WAIT: Duration = Duration::from_secs(30);

/// How to log in to the account's ManageSieve server: the IMAP username
/// and password (RFC 5804 servers normally share them). `None` for an
/// account without a stored password (OAuth), which keeps rules local.
fn sieve_login(
    store: &Store,
    account: AccountId,
    extra_root_cert: Option<&[u8]>,
) -> Option<mailcrane_core::rules::server::SieveLogin> {
    // Never for a local-only account (#289); an unreadable marker counts as
    // local, so nothing connects.
    if store.is_local_account(account).unwrap_or(true) {
        return None;
    }
    let row = store.get_account(account).ok()??;
    let password = crate::secrets::secret_store()
        .and_then(|s| s.get(&SecretKey::account_password(account.0)))
        .ok()??;
    let password = std::str::from_utf8(password.expose()).ok()?.to_owned();
    let domain = row.details.email.rsplit_once('@')?.1.to_ascii_lowercase();
    Some(mailcrane_core::rules::server::SieveLogin {
        username: row.details.username,
        password: mailcrane_core::rules::server::Secret::new(password),
        imap_host: row.details.imap_host,
        email_domain: domain,
        extra_root_cert: extra_root_cert.map(<[u8]>::to_vec),
        port_override: None,
    })
}

mod folder_admin;
mod jmap;
mod pool;
mod prefetch;
mod push;

pub(crate) use jmap::jmap_auth;
pub use pool::CloseReason;
pub(crate) use pool::{ConnectionPool, Retry};
pub use push::{classify_push, PushBody};

/// Passes events on, except `SyncError`: inside a pooled job a failure may
/// still be retried, and the final one is reported by the caller.
pub(crate) struct HoldErrors(pub(crate) ListenerAdapter);

impl EventListener for HoldErrors {
    fn on_event(&self, event: events::CoreEvent) {
        if !matches!(event, events::CoreEvent::SyncError { .. }) {
            self.0.on_event(event);
        }
    }
}

pub(crate) use prefetch::{gate_for, AccountGate, PrefetchJob, PrefetchOnChange, Prefetcher};
pub use prefetch::{BackgroundDownload, DeviceConditions};

#[cfg(test)]
mod pool_tests;
#[cfg(test)]
mod prefetch_tests;
#[cfg(test)]
mod push_tests;
#[cfg(test)]
mod stack_tests;
#[cfg(test)]
mod tests;
