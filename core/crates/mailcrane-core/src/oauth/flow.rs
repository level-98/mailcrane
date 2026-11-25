//! The authorization code flow (RFC 6749 §4.1) with PKCE (RFC 7636).

use std::time::Duration;

use serde_json::Value;

use super::http::{form, post_form};
use super::pkce::{random_token, CodeVerifier};
use super::{OAuthError, OAuthProvider};

/// What the app opens in the system browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRequest {
    pub url: url::Url,
    /// Also kept in [`PendingAuth`]; the redirect must carry it back.
    pub state: String,
}

/// A started sign-in: what [`complete`] needs. Memory only, never stored;
/// `Debug` hides the verifier and state.
#[derive(Clone)]
pub struct PendingAuth {
    pub provider: OAuthProvider,
    verifier: CodeVerifier,
    state: String,
}

impl std::fmt::Debug for PendingAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingAuth")
            .field("provider", &self.provider.id)
            .finish_non_exhaustive()
    }
}

impl PendingAuth {
    /// The `state` this sign-in expects back.
    pub fn state(&self) -> &str {
        &self.state
    }
}

/// Tokens from the token endpoint (RFC 6749 §5.1). `Debug` hides them.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    /// Seconds until the access token expires, if the provider said.
    pub expires_in: Option<u64>,
    /// A new refresh token, if the provider issued (or rotated) one.
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_in", &self.expires_in)
            .field("has_refresh_token", &self.refresh_token.is_some())
            .finish_non_exhaustive()
    }
}

/// Starts a sign-in: a fresh verifier and `state` (OS random), and the
/// authorization URL with `response_type=code`, the S256 challenge and
/// `login_hint` if given.
pub fn begin(
    provider: &OAuthProvider,
    login_hint: Option<&str>,
) -> Result<(AuthRequest, PendingAuth), OAuthError> {
    provider.check()?;
    let verifier = CodeVerifier::random()?;
    let state = random_token()?;
    let mut url = provider.authorization_endpoint.clone();
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &provider.client_id)
            .append_pair("redirect_uri", &provider.redirect_uri)
            .append_pair("scope", &provider.scopes.join(" "))
            .append_pair("state", &state)
            .append_pair("code_challenge", &verifier.challenge())
            .append_pair("code_challenge_method", "S256");
        if let Some(hint) = login_hint.filter(|h| !h.is_empty()) {
            q.append_pair("login_hint", hint);
        }
        for (k, v) in &provider.extra_auth_params {
            q.append_pair(k, v);
        }
    }
    Ok((
        AuthRequest {
            url,
            state: state.clone(),
        },
        PendingAuth {
            provider: provider.clone(),
            verifier,
            state,
        },
    ))
}

/// Finishes a sign-in from the redirect the browser came back with: checks
/// it's for our redirect URI and carries our `state` (RFC 6749 §10.12),
/// then exchanges the code (with the verifier) for tokens.
pub async fn complete(
    pending: &PendingAuth,
    redirect_url: &str,
    extra_roots: &[Vec<u8>],
    timeout: Duration,
) -> Result<Tokens, OAuthError> {
    let redirect = url::Url::parse(redirect_url).map_err(|_| OAuthError::MalformedRedirect)?;
    let expected =
        url::Url::parse(&pending.provider.redirect_uri).map_err(|_| OAuthError::InvalidProvider)?;
    if redirect.scheme() != expected.scheme() || redirect.path() != expected.path() {
        return Err(OAuthError::MalformedRedirect);
    }
    let param = |name: &str| {
        redirect
            .query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    // The state is checked first: nothing else in a forged redirect is trusted.
    match param("state") {
        Some(s) if constant_time_eq(s.as_bytes(), pending.state.as_bytes()) => {}
        _ => return Err(OAuthError::StateMismatch),
    }
    if param("error").is_some() {
        return Err(OAuthError::Denied);
    }
    let code = param("code")
        .filter(|c| !c.is_empty())
        .ok_or(OAuthError::MalformedRedirect)?;
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &pending.provider.redirect_uri),
        ("client_id", &pending.provider.client_id),
        ("code_verifier", pending.verifier.as_str()),
    ]);
    token_request(&pending.provider, body, extra_roots, timeout).await
}

/// Gets a new access token with a refresh token (RFC 6749 §6).
/// `invalid_grant` means the user must sign in again.
pub async fn refresh(
    provider: &OAuthProvider,
    refresh_token: &str,
    extra_roots: &[Vec<u8>],
    timeout: Duration,
) -> Result<Tokens, OAuthError> {
    provider.check()?;
    let body = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", &provider.client_id),
    ]);
    token_request(provider, body, extra_roots, timeout).await
}

