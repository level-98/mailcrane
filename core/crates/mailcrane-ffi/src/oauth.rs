//! OAuth sign-in for the apps (M3-01). The apps open the URL from
//! [`oauth_begin`] in the system browser (Custom Tabs on Android,
//! `ASWebAuthenticationSession` on iOS; never an in-app web view), pass the
//! redirect to [`oauth_complete`], and once the account exists call
//! [`oauth_attach`].
//!
//! Verifiers, `state` and tokens stay in the core's memory; the apps only
//! hold an opaque sign-in id. Refresh tokens go to the platform's secure
//! storage only when attached to an account.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use mailcrane_core::oauth::{self as core, OAuthProvider, PendingAuth, TokenCache, Tokens};

use crate::secrets;

/// A started sign-in is forgotten after this long.
const PENDING_FOR: Duration = Duration::from_secs(10 * 60);
/// A finished sign-in not attached to an account is forgotten after this long.
const FINISHED_FOR: Duration = Duration::from_secs(10 * 60);
const TIMEOUT: Duration = Duration::from_secs(30);

/// A provider's OAuth settings, as the apps get them from the core's
/// provider list (M3-02, M3-03).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OAuthProviderConfig {
    pub id: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub redirect_uri: String,
    pub extra_auth_params: Vec<OAuthParam>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OAuthParam {
    pub name: String,
    pub value: String,
}

/// What to open in the system browser.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OAuthStart {
    pub url: String,
}

/// Why sign-in failed. Never carries a token or server text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum OAuthFailure {
    #[error("invalid provider")]
    InvalidProvider,
    /// No sign-in is waiting for this redirect (expired, or not ours).
    #[error("no matching sign-in")]
    NoSuchSignIn,
    /// The redirect didn't match the request: nothing was exchanged.
    #[error("the response didn't match the request")]
    StateMismatch,
    #[error("cancelled or refused")]
    Denied,
    #[error("sign in again")]
    ReauthRequired,
    #[error("network error")]
    Network,
    #[error("TLS error")]
    Tls,
    #[error("the provider refused")]
    Refused,
    #[error("secure storage error")]
    Storage,
}

impl From<core::OAuthError> for OAuthFailure {
    fn from(e: core::OAuthError) -> Self {
        use core::OAuthError as E;
        match e {
            E::InvalidProvider | E::Random => Self::InvalidProvider,
            E::StateMismatch | E::MalformedRedirect => Self::StateMismatch,
            E::Denied => Self::Denied,
            E::ReauthRequired => Self::ReauthRequired,
            E::Network => Self::Network,
            E::Tls => Self::Tls,
            E::Refused | E::Malformed => Self::Refused,
            E::Storage => Self::Storage,
        }
    }
}

impl TryFrom<OAuthProviderConfig> for OAuthProvider {
    type Error = OAuthFailure;
    fn try_from(c: OAuthProviderConfig) -> Result<Self, OAuthFailure> {
        let parse = |s: &str| url::Url::parse(s).map_err(|_| OAuthFailure::InvalidProvider);
        Ok(OAuthProvider {
            id: c.id,
            authorization_endpoint: parse(&c.authorization_endpoint)?,
            token_endpoint: parse(&c.token_endpoint)?,
            client_id: c.client_id,
            scopes: c.scopes,
            redirect_uri: c.redirect_uri,
            extra_auth_params: c
                .extra_auth_params
                .into_iter()
                .map(|p| (p.name, p.value))
                .collect(),
        })
    }
}

#[derive(Default)]
struct Sessions {
    /// state → started sign-in
    pending: HashMap<String, (PendingAuth, Instant)>,
    /// sign-in id → tokens waiting for an account
    finished: HashMap<String, (Tokens, Instant)>,
    next: u64,
}

fn sessions() -> MutexGuard<'static, Sessions> {
    static S: OnceLock<Mutex<Sessions>> = OnceLock::new();
    let mut s = S
        .get_or_init(|| Mutex::new(Sessions::default()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    s.pending
        .retain(|_, (_, at)| now.duration_since(*at) < PENDING_FOR);
    s.finished
        .retain(|_, (_, at)| now.duration_since(*at) < FINISHED_FOR);
    s
}

/// The process-wide access-token cache (memory only).
pub(crate) fn token_cache() -> &'static TokenCache {
    // The core's, so sync and sending see the tokens a sign-in saved.
    core::token_cache()
}

/// Starts a sign-in with `provider`: returns the URL to open in the system
/// browser. `login_hint` pre-fills the account at the provider.
#[uniffi::export]
pub fn oauth_begin(
    provider: OAuthProviderConfig,
    login_hint: Option<String>,
) -> Result<OAuthStart, OAuthFailure> {
    let provider = OAuthProvider::try_from(provider)?;
    let (request, pending) = core::begin(&provider, login_hint.as_deref())?;
    sessions()
        .pending
        .insert(request.state.clone(), (pending, Instant::now()));
    Ok(OAuthStart {
        url: request.url.to_string(),
    })
}

/// Finishes a sign-in from the URL the browser redirected to. Returns an
/// opaque sign-in id for [`oauth_attach`]; the tokens stay in the core.
#[uniffi::export(async_runtime = "tokio")]
pub async fn oauth_complete(redirect_url: String) -> Result<String, OAuthFailure> {
    complete_with(redirect_url, Vec::new()).await
}

