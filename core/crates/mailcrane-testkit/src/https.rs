//! A tiny HTTPS server for tests that make one-off web requests (one-click
//! unsubscribe, M4-10). It records every request and answers with a fixed
//! status, and a `Location` header for redirects.

use crate::loopback::Loopback;
use crate::tls;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// One request the server received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsRequest {
    pub method: String,
    /// Path and query.
    pub target: String,
    /// Header names in lowercase, with their values, in order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpsRequest {
    /// The value of header `name` (any case), if present.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Builds a [`FakeHttps`].
pub struct FakeHttpsBuilder {
    status: u16,
    location: Option<String>,
    delay: Option<std::time::Duration>,
}

/// A running fake HTTPS server on the loopback interface, with a
/// certificate for `localhost` signed by [`FakeHttps::root_cert_der`].
#[derive(Debug, Clone)]
pub struct FakeHttps {
    addr: SocketAddr,
    root_cert_der: Vec<u8>,
    requests: Arc<RwLock<Vec<HttpsRequest>>>,
    handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl FakeHttpsBuilder {
    /// The status to answer with (default 200).
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// A `Location` header to send (for redirects).
    pub fn location(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    /// Waits this long before answering.
    pub fn delay(mut self, delay: std::time::Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    pub async fn start(self) -> Result<FakeHttps, String> {
        let tls = tls::generate()?;
        let listener = Loopback::bind().await?;
        let addr = listener.addr();
        let requests = Arc::new(RwLock::new(Vec::new()));
        let recorded = requests.clone();
        let status = StatusCode::from_u16(self.status).map_err(|e| e.to_string())?;
        let location = self.location;
        let delay = self.delay;
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let recorded = recorded.clone();
                let config = tls.config.clone();
                let location = location.clone();
                tokio::spawn(async move {
                    let acceptor = tokio_rustls::TlsAcceptor::from(config);
                    let Ok(stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                        let recorded = recorded.clone();
                        let location = location.clone();
                        async move {
                            let method = request.method().to_string();
                            let target = request
                                .uri()
                                .path_and_query()
                                .map(|p| p.to_string())
                                .unwrap_or_default();
                            let headers = request
                                .headers()
                                .iter()
                                .map(|(n, v)| {
                                    (
                                        n.as_str().to_owned(),
                                        v.to_str().unwrap_or_default().to_owned(),
                                    )
                                })
                                .collect();
                            let body = request
                                .into_body()
                                .collect()
                                .await
                                .map(|b| b.to_bytes().to_vec())
                                .unwrap_or_default();
                            recorded.write().await.push(HttpsRequest {
                                method,
                                target,
                                headers,
                                body,
                            });
                            if let Some(d) = delay {
                                tokio::time::sleep(d).await;
                            }
                            let mut response = Response::new(Full::new(Bytes::from_static(b"ok")));
                            *response.status_mut() = status;
                            if let Some(l) = location.as_deref().and_then(|l| l.parse().ok()) {
                                response.headers_mut().insert(hyper::header::LOCATION, l);
                            }
                            Ok::<_, Infallible>(response)
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Ok(FakeHttps {
            addr,
            root_cert_der: tls.cert_der,
            requests,
            handle: Arc::new(RwLock::new(Some(handle))),
        })
    }
}

impl FakeHttps {
    pub fn builder() -> FakeHttpsBuilder {
        FakeHttpsBuilder {
            status: 200,
            location: None,
            delay: None,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `https://localhost:<port><path>`.
    pub fn url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.addr.port())
    }

    pub fn root_cert_der(&self) -> &[u8] {
        &self.root_cert_der
    }

    pub async fn requests(&self) -> Vec<HttpsRequest> {
        self.requests.read().await.clone()
    }
}

impl Drop for FakeHttps {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.handle.try_write() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}
