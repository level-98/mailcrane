//! A fake JMAP server (RFC 8620, RFC 8621) over TLS on loopback: a session,
//! mailboxes, emails that can be queried, changed, imported, uploaded,
//! downloaded and submitted. Enough for the client's tests, not a full
//! server. The email state is `e<n>`, starting at `e1`; every change bumps
//! it and is logged, so `Email/changes` reports what happened since a state.
//! From `e0` every email counts as created; a state the fake never gave
//! answers `cannotCalculateChanges`.

use crate::loopback::Loopback;
use crate::tls;
use crate::{Fault, Quirk};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

const CAP_SUBMISSION: &str = "urn:ietf:params:jmap:submission";

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

/// An `EmailSubmission` the fake accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeSubmission {
    pub id: String,
    pub identity_id: String,
    pub email_id: String,
    pub mail_from: String,
    pub rcpt_to: Vec<String>,
}

/// An email as the fake holds it, after any changes.
#[derive(Debug, Clone)]
struct Stored {
    id: String,
    blob_id: String,
    received_at: String,
    subject: String,
    from: String,
    raw: Vec<u8>,
    mailboxes: BTreeSet<String>,
    keywords: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Created,
    Updated,
    Destroyed,
}

#[derive(Debug, Default)]
struct State {
    emails: Vec<Stored>,
    blobs: HashMap<String, Vec<u8>>,
    submissions: Vec<FakeSubmission>,
    next: u32,
    /// The email state number; the state string is `e<version>`.
    version: u64,
    /// Every change: (version it led to, what, email id).
    log: Vec<(u64, Change, String)>,
    /// Emails whose updates and destroys are refused with `forbidden`.
    refused: BTreeSet<String>,
}

impl State {
    fn email_state(&self) -> String {
        format!("e{}", self.version)
    }
    fn bump(&mut self, change: Change, id: &str) {
        self.version += 1;
        self.log.push((self.version, change, id.to_owned()));
    }
    /// `Email/changes` arguments since `since`, or `None` for a state this
    /// fake never gave (RFC 8620 §5.2: `cannotCalculateChanges`).
    fn changes_since(&self, since: &str) -> Option<Value> {
        let n: u64 = since.strip_prefix('e')?.parse().ok()?;
        if n > self.version {
            return None;
        }
        let (mut created, mut updated, mut destroyed) = (Vec::new(), Vec::new(), Vec::new());
        if n == 0 {
            created = self.emails.iter().map(|e| e.id.clone()).collect();
        } else {
            let mut seen: Vec<&str> = Vec::new();
            for (_, _, id) in self.log.iter().filter(|(v, _, _)| *v > n) {
                if seen.contains(&id.as_str()) {
                    continue;
                }
                seen.push(id);
                let kinds: Vec<Change> = self
                    .log
                    .iter()
                    .filter(|(v, _, i)| *v > n && i == id)
                    .map(|(_, c, _)| *c)
                    .collect();
                let was_created = kinds.contains(&Change::Created);
                let gone = kinds.contains(&Change::Destroyed);
                match (was_created, gone) {
                    (true, true) => {}
                    (true, false) => created.push(id.clone()),
                    (false, true) => destroyed.push(id.clone()),
                    (false, false) => updated.push(id.clone()),
                }
            }
        }
        Some(json!({
            "accountId": "A1", "oldState": since, "newState": self.email_state(),
            "hasMoreChanges": false,
            "created": created, "updated": updated, "destroyed": destroyed
        }))
    }
    fn next_id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}-{}", self.next)
    }
    fn email(&mut self, id: &str) -> Option<&mut Stored> {
        self.emails.iter_mut().find(|e| e.id == id)
    }
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
    state: Arc<Mutex<State>>,
    shutdown: Arc<RwLock<Option<JoinHandle<()>>>>,
}

