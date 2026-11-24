//! The SMTP conversation (RFC 5321).

use crate::data;
use crate::envelope::{check_address, Envelope};
use crate::error::{classify, SmtpError};
use crate::Secret;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::ClientConfig;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_LINE: usize = 4096;
const MAX_REPLY_LINES: usize = 100;

/// How the connection is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpSecurity {
    /// TLS from the first byte (RFC 8314), usually port 465.
    ImplicitTls,
    /// Start in clear text, then upgrade with STARTTLS (RFC 3207), usually
    /// port 587. Fails if the server does not offer it.
    StartTls,
}

/// Where to connect.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    /// Name given to `EHLO`.
    pub client_name: String,
    /// Extra trusted root certificates (DER), added to the platform's. For
    /// tests; this adds trust, it cannot turn checking off.
    pub extra_root_certs: Vec<Vec<u8>>,
    pub connect_timeout: Duration,
    pub command_timeout: Duration,
    pub data_timeout: Duration,
}

impl SmtpConfig {
    /// Default time limits: connect 30 s, command 60 s, DATA 10 minutes.
    pub fn new(host: impl Into<String>, port: u16, security: SmtpSecurity) -> Self {
        Self {
            host: host.into(),
            port,
            security,
            client_name: "localhost".into(),
            extra_root_certs: Vec::new(),
            connect_timeout: Duration::from_secs(30),
            command_timeout: Duration::from_secs(60),
            data_timeout: Duration::from_secs(600),
        }
    }
}

/// How to sign in.
#[derive(Debug, Clone)]
pub enum SmtpAuth {
    Password {
        username: String,
        password: Secret,
    },
    /// Declared for M3-01; not implemented yet, so it fails with `AuthFailed`.
    XOAuth2 {
        username: String,
        token: Secret,
    },
}

/// The result of a send that reached the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendReport {
    /// Recipients the server accepted, and the message went to.
    pub accepted: Vec<String>,
    /// Recipients the server refused.
    pub rejected: Vec<String>,
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

struct Reply {
    code: u16,
    lines: Vec<String>,
}

impl Reply {
    fn text(&self) -> String {
        self.lines.join(" ")
    }
    fn has_capability(&self, name: &str) -> bool {
        self.lines.iter().skip(1).any(|l| {
            l.split_ascii_whitespace()
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case(name))
        })
    }
    fn auth_mechanisms(&self) -> Vec<String> {
        self.lines
            .iter()
            .skip(1)
            .filter_map(|l| {
                let mut words = l.split_ascii_whitespace();
                words
                    .next()
                    .filter(|w| w.eq_ignore_ascii_case("AUTH"))
                    .map(|_| words.map(|w| w.to_ascii_uppercase()).collect::<Vec<_>>())
            })
            .flatten()
            .collect()
    }
}

struct Conn {
    io: Box<dyn Io>,
    buf: Vec<u8>,
    command_timeout: Duration,
}

async fn within<T>(limit: Duration, fut: impl Future<Output = T>) -> Result<T, SmtpError> {
    tokio::time::timeout(limit, fut)
        .await
        .map_err(|_| SmtpError::Timeout)
}

impl Conn {
    async fn read_line(&mut self) -> Result<String, SmtpError> {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=pos).collect();
                let line = line.strip_suffix(b"\n").unwrap_or(&line);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                return String::from_utf8(line.to_vec()).map_err(|_| SmtpError::CannotConnect);
            }
            if self.buf.len() > MAX_LINE {
                return Err(SmtpError::CannotConnect);
            }
            let mut chunk = [0u8; 1024];
            let n = self
                .io
                .read(&mut chunk)
                .await
                .map_err(|_| SmtpError::CannotConnect)?;
            if n == 0 {
                return Err(SmtpError::CannotConnect);
            }
            self.buf
                .extend_from_slice(chunk.get(..n).unwrap_or_default());
        }
    }

    /// Reads one complete, possibly multi-line, reply (RFC 5321 §4.2.1).
    async fn read_reply(&mut self) -> Result<Reply, SmtpError> {
        let mut lines = Vec::new();
        loop {
            let line = self.read_line().await?;
            let code: u16 = line
                .get(..3)
                .and_then(|c| c.parse().ok())
                .ok_or(SmtpError::CannotConnect)?;
            let sep = line.as_bytes().get(3).copied();
            let text = line.get(4..).unwrap_or_default().to_string();
            lines.push(text);
            match sep {
                Some(b'-') if lines.len() < MAX_REPLY_LINES => continue,
                Some(b' ') | None => return Ok(Reply { code, lines }),
                _ => return Err(SmtpError::CannotConnect),
            }
        }
    }

    async fn reply(&mut self) -> Result<Reply, SmtpError> {
        within(self.command_timeout, self.read_reply()).await?
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), SmtpError> {
        self.io
            .write_all(bytes)
            .await
            .map_err(|_| SmtpError::CannotConnect)?;
        self.io.flush().await.map_err(|_| SmtpError::CannotConnect)
    }

    async fn command(&mut self, line: &str) -> Result<Reply, SmtpError> {
        let limit = self.command_timeout;
        within(limit, self.write(format!("{line}\r\n").as_bytes())).await??;
        self.reply().await
    }

    /// Sends a command and requires a reply starting with `ok` (2 or 3).
    async fn expect(&mut self, line: &str, ok: u16) -> Result<Reply, SmtpError> {
        let reply = self.command(line).await?;
        if reply.code / 100 == ok {
            Ok(reply)
        } else {
            Err(classify(reply.code, &reply.text()))
        }
    }
}

