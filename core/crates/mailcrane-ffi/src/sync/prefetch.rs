//! Downloading recent messages' bodies in the background (M4-25), so most
//! messages open at once and offline.
//!
//! After a successful sync of an account's inbox (by `sync_folder`, the
//! background check, or live updates), a thread of its own downloads the
//! inbox's recent bodies in batches over a connection of its own. It holds
//! the account's lock only for one batch at a time, and before each batch
//! it gives way to any call waiting for that lock, such as opening a
//! message: that call waits for at most the batch in progress.
//!
//! When it runs comes from what the app reports
//! ([`SyncEngine::set_device_conditions`]); until then nothing is
//! downloaded. Failures are logged with the safe fields of `log_failure`
//! and never reported to the user: the next sync carries on.

use super::pool::PoolError;
use super::*;
use mailcrane_core::sync::prefetch::{
    self, Candidate, PrefetchConditions, PrefetchLimits, PrefetchMode,
};
use mailcrane_core::sync::{JmapSyncer, WriteLock};
use mailcrane_store::Protocol;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The "Download messages in the background" setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BackgroundDownload {
    /// Only on Wi-Fi or another unmetered network. The default.
    WifiOnly,
    /// Also on mobile data.
    Always,
    /// Never.
    Never,
}

/// What the app knows about the device, for deciding whether to download
/// bodies in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct DeviceConditions {
    /// The user's setting.
    pub setting: BackgroundDownload,
    /// Whether the current network is unmetered (Wi-Fi, Ethernet).
    pub unmetered: bool,
    /// Whether Battery Saver is on.
    pub battery_saver: bool,
    /// Free bytes on the storage holding the database.
    pub free_bytes: u64,
}

impl From<DeviceConditions> for PrefetchConditions {
    fn from(c: DeviceConditions) -> Self {
        Self {
            mode: match c.setting {
                BackgroundDownload::WifiOnly => PrefetchMode::UnmeteredOnly,
                BackgroundDownload::Always => PrefetchMode::Always,
                BackgroundDownload::Never => PrefetchMode::Never,
            },
            unmetered: c.unmetered,
            battery_saver: c.battery_saver,
            free_bytes: c.free_bytes,
        }
    }
}

#[uniffi::export]
impl SyncEngine {
    /// Tells the engine about the network, Battery Saver, free storage and
    /// the "Download messages in the background" setting. Call it at start
    /// and whenever one changes. Until it is called, nothing is downloaded
    /// in the background. A download in progress stops after its current
    /// batch once the conditions no longer allow it.
    pub fn set_device_conditions(&self, conditions: DeviceConditions) {
        self.prefetch.set_conditions(conditions.into());
    }
}

/// One account's writer lock (issue #282) and who is waiting for it. The
/// lock is the account's single [`WriteLock`], also held by its live
/// updates for each sync, so only one writer touches the account's rows at
/// a time. Calls the user waits for take it with [`AccountGate::lock`];
/// background downloads take it per batch and check
/// [`AccountGate::foreground_waiting`] before each batch, and give way.
#[derive(Default)]
pub(crate) struct AccountGate {
    write_lock: WriteLock,
    /// How many calls the user waits for want the lock.
    waiting: AtomicUsize,
}

impl AccountGate {
    /// The shared writer lock, for live updates (which take it with
    /// `lock().await` inside their runtime).
    pub(crate) fn write_lock(&self) -> WriteLock {
        self.write_lock.clone()
    }

