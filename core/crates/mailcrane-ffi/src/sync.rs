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
    live_stop, run_live_updates, FolderSyncer, ImapLogin, LiveConfig, LiveStop, SyncConfig,
    SyncError as CoreSyncError,
};
use mailcrane_imap::{save_folders, ImapConfig, ImapError, ImapSecurity, Secret, Session};
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
}

impl From<events::CoreEvent> for CoreEvent {
    fn from(event: events::CoreEvent) -> Self {
        match event {
            events::CoreEvent::FolderChanged { folder_id } => Self::FolderChanged {
                folder_id: folder_id.0,
            },
            events::CoreEvent::MessagesChanged { folder_id } => Self::MessagesChanged {
                folder_id: folder_id.0,
            },
            events::CoreEvent::SyncError { account_id, kind } => Self::SyncError {
                account_id: account_id.0,
                kind: kind.into(),
            },
        }
    }
}

/// Receives events, implemented in Kotlin and Swift. It may be called from
/// any thread and must not block.
#[uniffi::export(with_foreign)]
pub trait CoreEventListener: Send + Sync {
    /// Called for every event.
    fn on_event(&self, event: CoreEvent);
}

struct ListenerAdapter(Arc<dyn CoreEventListener>);

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

fn failed(kind: events::SyncErrorKind) -> SyncFailure {
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
}