fn tls_config(extra_roots: &[Vec<u8>]) -> Result<Arc<ClientConfig>, SmtpError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = extra_roots
        .iter()
        .map(|der| CertificateDer::from(der.clone()));
    // The platform verifier checks the chain, the host name and validity.
    // `dangerous()` is only how rustls spells "bring your own verifier".
    #[cfg(not(target_os = "android"))]
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(roots, provider.clone())
            .map_err(|_| SmtpError::TlsFailure)?;
    #[cfg(target_os = "android")]
    let verifier = {
        let _ = roots;
        rustls_platform_verifier::Verifier::new(provider.clone())
            .map_err(|_| SmtpError::TlsFailure)?
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| SmtpError::TlsFailure)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Connects, secures the connection, and returns it with the server's
/// capabilities from the (post-TLS) `EHLO`.
async fn open(config: &SmtpConfig) -> Result<(Conn, Reply), SmtpError> {
    if config.host.is_empty() || config.host.chars().any(|c| c.is_control() || c == ' ') {
        return Err(SmtpError::InvalidInput);
    }
    let client_name = if config.client_name.is_empty()
        || config
            .client_name
            .chars()
            .any(|c| c.is_control() || c == ' ')
    {
        return Err(SmtpError::InvalidInput);
    } else {
        &config.client_name
    };
    let tcp = within(
        config.connect_timeout,
        TcpStream::connect((config.host.as_str(), config.port)),
    )
    .await?
    .map_err(|_| SmtpError::CannotConnect)?;

    let mut conn = match config.security {
        SmtpSecurity::ImplicitTls => Conn {
            io: Box::new(upgrade(config, Box::new(tcp)).await?),
            buf: Vec::new(),
            command_timeout: config.command_timeout,
        },
        SmtpSecurity::StartTls => Conn {
            io: Box::new(tcp),
            buf: Vec::new(),
            command_timeout: config.command_timeout,
        },
    };

    let greeting = conn.reply().await?;
    if greeting.code != 220 {
        return Err(classify(greeting.code, &greeting.text()));
    }
    let mut caps = conn.expect(&format!("EHLO {client_name}"), 2).await?;

    if config.security == SmtpSecurity::StartTls {
        // Without STARTTLS we stop here: no AUTH, no MAIL FROM.
        if !caps.has_capability("STARTTLS") {
            return Err(SmtpError::StartTlsNotOffered);
        }
        let reply = conn.command("STARTTLS").await?;
        if reply.code != 220 {
            return Err(SmtpError::StartTlsNotOffered);
        }
        // Anything buffered now came from before TLS and could be injected
        // by someone in the middle; refuse it (CVE-2011-0411 class).
        if !conn.buf.is_empty() {
            return Err(SmtpError::TlsFailure);
        }
        let tls = upgrade(config, conn.io).await?;
        conn = Conn {
            io: Box::new(tls),
            buf: Vec::new(),
            command_timeout: config.command_timeout,
        };
        // RFC 3207 §4.2: forget what we learned and say EHLO again.
        caps = conn.expect(&format!("EHLO {client_name}"), 2).await?;
    }
    Ok((conn, caps))
}

async fn upgrade(
    config: &SmtpConfig,
    io: Box<dyn Io>,
) -> Result<tokio_rustls::client::TlsStream<Box<dyn Io>>, SmtpError> {
    let name = ServerName::try_from(config.host.clone()).map_err(|_| SmtpError::TlsFailure)?;
    let connector = tokio_rustls::TlsConnector::from(tls_config(&config.extra_root_certs)?);
    within(config.command_timeout, connector.connect(name, io))
        .await?
        .map_err(|_| SmtpError::TlsFailure)
}