    /// Takes the lock for a call the user waits for. Outside any runtime
    /// only (`blocking_lock`).
    pub(crate) fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        super::pool::check_lock_order();
        let _waiting = self.announce();
        self.write_lock.blocking_lock()
    }

    /// Marks a call the user waits for as waiting, until the returned guard
    /// is dropped, so background downloads give way from the start of the
    /// call rather than only once it reaches the lock.
    pub(crate) fn announce(&self) -> Waiting<'_> {
        step(&self.waiting, |n| n.checked_add(1));
        Waiting(&self.waiting)
    }

    /// Takes the lock for background work, without counting as waiting.
    /// Outside any runtime only.
    fn lock_background(&self) -> tokio::sync::MutexGuard<'_, ()> {
        super::pool::check_lock_order();
        self.write_lock.blocking_lock()
    }

    /// Whether a call the user waits for wants the lock.
    fn foreground_waiting(&self) -> bool {
        self.waiting.load(Ordering::SeqCst) > 0
    }

    /// Whether the lock is free right now (tests).
    #[cfg(test)]
    pub(crate) fn is_free(&self) -> bool {
        self.write_lock.try_lock().is_ok()
    }
}

/// Counts a waiting call while it lives; see [`AccountGate::announce`].
pub(crate) struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        step(self.0, |n| n.checked_sub(1));
    }
}

/// Changes `counter` by `change`, leaving it as it is where that would
/// overflow.
fn step(counter: &AtomicUsize, change: impl Fn(usize) -> Option<usize>) {
    let mut current = counter.load(Ordering::SeqCst);
    while let Some(next) = change(current) {
        match counter.compare_exchange_weak(current, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

/// The lock for `account` in `locks`, made on first use.
pub(crate) fn gate_for(
    locks: &Mutex<HashMap<i64, Arc<AccountGate>>>,
    account: AccountId,
) -> Arc<AccountGate> {
    let mut locks = locks.lock().unwrap_or_else(|p| p.into_inner());
    locks.entry(account.0).or_default().clone()
}

/// A running download thread.
struct Running {
    thread: JoinHandle<()>,
}

/// Background downloads of one engine: the conditions and the running
/// threads, one per account at most.
pub(crate) struct Prefetcher {
    conditions: Mutex<PrefetchConditions>,
    limits: Mutex<PrefetchLimits>,
    running: Mutex<HashMap<i64, Running>>,
    /// Set when the engine is dropped: threads stop after their batch.
    stopped: AtomicBool,
}

/// What a download thread needs; it owns all of it.
pub(crate) struct PrefetchJob {
    pub(crate) db_path: String,
    pub(crate) listener: Arc<dyn CoreEventListener>,
    pub(crate) extra_root_cert: Option<Vec<u8>>,
    pub(crate) gate: Arc<AccountGate>,
    /// The engine's reused connections: IMAP downloads run on the
    /// account's one (#294).
    pub(crate) pool: Arc<ConnectionPool>,
    pub(crate) account: AccountId,
    pub(crate) folder: FolderId,
    pub(crate) sync_config: SyncConfig,
}

/// How long a download thread waits before checking again whether the
/// call it gave way to has its lock.
const GIVE_WAY_POLL: Duration = Duration::from_millis(10);

impl Prefetcher {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            conditions: Mutex::new(PrefetchConditions::UNKNOWN),
            limits: Mutex::new(PrefetchLimits::default()),
            running: Mutex::new(HashMap::new()),
            stopped: AtomicBool::new(false),
        })
    }

    fn set_conditions(&self, conditions: PrefetchConditions) {
        *self.conditions.lock().unwrap_or_else(|p| p.into_inner()) = conditions;
    }

    fn allows(&self) -> bool {
        !self.stopped.load(Ordering::SeqCst)
            && self
                .conditions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .allows()
    }

    fn limits(&self) -> PrefetchLimits {
        *self.limits.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn set_limits(&self, limits: PrefetchLimits) {
        *self.limits.lock().unwrap_or_else(|p| p.into_inner()) = limits;
    }

    /// Tells running threads to stop after their current batch.
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Starts downloading the folder's recent bodies in the background, if
    /// the conditions allow it, there is something to download and no
    /// download runs for the account yet. Returns at once.
    pub(crate) fn start(self: &Arc<Self>, job: PrefetchJob) {
        if !self.allows() {
            return;
        }
        // Don't open a connection for nothing.
        let todo = Store::open(&job.db_path).ok().and_then(|store| {
            prefetch::candidates(
                &store,
                job.folder,
                unix_now(),
                &self.limits(),
                &HashSet::new(),
            )
            .ok()
        });
        if todo.is_none_or(|t| t.is_empty()) {
            return;
        }
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        if running
            .get(&job.account.0)
            .is_some_and(|r| !r.thread.is_finished())
        {
            return;
        }
        let account = job.account.0;
        let prefetcher = self.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("mailcrane-prefetch-{account}"))
            .spawn(move || download(&prefetcher, &job));
        if let Ok(thread) = spawned {
            running.insert(account, Running { thread });
        }
    }

    /// Waits up to `limit` for every running download to finish. Returns
    /// whether they all did.
    pub(crate) fn wait(&self, limit: Duration) -> bool {
        let started = std::time::Instant::now();
        loop {
            let busy = self
                .running
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .values()
                .any(|r| !r.thread.is_finished());
            if !busy {
                return true;
            }
            if started.elapsed() >= limit {
                return false;
            }
            std::thread::sleep(GIVE_WAY_POLL);
        }
    }
}

