use crate::fault::{Fault, Quirk};
use crate::loopback::Loopback;
use crate::mailbox::Mailbox;
use crate::tls;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, RwLock};

/// Per-server settings shared by every connection.
struct Shared {
    mailbox: Mailbox,
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
    user: (String, String),
    special_use: Vec<(String, String)>,
    received: Arc<Mutex<Vec<String>>>,
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
}

#[derive(Debug, Clone)]
pub struct FakeImap {
    addr: SocketAddr,
    starttls_addr: SocketAddr,
    root_cert_der: Vec<u8>,
    mailbox: Mailbox,
    received: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<RwLock<Option<JoinHandle<()>>>>,
}

pub struct FakeImapBuilder {
    user: Option<(String, String)>,
    folders: Vec<(String, Vec<Vec<u8>>)>,
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
    special_use: Vec<(String, String)>,
    mailbox: Option<Mailbox>,
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
    pub fn special_use(mut self, name: impl Into<String>, attribute: impl Into<String>) -> Self {
        self.special_use.push((name.into(), attribute.into()));
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

    pub async fn start(self) -> Result<FakeImap, String> {
        let started = Instant::now();
        let tls = tls::generate()?;
        // Like a real server (993 and 143), one port is implicit TLS and the
        // other starts in plain text for STARTTLS. Guessing the mode from
        // whether a TLS hello arrives in time fails when the client is slow
        // to build its TLS config (Windows' verifier often is).
        let tls_listener = Loopback::bind().await?;
        let plain_listener = Loopback::bind().await?;
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
        let shared = Arc::new(Shared {
            mailbox: mailbox_for_server,
            faults: self.faults,
            quirks: self.quirks,
            user,
            special_use: self.special_use,
            received: received.clone(),
        });
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
                tokio::spawn(async move {
                    let _ = handle_connection(stream, implicit_tls, tls_config, shared).await;
                });
            }
        });
        if started.elapsed() >= Duration::from_millis(100) {
            return Err("IMAP server start exceeded 100 ms".into());
        }
        Ok(FakeImap {
            addr,
            starttls_addr,
            root_cert_der: tls.cert_der,
            mailbox,
            received,
            shutdown: Arc::new(RwLock::new(Some(handle))),
        })
    }
}