/// Syncs folders of the accounts in one database. Calls are blocking: call
/// them from a background thread or dispatcher. Only one runs at a time, so
/// only one folder syncs at a time. Live updates run on threads of their
/// own, with their own connections, and never wait for these calls.
#[derive(uniffi::Object)]
pub struct SyncEngine {
    db_path: String,
    listener: Arc<dyn CoreEventListener>,
    config: SyncConfig,
    live_config: LiveConfig,
    /// Serialises sync calls.
    busy: Mutex<()>,
    /// Live updates, by account id.
    live: Mutex<HashMap<i64, LiveThread>>,
    #[cfg(test)]
    extra_root_cert: Mutex<Option<Vec<u8>>>,
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
            busy: Mutex::new(()),
            live: Mutex::new(HashMap::new()),
            #[cfg(test)]
            extra_root_cert: Mutex::new(None),
        })
    }

    /// Fetches headers for the newest messages of the folder (200 at first,
    /// in batches of 50, newest first). Returns how many were stored.
    pub fn sync_folder(&self, folder_id: i64) -> Result<u32, SyncFailure> {
        let stored = self.run(FolderId(folder_id), |syncer| {
            Box::pin(async move { syncer.sync_folder(FolderId(folder_id)).await })
        })?;
        Ok(u32::try_from(stored).unwrap_or(u32::MAX))
    }

    /// Fetches the next `count` messages older than the oldest stored one.
    /// Returns how many were stored.
    pub fn load_older(&self, folder_id: i64, count: u32) -> Result<u32, SyncFailure> {
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
        let folder = store
            .get_message_header(MessageId(message_id))
            .map_err(|_| failed(events::SyncErrorKind::Storage))?
            .ok_or_else(|| failed(events::SyncErrorKind::NotFound))?
            .details
            .folder_id;
        drop(store);
        self.run(folder, |syncer| {
            Box::pin(async move { syncer.fetch_body(MessageId(message_id)).await })
        })
    }

    /// Marks the message as read. For now this changes only the local copy
    /// (until read state is synced back to the server, M1-14), so a later
    /// sync that reports the server's flags can undo it. Sends
    /// `MessagesChanged` for its folder when something changed. Doesn't wait
    /// for a running sync.
    pub fn mark_read(&self, message_id: i64) -> Result<(), SyncFailure> {
        let store = self.open_store()?;
        let id = MessageId(message_id);
        let header = store
            .get_message_header(id)
            .map_err(|_| failed(events::SyncErrorKind::Storage))?
            .ok_or_else(|| failed(events::SyncErrorKind::NotFound))?;
        let mut flags = header.details.flags;
        if flags.seen {
            return Ok(());
        }
        flags.seen = true;
        let changed = store
            .set_flags(id, flags)
            .map_err(|_| failed(events::SyncErrorKind::Storage))?;
        if !changed {
            return Err(failed(events::SyncErrorKind::NotFound));
        }
        self.listener.on_event(CoreEvent::MessagesChanged {
            folder_id: header.details.folder_id.0,
        });
        Ok(())
    }

    /// Lists the account's folders on the server and saves them, keeping
    /// each folder's sync state. Returns the inbox's folder id, or `None` if
    /// the server has no inbox. On failure the saved folders are unchanged.
    pub fn refresh_folders(&self, account_id: i64) -> Result<Option<i64>, SyncFailure> {
        let _guard = self.busy.lock().unwrap_or_else(|p| p.into_inner());
        let account = AccountId(account_id);
        let store = self.open_store()?;
        let report = |kind| self.report(account, kind);
        let runtime = runtime().map_err(report)?;
        runtime.block_on(async {
            let mut session = self.connect(&store, account).await.map_err(report)?;
            let listed = session.list_folders().await;
            session.logout().await;
            let folders = listed.map_err(|e| report(CoreSyncError::from(e).kind()))?;
            save_folders(&store, account, &folders)
                .map_err(|e| report(CoreSyncError::from(e).kind()))?;
            Ok::<(), SyncFailure>(())
        })?;
        Ok(inbox_of(&store, account)?.map(|f| f.0))
    }

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
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        if live
            .get(&account_id)
            .is_some_and(|running| !running.thread.is_finished())
        {
            return Ok(());
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
        let listener = ListenerAdapter(self.listener.clone());
        let (sync_config, live_config) = (self.config, self.live_config);
        let extra_root_cert = self.extra_root_cert();
        let thread = std::thread::Builder::new()
            .name(format!("mailcrane-live-{account_id}"))
            .spawn(move || {
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
                    account,
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
        live.insert(account_id, LiveThread { stop, done, thread });
        Ok(())
    }

    /// Stops live updates for the account: ends IDLE and logs out, within
    /// about 2 seconds. Does nothing if they are not running.
    pub fn stop_live_updates(&self, account_id: i64) {
        let running = self
            .live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&account_id);
        let Some(running) = running else {
            return;
        };
        running.stop.stop();
        // The loop gives DONE and LOGOUT `stop_within`; allow a little more
        // for the thread to finish, but never block forever.
        let limit = self.live_config.stop_within + Duration::from_millis(500);
        match running.done.recv_timeout(limit) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = running.thread.join();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

impl Drop for SyncEngine {
    fn drop(&mut self) {
        // Dropping each `LiveStop` tells its thread to stop.
        self.live.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
}

type SyncFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, CoreSyncError>> + 'a>>;

impl SyncEngine {
    fn open_store(&self) -> Result<Store, SyncFailure> {
        Store::open(&self.db_path).map_err(|_| failed(events::SyncErrorKind::Storage))
    }

    /// Connects for the folder's account and runs `job` with a syncer.
    fn run<T>(
        &self,
        folder: FolderId,
        job: impl for<'a> FnOnce(&'a mut FolderSyncer<'_>) -> SyncFuture<'a, T>,
    ) -> Result<T, SyncFailure> {
        // A poisoned lock only means an earlier call panicked; nothing in it
        // is left half-done.
        let _guard = self.busy.lock().unwrap_or_else(|p| p.into_inner());
        let store = self.open_store()?;
        let account = store
            .get_folder(folder)
            .map_err(|_| failed(events::SyncErrorKind::Storage))?
            .ok_or_else(|| failed(events::SyncErrorKind::NotFound))?
            .details
            .account_id;
        let listener = ListenerAdapter(self.listener.clone());
        let report = |kind| self.report(account, kind);
        let runtime = runtime().map_err(report)?;
        runtime.block_on(async {
            let mut session = self.connect(&store, account).await.map_err(report)?;
            // The syncer borrows the session; end that borrow before logout.
            let result = {
                let mut syncer =
                    FolderSyncer::new(&store, &mut session, &listener, account, self.config);
                job(&mut syncer).await
            };
            session.logout().await;
            result.map_err(SyncFailure::from)
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
        let login = imap_login(store, account, self.extra_root_cert().as_deref())?;
        Session::connect(&login.config, &login.username, &login.password)
            .await
            .map_err(|e: ImapError| CoreSyncError::from(e).kind())
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, events::SyncErrorKind> {
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
    let password = crate::secrets::secret_store()
        .and_then(|s| s.get(&SecretKey::account_password(account.0)))
        .map_err(|_| events::SyncErrorKind::Auth)?
        .ok_or(events::SyncErrorKind::Auth)?;
    let password = std::str::from_utf8(password.expose())
        .map_err(|_| events::SyncErrorKind::Auth)?
        .to_owned();
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
        password: Secret::new(password),
    })
}

#[cfg(test)]
mod tests;