/// The connection a download thread uses: the account's reused IMAP
/// connection (#294), or a JMAP session of its own.
enum Link<'a> {
    Imap {
        /// The folder's name on the server.
        remote_name: String,
        /// Its UIDVALIDITY as stored: a UID means the stored message only
        /// while the server still reports this.
        uidvalidity: Option<i64>,
    },
    Jmap(Box<JmapSyncer<'a>>),
}

impl Link<'_> {
    /// Downloads one batch. Blocking: IMAP runs on the pool's connection and
    /// runtime, JMAP on `runtime`.
    fn batch(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        job: &PrefetchJob,
        store: &Store,
        batch: &[Candidate],
        max_size: u32,
    ) -> Result<prefetch::BatchOutcome, CoreSyncError> {
        match self {
            Link::Imap {
                remote_name,
                uidvalidity,
            } => {
                let login = || imap_login(store, job.account, job.extra_root_cert.as_deref());
                let ctx = (store, remote_name.as_str(), *uidvalidity, batch);
                job.pool
                    .run(
                        job.account,
                        login,
                        Retry::Idempotent,
                        &ctx,
                        |session, &(store, remote_name, uidvalidity, batch)| {
                            Box::pin(async move {
                                let info = session.select_reusing(remote_name).await?;
                                if uidvalidity != Some(i64::from(info.uidvalidity)) {
                                    // The next sync starts the folder over.
                                    return Err(CoreSyncError::FolderReset);
                                }
                                prefetch::imap_batch(store, session, batch).await
                            })
                        },
                    )
                    .map_err(|e| match e {
                        PoolError::Job(e) => e,
                        // Already logged when the connection failed.
                        PoolError::Connect(_) => CoreSyncError::Imap(ImapError::CannotConnect),
                    })
            }
            Link::Jmap(syncer) => runtime.block_on(syncer.prefetch_batch(batch, max_size)),
        }
    }
}

/// What the download thread connects with: for IMAP the folder to select
/// on the reused connection, for JMAP a session. None if that fails
/// (logged).
async fn connect<'a>(
    job: &PrefetchJob,
    store: &'a Store,
    listener: &'a ListenerAdapter,
) -> Option<Link<'a>> {
    match store.account_protocol(job.account) {
        Ok(Some(Protocol::Jmap { session_url })) => {
            let auth = match jmap::jmap_auth(store, job.account) {
                Ok(auth) => auth,
                Err(kind) => {
                    tracing::warn!(stage = "prefetch login", ?kind, "prefetch stopped");
                    return None;
                }
            };
            let mut config = mailcrane_jmap::ClientConfig::default();
            if let Some(der) = &job.extra_root_cert {
                config.extra_roots.push(der.clone());
            }
            match JmapSyncer::connect(
                store,
                listener,
                job.account,
                &session_url,
                auth,
                config,
                job.sync_config,
            )
            .await
            {
                Ok(syncer) => Some(Link::Jmap(Box::new(syncer))),
                Err(e) => {
                    log_failure("prefetch connect", &e, None);
                    None
                }
            }
        }
        Ok(_) => {
            let folder = store.get_folder(job.folder).ok().flatten()?;
            Some(Link::Imap {
                remote_name: folder.details.remote_name,
                uidvalidity: folder.details.uidvalidity,
            })
        }
        Err(_) => None,
    }
}