async fn token_request(
    provider: &OAuthProvider,
    body: String,
    extra_roots: &[Vec<u8>],
    timeout: Duration,
) -> Result<Tokens, OAuthError> {
    let reply = post_form(&provider.token_endpoint, body, extra_roots, timeout).await?;
    let json: Value = serde_json::from_slice(&reply.body).map_err(|_| OAuthError::Malformed)?;
    if !(200..300).contains(&reply.status) {
        // RFC 6749 §5.2: the error code, never shown or logged as text.
        return Err(match json.get("error").and_then(Value::as_str) {
            Some("invalid_grant") => OAuthError::ReauthRequired,
            _ => OAuthError::Refused,
        });
    }
    let access_token = json
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or(OAuthError::Malformed)?
        .to_owned();
    // RFC 6749 §5.1: token_type is case-insensitive; we only use Bearer.
    let bearer = json
        .get("token_type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.eq_ignore_ascii_case("bearer"));
    if !bearer {
        return Err(OAuthError::Malformed);
    }
    Ok(Tokens {
        access_token,
        expires_in: json.get("expires_in").and_then(Value::as_u64),
        refresh_token: json
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned),
    })
}

/// Compares without stopping at the first difference.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn provider(token: &str) -> OAuthProvider {
        OAuthProvider {
            id: "test".into(),
            authorization_endpoint: url::Url::parse("https://auth.example.org/authorize")
                .expect("url"),
            token_endpoint: url::Url::parse(token).expect("url"),
            client_id: "client-1".into(),
            scopes: vec!["mail".into(), "offline".into()],
            redirect_uri: "app.mailcrane:/oauth2redirect".into(),
            extra_auth_params: vec![("prompt".into(), "consent".into())],
        }
    }

    #[test]
    fn the_authorization_url_has_pkce_and_state() {
        let (req, pending) = begin(
            &provider("https://auth.example.org/token"),
            Some("ana@example.org"),
        )
        .expect("begin");
        let q: std::collections::HashMap<String, String> =
            req.url.query_pairs().into_owned().collect();
        assert_eq!(q.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            q.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(q.get("code_challenge"), Some(&pending.verifier.challenge()));
        assert_eq!(q.get("state"), Some(&req.state));
        assert_eq!(q.get("scope").map(String::as_str), Some("mail offline"));
        assert_eq!(
            q.get("login_hint").map(String::as_str),
            Some("ana@example.org")
        );
        assert_eq!(q.get("prompt").map(String::as_str), Some("consent"));
        // The verifier itself never leaves the device before the exchange.
        assert!(!req.url.as_str().contains(pending.verifier.as_str()));
        assert!(!format!("{pending:?}").contains(&req.state));
    }

    #[test]
    fn plain_http_endpoints_are_refused() {
        let mut p = provider("https://auth.example.org/token");
        p.token_endpoint = url::Url::parse("http://auth.example.org/token").expect("url");
        assert!(matches!(begin(&p, None), Err(OAuthError::InvalidProvider)));
    }

    #[tokio::test]
    async fn a_wrong_or_missing_state_is_rejected_before_any_request() {
        let (_, pending) = begin(&provider("https://127.0.0.1:9/token"), None).expect("begin");
        for redirect in [
            "app.mailcrane:/oauth2redirect?code=abc&state=forged",
            "app.mailcrane:/oauth2redirect?code=abc",
            "app.mailcrane:/oauth2redirect?code=abc&state=",
        ] {
            assert_eq!(
                complete(&pending, redirect, &[], Duration::from_secs(1)).await,
                Err(OAuthError::StateMismatch),
                "{redirect}"
            );
        }
        let other_app = format!(
            "evil.app:/oauth2redirect?code=abc&state={}",
            pending.state()
        );
        assert_eq!(
            complete(&pending, &other_app, &[], Duration::from_secs(1)).await,
            Err(OAuthError::MalformedRedirect)
        );
        let denied = format!(
            "app.mailcrane:/oauth2redirect?error=access_denied&state={}",
            pending.state()
        );
        assert_eq!(
            complete(&pending, &denied, &[], Duration::from_secs(1)).await,
            Err(OAuthError::Denied)
        );
    }

    #[test]
    fn tokens_never_show_in_debug() {
        let t = Tokens {
            access_token: "secret-access".into(),
            expires_in: Some(3600),
            refresh_token: Some("secret-refresh".into()),
        };
        let shown = format!("{t:?}");
        assert!(!shown.contains("secret"));
    }
}