/// What the request handler needs to know.
struct Config {
    port: u16,
    faults: Vec<Fault>,
    user: Option<(String, String)>,
    mailboxes: Vec<FakeMailbox>,
    redirect_session: bool,
    state: Arc<Mutex<State>>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    /// Adds a mailbox to `Mailbox/get`. Without any, Inbox, Drafts, Sent
    /// and Trash.
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
    /// Adds an email. With any, `Email/changes` from `e0` reports every
    /// email as created, from `e1` nothing, and from anything else
    /// `cannotCalculateChanges`.
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
        // The certificate is made once per process (see tls::generate).
        // Starting is quick (see `servers_start_in_under_100_ms`), but it is
        // not timed here: a busy machine would fail unrelated tests (#268).
        let tls = tls::generate()?;
        let listener = Loopback::bind().await?;
        let addr = listener.addr();
        let requests = Arc::new(RwLock::new(Vec::new()));
        let requests_for_server = requests.clone();
        let mailboxes = if self.mailboxes.is_empty() {
            [
                ("mb-inbox", "Inbox", "inbox"),
                ("mb-drafts", "Drafts", "drafts"),
                ("mb-sent", "Sent", "sent"),
                ("mb-trash", "Trash", "trash"),
            ]
            .into_iter()
            .map(|(id, name, role)| FakeMailbox {
                id: id.into(),
                name: name.into(),
                role: Some(role.into()),
                parent_id: None,
            })
            .collect()
        } else {
            self.mailboxes
        };
        let state = Arc::new(Mutex::new(State {
            emails: self
                .emails
                .into_iter()
                .map(|e| Stored {
                    blob_id: format!("blob-{}", e.id),
                    mailboxes: BTreeSet::from([e.mailbox_id]),
                    keywords: if e.seen {
                        BTreeSet::from(["$seen".to_owned()])
                    } else {
                        BTreeSet::new()
                    },
                    raw: e.raw.into_bytes(),
                    id: e.id,
                    received_at: e.received_at,
                    subject: e.subject,
                    from: e.from,
                })
                .collect(),
            version: 1,
            ..State::default()
        }));
        let config = Arc::new(Config {
            port: addr.port(),
            faults: self.faults,
            user: self.user,
            mailboxes,
            redirect_session: self.redirect_session,
            state: state.clone(),
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
        Ok(FakeJmap {
            addr,
            root_cert_der: tls.cert_der,
            requests,
            state,
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
    /// The mailboxes an email is in now, or `None` if it doesn't exist.
    pub fn email_mailboxes(&self, id: &str) -> Option<Vec<String>> {
        lock(&self.state)
            .email(id)
            .map(|e| e.mailboxes.iter().cloned().collect())
    }
    /// An email's keywords now, or `None` if it doesn't exist.
    pub fn email_keywords(&self, id: &str) -> Option<Vec<String>> {
        lock(&self.state)
            .email(id)
            .map(|e| e.keywords.iter().cloned().collect())
    }
    /// An email's raw message, or `None` if it doesn't exist.
    pub fn email_raw(&self, id: &str) -> Option<Vec<u8>> {
        lock(&self.state).email(id).map(|e| e.raw.clone())
    }
    /// The ids of the emails in a mailbox now.
    pub fn emails_in(&self, mailbox_id: &str) -> Vec<String> {
        lock(&self.state)
            .emails
            .iter()
            .filter(|e| e.mailboxes.contains(mailbox_id))
            .map(|e| e.id.clone())
            .collect()
    }
    /// The number of emails on the server, in any mailbox.
    pub fn email_count(&self) -> usize {
        lock(&self.state).emails.len()
    }
    /// Adds an email to the running server, as if it had just arrived.
    pub fn deliver(&self, e: FakeEmail) {
        let mut state = lock(&self.state);
        state.emails.push(Stored {
            blob_id: format!("blob-{}", e.id),
            mailboxes: BTreeSet::from([e.mailbox_id]),
            keywords: if e.seen {
                BTreeSet::from(["$seen".to_owned()])
            } else {
                BTreeSet::new()
            },
            raw: e.raw.into_bytes(),
            id: e.id.clone(),
            received_at: e.received_at,
            subject: e.subject,
            from: e.from,
        });
        state.bump(Change::Created, &e.id);
    }

    /// From now on, changes to email `id` are refused with `forbidden`
    /// (RFC 8620 §5.3 SetError), as for a message the user may not change.
    pub fn refuse_changes(&self, id: &str) {
        lock(&self.state).refused.insert(id.to_owned());
    }

    /// The submissions accepted so far.
    pub fn submissions(&self) -> Vec<FakeSubmission> {
        lock(&self.state).submissions.clone()
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

fn reply_bytes(body: Vec<u8>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::CONTENT_TYPE, "message/rfc822")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
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
            "urn:ietf:params:jmap:core": {"maxSizeRequest": 10_000_000, "maxSizeUpload": 50_000_000},
            "urn:ietf:params:jmap:mail": {},
            CAP_SUBMISSION: {}
        },
        "accounts": {"A1": {"name": "alice@example.org", "isPersonal": true}},
        "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1", CAP_SUBMISSION: "A1"},
        "username": "alice@example.org",
        "apiUrl": format!("{base}/api"),
        "downloadUrl": format!("{base}/download/{{accountId}}/{{blobId}}/{{name}}?type={{type}}"),
        "uploadUrl": format!("{base}/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base}/events"),
        "state": "session-1"
    })
    .to_string()
}

fn email_json(e: &Stored) -> Value {
    let mailboxes: Map<String, Value> = e
        .mailboxes
        .iter()
        .map(|m| (m.clone(), Value::Bool(true)))
        .collect();
    let keywords: Map<String, Value> = e
        .keywords
        .iter()
        .map(|k| (k.clone(), Value::Bool(true)))
        .collect();
    json!({
        "id": e.id, "blobId": e.blob_id, "threadId": format!("t-{}", e.id),
        "mailboxIds": mailboxes, "keywords": keywords,
        "size": e.raw.len(), "receivedAt": e.received_at, "sentAt": e.received_at,
        "messageId": [format!("{}@example.org", e.id)], "inReplyTo": null, "references": null,
        "from": [{"name": null, "email": e.from}], "to": [], "cc": [],
        "subject": e.subject, "hasAttachment": false, "preview": ""
    })
}

/// A header's value from a raw message, enough for the fake.
fn header(raw: &[u8], name: &str) -> String {
    let text = String::from_utf8_lossy(raw);
    let prefix = format!("{}:", name.to_ascii_lowercase());
    text.lines()
        .take_while(|l| !l.is_empty())
        .find(|l| l.to_ascii_lowercase().starts_with(&prefix))
        .map(|l| l[prefix.len()..].trim().to_owned())
        .unwrap_or_default()
}

/// Applies an `Email/set` patch (RFC 8620 §5.3) to an email.
fn apply_patch(e: &mut Stored, patch: &Map<String, Value>) {
    let unescape = |t: &str| t.replace("~1", "/").replace("~0", "~");
    for (path, value) in patch {
        let on = value.as_bool() == Some(true);
        let (set, key) = if let Some(k) = path.strip_prefix("keywords/") {
            (&mut e.keywords, unescape(k))
        } else if let Some(k) = path.strip_prefix("mailboxIds/") {
            (&mut e.mailboxes, unescape(k))
        } else if path == "keywords" || path == "mailboxIds" {
            let whole: BTreeSet<String> = value
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            if path == "keywords" {
                e.keywords = whole;
            } else {
                e.mailboxes = whole;
            }
            continue;
        } else {
            continue;
        };
        if on {
            set.insert(key);
        } else {
            set.remove(&key);
        }
    }
}

/// Resolves `#creationId` references (RFC 8620 §5.3) from earlier calls in
/// the same request.
fn resolve(id: &str, created: &HashMap<String, String>) -> Option<String> {
    match id.strip_prefix('#') {
        Some(cid) => created.get(cid).cloned(),
        None => Some(id.to_owned()),
    }
}

fn email_set(state: &mut State, args: &Value, id: &str) -> Value {
    let old_state = state.email_state();
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    if let Some(update) = args.get("update").and_then(Value::as_object) {
        for (eid, patch) in update {
            if state.refused.contains(eid) {
                not_updated.insert(eid.clone(), json!({"type": "forbidden"}));
                continue;
            }
            match (state.email(eid), patch.as_object()) {
                (Some(e), Some(p)) => {
                    apply_patch(e, p);
                    updated.insert(eid.clone(), Value::Null);
                    state.bump(Change::Updated, eid);
                }
                (None, _) => {
                    not_updated.insert(eid.clone(), json!({"type": "notFound"}));
                }
                (Some(_), None) => {
                    not_updated.insert(eid.clone(), json!({"type": "invalidPatch"}));
                }
            }
        }
    }
    let mut destroyed = Vec::new();
    let mut not_destroyed = Map::new();
    if let Some(ids) = args.get("destroy").and_then(Value::as_array) {
        for eid in ids.iter().filter_map(Value::as_str) {
            if state.refused.contains(eid) {
                not_destroyed.insert(eid.to_owned(), json!({"type": "forbidden"}));
                continue;
            }
            let before = state.emails.len();
            state.emails.retain(|e| e.id != eid);
            if state.emails.len() < before {
                destroyed.push(eid.to_owned());
                state.bump(Change::Destroyed, eid);
            } else {
                not_destroyed.insert(eid.to_owned(), json!({"type": "notFound"}));
            }
        }
    }
    let new_state = state.email_state();
    json!(["Email/set", {
        "accountId": "A1", "oldState": old_state, "newState": new_state,
        "updated": updated, "notUpdated": not_updated,
        "destroyed": destroyed, "notDestroyed": not_destroyed
    }, id])
}

fn email_import(
    state: &mut State,
    args: &Value,
    id: &str,
    created_ids: &mut HashMap<String, String>,
) -> Value {
    let old_state = state.email_state();
    let mut created = Map::new();
    let mut not_created = Map::new();
    if let Some(emails) = args.get("emails").and_then(Value::as_object) {
        for (cid, spec) in emails {
            let blob_id = spec
                .get("blobId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(raw) = state.blobs.get(blob_id).cloned() else {
                not_created.insert(cid.clone(), json!({"type": "blobNotFound"}));
                continue;
            };
            let keys = |k: &str| -> BTreeSet<String> {
                spec.get(k)
                    .and_then(Value::as_object)
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default()
            };
            let new_id = state.next_id("imp");
            state.emails.push(Stored {
                id: new_id.clone(),
                blob_id: blob_id.to_owned(),
                received_at: "2026-11-25T08:00:00Z".into(),
                subject: header(&raw, "Subject"),
                from: header(&raw, "From"),
                raw: raw.clone(),
                mailboxes: keys("mailboxIds"),
                keywords: keys("keywords"),
            });
            created_ids.insert(cid.clone(), new_id.clone());
            created.insert(
                cid.clone(),
                json!({"id": new_id, "blobId": blob_id, "threadId": format!("t-{new_id}"), "size": raw.len()}),
            );
            state.bump(Change::Created, &new_id);
        }
    }
    let new_state = state.email_state();
    json!(["Email/import", {
        "accountId": "A1", "oldState": old_state, "newState": new_state,
        "created": created, "notCreated": not_created
    }, id])
}

/// `EmailSubmission/set` (RFC 8621 §7.5), followed by the implicit
/// `Email/set` for `onSuccessUpdateEmail` and `onSuccessDestroyEmail`.
fn submission_set(
    state: &mut State,
    args: &Value,
    id: &str,
    created_ids: &mut HashMap<String, String>,
) -> Vec<Value> {
    let mut created = Map::new();
    let mut not_created = Map::new();
    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (cid, spec) in create {
            let identity = spec
                .get("identityId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let email = spec
                .get("emailId")
                .and_then(Value::as_str)
                .and_then(|e| resolve(e, created_ids));
            let email = email.filter(|e| state.emails.iter().any(|s| s.id == *e));
            let mail_from = spec
                .pointer("/envelope/mailFrom/email")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let rcpt_to: Vec<String> = spec
                .pointer("/envelope/rcptTo")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|r| r.get("email")?.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            match email {
                Some(email_id) if identity == "id-1" && !rcpt_to.is_empty() => {
                    let sub_id = state.next_id("sub");
                    state.submissions.push(FakeSubmission {
                        id: sub_id.clone(),
                        identity_id: identity.to_owned(),
                        email_id: email_id.clone(),
                        mail_from,
                        rcpt_to,
                    });
                    created_ids.insert(cid.clone(), sub_id.clone());
                    created.insert(cid.clone(), json!({"id": sub_id, "undoStatus": "final"}));
                }
                _ => {
                    not_created.insert(cid.clone(), json!({"type": "invalidProperties"}));
                }
            }
        }
    }
    let mut out = vec![json!(["EmailSubmission/set", {
        "accountId": "A1", "oldState": "s1", "newState": "s2",
        "created": created, "notCreated": not_created
    }, id])];
    // The email a submission (by id or `#creation-id`) was for.
    let email_of = |state: &State, sub_ref: &str| {
        let sub_id = resolve(sub_ref, created_ids)?;
        state
            .submissions
            .iter()
            .find(|s| s.id == sub_id)
            .map(|s| s.email_id.clone())
    };
    let mut update = Map::new();
    if let Some(updates) = args.get("onSuccessUpdateEmail").and_then(Value::as_object) {
        for (sub_ref, patch) in updates {
            if let Some(email_id) = email_of(state, sub_ref) {
                update.insert(email_id, patch.clone());
            }
        }
    }
    let destroy: Vec<String> = args
        .get("onSuccessDestroyEmail")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .filter_map(Value::as_str)
                .filter_map(|r| email_of(state, r))
                .collect()
        })
        .unwrap_or_default();
    if !update.is_empty() || !destroy.is_empty() {
        out.push(email_set(
            state,
            &json!({"update": update, "destroy": destroy}),
            id,
        ));
    }
    out
}