impl FakeImap {
    pub fn builder() -> FakeImapBuilder {
        FakeImapBuilder {
            user: None,
            folders: Vec::new(),
            faults: Vec::new(),
            quirks: Vec::new(),
            special_use: Vec::new(),
            mailbox: None,
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
            let parts: Vec<&str> = text.split_whitespace().collect();
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
                    if tls_stream.write_all(b"+ go ahead\r\n").await.is_err() {
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
        let parts: Vec<&str> = text.split_whitespace().collect();
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
            if !quirks.contains(&Quirk::NoUidplus) {
                caps.push_str(" UIDPLUS");
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
        "AUTHENTICATE" => {
            if faults.contains(&Fault::RejectLogin) {
                format!("{tag} NO authentication failed\r\n")
            } else {
                stream
                    .write_all(b"+ \r\n")
                    .await
                    .map_err(|error| error.to_string())?;
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
            for name in &listed {
                let prefix = format!("{name}/");
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
                output.push_str(&format!("* LIST ({attributes}) \"/\" \"{quoted}\"\r\n"));
            }
            if quirks.contains(&Quirk::GmailLabels) {
                output.push_str("* LIST (\\All) \"/\" \"[Gmail]/All Mail\"\r\n");
                output.push_str("* XLIST (\\HasNoChildren) \"/\" \"[Gmail]/All Mail\"\r\n");
            }
            output.push_str(&format!("{tag} OK LIST completed\r\n"));
            output
        }
        "SELECT" | "EXAMINE" => {
            let folder = parts.get(2).copied().unwrap_or("INBOX").trim_matches('"');
            let params = parts.get(3..).unwrap_or_default().join(" ");
            let qresync = qresync_params(&params);
            if folder != "INBOX" && !mailbox.folder_names().await.iter().any(|n| n == folder) {
                format!("{tag} NO [NONEXISTENT] no such folder\r\n")
            } else if !params.is_empty() && (qresync.is_none() || !state.qresync) {
                // RFC 7162 §3.2.5: QRESYNC parameters need ENABLE first.
                format!("{tag} BAD unexpected SELECT parameters\r\n")
            } else {
                state.selected = Some(folder.to_string());
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
                stream
                    .write_all(b"+ idling\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                let notified = mailbox.notify.notified();
                tokio::pin!(notified);
                match tokio::time::timeout(Duration::from_millis(900), notified.as_mut()).await {
                    Ok(()) => {
                        let count = mailbox.exists("INBOX").await;
                        format!("* {count} EXISTS\r\n")
                    }
                    Err(_) => String::new(),
                }
            }
        }
        "DONE" => format!("{tag} OK IDLE completed\r\n"),
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
    }
    if faults.contains(&Fault::TempFailure) {
        stream
            .write_all(format!("{tag} NO temporary failure\r\n").as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())
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

fn subject_of(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .split("\r\n")
        .take_while(|line| !line.is_empty())
        .find_map(|line| line.strip_prefix("Subject: ").map(str::to_string))
        .unwrap_or_else(|| "Test".to_string())
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
                    line.push_str(" INTERNALDATE \"24-Nov-2026 10:46:51 +0000\"");
                }
                if items.contains("RFC822.SIZE") {
                    line.push_str(&format!(" RFC822.SIZE {}", message.raw.len()));
                }
                if items.contains("ENVELOPE") {
                    if message.corrupt_envelope {
                        line.push_str(" ENVELOPE (\"only\" \"two\")");
                    } else {
                        line.push_str(&format!(" ENVELOPE (\"Wed, 24 Nov 2026 10:46:51 +0000\" \"{}\" ((\"Alice\" NIL \"alice\" \"example.org\")) ((\"Alice\" NIL \"alice\" \"example.org\")) ((\"Alice\" NIL \"alice\" \"example.org\")) ((\"Recipient\" NIL \"recipient\" \"example.net\")) NIL NIL NIL \"<{}@example.org>\")", quoted(&subject_of(&message.raw)), message.uid));
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
                        ],
                    );
                    out.extend_from_slice(
                        format!(" BODY[HEADER.FIELDS (REFERENCES IN-REPLY-TO LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST)] {{{}}}\r\n", fields.len()).as_bytes(),
                    );
                    out.extend_from_slice(&fields);
                }
                if items.contains("BODY[]") || items.contains("BODY.PEEK[]") {
                    out.extend_from_slice(
                        format!(" BODY[] {{{}}}\r\n", message.raw.len()).as_bytes(),
                    );
                    out.extend_from_slice(&message.raw);
                }
                out.extend_from_slice(b")\r\n");
            }
            out.extend_from_slice(format!("{tag} OK UID FETCH completed\r\n").as_bytes());
            // The fake's messages are ASCII, so nothing is lost here.
            String::from_utf8_lossy(&out).into_owned()
        }
        "SEARCH" => {
            let messages = mailbox.messages(folder).await;
            let max = messages.iter().map(|m| m.uid).max().unwrap_or(0);
            let uids: Vec<String> = messages
                .iter()
                .filter(
                    |m| match parts.get(3).map(|p| p.to_ascii_uppercase()).as_deref() {
                        Some("UID") => in_uid_set(parts.get(4).copied().unwrap_or(""), m.uid, max),
                        // `HEADER <name> <value>`: a substring match on that
                        // header, case-insensitive (RFC 9051 §6.4.4).
                        Some("HEADER") => header_contains(
                            &m.raw,
                            parts.get(4).copied().unwrap_or(""),
                            parts.get(5).copied().unwrap_or("").trim_matches('"'),
                        ),
                        // `CHARSET UTF-8 <keys>`: every key must match.
                        Some("CHARSET") => search_keys_match(m, parts.get(5..).unwrap_or_default()),
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
            if subcommand == "MOVE" {
                mailbox.move_messages(folder, pick, to).await;
            } else {
                mailbox.copy(folder, pick, to).await;
            }
            format!("{tag} OK UID {subcommand} completed\r\n")
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