fn b64(text: &str) -> String {
    STANDARD.encode(text)
}

/// RFC 4616 PLAIN, falling back to the de-facto LOGIN mechanism.
async fn authenticate(conn: &mut Conn, caps: &Reply, auth: &SmtpAuth) -> Result<(), SmtpError> {
    let SmtpAuth::Password { username, password } = auth else {
        return Err(SmtpError::AuthFailed);
    };
    let password = password.expose();
    let bad = |s: &str| s.is_empty() || s.contains(['\0', '\r', '\n']);
    if bad(username) || bad(password) {
        return Err(SmtpError::InvalidInput);
    }
    let mechanisms = caps.auth_mechanisms();
    let plain = mechanisms.iter().any(|m| m == "PLAIN");
    let login = mechanisms.iter().any(|m| m == "LOGIN");
    if !plain && !login {
        return Err(SmtpError::AuthFailed);
    }
    if plain {
        let reply = conn
            .command(&format!(
                "AUTH PLAIN {}",
                b64(&format!("\0{username}\0{password}"))
            ))
            .await?;
        match reply.code {
            235 => return Ok(()),
            400..=499 => return Err(classify(reply.code, &reply.text())),
            // Wrong credentials: do not retry with another mechanism.
            _ if !login => return Err(SmtpError::AuthFailed),
            535 => return Err(SmtpError::AuthFailed),
            _ => {}
        }
    }
    let reply = conn.command("AUTH LOGIN").await?;
    if reply.code != 334 {
        return Err(SmtpError::AuthFailed);
    }
    let reply = conn.command(&b64(username)).await?;
    if reply.code != 334 {
        return Err(SmtpError::AuthFailed);
    }
    let reply = conn.command(&b64(password)).await?;
    match reply.code {
        235 => Ok(()),
        400..=499 => Err(classify(reply.code, &reply.text())),
        _ => Err(SmtpError::AuthFailed),
    }
}

async fn quit(conn: &mut Conn) {
    let _ = within(Duration::from_secs(5), async {
        let _ = conn.write(b"QUIT\r\n").await;
        let _ = conn.io.shutdown().await;
    })
    .await;
}

/// Connects and signs in, then disconnects. Used when adding an account.
pub async fn check_login(config: &SmtpConfig, auth: &SmtpAuth) -> Result<(), SmtpError> {
    let (mut conn, caps) = open(config).await?;
    let result = authenticate(&mut conn, &caps, auth).await;
    quit(&mut conn).await;
    result
}

/// Sends `message` to the envelope's recipients.
///
/// `message` is the RFC 5322 message. Any `Bcc:` header is removed before it
/// is transmitted; Bcc recipients only appear in the envelope. Recipients the
/// server refuses are reported in [`SendReport`]; the message still goes to
/// the others. If every recipient is refused, nothing is sent.
pub async fn send(
    config: &SmtpConfig,
    auth: &SmtpAuth,
    envelope: &Envelope,
    message: &[u8],
) -> Result<SendReport, SmtpError> {
    envelope.validate()?;
    let (mut conn, caps) = open(config).await?;
    let result = transaction(&mut conn, &caps, config, auth, envelope, message).await;
    quit(&mut conn).await;
    result
}

async fn transaction(
    conn: &mut Conn,
    caps: &Reply,
    config: &SmtpConfig,
    auth: &SmtpAuth,
    envelope: &Envelope,
    message: &[u8],
) -> Result<SendReport, SmtpError> {
    authenticate(conn, caps, auth).await?;
    conn.expect(&format!("MAIL FROM:<{}>", envelope.from), 2)
        .await?;

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for to in &envelope.to {
        check_address(to)?;
        let reply = conn.command(&format!("RCPT TO:<{to}>")).await?;
        if reply.code / 100 == 2 {
            accepted.push(to.clone());
        } else {
            rejected.push(to.clone());
        }
    }
    if accepted.is_empty() {
        let _ = conn.command("RSET").await;
        return Err(SmtpError::RecipientsRejected { rejected });
    }

    conn.expect("DATA", 3).await?;
    let body = data::prepare(message);
    let finish = async {
        conn.write(&body).await?;
        conn.read_reply().await
    };
    let reply = within(config.data_timeout, finish).await??;
    if reply.code / 100 != 2 {
        return Err(classify(reply.code, &reply.text()));
    }
    Ok(SendReport { accepted, rejected })
}