fn method_responses(
    name: &str,
    args: &Value,
    id: &str,
    using: &[&str],
    config: &Config,
    created_ids: &mut HashMap<String, String>,
) -> Vec<Value> {
    // RFC 8620 §3.6.2: a method from a capability not in `using` is unknown.
    let needs_submission = matches!(name, "Identity/get" | "EmailSubmission/set");
    if needs_submission && !using.contains(&CAP_SUBMISSION) {
        return vec![json!(["error", {"type": "unknownMethod"}, id])];
    }
    let mut state = lock(&config.state);
    let state = &mut *state;
    let one = match name {
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
        "Identity/get" => json!(["Identity/get", {
            "accountId": "A1", "state": "i1", "notFound": [],
            "list": [{"id": "id-1", "name": "Alice", "email": "alice@example.org", "mayDelete": false}]
        }, id]),
        "Email/query" if state.emails.is_empty() => {
            json!(["Email/query", {"ids": [], "position": 0, "total": 0}, id])
        }
        "Email/query" => {
            let mailbox = args
                .pointer("/filter/inMailbox")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Newest first; RFC 3339 in UTC sorts as text.
            let mut matching: Vec<&Stored> = state
                .emails
                .iter()
                .filter(|e| e.mailboxes.contains(mailbox))
                .collect();
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
        "Email/get" if state.emails.is_empty() => json!(["Email/get", {"list": []}, id]),
        "Email/get" => {
            let wanted: Vec<&str> = args
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let list: Vec<Value> = wanted
                .iter()
                .filter_map(|w| state.emails.iter().find(|e| e.id == *w))
                .map(email_json)
                .collect();
            let not_found: Vec<&str> = wanted
                .iter()
                .copied()
                .filter(|w| !state.emails.iter().any(|e| e.id == *w))
                .collect();
            json!(["Email/get", {"accountId": "A1", "state": state.email_state(), "list": list, "notFound": not_found}, id])
        }
        // No sinceState at all: the smoke test's bare call gets an empty answer.
        "Email/changes" if args.get("sinceState").is_none() => json!(["Email/changes", {
            "accountId": "A1", "oldState": state.email_state(), "newState": state.email_state(),
            "hasMoreChanges": false, "created": [], "updated": [], "destroyed": []
        }, id]),
        "Email/changes" => match args
            .get("sinceState")
            .and_then(Value::as_str)
            .and_then(|since| state.changes_since(since))
        {
            Some(changes) => json!(["Email/changes", changes, id]),
            None => json!(["error", {"type": "cannotCalculateChanges"}, id]),
        },
        "Email/set" => email_set(state, args, id),
        "Email/import" => email_import(state, args, id, created_ids),
        "EmailSubmission/set" => return submission_set(state, args, id, created_ids),
        _ => json!(["error", {"type": "unknownMethod"}, id]),
    };
    vec![one]
}

async fn body_bytes(request: Request<hyper::body::Incoming>) -> Vec<u8> {
    request
        .collect()
        .await
        .map(|body| body.to_bytes().to_vec())
        .unwrap_or_default()
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
    let is_post = method == Method::POST.as_str();
    Ok(match path.as_str() {
        "/.well-known/jmap" if config.redirect_session => reply(
            StatusCode::TEMPORARY_REDIRECT,
            String::new(),
            Some("/jmap/session"),
        ),
        "/.well-known/jmap" | "/jmap/session" => reply(StatusCode::OK, session(&config), None),
        p if p.starts_with("/download/A1/") => {
            let blob = p.trim_start_matches("/download/A1/").split('/').next();
            let state = lock(&config.state);
            let raw = state
                .emails
                .iter()
                .find(|e| blob == Some(e.blob_id.as_str()))
                .map(|e| e.raw.clone())
                .or_else(|| blob.and_then(|b| state.blobs.get(b).cloned()));
            match raw {
                Some(raw) => reply_bytes(raw),
                None => reply(StatusCode::NOT_FOUND, String::new(), None),
            }
        }
        // RFC 8620 §6.1: the body is the blob; the answer names it.
        "/upload/A1/" if is_post => {
            let content_type = request
                .headers()
                .get(hyper::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_owned();
            let bytes = body_bytes(request).await;
            let mut state = lock(&config.state);
            let blob_id = state.next_id("up");
            let size = bytes.len();
            state.blobs.insert(blob_id.clone(), bytes);
            reply(
                StatusCode::CREATED,
                json!({"accountId": "A1", "blobId": blob_id, "type": content_type, "size": size})
                    .to_string(),
                None,
            )
        }
        "/api" if is_post => {
            let bytes = body_bytes(request).await;
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => {
                    let using: Vec<&str> = v
                        .get("using")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    // RFC 8620 §3.4: responses in order, same call id.
                    let calls = v
                        .get("methodCalls")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut created_ids = HashMap::new();
                    let responses: Vec<Value> = calls
                        .iter()
                        .filter_map(|c| {
                            let c = c.as_array()?;
                            Some(method_responses(
                                c.first()?.as_str()?,
                                c.get(1).unwrap_or(&Value::Null),
                                c.get(2)?.as_str()?,
                                &using,
                                &config,
                                &mut created_ids,
                            ))
                        })
                        .flatten()
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
