use crate::fault::{Fault, Quirk};
use crate::loopback::Loopback;
use crate::mailbox::Mailbox;
use crate::tls;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, RwLock};
use tokio::task::AbortHandle;

/// Per-server settings shared by every connection.
struct Shared {
    mailbox: Mailbox,
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
    user: (String, String),
    /// The access token XOAUTH2 accepts for the user, if it's offered.
    xoauth2_token: Option<String>,
    special_use: Vec<(String, String)>,
    /// Folders listed as `\Noselect` parents (RFC 9051 §7.3.1).
    noselect: Vec<String>,
    /// The personal namespace (RFC 2342) and its delimiter, if any. Without
    /// one, the delimiter is `/`.
    namespace: Option<(String, char)>,
    received: Arc<Mutex<Vec<String>>>,
    /// The folder names SELECT and EXAMINE asked for, in order.
    selected: Arc<Mutex<Vec<String>>>,
    /// The `DropAfterCommandOnce` faults that have fired.
    fired: std::sync::Mutex<Vec<String>>,
}
use tokio::task::JoinHandle;

/// What one connection has selected and enabled.
#[derive(Debug, Default)]
struct ConnState {
    selected: Option<String>,
    /// The literal sent with the current command, if any (RFC 9051 §4.3).
    literal: Option<Vec<u8>>,
    /// `ENABLE QRESYNC` succeeded (RFC 7162 §3.2.3).
    qresync: bool,
    /// The selected folder as this connection was last told about it (UIDs
    /// in sequence order, with mod-sequences): set by `SELECT`, brought up
    /// to date by `IDLE`. A real server keeps this per session and reports
    /// anything newer at the next chance (RFC 9051 §5.2), so mail delivered
    /// between a sync and `IDLE` is announced when `IDLE` starts.
    view: Option<Vec<(u32, u64)>>,
}

#[derive(Debug, Clone)]
pub struct FakeImap {
    addr: SocketAddr,
    starttls_addr: SocketAddr,
    root_cert_der: Vec<u8>,
    mailbox: Mailbox,
    received: Arc<Mutex<Vec<String>>>,
    selected: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<RwLock<Option<JoinHandle<()>>>>,
    connections: Arc<std::sync::Mutex<Vec<AbortHandle>>>,
    accepted: Arc<AtomicUsize>,
}

pub struct FakeImapBuilder {
    user: Option<(String, String)>,
    xoauth2_token: Option<String>,
    folders: Vec<(String, Vec<Vec<u8>>)>,
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
    special_use: Vec<(String, String)>,
    noselect: Vec<String>,
    namespace: Option<(String, char)>,
    mailbox: Option<Mailbox>,
    tls: Option<tls::TestTls>,
    ports: Option<(u16, u16)>,
}

impl FakeImapBuilder {
    /// Serves an existing mailbox, for example another server's, instead of
    /// one built from [`FakeImapBuilder::folder`]. Two servers with
    /// different faults can then share the same messages.
    pub fn mailbox(mut self, mailbox: Mailbox) -> Self {
        self.mailbox = Some(mailbox);
        self
    }

    /// Marks folder `name` with a SPECIAL-USE attribute such as `\Sent`
    /// (RFC 6154). Not sent when [`Quirk::NoSpecialUse`] is set.
    /// Advertises `NAMESPACE` with this personal namespace (RFC 2342), such
    /// as `("INBOX.", '.')`, and uses its delimiter in `LIST` and `RENAME`.
    /// Folder names are still given in full (`INBOX.Projects`).
    pub fn namespace(mut self, prefix: impl Into<String>, delimiter: char) -> Self {
        self.namespace = Some((prefix.into(), delimiter));
        self
    }

    pub fn special_use(mut self, name: impl Into<String>, attribute: impl Into<String>) -> Self {
        self.special_use.push((name.into(), attribute.into()));
        self
    }

    /// Lists `name` as a `\Noselect` parent: it holds other folders but
    /// can't be opened, and SELECT or EXAMINE of it is refused with
    /// `NO [NONEXISTENT]`, as Gmail does for `[Gmail]`.
    pub fn noselect(mut self, name: impl Into<String>) -> Self {
        self.noselect.push(name.into());
        self
    }

    /// Gmail's folder layout: a `[Gmail]` `\Noselect` parent whose children
    /// carry SPECIAL-USE attributes and have spaces in their names, plus
    /// `X-GM-EXT-1` ([`Quirk::GmailLabels`]). Each child starts empty.
    pub fn gmail_layout(mut self) -> Self {
        const CHILDREN: &[(&str, &str)] = &[
            ("[Gmail]/All Mail", "\\All"),
            ("[Gmail]/Drafts", "\\Drafts"),
            ("[Gmail]/Important", "\\Important"),
            ("[Gmail]/Sent Mail", "\\Sent"),
            ("[Gmail]/Spam", "\\Junk"),
            ("[Gmail]/Starred", "\\Flagged"),
            ("[Gmail]/Trash", "\\Trash"),
        ];
        self = self.noselect("[Gmail]");
        for (name, attribute) in CHILDREN {
            self = self
                .folder(*name, Vec::new())
                .special_use(*name, *attribute);
        }
        self.quirk(Quirk::GmailLabels)
    }

    /// Offers `AUTH=XOAUTH2` (Gmail, M3-02) and accepts `token` for the
    /// user. A wrong token gets Google's answer: a `+` with a base64 JSON
    /// error, then `NO` after the client's empty line.
    pub fn xoauth2_token(mut self, token: impl Into<String>) -> Self {
        self.xoauth2_token = Some(token.into());
        self
    }

    pub fn user(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.user = Some((username.into(), password.into()));
        self
    }

    pub fn folder(mut self, name: impl Into<String>, messages: Vec<Vec<u8>>) -> Self {
        self.folders.push((name.into(), messages));
        self
    }

    pub fn fault(mut self, fault: Fault) -> Self {
        self.faults.push(fault);
        self
    }

    pub fn quirk(mut self, quirk: Quirk) -> Self {
        self.quirks.push(quirk);
        self
    }

    /// Serves this certificate instead of the shared self-signed one.
    pub fn tls(mut self, tls: tls::TestTls) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Listens on these ports (implicit TLS, then STARTTLS) instead of free
    /// ones picked at random.
    pub fn ports(mut self, tls_port: u16, starttls_port: u16) -> Self {
        self.ports = Some((tls_port, starttls_port));
        self
    }

