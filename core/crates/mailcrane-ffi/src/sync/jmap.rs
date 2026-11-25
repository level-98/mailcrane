//! JMAP accounts in the sync engine (M3-04): the same calls as IMAP, run by
//! `JmapSyncer` instead of an IMAP session.

use super::*;
use mailcrane_core::sync::JmapSyncer;
use mailcrane_jmap::{Auth, ClientConfig};
use mailcrane_store::Protocol;

impl SyncEngine {
    /// Runs `job` for a JMAP account and threads what it stored; `None` if
    /// the account speaks IMAP (the caller then does the IMAP work).
    pub(crate) fn run_jmap<T>(
        &self,
        account: AccountId,
        job: impl for<'a> FnOnce(&'a mut JmapSyncer<'_>) -> SyncFuture<'a, T>,
    ) -> Option<Result<T, SyncFailure>> {
        let store = match self.open_store() {
            Ok(store) => store,
            Err(e) => return Some(Err(e)),
        };
        let session_url = match store.account_protocol(account) {
            Ok(Some(Protocol::Jmap { session_url })) => session_url,
            Ok(_) => return None,
            Err(_) => return Some(Err(failed(events::SyncErrorKind::Storage))),
        };
        Some(self.jmap_inner(&store, account, &session_url, job))
    }

    fn jmap_inner<T>(
        &self,
        store: &Store,
        account: AccountId,
        session_url: &str,
        job: impl for<'a> FnOnce(&'a mut JmapSyncer<'_>) -> SyncFuture<'a, T>,
    ) -> Result<T, SyncFailure> {
        let lock = self.account_lock(account);
        let _guard = lock.lock();
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
            if result.is_ok() {
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
pub(super) fn jmap_auth(store: &Store, account: AccountId) -> Result<Auth, events::SyncErrorKind> {
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
