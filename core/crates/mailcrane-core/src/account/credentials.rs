//! How an account signs in, for every connection: the password from
//! secure storage, or for an OAuth account (M3-02) an access token from the
//! refresh token in secure storage.
//!
//! Building a login is synchronous; call [`refresh_if_needed`] first on an
//! async path, so an OAuth account has a fresh token in the cache.

use std::time::Duration;

use mailcrane_store::{AccountId, Store};

use crate::oauth::{providers, token_cache, AccessToken, OAuthError, OAuthProvider};
use crate::secrets::{SecretKey, SecretStore};

/// What to sign in with. `Debug` never shows the password or token.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    Password(String),
    OAuth(AccessToken),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Password(_) => "Credential::Password(…)",
            Self::OAuth(_) => "Credential::OAuth(…)",
        })
    }
}

/// Why there's nothing to sign in with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    /// No password stored, or the account isn't there.
    #[error("no credentials")]
    Missing,
    /// The store or secure storage failed.
    #[error("storage error")]
    Storage,
    /// An OAuth account whose refresh token no longer works, or that has
    /// no fresh access token: the user must sign in again.
    #[error("sign in again")]
    ReauthRequired,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// The account's credential: its password, or its cached OAuth access token
/// (`ReauthRequired` if there is none; run [`refresh_if_needed`] first).
pub fn credential(
    store: &Store,
    secrets: &dyn SecretStore,
    account: AccountId,
) -> Result<Credential, CredentialError> {
    let provider = store
        .oauth_provider(account)
        .map_err(|_| CredentialError::Storage)?;
    if provider.is_some() {
        return token_cache()
            .cached(store.cache_scope(), account.0, now())
            .map(Credential::OAuth)
            .ok_or(CredentialError::ReauthRequired);
    }
    let password = secrets
        .get(&SecretKey::account_password(account.0))
        .map_err(|_| CredentialError::Storage)?
        .ok_or(CredentialError::Missing)?;
    let password = std::str::from_utf8(password.expose()).map_err(|_| CredentialError::Missing)?;
    Ok(Credential::Password(password.to_owned()))
}

/// The OAuth provider the account signs in with, or `None` for a password.
/// `ReauthRequired` for a provider Mailcrane no longer knows.
pub fn oauth_provider_of(
    store: &Store,
    account: AccountId,
) -> Result<Option<OAuthProvider>, CredentialError> {
    match store
        .oauth_provider(account)
        .map_err(|_| CredentialError::Storage)?
    {
        None => Ok(None),
        Some(id) => providers::by_id(&id)
            .map(Some)
            .ok_or(CredentialError::ReauthRequired),
    }
}

/// For an OAuth account, makes sure the cache holds an access token good
/// for at least 5 more minutes, refreshing it with the stored refresh token
/// if needed. Does nothing for a password account.
pub async fn refresh_if_needed(
    store: &Store,
    secrets: &dyn SecretStore,
    account: AccountId,
    extra_roots: &[Vec<u8>],
    timeout: Duration,
) -> Result<(), CredentialError> {
    let Some(provider) = oauth_provider_of(store, account)? else {
        return Ok(());
    };
    refresh_token(
        secrets,
        store.cache_scope(),
        account,
        &provider,
        extra_roots,
        timeout,
    )
    .await
}

/// [`refresh_if_needed`] once the provider is known, without the store, so
/// a future that must be `Send` can await it. `scope` is the store's
/// `cache_scope` (or `Store::scope_of` its path).
pub async fn refresh_token(
    secrets: &dyn SecretStore,
    scope: &str,
    account: AccountId,
    provider: &OAuthProvider,
    extra_roots: &[Vec<u8>],
    timeout: Duration,
) -> Result<(), CredentialError> {
    token_cache()
        .access_token(
            secrets,
            scope,
            account.0,
            provider,
            now(),
            extra_roots,
            timeout,
        )
        .await
        .map(|_| ())
        .map_err(|e| match e {
            OAuthError::Storage => CredentialError::Storage,
            // Couldn't reach Google now: a cached token may still work, and
            // the connection will say if not.
            OAuthError::Network | OAuthError::Tls => CredentialError::Missing,
            _ => CredentialError::ReauthRequired,
        })
}