    pub async fn start(self) -> Result<FakeImap, String> {
        // The certificate is made once per process (see tls::generate).
        // Starting is quick (see `servers_start_in_under_100_ms`), but it is
        // not timed here: a busy machine would fail unrelated tests (#268).
        let tls = match self.tls {
            Some(tls) => tls,
            None => tls::generate()?,
        };
        // Like a real server (993 and 143), one port is implicit TLS and the
        // other starts in plain text for STARTTLS. Guessing the mode from
        // whether a TLS hello arrives in time fails when the client is slow
        // to build its TLS config (Windows' verifier often is).
        let tls_listener = Loopback::bind_or_any(self.ports.map(|p| p.0)).await?;
        let plain_listener = Loopback::bind_or_any(self.ports.map(|p| p.1)).await?;
        let addr = tls_listener.addr();
        let starttls_addr = plain_listener.addr();
        let mailbox = self.mailbox.unwrap_or_else(|| {
            Mailbox::new(
                &self
                    .folders
                    .iter()
                    .map(|(name, messages)| (name.as_str(), messages.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        let mailbox_for_server = mailbox.clone();
        let user = self
            .user
            .unwrap_or_else(|| ("alice@example.org".into(), "password".into()));
        let received = Arc::new(Mutex::new(Vec::new()));
        let selected = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new(Shared {
            mailbox: mailbox_for_server,
            faults: self.faults,
            quirks: self.quirks,
            user,
            xoauth2_token: self.xoauth2_token,
            special_use: self.special_use,
            noselect: self.noselect,
            namespace: self.namespace,
            received: received.clone(),
            selected: selected.clone(),
            fired: std::sync::Mutex::new(Vec::new()),
        });
        let connections = Arc::new(std::sync::Mutex::new(Vec::<AbortHandle>::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let (loop_connections, loop_accepted) = (connections.clone(), accepted.clone());
        let handle = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    accepted = tls_listener.accept() => accepted.map(|(stream, _)| (stream, true)),
                    accepted = plain_listener.accept() => accepted.map(|(stream, _)| (stream, false)),
                };
                let Ok((stream, implicit_tls)) = accepted else {
                    break;
                };
                let tls_config = tls.config.clone();
                let shared = shared.clone();
                loop_accepted.fetch_add(1, Ordering::SeqCst);
                let task = tokio::spawn(async move {
                    let _ = handle_connection(stream, implicit_tls, tls_config, shared).await;
                });
                if let Ok(mut open) = loop_connections.lock() {
                    open.retain(|h| !h.is_finished());
                    open.push(task.abort_handle());
                }
            }
        });
        Ok(FakeImap {
            addr,
            starttls_addr,
            root_cert_der: tls.cert_der,
            mailbox,
            received,
            selected,
            shutdown: Arc::new(RwLock::new(Some(handle))),
            connections,
            accepted,
        })
    }
}

impl FakeImap {
    pub fn builder() -> FakeImapBuilder {
        FakeImapBuilder {
            user: None,
            xoauth2_token: None,
            folders: Vec::new(),
            faults: Vec::new(),
            quirks: Vec::new(),
            special_use: Vec::new(),
            noselect: Vec::new(),
            namespace: None,
            mailbox: None,
            tls: None,
            ports: None,
        }
    }

    /// The implicit TLS port (like 993).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    /// The port that starts in plain text and offers STARTTLS (like 143).
    pub fn starttls_addr(&self) -> SocketAddr {
        self.starttls_addr
    }
    pub fn root_cert_der(&self) -> &[u8] {
        &self.root_cert_der
    }
    pub fn mailbox(&self) -> Mailbox {
        self.mailbox.clone()
    }

    /// The command names (upper case, without arguments) the server has
    /// received on any connection, in order, including refused ones. `UID`
    /// commands include their subcommand, e.g. `UID FETCH`.
    pub async fn received_commands(&self) -> Vec<String> {
        self.received.lock().await.clone()
    }

    /// The folder names SELECT and EXAMINE asked for, in order, unquoted.
    pub async fn selected_folders(&self) -> Vec<String> {
        self.selected.lock().await.clone()
    }

    /// How many connections the server has accepted so far, on either port.
    pub fn connections_accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Closes every open connection at once, without a `BYE`, as a network
    /// drop or a server restart would. The server keeps accepting new ones.
    pub fn drop_connections(&self) {
        if let Ok(mut open) = self.connections.lock() {
            for handle in open.drain(..) {
                handle.abort();
            }
        }
    }

    pub async fn shutdown(&self) {
        if let Some(handle) = self.shutdown.write().await.take() {
            handle.abort();
        }
    }
}

impl Drop for FakeImap {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shutdown.try_write() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

/// Splits a command line into tokens. A quoted string (RFC 9051 §4.3)
/// stays one token, quotes and escapes included, so names with spaces such
/// as `"[Gmail]/Sent Mail"` survive.
fn split_command(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(bytes.len());
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
        }
        if let Some(token) = text.get(start..i) {
            tokens.push(token);
        }
    }
    tokens
}

/// A token without its quotes and escapes.
fn unquote(token: &str) -> String {
    let inner = token
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(token);
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

async fn record(shared: &Shared, parts: &[&str]) {
    let mut command = parts.get(1).copied().unwrap_or("").to_ascii_uppercase();
    if command == "UID" {
        if let Some(sub) = parts.get(2) {
            command.push(' ');
            command.push_str(&sub.to_ascii_uppercase());
        }
    }
    shared.received.lock().await.push(command);
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    implicit_tls: bool,
    tls_config: Arc<rustls::ServerConfig>,
    shared: Arc<Shared>,
) -> Result<(), String> {
    let faults = &shared.faults;
    let command_count = Arc::new(AtomicUsize::new(0));
    let mut tls_stream = if implicit_tls {
        let mut tls_stream = tokio_rustls::TlsAcceptor::from(tls_config.clone())
            .accept(stream)
            .await
            .map_err(|error| error.to_string())?;
        tls_stream
            .write_all(b"* OK Mailcrane fake IMAP ready\r\n")
            .await
            .map_err(|error| error.to_string())?;
        tls_stream
    } else {
        stream
            .write_all(b"* OK Mailcrane fake IMAP ready\r\n")
            .await
            .map_err(|error| error.to_string())?;
        loop {
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while stream.read_exact(&mut byte).await.is_ok() {
                line.push(byte[0]);
                if line.ends_with(b"\r\n") {
                    break;
                }
            }
            if line.is_empty() {
                return Ok(());
            }
            let text = String::from_utf8_lossy(&line);
            let parts: Vec<&str> = split_command(&text);
            let tag = parts.first().copied().unwrap_or("");
            let command = parts.get(1).copied().unwrap_or("").to_ascii_uppercase();
            record(&shared, &parts).await;
            if parts
                .get(1)
                .is_some_and(|value| value.eq_ignore_ascii_case("STARTTLS"))
            {
                if faults.contains(&Fault::NoStartTls) {
                    stream
                        .write_all(format!("{tag} NO STARTTLS unavailable\r\n").as_bytes())
                        .await
                        .map_err(|error| error.to_string())?;
                    continue;
                }
                stream
                    .write_all(format!("{tag} OK begin STARTTLS\r\n").as_bytes())
                    .await
                    .map_err(|error| error.to_string())?;
                break tokio_rustls::TlsAcceptor::from(tls_config)
                    .accept(stream)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            if ["CAPABILITY", "NOOP", "LOGOUT"]
                .iter()
                .any(|allowed| *allowed == command)
            {
                handle_imap_command(
                    &mut stream,
                    parts.as_slice(),
                    &shared,
                    false,
                    &command_count,
                    &mut ConnState::default(),
                )
                .await?;
            } else {
                stream
                    .write_all(format!("{tag} BAD command refused before STARTTLS\r\n").as_bytes())
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
    };

    let mut state = ConnState::default();
    loop {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        state.literal = None;
        while tls_stream.read_exact(&mut byte).await.is_ok() {
            line.push(byte[0]);
            if line.ends_with(b"\r\n") {
                // A synchronizing literal `{n}`: say go ahead, read n bytes,
                // then the rest of the command line (RFC 9051 §4.3).
                if let Some(size) = literal_size(&line) {
                    line.truncate(line.len() - 2);
                    while line.last().is_some_and(|b| *b != b'{') {
                        line.pop();
                    }
                    line.pop();
                    line.extend_from_slice(b"{LITERAL}");
                    if tls_stream.write_all(b"+ go ahead\r\n").await.is_err()
                        || tls_stream.flush().await.is_err()
                    {
                        return Ok(());
                    }
                    let mut literal = vec![0u8; size];
                    if tls_stream.read_exact(&mut literal).await.is_err() {
                        return Ok(());
                    }
                    state.literal = Some(literal);
                    continue;
                }
                break;
            }
        }
        if line.is_empty() {
            break;
        }
        let text = String::from_utf8_lossy(&line);
        let parts: Vec<&str> = split_command(&text);
        record(&shared, &parts).await;
        if handle_imap_command(
            &mut tls_stream,
            parts.as_slice(),
            &shared,
            true,
            &command_count,
            &mut state,
        )
        .await
        .is_err()
        {
            break;
        }
    }
    Ok(())
}

async fn handle_imap_command(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    parts: &[&str],
    shared: &Shared,
    secure: bool,
    command_count: &AtomicUsize,
    state: &mut ConnState,
) -> Result<(), String> {
    let (mailbox, faults, quirks, user) = (
        &shared.mailbox,
        &shared.faults,
        &shared.quirks,
        &shared.user,
    );
    let count = command_count.fetch_add(1, Ordering::SeqCst) + 1;
    let tag = parts.first().copied().unwrap_or("*");
    let command = parts.get(1).copied().unwrap_or("").to_ascii_uppercase();
    let line = parts
        .get(1..)
        .unwrap_or_default()
        .join(" ")
        .to_ascii_uppercase();
    for fault in faults {
        if let Fault::RefuseCommand { folder, text } = fault {
            let in_folder = folder
                .as_ref()
                .is_none_or(|f| state.selected.as_deref() == Some(f.as_str()));
            if in_folder && line.contains(&text.to_ascii_uppercase()) {
                stream
                    .write_all(format!("{tag} NO refused by the test\r\n").as_bytes())
                    .await
                    .map_err(|error| error.to_string())?;
                stream.flush().await.map_err(|error| error.to_string())?;
                return Ok(());
            }
        }
    }
    for fault in faults {
        if let Fault::Delay(duration) = fault {
            tokio::time::sleep(*duration).await;
        }
    }
    if faults.contains(&Fault::MalformedResponse) {
        stream
            .write_all(b"\xFF\xFEgarbage\r\n")
            .await
            .map_err(|error| error.to_string())?;
        stream.flush().await.map_err(|error| error.to_string())?;
        return Ok(());
    }
    let response = match command.as_str() {
        "CAPABILITY" => {
            // IMAP4rev2 includes MOVE and UID EXPUNGE (RFC 9051 §1), so a server
            // without either speaks only IMAP4rev1; it keeps SPECIAL-USE through
            // LIST-EXTENDED (RFC 6154 §5.1).
            let mut caps = if quirks.contains(&Quirk::NoMove) || quirks.contains(&Quirk::NoUidplus)
            {
                "IMAP4rev1 LIST-EXTENDED".to_string()
            } else {
                "IMAP4rev2 IMAP4rev1".to_string()
            };
            if secure {
                caps.push_str(" AUTH=PLAIN");
                if shared.xoauth2_token.is_some() {
                    caps.push_str(" AUTH=XOAUTH2");
                }
            } else {
                // RFC 9051 §6.2.1 and §7.2.2: offer STARTTLS, refuse login.
                if !faults.contains(&Fault::NoStartTls) {
                    caps.push_str(" STARTTLS");
                }
                caps.push_str(" LOGINDISABLED");
            }
            if !quirks.contains(&Quirk::NoCondstore) {
                caps.push_str(" CONDSTORE");
            }
            // QRESYNC needs CONDSTORE (RFC 7162 §3.2).
            if !quirks.contains(&Quirk::NoQresync) && !quirks.contains(&Quirk::NoCondstore) {
                caps.push_str(" QRESYNC");
            }
            if !quirks.contains(&Quirk::NoSpecialUse) {
                caps.push_str(" SPECIAL-USE");
            }
            if shared.namespace.is_some() {
                caps.push_str(" NAMESPACE");
            }
            if !quirks.contains(&Quirk::NoUidplus) {
                caps.push_str(" UIDPLUS");
            }
            if !quirks.contains(&Quirk::NoIdle) {
                caps.push_str(" IDLE");
            }
            if !quirks.contains(&Quirk::NoMove) {
                caps.push_str(" MOVE");
            }
            if quirks.contains(&Quirk::GmailLabels) {
                caps.push_str(" X-GM-EXT-1");
            }
            format!("* CAPABILITY {caps}\r\n{tag} OK CAPABILITY completed\r\n")
        }
        "LOGIN" => {
            let reject = faults.contains(&Fault::RejectLogin);
            let ok = !reject
                && parts.get(2).is_some_and(|name| *name == user.0)
                && parts.get(3).is_some_and(|pass| *pass == user.1);
            if ok {
                format!("{tag} OK LOGIN completed\r\n")
            } else {
                format!("{tag} NO authentication failed\r\n")
            }
        }
        "AUTHENTICATE"
            if parts
                .get(2)
                .is_some_and(|m| m.eq_ignore_ascii_case("XOAUTH2")) =>
        {
            let Some(expected) = shared.xoauth2_token.as_deref() else {
                return stream
                    .write_all(format!("{tag} NO unsupported mechanism\r\n").as_bytes())
                    .await
                    .map_err(|error| error.to_string());
            };
            stream
                .write_all(b"+ \r\n")
                .await
                .map_err(|error| error.to_string())?;
            stream.flush().await.map_err(|error| error.to_string())?;
            let mut response = [0u8; 2048];
            let read = stream
                .read(&mut response)
                .await
                .map_err(|error| error.to_string())?;
            let line = String::from_utf8_lossy(response.get(..read).unwrap_or_default());
            let decoded = decode_base64(line.trim()).unwrap_or_default();
            let want = format!("user={}\x01auth=Bearer {expected}\x01\x01", user.0);
            if decoded == want.as_bytes() && !faults.contains(&Fault::RejectLogin) {
                format!("{tag} OK AUTHENTICATE completed\r\n")
            } else {
                // {"status":"401","schemes":"Bearer","scope":"https://mail.google.com/"}
                stream
                    .write_all(b"+ eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiQmVhcmVyIiwic2NvcGUiOiJodHRwczovL21haWwuZ29vZ2xlLmNvbS8ifQ==\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                stream.flush().await.map_err(|error| error.to_string())?;
                let mut empty = [0u8; 16];
                let _ = stream
                    .read(&mut empty)
                    .await
                    .map_err(|error| error.to_string())?;
                format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)\r\n")
            }
        }
        "AUTHENTICATE" => {
            if faults.contains(&Fault::RejectLogin) {
                format!("{tag} NO authentication failed\r\n")
            } else {
                stream
                    .write_all(b"+ \r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                stream.flush().await.map_err(|error| error.to_string())?;
                let mut response = [0u8; 1024];
                let read = stream
                    .read(&mut response)
                    .await
                    .map_err(|error| error.to_string())?;
                if read == 0 {
                    return Err("AUTHENTICATE PLAIN closed before challenge".into());
                }
                // RFC 4616: authzid NUL authcid NUL passwd, base64 encoded.
                let line = String::from_utf8_lossy(response.get(..read).unwrap_or_default());
                let decoded = decode_base64(line.trim()).unwrap_or_default();
                let mut fields = decoded.split(|byte| *byte == 0).skip(1);
                let ok = fields.next() == Some(user.0.as_bytes())
                    && fields.next() == Some(user.1.as_bytes());
                if ok {
                    format!("{tag} OK AUTHENTICATE completed\r\n")
                } else {
                    format!("{tag} NO [AUTHENTICATIONFAILED] authentication failed\r\n")
                }
            }
        }
        "LIST" => {
            let mut listed = vec!["INBOX".to_string()];
            listed.extend(
                mailbox
                    .folder_names()
                    .await
                    .into_iter()
                    .filter(|name| name != "INBOX"),
            );
            let mut output = String::new();
            let delim = shared.namespace.as_ref().map_or('/', |(_, d)| *d);
            for name in &shared.noselect {
                let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
                output.push_str(&format!(
                    "* LIST (\\HasChildren \\Noselect) \"{delim}\" \"{quoted}\"\r\n"
                ));
            }
            for name in &listed {
                let prefix = format!("{name}{delim}");
                let mut attributes = if listed.iter().any(|other| other.starts_with(&prefix)) {
                    "\\HasChildren".to_string()
                } else {
                    "\\HasNoChildren".to_string()
                };
                if !quirks.contains(&Quirk::NoSpecialUse) {
                    for (folder, attribute) in &shared.special_use {
                        if folder == name {
                            attributes.push(' ');
                            attributes.push_str(attribute);
                        }
                    }
                }
                let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
                output.push_str(&format!(
                    "* LIST ({attributes}) \"{delim}\" \"{quoted}\"\r\n"
                ));
            }
            if quirks.contains(&Quirk::GmailLabels)
                && !listed.iter().any(|n| n == "[Gmail]/All Mail")
            {
                output.push_str("* LIST (\\All) \"/\" \"[Gmail]/All Mail\"\r\n");
                output.push_str("* XLIST (\\HasNoChildren) \"/\" \"[Gmail]/All Mail\"\r\n");
            }
            output.push_str(&format!("{tag} OK LIST completed\r\n"));
            output
        }
        "SELECT" | "EXAMINE" => {
            let unquoted = unquote(parts.get(2).copied().unwrap_or("INBOX"));
            let folder = unquoted.as_str();
            shared.selected.lock().await.push(unquoted.clone());
            let params = parts.get(3..).unwrap_or_default().join(" ");
            let qresync = qresync_params(&params);
            if shared.noselect.iter().any(|n| n == folder) {
                format!("{tag} NO [NONEXISTENT] Unknown Mailbox: {folder} (Failure)\r\n")
            } else if folder != "INBOX" && !mailbox.folder_names().await.iter().any(|n| n == folder)
            {
                format!("{tag} NO [NONEXISTENT] no such folder\r\n")
            } else if !params.is_empty() && (qresync.is_none() || !state.qresync) {
                // RFC 7162 §3.2.5: QRESYNC parameters need ENABLE first.
                format!("{tag} BAD unexpected SELECT parameters\r\n")
            } else {
                state.selected = Some(folder.to_string());
                state.view = Some(snapshot(mailbox, folder).await);
                select_response(mailbox, quirks, folder, qresync, tag, &command).await
            }
        }
        "UID" => {
            let folder = state.selected.as_deref().unwrap_or("INBOX");
            handle_uid_command(parts, mailbox, quirks, folder).await
        }
        "ENABLE" => {
            // RFC 5161 §3.1: list only the extensions that were enabled.
            let can_qresync =
                !quirks.contains(&Quirk::NoQresync) && !quirks.contains(&Quirk::NoCondstore);
            let mut enabled = Vec::new();
            for wanted in parts.get(2..).unwrap_or_default() {
                let wanted = wanted.to_ascii_uppercase();
                let ok = match wanted.as_str() {
                    "QRESYNC" => can_qresync,
                    "CONDSTORE" => !quirks.contains(&Quirk::NoCondstore),
                    _ => false,
                };
                if ok {
                    if wanted == "QRESYNC" {
                        state.qresync = true;
                    }
                    enabled.push(wanted);
                }
            }
            let list: String = enabled.iter().map(|e| format!(" {e}")).collect();
            format!("* ENABLED{list}\r\n{tag} OK ENABLE completed\r\n")
        }
        "EXPUNGE" => {
            // A plain EXPUNGE removes every \Deleted message (RFC 9051 §6.4.3).
            let folder = state.selected.as_deref().unwrap_or("INBOX");
            mailbox.expunge_deleted(folder, |_| true).await;
            format!("{tag} OK EXPUNGE completed\r\n")
        }
        "CREATE" => {
            // RFC 9051 §6.3.4; an existing name is refused with ALREADYEXISTS.
            let name = parts.get(2).copied().unwrap_or("").trim_matches('"');
            if name.is_empty() {
                format!(
                    "{tag} BAD CREATE needs a name
"
                )
            } else if name.eq_ignore_ascii_case("INBOX")
                || mailbox.folder_names().await.iter().any(|n| n == name)
            {
                format!(
                    "{tag} NO [ALREADYEXISTS] folder exists
"
                )
            } else {
                mailbox.create_folder(name).await;
                format!(
                    "{tag} OK CREATE completed
"
                )
            }
        }
        "RENAME" => {
            // RFC 9051 §6.3.6: children move with it; INBOX and an existing
            // target are refused.
            let old = parts.get(2).copied().unwrap_or("").trim_matches('"');
            let new = parts.get(3).copied().unwrap_or("").trim_matches('"');
            let names = mailbox.folder_names().await;
            if old.is_empty() || new.is_empty() {
                format!("{tag} BAD RENAME needs two names\r\n")
            } else if old.eq_ignore_ascii_case("INBOX") || !names.iter().any(|n| n == old) {
                format!("{tag} NO [NONEXISTENT] no such folder\r\n")
            } else if new.eq_ignore_ascii_case("INBOX") || names.iter().any(|n| n == new) {
                format!("{tag} NO [ALREADYEXISTS] folder exists\r\n")
            } else {
                let delim = shared.namespace.as_ref().map_or('/', |(_, d)| *d);
                mailbox.rename_folder_with(old, new, delim).await;
                format!("{tag} OK RENAME completed\r\n")
            }
        }
        "DELETE" => {
            // RFC 9051 §6.3.5.
            let name = parts.get(2).copied().unwrap_or("").trim_matches('"');
            if name.eq_ignore_ascii_case("INBOX") {
                format!("{tag} NO [CANNOT] the inbox can't be deleted\r\n")
            } else if !mailbox.folder_names().await.iter().any(|n| n == name) {
                format!("{tag} NO [NONEXISTENT] no such folder\r\n")
            } else {
                mailbox.delete_folder(name).await;
                format!("{tag} OK DELETE completed\r\n")
            }
        }
        "NAMESPACE" => {
            // RFC 2342 §5: only a personal namespace, if one was set.
            let personal = match &shared.namespace {
                Some((prefix, delim)) => format!("((\"{prefix}\" \"{delim}\"))"),
                None => "NIL".to_string(),
            };
            format!("* NAMESPACE {personal} NIL NIL\r\n{tag} OK NAMESPACE completed\r\n")
        }
        "NOOP" => "* 0 RECENT\r\n".to_string() + &format!("{tag} OK NOOP completed\r\n"),
        "LOGOUT" => format!("{tag} OK LOGOUT completed\r\n"),
        "APPEND" => {
            // `APPEND <folder> [(<flags>)] {LITERAL}` (RFC 9051 §6.3.12).
            let folder = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim_matches('"')
                .to_string();
            let flags: Vec<String> = parts
                .iter()
                .skip(3)
                .take_while(|p| **p != "{LITERAL}")
                .map(|p| p.trim_matches(|c| c == '(' || c == ')').to_string())
                .filter(|p| !p.is_empty())
                .collect();
            match state.literal.take() {
                None => format!("{tag} BAD APPEND needs a message\r\n"),
                Some(raw) => match mailbox.append(&folder, raw, flags).await {
                    None => format!("{tag} NO [TRYCREATE] no such folder\r\n"),
                    Some(uid) if !quirks.contains(&Quirk::NoUidplus) => format!(
                        "{tag} OK [APPENDUID {} {uid}] APPEND completed\r\n",
                        mailbox.uidvalidity()
                    ),
                    Some(_) => format!("{tag} OK APPEND completed\r\n"),
                },
            }
        }
        "IDLE" => {
            if quirks.contains(&Quirk::NoIdle) {
                format!("{tag} BAD IDLE disabled\r\n")
            } else {
                let folder = state.selected.clone().unwrap_or_else(|| "INBOX".into());
                idle(stream, tag, shared, &folder, &mut state.view).await?
            }
        }
        _ => format!("{tag} BAD unknown command\r\n"),
    };
    if faults.contains(&Fault::DropMidResponse) {
        if let Some(split) = response.len().checked_div(2) {
            stream
                .write_all(response.as_bytes().get(..split).unwrap_or_default())
                .await
                .map_err(|error| error.to_string())?;
            // Close the connection with the response unfinished.
            return Err("dropped mid-response".into());
        }
    }
    for fault in faults {
        if let Fault::DropAfterCommands(limit) = fault {
            if count >= *limit {
                // Close the connection without answering this command.
                return Err("dropped after commands".into());
            }
        }
        if let Fault::DropAfterCommandOnce(text) = fault {
            if line.contains(&text.to_ascii_uppercase()) {
                let first = match shared.fired.lock() {
                    Ok(mut fired) if !fired.contains(text) => {
                        fired.push(text.clone());
                        true
                    }
                    _ => false,
                };
                if first {
                    // Carried out above; the reply never arrives.
                    return Err("dropped after the command".into());
                }
            }
        }
    }
    if faults.contains(&Fault::TempFailure) {
        stream
            .write_all(format!("{tag} NO temporary failure\r\n").as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        stream.flush().await.map_err(|error| error.to_string())?;
        return Ok(());
    }
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream.flush().await.map_err(|error| error.to_string())
}

/// `IDLE` (RFC 2177): answers `+`, then reports deliveries (`EXISTS`),
/// expunges (`EXPUNGE`) and flag changes (`FETCH`) in `folder` as they
/// happen, until the client sends `DONE`. Changes since the connection's
/// `view` (what it was last told) come first. Returns the tagged reply.
async fn idle(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    tag: &str,
    shared: &Shared,
    folder: &str,
    view: &mut Option<Vec<(u32, u64)>>,
) -> Result<String, String> {
    let mailbox = &shared.mailbox;
    stream
        .write_all(b"+ idling\r\n")
        .await
        .map_err(|error| error.to_string())?;
    stream.flush().await.map_err(|error| error.to_string())?;
    // What the client knows: UIDs in sequence order, with mod-sequences.
    let mut known = match view.take() {
        Some(known) => known,
        None => snapshot(mailbox, folder).await,
    };
    let mut line = Vec::new();
    loop {
        // Register before looking, so no change slips between the two.
        let notified = mailbox.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let now = mailbox.messages(folder).await;
        let mut out = String::new();
        // RFC 9051 §7.5.1: each EXPUNGE renumbers the ones after it.
        let mut index = 0;
        while let Some((uid, _)) = known.get(index).copied() {
            if now.iter().any(|m| m.uid == uid) {
                index += 1;
            } else {
                out.push_str(&format!("* {} EXPUNGE\r\n", index + 1));
                known.remove(index);
            }
        }
        for (index, (uid, modseq)) in known.iter_mut().enumerate() {
            if let Some(message) = now.iter().find(|m| m.uid == *uid) {
                if message.modseq != *modseq {
                    *modseq = message.modseq;
                    out.push_str(&format!(
                        "* {} FETCH (UID {} FLAGS ({}))\r\n",
                        index + 1,
                        uid,
                        message.flags.join(" ")
                    ));
                }
            }
        }
        let before = known.len();
        for message in &now {
            if !known.iter().any(|(uid, _)| *uid == message.uid) {
                known.push((message.uid, message.modseq));
            }
        }
        if known.len() != before {
            out.push_str(&format!("* {} EXISTS\r\n", known.len()));
        }
        if !out.is_empty() {
            stream
                .write_all(out.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            stream.flush().await.map_err(|error| error.to_string())?;
        }
        let mut byte = [0u8; 1];
        tokio::select! {
            () = &mut notified => {}
            read = stream.read(&mut byte) => match read {
                Ok(1) => {
                    line.push(byte[0]);
                    if line.ends_with(b"\r\n") {
                        let done = line.trim_ascii().eq_ignore_ascii_case(b"DONE");
                        shared.received.lock().await.push(
                            String::from_utf8_lossy(line.trim_ascii()).to_ascii_uppercase(),
                        );
                        *view = Some(known);
                        return Ok(if done {
                            format!("{tag} OK IDLE terminated\r\n")
                        } else {
                            format!("{tag} BAD expected DONE\r\n")
                        });
                    }
                }
                _ => return Err("connection closed during IDLE".into()),
            },
        }
    }
}

/// The folder's UIDs in sequence order, with their mod-sequences.
async fn snapshot(mailbox: &Mailbox, folder: &str) -> Vec<(u32, u64)> {
    mailbox
        .messages(folder)
        .await
        .iter()
        .map(|m| (m.uid, m.modseq))
        .collect()
}

/// `(QRESYNC (uidvalidity modseq [known-uids]))` (RFC 7162 §3.2.5) from
/// the text after the folder name.
struct QresyncParams {
    uidvalidity: u32,
    modseq: u64,
    known: Option<String>,
}

fn qresync_params(text: &str) -> Option<QresyncParams> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    let keyword = inner.get(..7)?;
    if !keyword.eq_ignore_ascii_case("QRESYNC") {
        return None;
    }
    let inner = inner.get(7..)?.trim();
    let inner = inner.strip_prefix('(')?.strip_suffix(')')?;
    let mut words = inner.split_whitespace();
    let uidvalidity = words.next()?.parse().ok()?;
    let modseq = words.next()?.parse().ok()?;
    let known = words.next().map(str::to_string);
    Some(QresyncParams {
        uidvalidity,
        modseq,
        known,
    })
}

async fn select_response(
    mailbox: &Mailbox,
    quirks: &[Quirk],
    folder: &str,
    qresync: Option<QresyncParams>,
    tag: &str,
    command: &str,
) -> String {
    let count = mailbox.exists(folder).await;
    let uidvalidity = mailbox.uidvalidity();
    let mut out = format!(
        "* {count} EXISTS\r\n* OK [UIDVALIDITY {uidvalidity}]\r\n* OK [UIDNEXT {}]\r\n",
        mailbox.uidnext(folder).await
    );
    // RFC 9051 §7.1: `\*` means new keywords can be stored permanently.
    if quirks.contains(&Quirk::NoCustomKeywords) {
        out.push_str("* OK [PERMANENTFLAGS (\\Seen \\Answered \\Flagged \\Deleted \\Draft)]\r\n");
    } else {
        out.push_str(
            "* OK [PERMANENTFLAGS (\\Seen \\Answered \\Flagged \\Deleted \\Draft \\*)]\r\n",
        );
    }
    if quirks.contains(&Quirk::NoCondstore) {
        return out + &format!("{tag} OK {command} completed\r\n");
    }
    let highest = mailbox.highest_modseq(folder).await;
    match highest {
        // RFC 7162 §3.1.2.1 and §3.1.2.2.
        Some(highest) => out.push_str(&format!("* OK [HIGHESTMODSEQ {highest}]\r\n")),
        None => out.push_str("* OK [NOMODSEQ]\r\n"),
    }
    // RFC 7162 §3.2.5.2: changes only when UIDVALIDITY still matches.
    if let (Some(params), Some(_)) = (qresync, highest) {
        if params.uidvalidity == uidvalidity {
            let max = mailbox.uidnext(folder).await.saturating_sub(1);
            let known = |uid: u32| {
                params
                    .known
                    .as_deref()
                    .is_none_or(|set| in_uid_set(set, uid, max))
            };
            let changes = mailbox.changes_since(folder, params.modseq, known).await;
            if !changes.vanished.is_empty() {
                let list: Vec<String> = changes.vanished.iter().map(u32::to_string).collect();
                out.push_str(&format!("* VANISHED (EARLIER) {}\r\n", list.join(",")));
            }
            for (seq, message) in changes.changed {
                out.push_str(&format!(
                    "* {seq} FETCH (UID {} FLAGS ({}) MODSEQ ({}))\r\n",
                    message.uid,
                    message.flags.join(" "),
                    message.modseq
                ));
            }
        }
    }
    out + &format!("{tag} OK {command} completed\r\n")
}

/// Decodes standard base64 (RFC 4648 §4); `None` if it is not valid.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes().filter(|byte| *byte != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = ((acc << 6) | u32::from(value)) & 0xFFFF;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// Parses an IMAP set such as `1:5,9,12:*` against the UIDs present.
/// Whether `token` is a sequence set such as `1:4096`, `4097:*` or `3,5`.
fn is_sequence_set(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b':' || b == b',' || b == b'*')
}

fn in_uid_set(set: &str, uid: u32, max: u32) -> bool {
    set.split(',').any(|item| {
        let bound = |text: &str| {
            if text == "*" {
                Some(max)
            } else {
                text.parse::<u32>().ok()
            }
        };
        match item.split_once(':') {
            Some((a, b)) => match (bound(a), bound(b)) {
                (Some(a), Some(b)) => (a.min(b)..=a.max(b)).contains(&uid),
                _ => false,
            },
            None => bound(item) == Some(uid),
        }
    })
}

/// Whether message `m` matches every search key in `keys` (TEXT, FROM, TO,
/// SUBJECT with one-word values; UNSEEN; FLAGGED). Case-insensitive
/// substring matches, like most servers.
fn search_keys_match(m: &crate::mailbox::Message, keys: &[&str]) -> bool {
    let text = String::from_utf8_lossy(&m.raw).to_lowercase();
    let mut i = 0;
    while let Some(key) = keys.get(i) {
        let value = keys.get(i + 1).map(|v| v.trim_matches('"').to_lowercase());
        let ok = match key.to_ascii_uppercase().as_str() {
            "TEXT" => value.as_ref().is_some_and(|v| text.contains(v.as_str())),
            "FROM" => value
                .as_ref()
                .is_some_and(|v| header_contains(&m.raw, "From", v)),
            "TO" => value
                .as_ref()
                .is_some_and(|v| header_contains(&m.raw, "To", v)),
            "SUBJECT" => value
                .as_ref()
                .is_some_and(|v| header_contains(&m.raw, "Subject", v)),
            "UNSEEN" => {
                i += 1;
                if m.flags.iter().any(|f| f == "\\Seen") {
                    return false;
                }
                continue;
            }
            "FLAGGED" => {
                i += 1;
                if !m.flags.iter().any(|f| f == "\\Flagged") {
                    return false;
                }
                continue;
            }
            _ => false,
        };
        if !ok {
            return false;
        }
        i += 2;
    }
    true
}

/// The size of a synchronizing literal `{n}` that ends a command line.
fn literal_size(line: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(line).ok()?.strip_suffix("\r\n")?;
    let open = text.rfind('{')?;
    text.get(open + 1..)?.strip_suffix('}')?.parse().ok()
}

/// Whether the message has header `name` containing `value`, both compared
/// ignoring case.
fn header_contains(raw: &[u8], name: &str, value: &str) -> bool {
    let text = String::from_utf8_lossy(raw);
    let value = value.to_ascii_lowercase();
    text.split("\r\n")
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .any(|(n, v)| {
            n.trim().eq_ignore_ascii_case(name) && v.to_ascii_lowercase().contains(&value)
        })
}

fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The header lines the client asked for with `HEADER.FIELDS`.
fn header_fields(raw: &[u8], wanted: &[&str]) -> Vec<u8> {
    let text = String::from_utf8_lossy(raw);
    let mut out = Vec::new();
    for line in text.split("\r\n") {
        if line.is_empty() {
            break;
        }
        let name = line.split(':').next().unwrap_or("").to_ascii_uppercase();
        if wanted.contains(&name.as_str()) {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// The first `name` header of `raw`, unfolded (RFC 5322 §2.2.3) and
/// trimmed, with CR, LF and NUL removed; `None` when absent or empty. Lines
/// end in CRLF, as `header_fields` and `body_structure` expect too.
fn header_value(raw: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let mut found: Option<String> = None;
    for line in text.split("\r\n") {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some(value) = found.as_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if found.is_some() {
            break;
        }
        if let Some((n, v)) = line.split_once(':') {
            if n.trim().eq_ignore_ascii_case(name) {
                found = Some(v.trim().to_string());
            }
        }
    }
    found
        .map(|v| v.replace(['\r', '\n', '\0'], "").trim().to_string())
        .filter(|v| !v.is_empty())
}

/// RFC 9051 §4.5 nstring: `NIL`, a quoted string, or for 8-bit text a
/// literal (`{n}` then the bytes), since a quoted string may only hold
/// 7-bit text without UTF8=ACCEPT (§4.3), as a real server sends it.
fn nstring(value: Option<&str>) -> String {
    match value {
        None => "NIL".to_string(),
        Some(v) if !v.is_ascii() => format!("{{{}}}\r\n{v}", v.len()),
        Some(v) => format!("\"{}\"", quoted(v)),
    }
}

/// Splits an address list at commas outside quotes, angle brackets and
/// comments, and drops RFC 5322 §3.4 group syntax (`name: a, b;`).
fn split_addresses(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let (mut in_quote, mut escaped, mut angle, mut comment) = (false, false, 0u32, 0u32);
    for c in value.chars() {
        if escaped {
            escaped = false;
            current.push(c);
            continue;
        }
        match c {
            '\\' if in_quote || comment > 0 => escaped = true,
            '"' if comment == 0 => in_quote = !in_quote,
            '<' if !in_quote && comment == 0 => angle = angle.saturating_add(1),
            '>' if !in_quote && comment == 0 => angle = angle.saturating_sub(1),
            '(' if !in_quote => comment = comment.saturating_add(1),
            ')' if !in_quote => comment = comment.saturating_sub(1),
            // A group's display name ends at ':'; its members follow.
            ':' if !in_quote && angle == 0 && comment == 0 => {
                current.clear();
                continue;
            }
            ',' | ';' if !in_quote && angle == 0 && comment == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// `text` without its `(comments)`, trimmed. Inside a quoted string,
/// parentheses are plain text (RFC 5322 §3.2.4), so they're kept there.
fn without_comments(text: &str) -> String {
    let (mut depth, mut in_quote, mut escaped) = (0u32, false, false);
    let mut out = String::new();
    for c in text.chars() {
        if escaped {
            escaped = false;
            if depth == 0 {
                out.push(c);
            }
            continue;
        }
        match c {
            '\\' if in_quote || depth > 0 => {
                escaped = true;
                if depth == 0 {
                    out.push(c);
                }
                continue;
            }
            '"' if depth == 0 => in_quote = !in_quote,
            '(' if !in_quote => {
                depth = depth.saturating_add(1);
                continue;
            }
            ')' if !in_quote && depth > 0 => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if depth == 0 {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// The byte offsets of the angle-addr `<...>` in a name-addr: the last `<`
/// outside quotes and comments, and the last `>` after it.
fn angle_addr(entry: &str) -> Option<(usize, usize)> {
    let (mut in_quote, mut escaped, mut comment) = (false, false, 0u32);
    let mut open = None;
    for (i, c) in entry.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quote || comment > 0 => escaped = true,
            '"' if comment == 0 => in_quote = !in_quote,
            '(' if !in_quote => comment = comment.saturating_add(1),
            ')' if !in_quote => comment = comment.saturating_sub(1),
            '<' if !in_quote && comment == 0 => open = Some(i),
            _ => {}
        }
    }
    let open = open?;
    let close = entry.rfind('>').filter(|close| *close > open)?;
    Some((open, close))
}

/// A display name with its surrounding quotes removed and every
/// quoted-pair (`\x`, RFC 5322 §3.2.1) unescaped.
fn unquoted(name: &str) -> String {
    let Some(inner) = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) else {
        return name.to_string();
    };
    let mut out = String::new();
    let mut escaped = false;
    for c in inner.chars() {
        if escaped || c != '\\' {
            out.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    out.trim().to_string()
}

/// One envelope address, `(name NIL mailbox host)` (RFC 9051 §7.5.2), from
/// `Name <local@host>`, `"Name" <local@host>` or a bare `local@host`.
/// `None` for an empty `<>`. A missing mailbox or host gets a placeholder,
/// as Dovecot does, since a NIL host would mark a group (§7.5.2).
fn envelope_address(entry: &str) -> Option<String> {
    let (name, address) = match angle_addr(entry) {
        Some((open, close)) => (
            entry.get(..open).map(without_comments).unwrap_or_default(),
            entry
                .get(open + 1..close)
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        None => (String::new(), without_comments(entry)),
    };
    let name = unquoted(&name);
    if address.is_empty() && name.is_empty() {
        return None;
    }
    let (mailbox, host) = match address.rsplit_once('@') {
        Some((local, host)) => (local.to_string(), host.to_string()),
        None => (address, String::new()),
    };
    let or = |value: String, placeholder: &str| {
        if value.is_empty() {
            placeholder.to_string()
        } else {
            value
        }
    };
    Some(format!(
        "({} NIL {} {})",
        nstring(Some(name.as_str()).filter(|n| !n.is_empty())),
        nstring(Some(or(mailbox, "MISSING_MAILBOX").as_str())),
        nstring(Some(or(host, "MISSING_DOMAIN").as_str()))
    ))
}

/// An envelope address list, `((...)(...))`, or `NIL` when the header is
/// absent or names nobody.
fn envelope_addresses(value: Option<&str>) -> String {
    let list: Vec<String> = value
        .map(split_addresses)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| envelope_address(entry))
        .collect();
    if list.is_empty() {
        "NIL".to_string()
    } else {
        format!("({})", list.concat())
    }
}

/// The message's `ENVELOPE` (RFC 9051 §7.5.2), built from its own headers
/// as a real server does: date, subject, from, sender, reply-to, to, cc,
/// bcc, in-reply-to, message-id. Values are sent as written (encoded words
/// stay encoded). Sender and Reply-To default to From when absent. A
/// message without a Message-ID gets `<folder.uid@example.org>`, stable and
/// different in every folder (#416).
fn envelope(raw: &[u8], folder: &str, uid: u32) -> String {
    let field = |name: &str| header_value(raw, name);
    let from = field("From");
    let sender = field("Sender").or_else(|| from.clone());
    let reply_to = field("Reply-To").or_else(|| from.clone());
    let message_id = field("Message-ID").unwrap_or_else(|| {
        let folder: String = folder
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        format!("<{folder}.{uid}@example.org>")
    });
    format!(
        "({} {} {} {} {} {} {} {} {} {})",
        nstring(field("Date").as_deref()),
        nstring(field("Subject").as_deref()),
        envelope_addresses(from.as_deref()),
        envelope_addresses(sender.as_deref()),
        envelope_addresses(reply_to.as_deref()),
        envelope_addresses(field("To").as_deref()),
        envelope_addresses(field("Cc").as_deref()),
        envelope_addresses(field("Bcc").as_deref()),
        nstring(field("In-Reply-To").as_deref()),
        nstring(Some(message_id.as_str())),
    )
}

fn body_structure(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    if text.contains("Content-Disposition: attachment") {
        "BODYSTRUCTURE ((\"TEXT\" \"PLAIN\" (\"CHARSET\" \"UTF-8\") NIL NIL \"7BIT\" 4 1)(\"APPLICATION\" \"PDF\" (\"NAME\" \"a.pdf\") NIL NIL \"BASE64\" 8 NIL (\"ATTACHMENT\" (\"FILENAME\" \"a.pdf\")) NIL) \"MIXED\")".to_string()
    } else {
        format!(
            "BODYSTRUCTURE (\"TEXT\" \"PLAIN\" (\"CHARSET\" \"UTF-8\") NIL NIL \"7BIT\" {} 1)",
            raw.len()
        )
    }
}

async fn max_uid(mailbox: &Mailbox, folder: &str) -> u32 {
    mailbox
        .messages(folder)
        .await
        .iter()
        .map(|m| m.uid)
        .max()
        .unwrap_or(0)
}

async fn handle_uid_command(
    parts: &[&str],
    mailbox: &Mailbox,
    quirks: &[Quirk],
    folder: &str,
) -> String {
    let tag = parts.first().copied().unwrap_or("*");
    let subcommand = parts.get(2).copied().unwrap_or("").to_ascii_uppercase();
    match subcommand.as_str() {
        "FETCH" => {
            let set = parts.get(3).copied().unwrap_or("");
            let items = parts
                .get(4..)
                .unwrap_or_default()
                .join(" ")
                .to_ascii_uppercase();
            let messages = mailbox.messages(folder).await;
            let max = messages.iter().map(|m| m.uid).max().unwrap_or(0);
            let condstore = !quirks.contains(&Quirk::NoCondstore);
            // RFC 7162 §3.1.4: `(CHANGEDSINCE n)` limits the answer to
            // messages with a higher mod-sequence and implies MODSEQ.
            let changed_since: Option<u64> = items
                .split_once("CHANGEDSINCE ")
                .and_then(|(_, rest)| rest.trim_end_matches(')').trim().parse().ok());
            if changed_since.is_some() && !condstore {
                return format!("{tag} BAD CHANGEDSINCE unsupported\r\n");
            }
            let with_modseq = condstore && (changed_since.is_some() || items.contains("MODSEQ"));
            let mut out: Vec<u8> = Vec::new();
            for (index, message) in messages.iter().enumerate() {
                if !in_uid_set(set, message.uid, max) {
                    continue;
                }
                if changed_since.is_some_and(|m| message.modseq <= m) {
                    continue;
                }
                let mut line = format!("* {} FETCH (UID {}", index + 1, message.uid);
                let marks_seen = items.contains("BODY[]") || items.contains("RFC822 ");
                if marks_seen {
                    mailbox.add_flag(folder, message.uid, "\\Seen").await;
                }
                if items.contains("FLAGS") {
                    let mut flags = message.flags.clone();
                    if marks_seen && !flags.iter().any(|f| f == "\\Seen") {
                        flags.push("\\Seen".to_string());
                    }
                    line.push_str(&format!(" FLAGS ({})", flags.join(" ")));
                }
                if with_modseq {
                    line.push_str(&format!(" MODSEQ ({})", message.modseq));
                }
                if items.contains("INTERNALDATE") {
                    // Now, like a message that just arrived, so code that
                    // cares how recent mail is (notifications) can be tested.
                    line.push_str(&format!(" INTERNALDATE \"{}\"", internal_date_now()));
                }
                if items.contains("RFC822.SIZE") {
                    line.push_str(&format!(" RFC822.SIZE {}", message.raw.len()));
                }
                if items.contains("ENVELOPE") {
                    if message.corrupt_envelope {
                        line.push_str(" ENVELOPE (\"only\" \"two\")");
                    } else {
                        line.push_str(" ENVELOPE ");
                        line.push_str(&envelope(&message.raw, folder, message.uid));
                    }
                }
                if items.contains("BODYSTRUCTURE") {
                    line.push(' ');
                    line.push_str(&body_structure(&message.raw));
                }
                out.extend_from_slice(line.as_bytes());
                if items.contains("HEADER.FIELDS") {
                    let fields = header_fields(
                        &message.raw,
                        &[
                            "REFERENCES",
                            "IN-REPLY-TO",
                            "LIST-UNSUBSCRIBE",
                            "LIST-UNSUBSCRIBE-POST",
                            "LIST-ID",
                        ],
                    );
                    out.extend_from_slice(
                        format!(" BODY[HEADER.FIELDS (REFERENCES IN-REPLY-TO LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST LIST-ID)] {{{}}}\r\n", fields.len()).as_bytes(),
                    );
                    out.extend_from_slice(&fields);
                }
                if items.contains("BODY[]") || items.contains("BODY.PEEK[]") {
                    // A partial fetch, `BODY.PEEK[]<start.length>` (RFC 9051
                    // §6.4.5), answers with that range as `BODY[]<start>`.
                    match partial_range(&items) {
                        Some((start, length)) => {
                            let from = start.min(message.raw.len());
                            let to = from.saturating_add(length).min(message.raw.len());
                            let part = message.raw.get(from..to).unwrap_or_default();
                            out.extend_from_slice(
                                format!(" BODY[]<{start}> {{{}}}\r\n", part.len()).as_bytes(),
                            );
                            out.extend_from_slice(part);
                        }
                        None => {
                            out.extend_from_slice(
                                format!(" BODY[] {{{}}}\r\n", message.raw.len()).as_bytes(),
                            );
                            out.extend_from_slice(&message.raw);
                        }
                    }
                }
                out.extend_from_slice(b")\r\n");
            }
            out.extend_from_slice(format!("{tag} OK UID FETCH completed\r\n").as_bytes());
            // The fake's messages are ASCII, so nothing is lost here.
            String::from_utf8_lossy(&out).into_owned()
        }
        "SEARCH" => {
            let mut messages = mailbox.messages(folder).await;
            // Message sequence numbers follow UID order (RFC 9051 §2.3.1.2).
            messages.sort_by_key(|m| m.uid);
            let max = messages.iter().map(|m| m.uid).max().unwrap_or(0);
            let count = u32::try_from(messages.len()).unwrap_or(u32::MAX);
            // `[CHARSET UTF-8] [<sequence set>] [keys]`.
            let mut args = parts.get(3..).unwrap_or_default();
            let charset = args
                .first()
                .is_some_and(|a| a.eq_ignore_ascii_case("CHARSET"));
            if charset {
                args = args.get(2..).unwrap_or_default();
            }
            let seqs = args.first().copied().filter(|a| is_sequence_set(a));
            if seqs.is_some() {
                args = args.get(1..).unwrap_or_default();
            }
            let uids: Vec<String> = messages
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    let seq = u32::try_from(index + 1).unwrap_or(u32::MAX);
                    seqs.is_none_or(|set| in_uid_set(set, seq, count))
                })
                .map(|(_, m)| m)
                .filter(
                    |m| match args.first().map(|p| p.to_ascii_uppercase()).as_deref() {
                        Some("UID") => in_uid_set(args.get(1).copied().unwrap_or(""), m.uid, max),
                        // `HEADER <name> <value>`: a substring match on that
                        // header, case-insensitive (RFC 9051 §6.4.4).
                        Some("HEADER") => header_contains(
                            &m.raw,
                            args.get(1).copied().unwrap_or(""),
                            args.get(2).copied().unwrap_or("").trim_matches('"'),
                        ),
                        // `CHARSET UTF-8 <keys>`: every key must match.
                        Some(_) if charset => search_keys_match(m, args),
                        _ => true,
                    },
                )
                .map(|m| m.uid.to_string())
                .collect();
            let list = if uids.is_empty() {
                String::new()
            } else {
                format!(" {}", uids.join(" "))
            };
            format!("* SEARCH{list}\r\n{tag} OK UID SEARCH completed\r\n")
        }
        "STORE" => {
            // `UID STORE <set> (+|-)FLAGS[.SILENT] (<flags>)` (RFC 9051 §6.4.6).
            let set = parts.get(3).copied().unwrap_or("");
            let op = parts.get(4).copied().unwrap_or("").to_ascii_uppercase();
            let flags = parts.get(5..).unwrap_or_default().join(" ");
            let flags: Vec<&str> = flags
                .trim_matches(|c| c == '(' || c == ')')
                .split_whitespace()
                .collect();
            let add = op.starts_with('+');
            if !(add || op.starts_with('-')) || !op.contains("FLAGS") {
                return format!("{tag} BAD unsupported STORE\r\n");
            }
            let max = max_uid(mailbox, folder).await;
            for flag in flags {
                mailbox
                    .store_flag(folder, |uid| in_uid_set(set, uid, max), flag, add)
                    .await;
            }
            format!("{tag} OK UID STORE completed\r\n")
        }
        "COPY" | "MOVE" => {
            let set = parts.get(3).copied().unwrap_or("");
            let to = parts.get(4).copied().unwrap_or("").trim_matches('"');
            if subcommand == "MOVE" && quirks.contains(&Quirk::NoMove) {
                return format!("{tag} BAD UID MOVE unsupported\r\n");
            }
            if !mailbox.folder_names().await.iter().any(|n| n == to) {
                // RFC 9051 §6.4.7: the target doesn't exist.
                return format!("{tag} NO [TRYCREATE] no such folder\r\n");
            }
            let max = max_uid(mailbox, folder).await;
            let pick = |uid| in_uid_set(set, uid, max);
            // COPYUID (RFC 4315 §3), with UIDPLUS: the copies get the
            // target's next UIDs, in order. For MOVE it comes in an untagged
            // OK before the tagged one (RFC 6851 §4.3).
            let picked: Vec<u32> = mailbox
                .messages(folder)
                .await
                .iter()
                .map(|m| m.uid)
                .filter(|uid| pick(*uid))
                .collect();
            let first = mailbox.uidnext(to).await;
            let copyuid = (!quirks.contains(&Quirk::NoUidplus) && !picked.is_empty()).then(|| {
                let last = first
                    .saturating_add(u32::try_from(picked.len()).unwrap_or(1))
                    .saturating_sub(1);
                let targets = if first == last {
                    first.to_string()
                } else {
                    format!("{first}:{last}")
                };
                let sources = picked
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[COPYUID {} {sources} {targets}] ", mailbox.uidvalidity())
            });
            if subcommand == "MOVE" {
                mailbox.move_messages(folder, pick, to).await;
                match copyuid {
                    Some(code) => format!("* OK {code}moved\r\n{tag} OK UID MOVE completed\r\n"),
                    None => format!("{tag} OK UID MOVE completed\r\n"),
                }
            } else {
                mailbox.copy(folder, pick, to).await;
                format!(
                    "{tag} OK {}UID COPY completed\r\n",
                    copyuid.unwrap_or_default()
                )
            }
        }
        "EXPUNGE" => {
            if quirks.contains(&Quirk::NoUidplus) {
                return format!("{tag} BAD UID EXPUNGE unsupported\r\n");
            }
            let set = parts.get(3).copied().unwrap_or("");
            let max = max_uid(mailbox, folder).await;
            mailbox
                .expunge_deleted(folder, |uid| in_uid_set(set, uid, max))
                .await;
            format!("{tag} OK UID EXPUNGE completed\r\n")
        }
        _ => format!("{tag} BAD unknown UID command\r\n"),
    }
}

/// The current time as an IMAP `date-time` (RFC 9051 §9), always in UTC:
/// `25-Nov-2026 10:46:51 +0000`.
/// The `<start.length>` of a `BODY[]` or `BODY.PEEK[]` partial fetch in
/// the (upper-cased) items, if there is one.
fn partial_range(items: &str) -> Option<(usize, usize)> {
    let (_, rest) = items.split_once("[]<")?;
    let (range, _) = rest.split_once('>')?;
    let (start, length) = range.split_once('.')?;
    Some((start.parse().ok()?, length.parse().ok()?))
}

fn internal_date_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let name = usize::try_from(month - 1)
        .ok()
        .and_then(|i| MONTHS.get(i))
        .copied()
        .unwrap_or("Jan");
    format!(
        "{day:02}-{name}-{year} {:02}:{:02}:{:02} +0000",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod envelope_tests {
    use super::envelope;

    /// #416: the envelope describes the message itself, field by field.
    #[test]
    fn the_envelope_comes_from_the_message_headers() {
        let raw = b"Date: Tue, 03 Mar 2026 09:15:00 +0100\r\n\
            From: \"Ana B.\" <ana@example.org>\r\n\
            To: Bo <bo@example.net>, carl@example.com\r\n\
            Cc: =?UTF-8?Q?D=C3=A9?= <de@example.org>\r\n\
            Subject: Lunch\r\n\
            \tplans\r\n\
            In-Reply-To: <p1@example.org>\r\n\
            Message-ID: <m1@example.org>\r\n\
            \r\nBody\r\n";
        assert_eq!(
            envelope(raw, "INBOX", 7),
            "(\"Tue, 03 Mar 2026 09:15:00 +0100\" \"Lunch plans\" \
             ((\"Ana B.\" NIL \"ana\" \"example.org\")) \
             ((\"Ana B.\" NIL \"ana\" \"example.org\")) \
             ((\"Ana B.\" NIL \"ana\" \"example.org\")) \
             ((\"Bo\" NIL \"bo\" \"example.net\")(NIL NIL \"carl\" \"example.com\")) \
             ((\"=?UTF-8?Q?D=C3=A9?=\" NIL \"de\" \"example.org\")) \
             NIL \"<p1@example.org>\" \"<m1@example.org>\")"
        );
    }

    /// Sender and Reply-To default to From (RFC 9051 §7.5.2); absent fields
    /// are NIL; a fixture without a Message-ID keeps its stable fallback.
    #[test]
    fn missing_fields_are_nil_and_sender_and_reply_to_default_to_from() {
        assert_eq!(
            envelope(b"From: ana@example.org\r\n\r\nx", "INBOX", 3),
            "(NIL NIL ((NIL NIL \"ana\" \"example.org\")) ((NIL NIL \"ana\" \"example.org\")) \
             ((NIL NIL \"ana\" \"example.org\")) NIL NIL NIL NIL \"<INBOX.3@example.org>\")"
        );
        let raw = b"From: ana@example.org\r\nSender: list@example.net\r\n\
            Reply-To: Team <team@example.com>\r\nBcc: undisclosed-recipients:;\r\n\r\n";
        let env = envelope(raw, "INBOX", 1);
        assert!(env.contains(" ((NIL NIL \"list\" \"example.net\")) ((\"Team\" NIL \"team\" \"example.com\")) NIL NIL NIL NIL "), "{env}");
    }

    /// Quotes and backslashes in names are escaped; groups and comments
    /// work; odd input never panics.
    #[test]
    fn odd_headers_are_escaped_and_never_panic() {
        let raw = b"From: \"A \\\"Q\\\" B\" <a@example.org>\r\nTo: Team: x@example.org, (c) y@example.net;\r\n\r\nz";
        let env = envelope(raw, "INBOX", 1);
        assert!(
            env.contains("((\"A \\\"Q\\\" B\" NIL \"a\" \"example.org\"))"),
            "{env}"
        );
        assert!(
            env.contains("((NIL NIL \"x\" \"example.org\")(NIL NIL \"y\" \"example.net\"))"),
            "{env}"
        );
        for raw in [
            &b""[..],
            b"\r\n\r\n",
            b"From:\r\n\r\n",
            b"From: <>\r\n",
            b"From: >a@b<\r\n",
            b"From: \"unterminated <a@b>\r\n",
            b"From: a@b@c, ,,, ;;; ::: ((((\r\n",
            b"\tcontinuation without a header\r\nFrom: x@y\r\n",
            b"From: \"\\\r\n",
            b"From: ((\"<\r\n",
        ] {
            let env = envelope(raw, "INBOX", 9);
            assert!(env.starts_with('(') && env.ends_with(')'), "{env}");
            assert!(!env.contains('\r') && !env.contains('\n'), "{env}");
        }
        // 8-bit text (here, invalid UTF-8 made lossy) goes out as a literal.
        let env = envelope(b"To: \xff\xfe@\xff\r\n", "INBOX", 9);
        assert!(env.contains("{3}\r\n\u{fffd}"), "{env}");
    }

    /// #424 review: a `<` or parentheses inside a quoted display name are
    /// part of the name, and every quoted-pair is unescaped.
    #[test]
    fn quoted_names_keep_angle_brackets_and_parentheses() {
        let env = envelope(b"From: \"Sales <EU>\" <s@example.org>\r\n", "INBOX", 1);
        assert!(
            env.contains("((\"Sales <EU>\" NIL \"s\" \"example.org\"))"),
            "{env}"
        );
        let env = envelope(b"From: (a <comment>) Ana <ana@example.org>\r\n", "INBOX", 1);
        assert!(
            env.contains("((\"Ana\" NIL \"ana\" \"example.org\"))"),
            "{env}"
        );
        let env = envelope(
            b"From: \"Jane Doe (Acme)\" <jane@example.org>\r\n",
            "INBOX",
            1,
        );
        assert!(
            env.contains("((\"Jane Doe (Acme)\" NIL \"jane\" \"example.org\"))"),
            "{env}"
        );
        let env = envelope(b"From: \"A \\(B\\) \\a\" <a@example.org>\r\n", "INBOX", 1);
        assert!(
            env.contains("((\"A (B) a\" NIL \"a\" \"example.org\"))"),
            "{env}"
        );
    }

    /// #424 review: a missing mailbox or host gets a placeholder, never NIL,
    /// which would read as an RFC 9051 §7.5.2 group marker; `<>` is left out.
    #[test]
    fn missing_parts_get_placeholders_not_group_markers() {
        let env = envelope(b"From: root\r\nTo: <>, b@example.org\r\n", "INBOX", 1);
        assert!(
            env.contains("((NIL NIL \"root\" \"MISSING_DOMAIN\"))"),
            "{env}"
        );
        assert!(env.contains(" ((NIL NIL \"b\" \"example.org\")) "), "{env}");
        let env = envelope(b"From: Ana <@example.org>\r\n", "INBOX", 1);
        assert!(
            env.contains("((\"Ana\" NIL \"MISSING_MAILBOX\" \"example.org\"))"),
            "{env}"
        );
    }

    /// #424 review: folded values carry no stray spaces, and the fallback
    /// Message-ID differs between folders.
    #[test]
    fn folded_values_are_trimmed_and_fallback_ids_differ_by_folder() {
        let env = envelope(
            b"Subject:\r\n Lunch\r\n \r\nTo:\r\n  a@example.org,\r\n  b@example.org\r\n\r\n",
            "INBOX",
            1,
        );
        assert!(env.contains(" \"Lunch\" "), "{env}");
        assert!(
            env.contains("((NIL NIL \"a\" \"example.org\")(NIL NIL \"b\" \"example.org\"))"),
            "{env}"
        );
        assert!(envelope(b"From: a@example.org\r\n\r\n", "INBOX", 1)
            .contains("\"<INBOX.1@example.org>\""));
        assert!(envelope(b"From: a@example.org\r\n\r\n", "Sent Items", 1)
            .contains("\"<Sent-Items.1@example.org>\""));
    }

    /// #424 review: 8-bit header text is sent as a literal, as RFC 9051
    /// §4.3 requires without UTF8=ACCEPT.
    #[test]
    fn eight_bit_values_are_literals() {
        let raw = "Subject: Grüße\r\nFrom: Zoë <zoe@example.org>\r\n\r\n".as_bytes();
        let env = envelope(raw, "INBOX", 1);
        assert!(env.contains(" {7}\r\nGrüße "), "{env}");
        assert!(
            env.contains("(({4}\r\nZoë NIL \"zoe\" \"example.org\"))"),
            "{env}"
        );
    }
}
