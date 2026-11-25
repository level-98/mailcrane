//! A ManageSieve session (RFC 5804).

use crate::error::SieveError;
use crate::parser::{parse_response_with, Limits, ParseError, Response, Status, Token};
use crate::tls;
use crate::Secret;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::future::Future;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// The ManageSieve port (RFC 5804 §1.8).
pub const DEFAULT_PORT: u16 = 4190;

/// Longest script name we send (RFC 5804 §1.6 sets no limit; servers do).
const MAX_NAME: usize = 512;

/// Where to connect.
#[derive(Debug, Clone)]
pub struct SieveConfig {
    pub host: String,
    pub port: u16,
    /// Extra trusted root certificates (DER), added to the platform's. For
    /// tests; this adds trust, it cannot turn checking off.
    pub extra_root_certs: Vec<Vec<u8>>,
    pub connect_timeout: Duration,
    pub command_timeout: Duration,
    pub limits: Limits,
}

impl SieveConfig {
    /// Default time limits: connect 20 s, each command 30 s.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            extra_root_certs: Vec::new(),
            connect_timeout: Duration::from_secs(20),
            command_timeout: Duration::from_secs(30),
            limits: Limits::default(),
        }
    }

    /// Adds a trusted root certificate (tests only add the testkit's).
    pub fn with_extra_root_cert(mut self, der: Vec<u8>) -> Self {
        self.extra_root_certs.push(der);
        self
    }
}

/// What the server says it can do (RFC 5804 §1.7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub implementation: Option<String>,
    /// SASL mechanisms, upper case.
    pub sasl: Vec<String>,
    /// Sieve extensions, lower case (`fileinto`, `imap4flags`, …).
    pub sieve: Vec<String>,
    pub starttls: bool,
    pub version: Option<String>,
}

impl Capabilities {
    /// Whether the server supports this Sieve extension.
    pub fn has_extension(&self, name: &str) -> bool {
        self.sieve.iter().any(|e| e.eq_ignore_ascii_case(name))
    }

    fn from_response(r: &Response) -> Result<Self, SieveError> {
        let mut caps = Capabilities::default();
        for line in &r.data {
            let (Some(Token::Str(name)), value) = (line.first(), line.get(1)) else {
                return Err(SieveError::Protocol);
            };
            let value = match value {
                Some(Token::Str(v)) => Some(v.as_str()),
                None => None,
                Some(_) => return Err(SieveError::Protocol),
            };
            let words = |v: Option<&str>| -> Vec<String> {
                v.unwrap_or_default()
                    .split_ascii_whitespace()
                    // A server can't need hundreds of these; a longer list is
                    // just cut off.
                    .take(256)
                    .map(str::to_owned)
                    .collect()
            };
            match name.to_ascii_uppercase().as_str() {
                "IMPLEMENTATION" => caps.implementation = value.map(str::to_owned),
                "SASL" => {
                    caps.sasl = words(value)
                        .into_iter()
                        .map(|w| w.to_ascii_uppercase())
                        .collect();
                }
                "SIEVE" => {
                    caps.sieve = words(value)
                        .into_iter()
                        .map(|w| w.to_ascii_lowercase())
                        .collect();
                }
                "STARTTLS" => caps.starttls = true,
                "VERSION" => caps.version = value.map(str::to_owned),
                // NOTIFY, MAXREDIRECTS, OWNER, LANGUAGE and unknown ones.
                _ => {}
            }
        }
        Ok(caps)
    }
}

/// A script on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInfo {
    pub name: String,
    pub active: bool,
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn within<T>(limit: Duration, fut: impl Future<Output = T>) -> Result<T, SieveError> {
    tokio::time::timeout(limit, fut)
        .await
        .map_err(|_| SieveError::Timeout)
}

struct Conn {
    io: Box<dyn Io>,
    buf: Vec<u8>,
    limits: Limits,
    timeout: Duration,
}

impl Conn {
    async fn read_response(&mut self) -> Result<Response, SieveError> {
        let timeout = self.timeout;
        within(timeout, self.read_inner()).await?
    }

