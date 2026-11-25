//! A fake OAuth 2.0 token endpoint (RFC 6749 §3.2) over TLS on loopback,
//! for the authorization_code (with PKCE, RFC 7636) and refresh_token
//! grants. Tests stand in for the browser step with [`FakeTokenEndpoint::authorize`].

use crate::loopback::Loopback;
use crate::tls;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::task::JoinHandle;

#[derive(Default)]
struct State {
    /// code → (code_challenge, redirect_uri, client_id)
    codes: HashMap<String, (String, String, String)>,
    /// Refresh tokens that still work.
    refresh_tokens: Vec<String>,
    /// Hand out a new refresh token on every refresh (rotation).
    rotate: bool,
    next: u32,
    /// Every grant_type seen, in order.
    grants: Vec<String>,
}

impl State {
    fn token(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}-{}", self.next)
    }
}

fn lock(s: &Mutex<State>) -> MutexGuard<'_, State> {
    s.lock().unwrap_or_else(|p| p.into_inner())
}

/// A running fake token endpoint.
#[derive(Clone)]
pub struct FakeTokenEndpoint {
    addr: SocketAddr,
    root_cert_der: Vec<u8>,
    state: Arc<Mutex<State>>,
    _handle: Arc<JoinHandle<()>>,
}

impl FakeTokenEndpoint {
    pub async fn start() -> Result<Self, String> {
        let tls = tls::generate()?;
        let listener = Loopback::bind().await?;
        let addr = listener.addr();
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let config = tls.config.clone();
                let state = shared.clone();
                tokio::spawn(async move {
                    let Ok(stream) = tokio_rustls::TlsAcceptor::from(config).accept(stream).await
                    else {
                        return;
                    };
                    let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                        let state = state.clone();
                        async move {
                            let body = req
                                .into_body()
                                .collect()
                                .await
                                .map(|b| b.to_bytes().to_vec())
                                .unwrap_or_default();
                            let (status, reply) = answer(&state, &body);
                            let mut response = Response::new(Full::new(Bytes::from(reply)));
                            *response.status_mut() = status;
                            Ok::<_, Infallible>(response)
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Ok(Self {
            addr,
            root_cert_der: tls.cert_der,
            state,
            _handle: Arc::new(handle),
        })
    }

    /// The token endpoint URL.
    pub fn token_url(&self) -> String {
        format!("https://localhost:{}/token", self.addr.port())
    }

    pub fn root_cert_der(&self) -> &[u8] {
        &self.root_cert_der
    }

    /// The browser step: the user approved the request with this
    /// `code_challenge`; returns the code the provider redirects back with.
    pub fn authorize(&self, code_challenge: &str, redirect_uri: &str, client_id: &str) -> String {
        let mut s = lock(&self.state);
        let code = s.token("code");
        s.codes.insert(
            code.clone(),
            (
                code_challenge.to_owned(),
                redirect_uri.to_owned(),
                client_id.to_owned(),
            ),
        );
        code
    }

    /// Every refresh token stops working (the user revoked access).
    pub fn revoke_all(&self) {
        lock(&self.state).refresh_tokens.clear();
    }

    /// Issue a new refresh token on every refresh.
    pub fn rotate_refresh_tokens(&self) {
        lock(&self.state).rotate = true;
    }

    /// The grant types requested so far.
    pub fn grants(&self) -> Vec<String> {
        lock(&self.state).grants.clone()
    }
}

fn answer(state: &Mutex<State>, body: &[u8]) -> (StatusCode, Vec<u8>) {
    let form: HashMap<String, String> = url::form_urlencoded::parse(body).into_owned().collect();
    let field = |k: &str| form.get(k).cloned().unwrap_or_default();
    let error = |code: &str| {
        (
            StatusCode::BAD_REQUEST,
            json!({"error": code}).to_string().into_bytes(),
        )
    };
    let mut s = lock(state);
    let grant = field("grant_type");
    s.grants.push(grant.clone());
    match grant.as_str() {
        "authorization_code" => {
            let Some((challenge, redirect, client)) = s.codes.remove(&field("code")) else {
                return error("invalid_grant");
            };
            let verified =
                mailcrane_core::oauth::pkce::challenge_s256(&field("code_verifier")) == challenge;
            if !verified || field("redirect_uri") != redirect || field("client_id") != client {
                return error("invalid_grant");
            }
            let access = s.token("access");
            let refresh = s.token("refresh");
            s.refresh_tokens.push(refresh.clone());
            (
                StatusCode::OK,
                json!({"access_token": access, "token_type": "Bearer", "expires_in": 3600, "refresh_token": refresh})
                    .to_string()
                    .into_bytes(),
            )
        }
        "refresh_token" => {
            let old = field("refresh_token");
            if !s.refresh_tokens.contains(&old) {
                return error("invalid_grant");
            }
            let access = s.token("access");
            let mut reply =
                json!({"access_token": access, "token_type": "Bearer", "expires_in": 3600});
            if s.rotate {
                let new = s.token("refresh");
                s.refresh_tokens.retain(|t| t != &old);
                s.refresh_tokens.push(new.clone());
                reply["refresh_token"] = json!(new);
            }
            (StatusCode::OK, reply.to_string().into_bytes())
        }
        _ => error("unsupported_grant_type"),
    }
}
