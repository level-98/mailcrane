//! The OAuth flow end to end against the fake token endpoint (M3-01).

#![allow(clippy::expect_used)] // test helpers may panic

use mailcrane_core::oauth::{begin, complete, OAuthError, OAuthProvider, TokenCache};
use mailcrane_core::secrets::{SecretKey, SecretStore};
use mailcrane_testkit::{FakeTokenEndpoint, MemorySecretStore};
use std::time::Duration;

const REDIRECT: &str = "app.mailcrane:/oauth2redirect";
const T: Duration = Duration::from_secs(10);
/// The database the tokens belong to (a `Store::cache_scope`).
const DB: &str = "file:test.db";

fn provider(server: &FakeTokenEndpoint) -> OAuthProvider {
    OAuthProvider {
        id: "test".into(),
        authorization_endpoint: url::Url::parse("https://auth.example.org/authorize").expect("url"),
        token_endpoint: url::Url::parse(&server.token_url()).expect("url"),
        client_id: "client-1".into(),
        scopes: vec!["mail".into()],
        redirect_uri: REDIRECT.into(),
        extra_auth_params: vec![],
    }
}

/// Starts a sign-in and plays the browser: returns the tokens.
async fn sign_in(server: &FakeTokenEndpoint) -> mailcrane_core::oauth::Tokens {
    let (request, pending) = begin(&provider(server), None).expect("begin");
    let challenge = request
        .url
        .query_pairs()
        .find(|(k, _)| k == "code_challenge")
        .map(|(_, v)| v.into_owned())
        .expect("challenge");
    let code = server.authorize(&challenge, REDIRECT, "client-1");
    let redirect = format!("{REDIRECT}?code={code}&state={}", request.state);
    complete(&pending, &redirect, &[server.root_cert_der().to_vec()], T)
        .await
        .expect("tokens")
}

#[tokio::test]
async fn code_exchange_with_pkce_gives_tokens() {
    let server = FakeTokenEndpoint::start().await.expect("server");
    let tokens = sign_in(&server).await;
    assert!(tokens.access_token.starts_with("access-"));
    assert_eq!(tokens.expires_in, Some(3600));
    assert!(tokens.refresh_token.is_some());
    assert_eq!(server.grants(), ["authorization_code"]);
}

#[tokio::test]
async fn a_code_used_with_the_wrong_verifier_is_refused() {
    let server = FakeTokenEndpoint::start().await.expect("server");
    let (request, pending) = begin(&provider(&server), None).expect("begin");
    // The provider saw a different challenge (another app's request).
    let code = server.authorize("not-our-challenge", REDIRECT, "client-1");
    let redirect = format!("{REDIRECT}?code={code}&state={}", request.state);
    let result = complete(&pending, &redirect, &[server.root_cert_der().to_vec()], T).await;
    assert_eq!(result, Err(OAuthError::ReauthRequired));
}

#[tokio::test]
async fn refresh_tokens_live_only_in_secure_storage_and_are_used_to_refresh() {
    let server = FakeTokenEndpoint::start().await.expect("server");
    server.rotate_refresh_tokens();
    let secrets = MemorySecretStore::new();
    let cache = TokenCache::new();
    let roots = [server.root_cert_der().to_vec()];
    let tokens = sign_in(&server).await;
    let first = cache.save(&secrets, DB, 7, &tokens, 1_000).expect("save");
    assert_eq!(
        secrets.keys(),
        vec![SecretKey::oauth_refresh_token(7).as_str().to_owned()]
    );

    // Still fresh: no request.
    let same = cache
        .access_token(&secrets, DB, 7, &provider(&server), 1_000 + 60, &roots, T)
        .await
        .expect("cached");
    assert_eq!(same, first);
    assert_eq!(server.grants(), ["authorization_code"]);

    // Within 5 minutes of expiry: refreshed, and the rotated refresh token is kept.
    let before = secrets
        .get(&SecretKey::oauth_refresh_token(7))
        .expect("get")
        .expect("stored");
    let fresh = cache
        .access_token(
            &secrets,
            DB,
            7,
            &provider(&server),
            1_000 + 3600 - 200,
            &roots,
            T,
        )
        .await
        .expect("refreshed");
    assert_ne!(fresh, first);
    assert_eq!(server.grants(), ["authorization_code", "refresh_token"]);
    let after = secrets
        .get(&SecretKey::oauth_refresh_token(7))
        .expect("get")
        .expect("stored");
    assert_ne!(
        before.expose(),
        after.expose(),
        "rotated refresh token saved"
    );
    assert!(!format!("{fresh:?}").contains("access-"));
}

#[tokio::test]
async fn invalid_grant_on_refresh_asks_to_sign_in_again() {
    let server = FakeTokenEndpoint::start().await.expect("server");
    let secrets = MemorySecretStore::new();
    let cache = TokenCache::new();
    let roots = [server.root_cert_der().to_vec()];
    let tokens = sign_in(&server).await;
    cache.save(&secrets, DB, 3, &tokens, 0).expect("save");
    server.revoke_all();
    let result = cache
        .access_token(&secrets, DB, 3, &provider(&server), 10_000, &roots, T)
        .await;
    assert_eq!(result, Err(OAuthError::ReauthRequired));
    // The dead refresh token is gone, so the app asks once, not on every sync.
    assert!(secrets.keys().is_empty());
    // And without one, it's ReauthRequired at once, with no request.
    let grants = server.grants().len();
    assert_eq!(
        cache
            .access_token(&secrets, DB, 3, &provider(&server), 10_000, &roots, T)
            .await,
        Err(OAuthError::ReauthRequired)
    );
    assert_eq!(server.grants().len(), grants);
}

#[tokio::test]
async fn the_same_account_id_in_two_databases_never_shares_a_token() {
    // #434: account IDs start at 1 in every database; tests running side by side (or two profiles) each have
    // an account 1, and one removing or replacing its cached access token must not touch the other's. (The
    // refresh tokens in the secret store are still keyed by account ID alone: #437.)
    let server = FakeTokenEndpoint::start().await.expect("server");
    let secrets = MemorySecretStore::new();
    let cache = TokenCache::new();
    let first = cache
        .save(&secrets, "file:a.db", 1, &sign_in(&server).await, 0)
        .expect("save a");
    let other = cache
        .save(&secrets, "file:b.db", 1, &sign_in(&server).await, 0)
        .expect("save b");
    assert_ne!(first, other);
    assert_eq!(cache.cached("file:a.db", 1, 60), Some(first.clone()));
    assert_eq!(cache.cached("file:b.db", 1, 60), Some(other));
    // Removing account 1 from b leaves a's alone.
    cache.forget("file:b.db", 1);
    assert_eq!(cache.cached("file:b.db", 1, 60), None);
    assert_eq!(cache.cached("file:a.db", 1, 60), Some(first));
}
