use crate::fault::{Fault, Quirk};
use crate::loopback::Loopback;
use crate::tls;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMessage {
    pub envelope_from: Option<String>,
    pub envelope_to: Vec<String>,
    pub data: Vec<u8>,
    /// Parameters after the address in `MAIL FROM`, e.g. `HOLDUNTIL=...`.
    pub mail_params: Vec<String>,
}

pub struct FakeSmtpBuilder {
    faults: Vec<Fault>,
    quirks: Vec<Quirk>,
}

#[derive(Debug, Clone)]
pub struct FakeSmtp {
    addr: SocketAddr,
    starttls_addr: SocketAddr,
    root_cert_der: Vec<u8>,
    sent: Arc<RwLock<Vec<SentMessage>>>,
    shutdown: Arc<RwLock<Option<JoinHandle<()>>>>,
    /// While set, connections are closed as soon as they arrive.
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl FakeSmtpBuilder {
    pub fn fault(mut self, fault: Fault) -> Self {
        self.faults.push(fault);
        self
    }
    pub fn quirk(mut self, quirk: Quirk) -> Self {
        self.quirks.push(quirk);
        self
    }
    pub async fn start(self) -> Result<FakeSmtp, String> {
        let started = std::time::Instant::now();
        let tls = tls::generate()?;
        // Like a real server (465 and 587), one port is implicit TLS and the
        // other starts in plain text for STARTTLS. Guessing the mode from
        // whether a TLS hello arrives in time fails when the client is slow
        // to build its TLS config (Windows' verifier often is).
        let tls_listener = Loopback::bind().await?;
        let plain_listener = Loopback::bind().await?;
        let addr = tls_listener.addr();
        let starttls_addr = plain_listener.addr();
        let sent = Arc::new(RwLock::new(Vec::new()));
        let sent_for_server = sent.clone();
        let faults = Arc::new(self.faults);
        let quirks = Arc::new(self.quirks);
        let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let down_for_server = down.clone();
        let handle = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    accepted = tls_listener.accept() => accepted.map(|(stream, _)| (stream, true)),
                    accepted = plain_listener.accept() => accepted.map(|(stream, _)| (stream, false)),
                };
                let Ok((stream, implicit_tls)) = accepted else {
                    break;
                };
                if down_for_server.load(std::sync::atomic::Ordering::SeqCst) {
                    drop(stream);
                    continue;
                }
                let sent = sent_for_server.clone();
                let faults = faults.clone();
                let quirks = quirks.clone();
                let tls_config = tls.config.clone();
                tokio::spawn(async move {
                    let _ = smtp_connection(stream, implicit_tls, tls_config, sent, faults, quirks)
                        .await;
                });
            }
        });
        if started.elapsed() >= std::time::Duration::from_millis(100) {
            handle.abort();
            return Err("SMTP server start exceeded 100 ms".into());
        }
        Ok(FakeSmtp {
            addr,
            starttls_addr,
            root_cert_der: tls.cert_der,
            sent,
            shutdown: Arc::new(RwLock::new(Some(handle))),
            down,
        })
    }
}