    async fn read_inner(&mut self) -> Result<Response, SieveError> {
        loop {
            match parse_response_with(&self.buf, &self.limits) {
                Ok((response, used)) => {
                    self.buf.drain(..used.min(self.buf.len()));
                    if response.status == Status::Bye {
                        return Err(SieveError::ServerBye);
                    }
                    return Ok(response);
                }
                Err(ParseError::Incomplete) => {}
                Err(ParseError::LineTooLong) => return Err(SieveError::LineTooLong),
                Err(ParseError::TooLarge) => return Err(SieveError::ResponseTooLarge),
                Err(ParseError::Invalid) => return Err(SieveError::Protocol),
            }
            let mut chunk = [0u8; 8192];
            let n = self
                .io
                .read(&mut chunk)
                .await
                .map_err(|_| SieveError::ConnectionClosed)?;
            if n == 0 {
                return Err(SieveError::ConnectionClosed);
            }
            self.buf
                .extend_from_slice(chunk.get(..n).unwrap_or_default());
        }
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), SieveError> {
        let timeout = self.timeout;
        within(timeout, async {
            self.io
                .write_all(bytes)
                .await
                .map_err(|_| SieveError::ConnectionClosed)?;
            self.io
                .flush()
                .await
                .map_err(|_| SieveError::ConnectionClosed)
        })
        .await?
    }

    /// Sends a command and reads its response.
    async fn command(&mut self, bytes: &[u8]) -> Result<Response, SieveError> {
        self.write(bytes).await?;
        self.read_response().await
    }
}

/// A string argument (RFC 5804 §4): quoted when short and simple, otherwise
/// a non-synchronising literal `{n+}`. CR, LF and NUL are only ever sent
/// inside a literal.
fn string_arg(out: &mut Vec<u8>, s: &str) {
    let simple = s.len() <= 1024 && !s.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0));
    if simple {
        out.push(b'"');
        for b in s.bytes() {
            if b == b'"' || b == b'\\' {
                out.push(b'\\');
            }
            out.push(b);
        }
        out.push(b'"');
    } else {
        out.extend_from_slice(format!("{{{}+}}\r\n", s.len()).as_bytes());
        out.extend_from_slice(s.as_bytes());
    }
}

/// A script name: UTF-8 without control characters (RFC 5804 §1.6).
fn check_name(name: &str) -> Result<(), SieveError> {
    if name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(SieveError::InvalidInput);
    }
    Ok(())
}

/// A script: no NUL (RFC 5228 §2.2 allows any UTF-8 text; NUL never has a
/// reason to be in one) and within the literal limit.
fn check_script(script: &str, limits: &Limits) -> Result<(), SieveError> {
    if script.len() > limits.max_literal || script.contains('\0') {
        return Err(SieveError::InvalidInput);
    }
    Ok(())
}

/// Turns a `NO` into the matching error (RFC 5804 §1.3 response codes).
fn refused(r: &Response, default: SieveError) -> SieveError {
    match r.code_name().as_deref() {
        Some(code) if code.starts_with("QUOTA") => SieveError::QuotaExceeded,
        _ => default,
    }
}

fn expect_ok(r: Response, default: SieveError) -> Result<Response, SieveError> {
    match r.status {
        Status::Ok => Ok(r),
        _ => Err(refused(&r, default)),
    }
}

/// A logged-in ManageSieve session over TLS.
pub struct Session {
    conn: Conn,
    caps: Capabilities,
}