pub(crate) async fn complete_with(
    redirect_url: String,
    extra_roots: Vec<Vec<u8>>,
) -> Result<String, OAuthFailure> {
    crate::worker::on_worker_thread(move || async move {
        let state = url::Url::parse(&redirect_url)
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "state")
                    .map(|(_, v)| v.into_owned())
            })
            .ok_or(OAuthFailure::StateMismatch)?;
        // Taken out, so a replayed redirect finds nothing.
        let pending = sessions()
            .pending
            .remove(&state)
            .map(|(p, _)| p)
            .ok_or(OAuthFailure::NoSuchSignIn)?;
        let tokens = core::complete(&pending, &redirect_url, &extra_roots, TIMEOUT).await?;
        let mut s = sessions();
        s.next = s.next.wrapping_add(1);
        let id = format!("signin-{}", s.next);
        s.finished.insert(id.clone(), (tokens, Instant::now()));
        Ok(id)
    })
    .await
    .unwrap_or(Err(OAuthFailure::Network))
}

/// Keeps a finished sign-in's tokens for `account_id` in the database at
/// `db_path`: the refresh token in secure storage, the access token in
/// memory. The sign-in id is then used up.
#[uniffi::export]
pub fn oauth_attach(
    db_path: String,
    sign_in_id: String,
    account_id: i64,
) -> Result<(), OAuthFailure> {
    let secrets = secrets::secret_store().map_err(|_| OAuthFailure::Storage)?;
    attach_with(
        secrets.as_ref(),
        &mailcrane_store::Store::scope_of(&db_path),
        &sign_in_id,
        account_id,
    )
}

/// [`oauth_attach`] with the secret store given, and the database named by
/// its `Store::cache_scope`.
pub(crate) fn attach_with(
    secrets: &dyn mailcrane_core::secrets::SecretStore,
    scope: &str,
    sign_in_id: &str,
    account_id: i64,
) -> Result<(), OAuthFailure> {
    let (tokens, _) = sessions()
        .finished
        .remove(sign_in_id)
        .ok_or(OAuthFailure::NoSuchSignIn)?;
    token_cache().save(secrets, scope, account_id, &tokens, now())?;
    Ok(())
}

/// The access token of a finished sign-in, still waiting for an account.
pub(crate) fn finished_access_token(sign_in_id: &str) -> Option<String> {
    sessions()
        .finished
        .get(sign_in_id)
        .map(|(tokens, _)| tokens.access_token.clone())
}

/// The Google provider's settings (M3-02), for `oauth_begin`.
#[uniffi::export]
pub fn google_oauth_provider() -> Result<OAuthProviderConfig, OAuthFailure> {
    let p = mailcrane_core::oauth::providers::google::provider()
        .ok_or(OAuthFailure::InvalidProvider)?;
    Ok(OAuthProviderConfig {
        id: p.id,
        authorization_endpoint: p.authorization_endpoint.to_string(),
        token_endpoint: p.token_endpoint.to_string(),
        client_id: p.client_id,
        scopes: p.scopes,
        redirect_uri: p.redirect_uri,
        extra_auth_params: p
            .extra_auth_params
            .into_iter()
            .map(|(name, value)| OAuthParam { name, value })
            .collect(),
    })
}

/// Whether setup should offer "Sign in with Google" first for `email`: a
/// gmail.com or googlemail.com address. Google Workspace accounts can use
/// it too.
#[uniffi::export]
pub fn is_gmail_address(email: String) -> bool {
    mailcrane_core::oauth::providers::google::is_gmail_address(&email)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_core::secrets::SecretKey;
    use mailcrane_testkit::{FakeTokenEndpoint, MemorySecretStore};

    fn config(server: &FakeTokenEndpoint) -> OAuthProviderConfig {
        OAuthProviderConfig {
            id: "test".into(),
            authorization_endpoint: "https://auth.example.org/authorize".into(),
            token_endpoint: server.token_url(),
            client_id: "client-1".into(),
            scopes: vec!["mail".into()],
            redirect_uri: "app.mailcrane:/oauth2redirect".into(),
            extra_auth_params: vec![],
        }
    }

    #[tokio::test]
    async fn begin_complete_attach_through_the_ffi() {
        let server = FakeTokenEndpoint::start().await.expect("server");
        let start = oauth_begin(config(&server), Some("ana@example.org".into())).expect("begin");
        let url = url::Url::parse(&start.url).expect("url");
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let code = server.authorize(
            &q["code_challenge"],
            "app.mailcrane:/oauth2redirect",
            "client-1",
        );
        let redirect = format!(
            "app.mailcrane:/oauth2redirect?code={code}&state={}",
            q["state"]
        );
        let roots = vec![server.root_cert_der().to_vec()];

        let id = complete_with(redirect.clone(), roots.clone())
            .await
            .expect("complete");
        // The same redirect again finds no sign-in (state is single use).
        assert_eq!(
            complete_with(redirect, roots).await,
            Err(OAuthFailure::NoSuchSignIn)
        );

        let secrets = MemorySecretStore::new();
        attach_with(&secrets, "file:test.db", &id, 42).expect("attach");
        assert_eq!(
            secrets.keys(),
            vec![SecretKey::oauth_refresh_token(42).as_str().to_owned()]
        );
        assert_eq!(
            attach_with(&secrets, "file:test.db", &id, 42),
            Err(OAuthFailure::NoSuchSignIn)
        );
    }

    #[tokio::test]
    async fn a_forged_redirect_is_refused() {
        let server = FakeTokenEndpoint::start().await.expect("server");
        oauth_begin(config(&server), None).expect("begin");
        let forged = "app.mailcrane:/oauth2redirect?code=x&state=someone-elses".to_string();
        assert_eq!(
            complete_with(forged, vec![]).await,
            Err(OAuthFailure::NoSuchSignIn)
        );
        assert_eq!(
            server.grants(),
            Vec::<String>::new(),
            "nothing was exchanged"
        );
    }
}
