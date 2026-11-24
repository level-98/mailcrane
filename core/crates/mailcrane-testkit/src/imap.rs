use crate::fault::{Fault, Quirk};
use crate::mailbox::Mailbox;
use crate::tls;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct FakeImap {
    addr: SocketAddr,
    root_cert_der: Vec<u8>,
    mailbox: Mailbox,
    shutdown: Arc<RwLock<Option<JoinHandle<()>>>>,
}

pub struct FakeImapBuilder {
    user: Option<(String, String)>,
    folders: Vec<(String, Vec<Vec<u8>>)>,
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
}

impl FakeImapBuilder {
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
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let addr = listener.local_addr().map_err(|error| error.to_string())?;
        let mailbox = Mailbox::new(
            &self
                .folders
                .iter()
                .map(|(name, messages)| (name.as_str(), messages.clone()))
                .collect::<Vec<_>>(),
        );
        let mailbox_for_server = mailbox.clone();
        let user = self
            .user
            .unwrap_or_else(|| ("alice@example.org".into(), "password".into()));
        let faults = Arc::new(self.faults);
        let quirks = Arc::new(self.quirks);
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let tls_config = tls.config.clone();
                let mailbox = mailbox_for_server.clone();
                let faults = faults.clone();
                let quirks = quirks.clone();
                let user = user.clone();
                tokio::spawn(async move {
                    let _ =
                        handle_connection(stream, tls_config, mailbox, faults, quirks, user).await;
                });
            }
        });
        if started.elapsed() >= Duration::from_millis(100) {
            return Err("IMAP server start exceeded 100 ms".into());
        }
        Ok(FakeImap {
            addr,
            root_cert_der: tls.cert_der,
            mailbox,
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
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    pub fn root_cert_der(&self) -> &[u8] {
        &self.root_cert_der
    }
    pub fn mailbox(&self) -> Mailbox {
        self.mailbox.clone()
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

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    tls_config: Arc<rustls::ServerConfig>,
    mailbox: Mailbox,
    faults: Arc<Vec<Fault>>,
    quirks: Arc<Vec<Quirk>>,
    user: (String, String),
) -> Result<(), String> {
    let command_count = Arc::new(AtomicUsize::new(0));
    let mut first = [0u8; 1];
    let implicit_tls =
        match tokio::time::timeout(Duration::from_millis(10), stream.peek(&mut first)).await {
            Ok(result) => result.unwrap_or(0) > 0 && first[0] == 0x16,
            Err(_) => false,
        };
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
                    &mailbox,
                    &faults,
                    &quirks,
                    &user,
                    &command_count,
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

    loop {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while tls_stream.read_exact(&mut byte).await.is_ok() {
            line.push(byte[0]);
            if line.ends_with(b"\r\n") {
                break;
            }
        }
        if line.is_empty() {
            break;
        }
        let text = String::from_utf8_lossy(&line);
        let parts: Vec<&str> = text.split_whitespace().collect();
        if handle_imap_command(
            &mut tls_stream,
            parts.as_slice(),
            &mailbox,
            &faults,
            &quirks,
            &user,
            &command_count,
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
    mailbox: &Mailbox,
    faults: &[Fault],
    quirks: &[Quirk],
    user: &(String, String),
    command_count: &AtomicUsize,
) -> Result<(), String> {
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
            let mut caps = "IMAP4rev2 IMAP4rev1 AUTH=PLAIN".to_string();
            if !quirks.contains(&Quirk::NoCondstore) {
                caps.push_str(" CONDSTORE");
            }
            if !quirks.contains(&Quirk::NoQresync) {
                caps.push_str(" QRESYNC");
            }
            if !quirks.contains(&Quirk::NoSpecialUse) {
                caps.push_str(" SPECIAL-USE");
            }
            if !quirks.contains(&Quirk::NoUidplus) {
                caps.push_str(" UIDPLUS");
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
                format!("{tag} OK AUTHENTICATE completed\r\n")
            }
        }
        "LIST" => {
            let mut output = "* LIST (\\HasNoChildren) \"/\" INBOX\r\n".to_string();
            if quirks.contains(&Quirk::GmailLabels) {
                output.push_str("* LIST (\\All) \"/\" \"[Gmail]/All Mail\"\r\n");
                output.push_str("* XLIST (\\HasNoChildren) \"/\" \"[Gmail]/All Mail\"\r\n");
            }
            if !quirks.contains(&Quirk::NoSpecialUse) {
                output.push_str("* LIST (\\HasNoChildren \\Sent) \"/\" Sent\r\n");
            }
            output.push_str(&format!("{tag} OK LIST completed\r\n"));
            output
        }
        "SELECT" | "EXAMINE" => {
            let folder = parts.get(2).copied().unwrap_or("INBOX").trim_matches('"');
            let count = mailbox.exists(folder).await;
            format!("* {count} EXISTS\r\n* OK [UIDVALIDITY 1]\r\n* OK [UIDNEXT {}]\r\n* OK [HIGHESTMODSEQ 1]\r\n{tag} OK {command} completed\r\n", count + 1)
        }
        "UID" => handle_uid_command(parts, mailbox, quirks).await,
        "ENABLE" => format!("{tag} ENABLED\r\n"),
        "NOOP" => "* 0 RECENT\r\n".to_string() + &format!("{tag} OK NOOP completed\r\n"),
        "LOGOUT" => format!("{tag} OK LOGOUT completed\r\n"),
        "APPEND" => format!("{tag} OK APPEND completed\r\n"),
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
                .write_all(&response.as_bytes()[..split])
                .await
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
    }
    for fault in faults {
        if let Fault::DropAfterCommands(limit) = fault {
            if count >= *limit {
                return Ok(());
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

async fn handle_uid_command(parts: &[&str], mailbox: &Mailbox, quirks: &[Quirk]) -> String {
    let tag = parts.first().copied().unwrap_or("*");
    let subcommand = parts.get(2).copied().unwrap_or("").to_ascii_uppercase();
    match subcommand.as_str() {
        "FETCH" => {
            let messages = mailbox.messages("INBOX").await;
            let mut out = String::new();
            for message in messages {
                out.push_str(&format!("* {} FETCH (UID {} FLAGS (\\Seen) RFC822.SIZE {} ENVELOPE (\"Wed, 24 Nov 2026 10:46:51 +0000\" \"Test\" ((\"Alice\" NIL \"alice\" \"example.org\")) ((\"Alice\" NIL \"alice\" \"example.org\")) ((\"Alice\" NIL \"alice\" \"example.org\")) ((\"Recipient\" NIL \"recipient\" \"example.net\")) NIL NIL NIL NIL) BODYSTRUCTURE (\"TEXT\" \"PLAIN\" (\"CHARSET\" \"UTF-8\") NIL NIL \"7BIT\" {} 0))\r\n", message.uid, message.uid, message.raw.len(), message.raw.len()));
            }
            out.push_str(&format!("{tag} OK UID FETCH completed\r\n"));
            out
        }
        "SEARCH" => format!("* SEARCH 1\r\n{tag} OK UID SEARCH completed\r\n"),
        "STORE" => format!("{tag} OK UID STORE completed\r\n"),
        "COPY" => format!("{tag} OK UID COPY completed\r\n"),
        "MOVE" => {
            if quirks.contains(&Quirk::NoMove) {
                format!("{tag} BAD UID MOVE unsupported\r\n")
            } else {
                format!("{tag} OK UID MOVE completed\r\n")
            }
        }
        "EXPUNGE" => format!("{tag} OK UID EXPUNGE completed\r\n"),
        _ => format!("{tag} BAD unknown UID command\r\n"),
    }
}
