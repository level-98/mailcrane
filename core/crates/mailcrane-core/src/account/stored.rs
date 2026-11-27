//! Logging in to a stored account and refreshing its folder list (#520).
//!
//! Platform-neutral: every app (Android through the FFI, a desktop app in
//! Rust) uses these instead of its own copy. Secrets come in through the
//! app's [`SecretStore`]; nothing here calls a platform API.

use std::time::Duration;

use mailcrane_imap::{save_folder_list, ImapAuth, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{AccountId, FolderId, FolderRole, Security, Store, StoreError};

use crate::account::credentials::{credential, refresh_if_needed, Credential, CredentialError};
use crate::events::{CoreEvent, EventListener, SyncErrorKind};
use crate::secrets::SecretStore;
use crate::sync::{log_failure, ImapLogin, SyncError};

/// How long refreshing an OAuth access token may take (M3-02).
pub const OAUTH_TIMEOUT: Duration = Duration::from_secs(20);

/// The account's IMAP server and credentials, from the store and
/// `secrets`. Doesn't refresh an OAuth token: [`connect`] does that first,
/// and callers outside an async runtime call [`refresh_if_needed`]
/// themselves. Never for a local-only account (#289): an unreadable
/// local-only marker is a storage failure, so nothing connects either way.
/// `extra_root_cert` is for tests only.
pub fn imap_login(
    store: &Store,
    secrets: &dyn SecretStore,
    account: AccountId,
    extra_root_cert: Option<&[u8]>,
) -> Result<ImapLogin, SyncErrorKind> {
    let row = remote_account(store, account)?;
    let auth = match credential(store, secrets, account) {
        Ok(Credential::Password(p)) => ImapAuth::Password(Secret::new(p)),
        Ok(Credential::OAuth(token)) => ImapAuth::XOAuth2(Secret::new(token.as_str())),
        Err(CredentialError::Storage) => return Err(SyncErrorKind::Storage),
        Err(_) => return Err(SyncErrorKind::Auth),
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

/// The account's row, if it exists and is not local-only (#289): `NotFound`
/// otherwise, and `Storage` if the store (or the local-only marker) can't
/// be read. Checked before any secret is read or connection made.
pub fn remote_account(
    store: &Store,
    account: AccountId,
) -> Result<mailcrane_store::Account, SyncErrorKind> {
    let row = store
        .get_account(account)
        .map_err(|_| SyncErrorKind::Storage)?
        .ok_or(SyncErrorKind::NotFound)?;
    if store
        .is_local_account(account)
        .map_err(|_| SyncErrorKind::Storage)?
    {
        return Err(SyncErrorKind::NotFound);
    }
    Ok(row)
}

/// Logs in to the account's IMAP server. A missing or local-only account is
/// refused first, before any secret is read (#289). An OAuth account (M3-02)
/// then gets a fresh access token; if the token server can't be reached,
/// the cached token is tried.
pub async fn connect(
    store: &Store,
    secrets: &dyn SecretStore,
    account: AccountId,
    extra_root_cert: Option<&[u8]>,
) -> Result<Session, SyncErrorKind> {
    remote_account(store, account)?;
    match refresh_if_needed(store, secrets, account, &[], OAUTH_TIMEOUT).await {
        Err(CredentialError::ReauthRequired) => return Err(SyncErrorKind::Auth),
        Err(CredentialError::Storage) => return Err(SyncErrorKind::Storage),
        // Not reachable now: try the cached token.
        Err(CredentialError::Missing) | Ok(()) => {}
    }
    let login = imap_login(store, secrets, account, extra_root_cert)?;
    Session::connect_auth(&login.config, &login.username, &login.auth)
        .await
        .map_err(|e| {
            let e = SyncError::from(e);
            log_failure("connect", &e, None);
            e.kind()
        })
}

/// Lists the account's folders on `session` and saves them, keeping each
/// folder's sync state. Sends `FolderChanged` for each folder gone from the
/// server (#323), so the apps re-read the list and leave it if it was open.
/// On failure the saved folders are unchanged. The caller logs out, and
/// finds the inbox with [`inbox_of`].
pub async fn refresh_folders(
    store: &Store,
    session: &mut Session,
    account: AccountId,
    listener: &dyn EventListener,
) -> Result<(), SyncError> {
    let listed = session.list_folders().await;
    let command = session.last_command();
    let folders = listed.map_err(|e| {
        let e = SyncError::from(e);
        log_failure("list folders", &e, command);
        e
    })?;
    let saved = save_folder_list(store, account, &folders).map_err(|e| {
        let e = SyncError::from(e);
        log_failure("save folders", &e, None);
        e
    })?;
    for folder in saved.removed {
        listener.on_event(CoreEvent::FolderChanged { folder_id: folder });
    }
    Ok(())
}

/// The account's inbox: the folder with the inbox role, otherwise the one
/// named `INBOX` in any case (RFC 9051 §5.1).
pub fn inbox_of(store: &Store, account: AccountId) -> Result<Option<FolderId>, StoreError> {
    let folders = store.list_folders(account)?;
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

#[cfg(test)]
#[path = "stored_tests.rs"]
mod tests;
