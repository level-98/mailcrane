//! Keeping tokens: refresh tokens only in the platform's secure storage
//! (`SecretKey::oauth_refresh_token`), access tokens only in memory with
//! their expiry, refreshed 5 minutes early.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use super::flow::{refresh, Tokens};
use super::{OAuthError, OAuthProvider};
use crate::secrets::{Secret, SecretKey, SecretStore};

/// Refresh this long before the access token expires.
const EARLY_SECS: i64 = 5 * 60;
/// Assumed lifetime when the provider doesn't say (RFC 6749 §5.1 makes
/// `expires_in` optional): short, so it's refreshed soon.
const DEFAULT_LIFETIME_SECS: u64 = 10 * 60;

/// An access token for one request. `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken(String);

impl AccessToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessToken(…)")
    }
}

/// Access tokens by database and account, in memory only (gone when the
/// process ends). `scope` is the database's `Store::cache_scope`: account
/// IDs start at 1 in every database, so two databases (tests running side
/// by side, profiles) must never share an entry (#434).
#[derive(Default)]
pub struct TokenCache {
    tokens: Mutex<HashMap<(String, i64), (AccessToken, i64)>>,
}

impl std::fmt::Debug for TokenCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenCache(…)")
    }
}

impl TokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps the result of a sign-in or refresh: the access token here, a
    /// new refresh token (if any) in `secrets`.
    pub fn save(
        &self,
        secrets: &dyn SecretStore,
        scope: &str,
        account: i64,
        tokens: &Tokens,
        now: i64,
    ) -> Result<AccessToken, OAuthError> {
        if let Some(refresh) = &tokens.refresh_token {
            secrets
                .put(
                    &SecretKey::oauth_refresh_token(account),
                    &Secret::from_string(refresh.clone()),
                )
                .map_err(|_| OAuthError::Storage)?;
        }
        let lifetime = tokens.expires_in.unwrap_or(DEFAULT_LIFETIME_SECS);
        let expires_at = now.saturating_add(i64::try_from(lifetime).unwrap_or(i64::MAX));
        let token = AccessToken(tokens.access_token.clone());
        self.lock()
            .insert((scope.to_owned(), account), (token.clone(), expires_at));
        Ok(token)
    }

    /// A usable access token for `account`: the cached one if it's good for
    /// more than 5 minutes, otherwise a fresh one from the refresh token in
    /// `secrets`. `ReauthRequired` if there is no refresh token or the
    /// provider no longer accepts it (the stored one is then deleted).
    #[allow(clippy::too_many_arguments)]
    pub async fn access_token(
        &self,
        secrets: &dyn SecretStore,
        scope: &str,
        account: i64,
        provider: &OAuthProvider,
        now: i64,
        extra_roots: &[Vec<u8>],
        timeout: Duration,
    ) -> Result<AccessToken, OAuthError> {
        if let Some((token, expires_at)) = self.lock().get(&(scope.to_owned(), account)) {
            if expires_at.saturating_sub(EARLY_SECS) > now {
                return Ok(token.clone());
            }
        }
        let stored = secrets
            .get(&SecretKey::oauth_refresh_token(account))
            .map_err(|_| OAuthError::Storage)?
            .ok_or(OAuthError::ReauthRequired)?;
        let refresh_token = std::str::from_utf8(stored.expose())
            .map_err(|_| OAuthError::ReauthRequired)?
            .to_owned();
        match refresh(provider, &refresh_token, extra_roots, timeout).await {
            Ok(tokens) => self.save(secrets, scope, account, &tokens, now),
            Err(OAuthError::ReauthRequired) => {
                self.forget(scope, account);
                let _ = secrets.delete(&SecretKey::oauth_refresh_token(account));
                Err(OAuthError::ReauthRequired)
            }
            Err(e) => Err(e),
        }
    }

    /// The cached access token for `account` if it hasn't expired, without
    /// refreshing: for the synchronous paths that build a login once
    /// [`TokenCache::access_token`] has run.
    pub fn cached(&self, scope: &str, account: i64, now: i64) -> Option<AccessToken> {
        self.lock()
            .get(&(scope.to_owned(), account))
            .filter(|(_, expires_at)| *expires_at > now)
            .map(|(token, _)| token.clone())
    }

    /// Drops the cached access token (after the server rejected it, or the
    /// account was removed).
    pub fn forget(&self, scope: &str, account: i64) {
        self.lock().remove(&(scope.to_owned(), account));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, i64), (AccessToken, i64)>> {
        self.tokens.lock().unwrap_or_else(|p| p.into_inner())
    }
}
