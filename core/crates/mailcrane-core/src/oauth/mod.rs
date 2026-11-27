//! OAuth 2.0 sign-in (M3-01): authorization code with PKCE (RFC 6749 §4.1,
//! RFC 7636) in the system browser, as native apps must (RFC 8252).
//!
//! The core does the protocol work; the apps only open
//! [`AuthRequest::url`] in the system browser and hand back the redirect.
//! The verifier and `state` stay in memory ([`PendingAuth`]); refresh tokens
//! go only to the platform's secure storage; access tokens are never
//! persisted. None of them appear in `Debug` output, errors or logs.
//!
//! Provider configurations (Google, Microsoft) and their client ids are
//! added by M3-02 and M3-03.

mod flow;
mod http;
pub mod pkce;
mod provider;
pub mod providers;
mod tokens;

pub use flow::{begin, complete, refresh, AuthRequest, PendingAuth, Tokens};
pub use provider::OAuthProvider;
pub use tokens::{AccessToken, TokenCache};

/// The process-wide access-token cache (memory only), shared by sync,
/// sending and setup. Entries are per database ([`TokenCache`]), so one
/// process can hold several databases (#434).
pub fn token_cache() -> &'static TokenCache {
    static CACHE: std::sync::OnceLock<TokenCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(TokenCache::new)
}

/// Why OAuth failed. Never carries a token, code or server text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthError {
    /// The provider configuration is unusable (an endpoint isn't `https:`).
    #[error("invalid OAuth provider configuration")]
    InvalidProvider,
    /// The OS random generator failed.
    #[error("no randomness available")]
    Random,
    /// The redirect's `state` is missing or isn't the one we sent (RFC 6749
    /// §10.12): possibly a forged redirect. Nothing was exchanged.
    #[error("the sign-in response didn't match the request")]
    StateMismatch,
    /// The redirect doesn't look like a response to our request.
    #[error("unexpected sign-in response")]
    MalformedRedirect,
    /// The user (or provider) declined (`error` in the redirect, RFC 6749
    /// §4.1.2.1).
    #[error("sign-in was cancelled or refused")]
    Denied,
    /// The refresh token no longer works (`invalid_grant`, RFC 6749 §5.2):
    /// the user must sign in again.
    #[error("sign in again")]
    ReauthRequired,
    /// The token endpoint refused for another reason.
    #[error("the provider refused the request")]
    Refused,
    /// Couldn't reach the provider.
    #[error("network error")]
    Network,
    /// The TLS connection failed (certificate checks are always on).
    #[error("TLS error")]
    Tls,
    /// The provider's answer couldn't be read.
    #[error("unreadable answer from the provider")]
    Malformed,
    /// Secure storage failed.
    #[error("secure storage error")]
    Storage,
}