impl Session {
    /// Connects, upgrades to TLS with STARTTLS (required), and logs in with
    /// SASL PLAIN.
    pub async fn connect(
        config: &SieveConfig,
        username: &str,
        password: &Secret,
    ) -> Result<Self, SieveError> {
        if username.is_empty()
            || username.contains('\0')
            || password.expose().contains('\0')
            || config.host.is_empty()
        {
            return Err(SieveError::InvalidInput);
        }
        let tcp = within(
            config.connect_timeout,
            TcpStream::connect((config.host.as_str(), config.port)),
        )
        .await?
        .map_err(|_| SieveError::CannotConnect)?;
        let mut conn = Conn {
            io: Box::new(tcp),
            buf: Vec::new(),
            limits: config.limits,
            timeout: config.command_timeout,
        };
        // RFC 5804 §1.7: the server greets with its capabilities.
        let greeting = conn.read_response().await?;
        if greeting.status != Status::Ok {
            return Err(SieveError::CannotConnect);
        }
        let caps = Capabilities::from_response(&greeting)?;
        // AGENTS.md §4.5: no STARTTLS means no session, never clear text.
        if !caps.starttls {
            return Err(SieveError::StartTlsNotOffered);
        }
        let r = conn.command(b"STARTTLS\r\n").await?;
        if r.status != Status::Ok {
            return Err(SieveError::StartTlsNotOffered);
        }
        // Anything the server sent after its OK, before the handshake, could
        // have been injected in clear text (CVE-2011-0411 style): refuse.
        if !conn.buf.is_empty() {
            return Err(SieveError::Protocol);
        }
        let tls = tls::handshake(
            &config.host,
            &config.extra_root_certs,
            config.connect_timeout,
            conn.io,
        )
        .await?;
        let mut conn = Conn {
            io: Box::new(tls),
            buf: Vec::new(),
            limits: config.limits,
            timeout: config.command_timeout,
        };
        // RFC 5804 §2.2: after TLS the server sends its capabilities again,
        // and the ones from before TLS are forgotten.
        let r = conn.read_response().await?;
        if r.status != Status::Ok {
            return Err(SieveError::Protocol);
        }
        let caps = Capabilities::from_response(&r)?;
        if !caps.sasl.iter().any(|m| m == "PLAIN") {
            return Err(SieveError::NoSupportedMechanism);
        }
        // RFC 4616: authzid NUL authcid NUL passwd, base64 (RFC 5804 §2.1).
        let mut plain = Vec::with_capacity(username.len() + password.expose().len() + 2);
        plain.push(0);
        plain.extend_from_slice(username.as_bytes());
        plain.push(0);
        plain.extend_from_slice(password.expose().as_bytes());
        let mut cmd = b"AUTHENTICATE \"PLAIN\" ".to_vec();
        string_arg(&mut cmd, &STANDARD.encode(&plain));
        cmd.extend_from_slice(b"\r\n");
        let r = conn.command(&cmd).await?;
        if r.status != Status::Ok {
            return Err(SieveError::AuthFailed);
        }
        let mut session = Session { conn, caps };
        // Capabilities can change after login (RFC 5804 §2.4).
        session.caps = session.capability().await?;
        Ok(session)
    }

    /// The capabilities from the last `CAPABILITY`.
    pub fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    /// `CAPABILITY` (RFC 5804 §2.4).
    pub async fn capability(&mut self) -> Result<Capabilities, SieveError> {
        let r = self.conn.command(b"CAPABILITY\r\n").await?;
        let r = expect_ok(r, SieveError::CommandFailed)?;
        let caps = Capabilities::from_response(&r)?;
        self.caps = caps.clone();
        Ok(caps)
    }

    /// `LISTSCRIPTS` (RFC 5804 §2.7).
    pub async fn list_scripts(&mut self) -> Result<Vec<ScriptInfo>, SieveError> {
        let r = self.conn.command(b"LISTSCRIPTS\r\n").await?;
        let r = expect_ok(r, SieveError::CommandFailed)?;
        r.data
            .iter()
            .map(|line| match line.as_slice() {
                [Token::Str(name)] => Ok(ScriptInfo {
                    name: name.clone(),
                    active: false,
                }),
                [Token::Str(name), Token::Atom(a)] if a.eq_ignore_ascii_case("ACTIVE") => {
                    Ok(ScriptInfo {
                        name: name.clone(),
                        active: true,
                    })
                }
                _ => Err(SieveError::Protocol),
            })
            .collect()
    }

    /// `GETSCRIPT` (RFC 5804 §2.9).
    pub async fn get_script(&mut self, name: &str) -> Result<String, SieveError> {
        check_name(name)?;
        let mut cmd = b"GETSCRIPT ".to_vec();
        string_arg(&mut cmd, name);
        cmd.extend_from_slice(b"\r\n");
        let r = self.conn.command(&cmd).await?;
        let r = expect_ok(r, SieveError::CommandFailed)?;
        match r.data.as_slice() {
            [line] => match line.as_slice() {
                [Token::Str(s)] => Ok(s.clone()),
                _ => Err(SieveError::Protocol),
            },
            _ => Err(SieveError::Protocol),
        }
    }

    /// `HAVESPACE` (RFC 5804 §2.5): whether a script of `size` octets named
    /// `name` fits.
    pub async fn have_space(&mut self, name: &str, size: usize) -> Result<(), SieveError> {
        check_name(name)?;
        let mut cmd = b"HAVESPACE ".to_vec();
        string_arg(&mut cmd, name);
        cmd.extend_from_slice(format!(" {size}\r\n").as_bytes());
        let r = self.conn.command(&cmd).await?;
        expect_ok(r, SieveError::QuotaExceeded).map(|_| ())
    }

