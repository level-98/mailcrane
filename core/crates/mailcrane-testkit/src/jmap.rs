use crate::loopback::Loopback;
use crate::tls;
use crate::{Fault, Quirk};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
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

/// A mailbox the fake serves from `Mailbox/get`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeMailbox {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub parent_id: Option<String>,
}

/// An email the fake serves from `Email/query`, `Email/get` and the
/// download URL (its blob id is `blob-<id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeEmail {
    pub id: String,
    pub mailbox_id: String,
    /// RFC 3339 in UTC, for example `2026-11-25T08:00:00Z`.
    pub received_at: String,
    pub subject: String,
    pub from: String,
    pub seen: bool,
    /// The raw message served by the download URL.
    pub raw: String,
}

pub struct FakeJmapBuilder {
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
    user: Option<(String, String)>,
    mailboxes: Vec<FakeMailbox>,
    emails: Vec<FakeEmail>,
    redirect_session: bool,
}

#[derive(Debug, Clone)]
pub struct FakeJmap {
    addr: SocketAddr,
    root_cert_der: Vec<u8>,
    requests: Arc<RwLock<Vec<JmapSentRequest>>>,
    shutdown: Arc<RwLock<Option<JoinHandle<()>>>>,
}

/// What the request handler needs to know.
struct Config {
    port: u16,
    faults: Vec<Fault>,
    user: Option<(String, String)>,
    mailboxes: Vec<FakeMailbox>,
    emails: Vec<FakeEmail>,
    redirect_session: bool,
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
    /// Only these Basic credentials are accepted (Bearer tokens still are).
    pub fn user(mut self, username: &str, password: &str) -> Self {
        self.user = Some((username.into(), password.into()));
        self
    }
    /// Adds a mailbox to `Mailbox/get`. Without any, Inbox, Sent and Trash.
    pub fn mailbox(
        mut self,
        id: &str,
        name: &str,
        role: Option<&str>,
        parent_id: Option<&str>,
    ) -> Self {
        self.mailboxes.push(FakeMailbox {
            id: id.into(),
            name: name.into(),
            role: role.map(Into::into),
            parent_id: parent_id.map(Into::into),
        });
        self
    }
    /// Adds an email. With any, the email state is `e1`; `Email/changes`
    /// from `e0` reports every email as created, from `e1` nothing, and
    /// from anything else `cannotCalculateChanges`.
    pub fn email(mut self, email: FakeEmail) -> Self {
        self.emails.push(email);
        self
    }
    /// `/.well-known/jmap` answers with a redirect to `/jmap/session`, as
    /// many providers do (RFC 8620 §2.2).
    pub fn redirect_session(mut self) -> Self {
        self.redirect_session = true;
        self
    }
    pub async fn start(self) -> Result<FakeJmap, String> {
        let started = std::time::Instant::now();
        let tls = tls::generate()?;
        let listener = Loopback::bind().await?;
        let addr = listener.addr();
        let requests = Arc::new(RwLock::new(Vec::new()));
        let requests_for_server = requests.clone();
        let mailboxes = if self.mailboxes.is_empty() {
            vec![
                FakeMailbox {
                    id: "mb-inbox".into(),
                    name: "Inbox".into(),
                    role: Some("inbox".into()),
                    parent_id: None,
                },
                FakeMailbox {
                    id: "mb-sent".into(),
                    name: "Sent".into(),
                    role: Some("sent".into()),
                    parent_id: None,
                },
                FakeMailbox {
                    id: "mb-trash".into(),
                    name: "Trash".into(),
                    role: Some("trash".into()),
                    parent_id: None,
                },
            ]
        } else {
            self.mailboxes
        };
        let config = Arc::new(Config {
            port: addr.port(),
            faults: self.faults,
            user: self.user,
            mailboxes,
            emails: self.emails,
            redirect_session: self.redirect_session,
        });
        let _quirks = self.quirks;
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = requests_for_server.clone();
                let tls_config = tls.config.clone();
                let config = config.clone();
                tokio::spawn(async move {
                    let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
                    let Ok(stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |request| {
                        let requests = requests.clone();
                        let config = config.clone();
                        async move { jmap_response(request, requests, config).await }
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
            user: None,
            mailboxes: Vec::new(),
            emails: Vec::new(),
            redirect_session: false,
        }
    }
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    /// `https://localhost:<port>`, the server's base URL.
    pub fn base_url(&self) -> String {
        format!("https://localhost:{}", self.addr.port())
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

fn reply(status: StatusCode, body: String, location: Option<&str>) -> Response<Full<Bytes>> {
    let mut b = Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json");
    if let Some(l) = location {
        b = b.header(hyper::header::LOCATION, l);
    }
    b.body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}"))))
}

/// Standard base64 decoding, enough to check Basic credentials.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| *c != b'=') {
        let v = u32::try_from(A.iter().position(|a| *a == c)?).ok()?;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buf >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

fn authorized(value: Option<&str>, config: &Config) -> bool {
    let Some(v) = value else { return false };
    if v.starts_with("Bearer ") {
        return true;
    }
    let Some(encoded) = v.strip_prefix("Basic ") else {
        return false;
    };
    match &config.user {
        None => true,
        Some((user, pass)) => {
            base64_decode(encoded).is_some_and(|d| d == format!("{user}:{pass}").as_bytes())
        }
    }
}

fn session(config: &Config) -> String {
    let base = format!("https://localhost:{}", config.port);
    json!({
        "capabilities": {
            "urn:ietf:params:jmap:core": {"maxSizeRequest": 10_000_000},
            "urn:ietf:params:jmap:mail": {}
        },
        "accounts": {"A1": {"name": "alice@example.org", "isPersonal": true}},
        "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
        "username": "alice@example.org",
        "apiUrl": format!("{base}/api"),
        "downloadUrl": format!("{base}/download/{{accountId}}/{{blobId}}/{{name}}?type={{type}}"),
        "uploadUrl": format!("{base}/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base}/events"),
        "state": "session-1"
    })
    .to_string()
}

fn email_json(e: &FakeEmail) -> Value {
    let keywords = if e.seen {
        json!({"$seen": true})
    } else {
        json!({})
    };
    json!({
        "id": e.id, "blobId": format!("blob-{}", e.id), "threadId": format!("t-{}", e.id),
        "mailboxIds": {e.mailbox_id.clone(): true}, "keywords": keywords,
        "size": e.raw.len(), "receivedAt": e.received_at, "sentAt": e.received_at,
        "messageId": [format!("{}@example.org", e.id)], "inReplyTo": null, "references": null,
        "from": [{"name": null, "email": e.from}], "to": [], "cc": [],
        "subject": e.subject, "hasAttachment": false, "preview": ""
    })
}

fn method_response(name: &str, args: &Value, id: &str, config: &Config) -> Value {
    let emails = &config.emails;
    match name {
        "Mailbox/get" => {
            let list: Vec<Value> = config
                .mailboxes
                .iter()
                .map(|m| {
                    json!({
                        "id": m.id, "name": m.name, "role": m.role, "parentId": m.parent_id,
                        "totalEmails": 0, "unreadEmails": 0, "sortOrder": 0
                    })
                })
                .collect();
            json!(["Mailbox/get", {"accountId": "A1", "state": "mb-state-1", "list": list, "notFound": []}, id])
        }
        "Email/query" if emails.is_empty() => {
            json!(["Email/query", {"ids": [], "position": 0, "total": 0}, id])
        }
        "Email/query" => {
            let mailbox = args
                .pointer("/filter/inMailbox")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Newest first; RFC 3339 in UTC sorts as text.
            let mut matching: Vec<&FakeEmail> =
                emails.iter().filter(|e| e.mailbox_id == mailbox).collect();
            matching.sort_by(|a, b| b.received_at.cmp(&a.received_at));
            let position = args.get("position").and_then(Value::as_u64).unwrap_or(0);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100);
            let ids: Vec<&str> = matching
                .iter()
                .skip(usize::try_from(position).unwrap_or(usize::MAX))
                .take(usize::try_from(limit).unwrap_or(usize::MAX))
                .map(|e| e.id.as_str())
                .collect();
            json!(["Email/query", {
                "accountId": "A1", "queryState": "q1", "canCalculateChanges": false,
                "position": position, "ids": ids, "total": matching.len()
            }, id])
        }
        "Email/get" if emails.is_empty() => json!(["Email/get", {"list": []}, id]),
        "Email/get" => {
            let wanted: Vec<&str> = args
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let list: Vec<Value> = wanted
                .iter()
                .filter_map(|w| emails.iter().find(|e| e.id == *w))
                .map(email_json)
                .collect();
            let not_found: Vec<&str> = wanted
                .iter()
                .copied()
                .filter(|w| !emails.iter().any(|e| e.id == *w))
                .collect();
            json!(["Email/get", {"accountId": "A1", "state": "e1", "list": list, "notFound": not_found}, id])
        }
        "Email/changes" if emails.is_empty() => json!(["Email/changes", {
            "oldState": "0", "newState": "1", "hasMoreChanges": false,
            "created": [], "updated": [], "destroyed": []
        }, id]),
        "Email/changes" => match args.get("sinceState").and_then(Value::as_str) {
            Some("e1") => json!(["Email/changes", {
                "accountId": "A1", "oldState": "e1", "newState": "e1", "hasMoreChanges": false,
                "created": [], "updated": [], "destroyed": []
            }, id]),
            Some("e0") => {
                let created: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
                json!(["Email/changes", {
                    "accountId": "A1", "oldState": "e0", "newState": "e1", "hasMoreChanges": false,
                    "created": created, "updated": [], "destroyed": []
                }, id])
            }
            _ => json!(["error", {"type": "cannotCalculateChanges"}, id]),
        },
        _ => json!(["error", {"type": "unknownMethod"}, id]),
    }
}

async fn jmap_response(
    request: Request<hyper::body::Incoming>,
    requests: Arc<RwLock<Vec<JmapSentRequest>>>,
    config: Arc<Config>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = request.uri().path().to_string();
    let method = request.method().to_string();
    requests.write().await.push(JmapSentRequest {
        method: method.clone(),
        path: path.clone(),
    });
    if config.faults.contains(&Fault::RejectLogin) {
        return Ok(reply(StatusCode::UNAUTHORIZED, String::new(), None));
    }
    let auth = request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if !authorized(auth, &config) {
        return Ok(reply(StatusCode::UNAUTHORIZED, String::new(), None));
    }
    Ok(match path.as_str() {
        "/.well-known/jmap" if config.redirect_session => reply(
            StatusCode::TEMPORARY_REDIRECT,
            String::new(),
            Some("/jmap/session"),
        ),
        "/.well-known/jmap" | "/jmap/session" => reply(StatusCode::OK, session(&config), None),
        p if p.starts_with("/download/A1/") => {
            let blob = p.trim_start_matches("/download/A1/").split('/').next();
            match config
                .emails
                .iter()
                .find(|e| blob == Some(format!("blob-{}", e.id).as_str()))
            {
                Some(e) => reply(StatusCode::OK, e.raw.clone(), None),
                None => reply(StatusCode::NOT_FOUND, String::new(), None),
            }
        }
        "/api" if method == Method::POST.as_str() => {
            let bytes = request
                .collect()
                .await
                .map(|body| body.to_bytes())
                .unwrap_or_default();
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => {
                    // RFC 8620 §3.4: one response per call, same call id.
                    let calls = v
                        .get("methodCalls")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let responses: Vec<Value> = calls
                        .iter()
                        .filter_map(|c| {
                            let c = c.as_array()?;
                            Some(method_response(
                                c.first()?.as_str()?,
                                c.get(1).unwrap_or(&Value::Null),
                                c.get(2)?.as_str()?,
                                &config,
                            ))
                        })
                        .collect();
                    reply(
                        StatusCode::OK,
                        json!({"methodResponses": responses, "sessionState": "session-1"})
                            .to_string(),
                        None,
                    )
                }
                Err(_) => reply(
                    StatusCode::BAD_REQUEST,
                    json!({"type": "urn:ietf:params:jmap:error:notJSON", "status": 400})
                        .to_string(),
                    None,
                ),
            }
        }
        _ => reply(
            StatusCode::NOT_FOUND,
            r#"{"error":"not found"}"#.into(),
            None,
        ),
    })
}
