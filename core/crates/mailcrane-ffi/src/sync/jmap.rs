//! JMAP accounts in the sync engine (M3-04): the same calls as IMAP, run by
//! `JmapSyncer` instead of an IMAP session.

use super::*;
use mailcrane_core::sync::JmapSyncer;
use mailcrane_jmap::{Auth, ClientConfig};
use mailcrane_store::Protocol;

/// Whether a JMAP job takes the account's writer lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JmapAccess {
    /// Syncs and other work that writes many rows: one writer at a time.
    Writer,
    /// Opening a message (#472): writes only that body, beside a sync.
    Open,
}

impl SyncEngine {
    /// Runs `job` for a JMAP account under its writer lock and threads what
    /// it stored; `None` if the account speaks IMAP (the caller then does the
    /// IMAP work). The protocol is read here, a short database read; the
    /// JMAP work runs on a thread with a stack this crate sets (#462), never
    /// on the caller's.
    pub(crate) fn run_jmap<T: Send>(
        &self,
        account: AccountId,
        job: impl for<'a> FnOnce(&'a mut JmapSyncer<'_>) -> SyncFuture<'a, T> + Send,
    ) -> Option<Result<T, SyncFailure>> {
        self.run_jmap_as(account, JmapAccess::Writer, job)
    }

    /// [`Self::run_jmap`] for opening a message (#472): no writer lock, so it
    /// never waits for the account's sync, and no threading pass, since it
    /// adds no rows. The job's only writes are the opened body's own.
    pub(crate) fn run_jmap_open<T: Send>(
        &self,
        account: AccountId,
        job: impl for<'a> FnOnce(&'a mut JmapSyncer<'_>) -> SyncFuture<'a, T> + Send,
    ) -> Option<Result<T, SyncFailure>> {
        self.run_jmap_as(account, JmapAccess::Open, job)
    }

    fn run_jmap_as<T: Send>(
        &self,
        account: AccountId,
        access: JmapAccess,
        job: impl for<'a> FnOnce(&'a mut JmapSyncer<'_>) -> SyncFuture<'a, T> + Send,
    ) -> Option<Result<T, SyncFailure>> {
        let session_url = {
            let store = match self.open_store() {
                Ok(store) => store,
                Err(e) => return Some(Err(e)),
            };
            match store.account_protocol(account) {
                Ok(Some(Protocol::Jmap { session_url })) => session_url,
                Ok(_) => return None,
                Err(_) => return Some(Err(failed(events::SyncErrorKind::Storage))),
            }
        };
        Some(on_job_thread(|| {
            let store = self.open_store()?;
            self.jmap_inner(&store, account, &session_url, access, job)
        }))
    }

    fn jmap_inner<T>(
        &self,
        store: &Store,
        account: AccountId,
        session_url: &str,
        access: JmapAccess,
        job: impl for<'a> FnOnce(&'a mut JmapSyncer<'_>) -> SyncFuture<'a, T>,
    ) -> Result<T, SyncFailure> {
        let lock = self.account_lock(account);
        let _guard = (access == JmapAccess::Writer).then(|| lock.lock());
        let report = |kind| self.report(account, kind);
        let auth = jmap_auth(store, account).map_err(report)?;
        let mut config = ClientConfig::default();
        if let Some(der) = self.extra_root_cert() {
            config.extra_roots.push(der);
        }
        let listener = ListenerAdapter(self.listener.clone());
        let runtime = runtime().map_err(report)?;
        runtime.block_on(async {
            let mut syncer = JmapSyncer::connect(
                store,
                &listener,
                account,
                session_url,
                auth,
                config,
                self.config,
            )
            .await
            .map_err(|e| report(e.kind()))?;
            let result = job(&mut syncer).await;
            drop(syncer);
            // New rows join their conversations (M4-02), as after IMAP syncs.
            if result.is_ok() && access == JmapAccess::Writer {
                let _ = mailcrane_core::threading::assign_new(store, Some(account));
            }
            result.map_err(SyncFailure::from)
        })
    }

    /// The account a folder belongs to.
    pub(crate) fn folder_account(&self, folder: FolderId) -> Result<AccountId, SyncFailure> {
        Ok(self
            .open_store()?
            .get_folder(folder)
            .map_err(|_| failed(events::SyncErrorKind::Storage))?
            .ok_or_else(|| failed(events::SyncErrorKind::NotFound))?
            .details
            .account_id)
    }

    /// Whether the account speaks JMAP.
    pub(super) fn is_jmap(&self, account: AccountId) -> bool {
        self.open_store()
            .ok()
            .and_then(|s| s.account_protocol(account).ok().flatten())
            .is_some_and(|p| matches!(p, Protocol::Jmap { .. }))
    }
}

/// HTTP Basic with the account's username and its password (an app
/// password at most providers) from secure storage.
pub(crate) fn jmap_auth(store: &Store, account: AccountId) -> Result<Auth, events::SyncErrorKind> {
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
    Ok(Auth::Basic {
        username: row.details.username,
        password,
    })
}