    /// `CHECKSCRIPT` (RFC 5804 §2.12): whether the server accepts the
    /// script, without storing it.
    pub async fn check_script(&mut self, script: &str) -> Result<(), SieveError> {
        check_script(script, &self.conn.limits)?;
        let mut cmd = b"CHECKSCRIPT ".to_vec();
        string_arg_literal(&mut cmd, script);
        cmd.extend_from_slice(b"\r\n");
        let r = self.conn.command(&cmd).await?;
        expect_ok(r, SieveError::ScriptRejected).map(|_| ())
    }

    /// `PUTSCRIPT` (RFC 5804 §2.6): stores (or replaces) a script. Replacing
    /// the active script keeps it active; the server checks it first and
    /// keeps the old one if it is refused.
    pub async fn put_script(&mut self, name: &str, script: &str) -> Result<(), SieveError> {
        check_name(name)?;
        check_script(script, &self.conn.limits)?;
        let mut cmd = b"PUTSCRIPT ".to_vec();
        string_arg(&mut cmd, name);
        cmd.push(b' ');
        string_arg_literal(&mut cmd, script);
        cmd.extend_from_slice(b"\r\n");
        let r = self.conn.command(&cmd).await?;
        expect_ok(r, SieveError::ScriptRejected).map(|_| ())
    }

    /// `SETACTIVE` (RFC 5804 §2.8): makes `name` the active script, which
    /// deactivates any other.
    pub async fn set_active(&mut self, name: &str) -> Result<(), SieveError> {
        check_name(name)?;
        let mut cmd = b"SETACTIVE ".to_vec();
        string_arg(&mut cmd, name);
        cmd.extend_from_slice(b"\r\n");
        let r = self.conn.command(&cmd).await?;
        expect_ok(r, SieveError::CommandFailed).map(|_| ())
    }

    /// `LOGOUT` (RFC 5804 §2.3). Errors are ignored: the session is over.
    pub async fn logout(mut self) {
        let _ = self.conn.write(b"LOGOUT\r\n").await;
        let _ = self.conn.read_response().await;
        let _ = within(self.conn.timeout, self.conn.io.shutdown()).await;
    }
}

/// Scripts always go as a literal, so line breaks survive.
fn string_arg_literal(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(format!("{{{}+}}\r\n", s.len()).as_bytes());
    out.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_response;

    #[test]
    fn strings_are_quoted_or_sent_as_literals() {
        let mut out = Vec::new();
        string_arg(&mut out, "a \"b\" \\c");
        assert_eq!(out, b"\"a \\\"b\\\" \\\\c\"");
        let mut out = Vec::new();
        string_arg(&mut out, "line\r\nbreak");
        assert_eq!(out, b"{11+}\r\nline\r\nbreak");
        let mut out = Vec::new();
        string_arg(&mut out, "Ünïcödé");
        assert_eq!(out, "\"Ünïcödé\"".as_bytes());
    }

    #[test]
    fn names_with_control_characters_are_refused() {
        assert_eq!(check_name("a\r\nPUTSCRIPT"), Err(SieveError::InvalidInput));
        assert_eq!(check_name(""), Err(SieveError::InvalidInput));
        assert_eq!(check_name(&"x".repeat(513)), Err(SieveError::InvalidInput));
        assert_eq!(check_name("mailcrane"), Ok(()));
    }

    #[test]
    fn capabilities_are_read_case_insensitively() {
        let (r, _) = parse_response(
            b"\"SASL\" \"plain login\"\r\n\"Sieve\" \"FileInto IMAP4FLAGS\"\r\n\"STARTTLS\"\r\nOK\r\n",
        )
        .expect("parses");
        let caps = Capabilities::from_response(&r).expect("caps");
        assert_eq!(caps.sasl, ["PLAIN", "LOGIN"]);
        assert!(caps.has_extension("fileinto") && caps.has_extension("imap4flags"));
        assert!(caps.starttls);
        let (r, _) = parse_response(b"SASL\r\nOK\r\n").expect("parses");
        assert_eq!(Capabilities::from_response(&r), Err(SieveError::Protocol));
    }

    #[test]
    fn quota_codes_map_to_quota_exceeded() {
        let (r, _) = parse_response(b"NO (QUOTA/MAXSCRIPTS) \"x\"\r\n").expect("parses");
        assert_eq!(
            refused(&r, SieveError::CommandFailed),
            SieveError::QuotaExceeded
        );
        let (r, _) = parse_response(b"NO \"x\"\r\n").expect("parses");
        assert_eq!(
            refused(&r, SieveError::CommandFailed),
            SieveError::CommandFailed
        );
    }
}