/// The download thread: batch after batch until nothing is left, the
/// conditions stop allowing it, or something fails.
fn download(prefetcher: &Prefetcher, job: &PrefetchJob) {
    let Ok(runtime) = runtime() else {
        return;
    };
    let Ok(store) = Store::open(&job.db_path) else {
        return;
    };
    let listener = ListenerAdapter(job.listener.clone());
    let limits = prefetcher.limits();
    let batch_size = limits.batch.max(1);
    // Messages already tried in this run: one the server didn't return is
    // not asked for again until the next run.
    let mut tried: HashSet<MessageId> = HashSet::new();
    let mut link: Option<Link<'_>> = None;
    loop {
        if !prefetcher.allows() {
            break;
        }
        // Give way: let every waiting call take the locks first.
        if job.gate.foreground_waiting() {
            std::thread::sleep(GIVE_WAY_POLL);
            continue;
        }
        // Lock order: WriteLock here, then the pooled connection in
        // `Link::batch`.
        let guard = job.gate.lock_background();
        if job.gate.foreground_waiting() {
            drop(guard);
            continue;
        }
        let todo = match prefetch::candidates(&store, job.folder, unix_now(), &limits, &tried) {
            Ok(todo) => todo,
            Err(e) => {
                log_failure("prefetch", &e, None);
                break;
            }
        };
        let batch = todo.get(..todo.len().min(batch_size)).unwrap_or_default();
        if batch.is_empty() {
            break;
        }
        if link.is_none() {
            link = runtime.block_on(connect(job, &store, &listener));
        }
        let Some(open) = link.as_mut() else {
            break;
        };
        let result = open.batch(&runtime, job, &store, batch, limits.max_size);
        drop(guard);
        // Some bodies may be saved even when a later one failed.
        let changed = !matches!(&result, Ok(o) if o.saved == 0);
        if changed {
            listener.on_event(events::CoreEvent::MessagesChanged {
                folder_id: job.folder,
            });
        }
        match result {
            Ok(outcome) => tried.extend(outcome.tried),
            Err(e) => {
                log_failure("prefetch", &e, None);
                break;
            }
        }
    }
}

/// Starts background downloads when live updates report changes in the
/// inbox, and passes every event on.
pub(crate) struct PrefetchOnChange {
    pub(crate) inner: ListenerAdapter,
    pub(crate) prefetcher: Arc<Prefetcher>,
    pub(crate) locks: Arc<Mutex<HashMap<i64, Arc<AccountGate>>>>,
    pub(crate) pool: Arc<ConnectionPool>,
    pub(crate) db_path: String,
    pub(crate) extra_root_cert: Option<Vec<u8>>,
    pub(crate) account: AccountId,
    pub(crate) sync_config: SyncConfig,
}

impl events::EventListener for PrefetchOnChange {
    fn on_event(&self, event: events::CoreEvent) {
        let folder = match &event {
            events::CoreEvent::MessagesChanged { folder_id } => Some(*folder_id),
            _ => None,
        };
        self.inner.on_event(event);
        let Some(folder) = folder else {
            return;
        };
        let is_inbox = Store::open(&self.db_path)
            .ok()
            .and_then(|store| inbox_of(&store, self.account).ok().flatten())
            == Some(folder);
        if is_inbox {
            self.prefetcher.start(PrefetchJob {
                db_path: self.db_path.clone(),
                listener: self.inner.0.clone(),
                extra_root_cert: self.extra_root_cert.clone(),
                gate: gate_for(&self.locks, self.account),
                pool: self.pool.clone(),
                account: self.account,
                folder,
                sync_config: self.sync_config,
            });
        }
    }
}
