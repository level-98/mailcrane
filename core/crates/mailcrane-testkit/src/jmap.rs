use crate::loopback::Loopback;
use crate::tls;
use crate::{Fault, Quirk};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmapSentRequest {
    pub method: String,
    pub path: String,
}

pub struct FakeJmapBuilder {
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
}

#[derive(Debug, Clone)]
pub struct FakeJmap {
    addr: SocketAddr,
    root_cert_der: Vec<u8>,
    requests: Arc<RwLock<Vec<JmapSentRequest>>>,
    shutdown: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl FakeJmapBuilder {
    pub fn fault(mut self, fault: Fault) -> Self {
        self.faults.push(fault);
        self
    }
    pub fn quirk(mut self, quirk: Quirk) -> Self {
        self.quirks.push(quirk);
        self
    }
    pub async fn start(self) -> Result<FakeJmap, String> {
        // The certificate is made once per process (see tls::generate);
        // the time limit is for starting the server itself.
        let tls = tls::generate()?;
        let started = std::time::Instant::now();
        let listener = Loopback::bind().await?;
        let addr = listener.addr();
        let requests = Arc::new(RwLock::new(Vec::new()));
        let requests_for_server = requests.clone();
        let faults = Arc::new(self.faults);
        let quirks = Arc::new(self.quirks);
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = requests_for_server.clone();
                let tls_config = tls.config.clone();
                let faults = faults.clone();
                let quirks = quirks.clone();
                tokio::spawn(async move {
                    let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
                    let Ok(stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |request| {
                        let requests = requests.clone();
                        let faults = faults.clone();
                        let quirks = quirks.clone();
                        async move { jmap_response(request, requests, faults, quirks).await }
                    });
                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        });
        if started.elapsed() >= std::time::Duration::from_millis(100) {
            handle.abort();
            return Err("JMAP server start exceeded 100 ms".into());
        }
        Ok(FakeJmap {
            addr,
            root_cert_der: tls.cert_der,
            requests,
            shutdown: Arc::new(RwLock::new(Some(handle))),
        })
    }
}

impl FakeJmap {
    pub fn builder() -> FakeJmapBuilder {
        FakeJmapBuilder {
            faults: Vec::new(),
            quirks: Vec::new(),
        }
    }
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    pub fn root_cert_der(&self) -> &[u8] {
        &self.root_cert_der
    }
    pub async fn requests(&self) -> Vec<JmapSentRequest> {
        self.requests.read().await.clone()
    }
    pub async fn shutdown(&self) {
        if let Some(handle) = self.shutdown.write().await.take() {
            handle.abort();
        }
    }
}

impl Drop for FakeJmap {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shutdown.try_write() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

async fn jmap_response(
    request: Request<hyper::body::Incoming>,
    requests: Arc<RwLock<Vec<JmapSentRequest>>>,
    faults: Arc<Vec<Fault>>,
    _quirks: Arc<Vec<Quirk>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = request.uri().path().to_string();
    let method = request.method().to_string();
    requests.write().await.push(JmapSentRequest {
        method: method.clone(),
        path: path.clone(),
    });
    let authorized = request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if faults.contains(&Fault::RejectLogin) {
        let response = Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Full::new(Bytes::new()));
        return Ok(response.unwrap_or_else(|_| Response::new(Full::new(Bytes::new()))));
    }
    let is_auth =
        authorized.is_some_and(|value| value.starts_with("Basic ") || value.starts_with("Bearer "));
    if !is_auth {
        let response = Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Full::new(Bytes::new()));
        return Ok(response.unwrap_or_else(|_| Response::new(Full::new(Bytes::new()))));
    }
    let body = if path == "/.well-known/jmap" {
        r#"{"apiUrl":"https://localhost/api","username":"alice@example.org","accounts":{}}"#
    } else if path == "/api" && method == Method::POST.as_str() {
        let bytes = request
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        if text.contains("Mailbox/get") {
            r#"{"methodResponses":[["Mailbox/get",{"list":[]}, "0"]]}"#
        } else if text.contains("Email/query") {
            r#"{"methodResponses":[["Email/query",{"ids":[],"position":0,"total":0},"0"]]}"#
        } else if text.contains("Email/get") {
            r#"{"methodResponses":[["Email/get",{"list":[]},"0"]]}"#
        } else if text.contains("Email/changes") {
            r#"{"methodResponses":[["Email/changes",{"oldState":"0","newState":"1","hasMoreChanges":false,"created":[],"updated":[],"destroyed":[]},"0"]]}"#
        } else {
            r#"{"methodResponses":[]}"#
        }
    } else {
        r#"{"error":"not found"}"#
    };
    let status = if path == "/.well-known/jmap" || path == "/api" {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    Ok(Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}")))))
}