impl FakeSmtp {
    pub fn builder() -> FakeSmtpBuilder {
        FakeSmtpBuilder {
            faults: Vec::new(),
            quirks: Vec::new(),
        }
    }
    /// The implicit TLS port (like 465).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    /// The port that starts in plain text and offers STARTTLS (like 587).
    pub fn starttls_addr(&self) -> SocketAddr {
        self.starttls_addr
    }
    pub fn root_cert_der(&self) -> &[u8] {
        &self.root_cert_der
    }
    /// Takes the server down (connections close at once) or brings it
    /// back, on the same addresses.
    pub fn set_down(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    pub async fn sent(&self) -> Vec<SentMessage> {
        self.sent.read().await.clone()
    }
    pub async fn shutdown(&self) {
        if let Some(handle) = self.shutdown.write().await.take() {
            handle.abort();
        }
    }
}

impl Drop for FakeSmtp {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shutdown.try_write() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

async fn smtp_connection(
    mut stream: tokio::net::TcpStream,
    implicit_tls: bool,
    tls_config: Arc<rustls::ServerConfig>,
    sent: Arc<RwLock<Vec<SentMessage>>>,
    faults: Arc<Vec<Fault>>,
    quirks: Arc<Vec<Quirk>>,
) -> Result<(), String> {
    let command_count = AtomicUsize::new(0);
    for fault in faults.iter() {
        if let Fault::Delay(duration) = fault {
            tokio::time::sleep(*duration).await;
        }
    }
    if implicit_tls {
        let mut tls_stream = tokio_rustls::TlsAcceptor::from(tls_config)
            .accept(stream)
            .await
            .map_err(|error| error.to_string())?;
        tls_stream
            .write_all(b"220 mailcrane.test ESMTP fake\r\n")
            .await
            .map_err(|error| error.to_string())?;
        return smtp_plain_stream(
            &mut tls_stream,
            sent,
            faults,
            quirks,
            &mut None,
            &mut Vec::new(),
            &command_count,
        )
        .await;
    }
    stream
        .write_all(b"220 mailcrane.test ESMTP fake\r\n")
        .await
        .map_err(|error| error.to_string())?;
    let mut from = None;
    let mut recipients = Vec::new();
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
            break;
        }
        let text = String::from_utf8_lossy(&line);
        let command = text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let count = command_count.fetch_add(1, Ordering::SeqCst) + 1;
        for fault in faults.iter() {
            if let Fault::DropAfterCommands(limit) = fault {
                if count >= *limit {
                    return Ok(());
                }
            }
            if let Fault::MalformedResponse = fault {
                stream
                    .write_all(b"\xff\xfeGARBLE\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                continue;
            }
            if let Fault::DropMidResponse = fault {
                stream
                    .write_all(b"250 ")
                    .await
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }
        }
        if faults.contains(&Fault::DropAfterCommands(1)) {
            break;
        }
        match command.as_str() {
            "EHLO" => stream
                .write_all(if faults.contains(&Fault::NoStartTls) {
                    b"250-mailcrane.test\r\n250 AUTH PLAIN LOGIN\r\n"
                } else {
                    b"250-mailcrane.test\r\n250-STARTTLS\r\n250 AUTH PLAIN LOGIN\r\n"
                })
                .await
                .map_err(|error| error.to_string())?,
            "STARTTLS" => {
                if faults.contains(&Fault::NoStartTls) {
                    stream
                        .write_all(b"454 TLS not available\r\n")
                        .await
                        .map_err(|error| error.to_string())?;
                } else {
                    stream
                        .write_all(b"220 Ready to start TLS\r\n")
                        .await
                        .map_err(|error| error.to_string())?;
                    let mut tls_stream = tokio_rustls::TlsAcceptor::from(tls_config.clone())
                        .accept(stream)
                        .await
                        .map_err(|error| error.to_string())?;
                    return smtp_plain_stream(
                        &mut tls_stream,
                        sent,
                        faults,
                        quirks,
                        &mut from,
                        &mut recipients,
                        &command_count,
                    )
                    .await;
                }
            }
            "AUTH" => stream
                .write_all(if faults.contains(&Fault::RejectLogin) {
                    b"535 Authentication failed\r\n"
                } else {
                    b"235 Authentication successful\r\n"
                })
                .await
                .map_err(|error| error.to_string())?,
            "MAIL" => {
                let start = text.find('<').unwrap_or(0);
                let address = text
                    .get(start..)
                    .and_then(|value| value.find('>').map(|end| value.get(..end)))
                    .flatten()
                    .map(|value| value.trim_start_matches('<').to_string());
                from = address;
                stream
                    .write_all(smtp_reply(
                        &faults,
                        b"250 sender accepted\r\n",
                        b"550 sender rejected\r\n",
                    ))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "RCPT" => {
                let address = text
                    .find('<')
                    .and_then(|start| text.get(start..))
                    .and_then(|value| value.find('>').map(|end| value.get(..end)))
                    .flatten()
                    .map(|value| value.trim_start_matches('<').to_string())
                    .unwrap_or_else(|| text.trim().to_string());
                if faults.contains(&Fault::RejectRecipient(address.clone())) {
                    stream
                        .write_all(b"550 recipient rejected\r\n")
                        .await
                        .map_err(|error| error.to_string())?;
                    continue;
                }
                recipients.push(address);
                stream
                    .write_all(smtp_reply(
                        &faults,
                        b"250 recipient accepted\r\n",
                        b"550 recipient rejected\r\n",
                    ))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "DATA" => {
                stream
                    .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                if faults.contains(&Fault::DropMidData) {
                    return Ok(());
                }
                let mut data = Vec::new();
                loop {
                    let mut line = Vec::new();
                    let mut byte = [0u8; 1];
                    while stream.read_exact(&mut byte).await.is_ok() {
                        line.push(byte[0]);
                        if line.ends_with(b"\r\n") {
                            break;
                        }
                    }
                    if line == b".\r\n" {
                        break;
                    }
                    if line.starts_with(b"..") {
                        line.remove(1);
                    }
                    data.extend_from_slice(&line);
                }
                sent.write().await.push(SentMessage {
                    envelope_from: from.take(),
                    envelope_to: recipients.clone(),
                    data,
                    mail_params: Vec::new(),
                });
                recipients.clear();
                stream
                    .write_all(smtp_reply(
                        &faults,
                        b"250 message accepted\r\n",
                        b"550 message rejected\r\n",
                    ))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "RSET" => {
                from = None;
                recipients.clear();
                stream
                    .write_all(b"250 reset\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "QUIT" => {
                stream
                    .write_all(b"221 goodbye\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                break;
            }
            _ => stream
                .write_all(b"500 unknown command\r\n")
                .await
                .map_err(|error| error.to_string())?,
        }
    }
    Ok(())
}

async fn smtp_plain_stream(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    sent: Arc<RwLock<Vec<SentMessage>>>,
    faults: Arc<Vec<Fault>>,
    quirks: Arc<Vec<Quirk>>,
    from: &mut Option<String>,
    recipients: &mut Vec<String>,
    command_count: &AtomicUsize,
) -> Result<(), String> {
    let mut params_store = Vec::new();
    let mail_params = &mut params_store;
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
            break;
        }
        let text = String::from_utf8_lossy(&line);
        let command = text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let count = command_count.fetch_add(1, Ordering::SeqCst) + 1;
        if faults
            .iter()
            .any(|fault| matches!(fault, Fault::DropAfterCommands(limit) if count >= *limit))
        {
            return Ok(());
        }
        match command.as_str() {
            "EHLO" => stream
                .write_all(if quirks.contains(&Quirk::FutureRelease) {
                    &b"250-mailcrane.test\r\n250-FUTURERELEASE 604800 2099-01-01T00:00:00Z\r\n250 AUTH PLAIN LOGIN\r\n"[..]
                } else {
                    &b"250-mailcrane.test\r\n250 AUTH PLAIN LOGIN\r\n"[..]
                })
                .await
                .map_err(|error| error.to_string())?,
            "AUTH" => stream
                .write_all(if faults.contains(&Fault::RejectLogin) {
                    b"535 Authentication failed\r\n"
                } else {
                    b"235 Authentication successful\r\n"
                })
                .await
                .map_err(|error| error.to_string())?,
            "MAIL" => {
                let address = text
                    .find('<')
                    .and_then(|start| text.get(start..))
                    .and_then(|value| value.find('>').map(|end| value.get(..end)))
                    .flatten()
                    .map(|value| value.trim_start_matches('<').to_string());
                *from = address;
                *mail_params = text
                    .find('>')
                    .and_then(|end| text.get(end + 1..))
                    .map(|rest| rest.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default();
                stream
                    .write_all(smtp_reply(
                        &faults,
                        b"250 sender accepted\r\n",
                        b"550 sender rejected\r\n",
                    ))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "RCPT" => {
                let address = text
                    .find('<')
                    .and_then(|start| text.get(start..))
                    .and_then(|value| value.find('>').map(|end| value.get(..end)))
                    .flatten()
                    .map(|value| value.trim_start_matches('<').to_string())
                    .unwrap_or_else(|| text.trim().to_string());
                if faults.contains(&Fault::RejectRecipient(address.clone())) {
                    stream
                        .write_all(b"550 recipient rejected\r\n")
                        .await
                        .map_err(|error| error.to_string())?;
                    continue;
                }
                recipients.push(address);
                stream
                    .write_all(smtp_reply(
                        &faults,
                        b"250 recipient accepted\r\n",
                        b"550 recipient rejected\r\n",
                    ))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "DATA" => {
                stream
                    .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                if faults.contains(&Fault::DropMidData) {
                    return Ok(());
                }
                let mut data = Vec::new();
                loop {
                    let mut line = Vec::new();
                    let mut byte = [0u8; 1];
                    while stream.read_exact(&mut byte).await.is_ok() {
                        line.push(byte[0]);
                        if line.ends_with(b"\r\n") {
                            break;
                        }
                    }
                    if line == b".\r\n" {
                        break;
                    }
                    if line.starts_with(b"..") {
                        line.remove(1);
                    }
                    data.extend_from_slice(&line);
                }
                sent.write().await.push(SentMessage {
                    envelope_from: from.take(),
                    envelope_to: recipients.clone(),
                    data,
                    mail_params: std::mem::take(mail_params),
                });
                recipients.clear();
                stream
                    .write_all(smtp_reply(
                        &faults,
                        b"250 message accepted\r\n",
                        b"550 message rejected\r\n",
                    ))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "RSET" => {
                *from = None;
                recipients.clear();
                stream
                    .write_all(b"250 reset\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
            }
            "QUIT" => {
                stream
                    .write_all(b"221 goodbye\r\n")
                    .await
                    .map_err(|error| error.to_string())?;
                break;
            }
            _ => stream
                .write_all(b"500 unknown command\r\n")
                .await
                .map_err(|error| error.to_string())?,
        }
    }
    Ok(())
}

fn smtp_reply(faults: &[Fault], success: &'static [u8], failure: &'static [u8]) -> &'static [u8] {
    if faults.contains(&Fault::TempFailure) {
        b"451 temporary failure\r\n"
    } else if faults.contains(&Fault::PermFailure) {
        failure
    } else {
        success
    }
}
