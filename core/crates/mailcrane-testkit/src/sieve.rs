//! A fake ManageSieve server (RFC 5804) for tests.
//!
//! It starts in clear text, offers STARTTLS, and only accepts
//! `AUTHENTICATE "PLAIN"` after TLS. It keeps scripts and the active one in
//! memory, and can be told to misbehave.

use crate::loopback::Loopback;
use crate::tls;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::task::JoinHandle;

/// Ways the fake server can misbehave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SieveFault {
    /// Don't offer STARTTLS.
    NoStartTls,
    /// Offer no PLAIN mechanism.
    NoPlain,
    /// Refuse every script in CHECKSCRIPT and PUTSCRIPT.
    RejectScripts,
    /// Answer HAVESPACE and PUTSCRIPT with `NO (QUOTA/MAXSIZE)`.
    NoSpace,
    /// Send this after the STARTTLS `OK`, in clear text, before TLS.
    InjectAfterStartTls(Vec<u8>),
    /// Send these bytes instead of the greeting.
    Greeting(Vec<u8>),
    /// Refuse SETACTIVE.
    RefuseSetActive,
}

#[derive(Debug, Default)]
struct State {
    scripts: BTreeMap<String, String>,
    active: Option<String>,
    /// Command names in order, with `+tls` when sent over TLS.
    commands: Vec<String>,
    /// Logins that succeeded.
    logins: usize,
}

struct Shared {
    user: (String, String),
    extensions: String,
    faults: Vec<SieveFault>,
    state: Mutex<State>,
}

impl Shared {
    fn has(&self, fault: &SieveFault) -> bool {
        self.faults.contains(fault)
    }
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut state)
    }
}

/// Builds a [`FakeSieve`].
pub struct FakeSieveBuilder {
    user: (String, String),
    extensions: String,
    scripts: Vec<(String, String)>,
    active: Option<String>,
    faults: Vec<SieveFault>,
}

impl FakeSieveBuilder {
    pub fn user(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.user = (username.into(), password.into());
        self
    }

    /// The `SIEVE` capability, e.g. `"fileinto imap4flags"`.
    pub fn extensions(mut self, list: impl Into<String>) -> Self {
        self.extensions = list.into();
        self
    }

    /// A script that is already there; `active` makes it the active one.
    pub fn script(
        mut self,
        name: impl Into<String>,
        body: impl Into<String>,
        active: bool,
    ) -> Self {
        let name = name.into();
        if active {
            self.active = Some(name.clone());
        }
        self.scripts.push((name, body.into()));
        self
    }

    pub fn fault(mut self, fault: SieveFault) -> Self {
        self.faults.push(fault);
        self
    }

    pub async fn start(self) -> Result<FakeSieve, String> {
        let tls = tls::generate()?;
        let listener = Loopback::bind().await?;
        let addr = listener.addr();
        let shared = Arc::new(Shared {
            user: self.user,
            extensions: self.extensions,
            faults: self.faults,
            state: Mutex::new(State {
                scripts: self.scripts.into_iter().collect(),
                active: self.active,
                ..State::default()
            }),
        });
        let for_loop = shared.clone();
        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let shared = for_loop.clone();
                let config = tls.config.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(stream, config, shared).await;
                });
            }
        });
        Ok(FakeSieve {
            addr,
            root_cert_der: tls::generate()?.cert_der,
            shared,
            handle: Mutex::new(Some(handle)),
        })
    }
}

/// A running fake ManageSieve server.
pub struct FakeSieve {
    addr: SocketAddr,
    root_cert_der: Vec<u8>,
    shared: Arc<Shared>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl FakeSieve {
    /// User `alice@example.org` / `password`, extensions
    /// `fileinto imap4flags`, no scripts.
    pub fn builder() -> FakeSieveBuilder {
        FakeSieveBuilder {
            user: ("alice@example.org".into(), "password".into()),
            extensions: "fileinto imap4flags".into(),
            scripts: Vec::new(),
            active: None,
            faults: Vec::new(),
        }
    }

    /// The port (plain text with STARTTLS, like 4190).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn root_cert_der(&self) -> &[u8] {
        &self.root_cert_der
    }

    /// Every script by name.
    pub fn scripts(&self) -> BTreeMap<String, String> {
        self.shared.with(|s| s.scripts.clone())
    }

    /// The active script's name.
    pub fn active(&self) -> Option<String> {
        self.shared.with(|s| s.active.clone())
    }

    /// Changes the active script, as the user might in another client.
    pub fn set_active(&self, name: Option<&str>) {
        self.shared.with(|s| s.active = name.map(str::to_owned));
    }

    /// Command names received, upper case, in order; `+tls` is appended to
    /// those that came over TLS (e.g. `"PUTSCRIPT+tls"`).
    pub fn commands(&self) -> Vec<String> {
        self.shared.with(|s| s.commands.clone())
    }

    /// How many logins succeeded.
    pub fn logins(&self) -> usize {
        self.shared.with(|s| s.logins)
    }

    pub fn shutdown(&self) {
        if let Some(h) = self.handle.lock().unwrap_or_else(|p| p.into_inner()).take() {
            h.abort();
        }
    }
}

impl Drop for FakeSieve {
    fn drop(&mut self) {
        self.shutdown();
    }
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

fn capabilities(shared: &Shared, tls: bool, authed: bool) -> Vec<u8> {
    let mut out = String::from("\"IMPLEMENTATION\" \"Mailcrane testkit\"\r\n");
    if !authed {
        let sasl = if shared.has(&SieveFault::NoPlain) {
            "LOGIN"
        } else {
            "PLAIN LOGIN"
        };
        out.push_str(&format!("\"SASL\" \"{sasl}\"\r\n"));
    }
    out.push_str(&format!("\"SIEVE\" \"{}\"\r\n", shared.extensions));
    if !tls && !shared.has(&SieveFault::NoStartTls) {
        out.push_str("\"STARTTLS\"\r\n");
    }
    out.push_str("\"VERSION\" \"1.0\"\r\nOK\r\n");
    out.into_bytes()
}

/// Reads one command: a line, plus any `{n+}` literals inside it. Returns
/// its arguments (the first is the command name), or `None` at the end.
async fn read_command(io: &mut Box<dyn Io>, buf: &mut Vec<u8>) -> Option<Vec<String>> {
    loop {
        if let Some((args, used)) = parse_command(buf) {
            buf.drain(..used);
            return Some(args);
        }
        if buf.len() > 4 * 1024 * 1024 {
            return None;
        }
        let mut chunk = [0u8; 8192];
        let n = io.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(chunk.get(..n)?);
    }
}

/// `None` when more bytes are needed (or the command is unreadable, which
/// the caller then treats as waiting forever: fine for a test server).
fn parse_command(buf: &[u8]) -> Option<(Vec<String>, usize)> {
    let mut args = Vec::new();
    let mut i = 0;
    loop {
        match buf.get(i)? {
            b' ' => i += 1,
            b'\r' => {
                if buf.get(i + 1)? == &b'\n' {
                    return Some((args, i + 2));
                }
                return None;
            }
            b'"' => {
                let mut s = Vec::new();
                i += 1;
                loop {
                    match buf.get(i)? {
                        b'\\' => {
                            s.push(*buf.get(i + 1)?);
                            i += 2;
                        }
                        b'"' => {
                            i += 1;
                            break;
                        }
                        b => {
                            s.push(*b);
                            i += 1;
                        }
                    }
                }
                args.push(String::from_utf8_lossy(&s).into_owned());
            }
            b'{' => {
                let close = buf.get(i..)?.iter().position(|b| *b == b'}')? + i;
                let n: usize = std::str::from_utf8(buf.get(i + 1..close)?)
                    .ok()?
                    .trim_end_matches('+')
                    .parse()
                    .ok()?;
                let start = close + 3;
                let bytes = buf.get(start..start + n)?;
                args.push(String::from_utf8_lossy(bytes).into_owned());
                i = start + n;
            }
            _ => {
                let end = buf
                    .get(i..)?
                    .iter()
                    .position(|b| *b == b' ' || *b == b'\r')?
                    + i;
                args.push(String::from_utf8_lossy(buf.get(i..end)?).into_owned());
                i = end;
            }
        }
    }
}

async fn send(io: &mut Box<dyn Io>, bytes: &[u8]) -> std::io::Result<()> {
    io.write_all(bytes).await?;
    io.flush().await
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    config: Arc<rustls::ServerConfig>,
    shared: Arc<Shared>,
) -> std::io::Result<()> {
    let mut io: Box<dyn Io> = Box::new(stream);
    let mut buf = Vec::new();
    let mut tls = false;
    let mut authed = false;
    let greeting = shared
        .faults
        .iter()
        .find_map(|f| match f {
            SieveFault::Greeting(g) => Some(g.clone()),
            _ => None,
        })
        .unwrap_or_else(|| capabilities(&shared, false, false));
    send(&mut io, &greeting).await?;
    while let Some(args) = read_command(&mut io, &mut buf).await {
        let name = args
            .first()
            .map(|a| a.to_ascii_uppercase())
            .unwrap_or_default();
        shared.with(|s| {
            s.commands.push(if tls {
                format!("{name}+tls")
            } else {
                name.clone()
            })
        });
        let arg = |n: usize| args.get(n).cloned().unwrap_or_default();
        match name.as_str() {
            "STARTTLS" if !tls && !shared.has(&SieveFault::NoStartTls) => {
                let mut ok = b"OK\r\n".to_vec();
                for f in &shared.faults {
                    if let SieveFault::InjectAfterStartTls(extra) = f {
                        ok.extend_from_slice(extra);
                    }
                }
                send(&mut io, &ok).await?;
                // The original stream is a TcpStream boxed as `dyn Io`; TLS
                // wraps the box.
                let accepted = tokio_rustls::TlsAcceptor::from(config.clone())
                    .accept(io)
                    .await?;
                io = Box::new(accepted);
                buf.clear();
                tls = true;
                send(&mut io, &capabilities(&shared, true, false)).await?;
            }
            "CAPABILITY" => send(&mut io, &capabilities(&shared, tls, authed)).await?,
            "AUTHENTICATE" if tls && !authed => {
                use base64::Engine as _;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(arg(2))
                    .unwrap_or_default();
                let parts: Vec<&[u8]> = decoded.split(|b| *b == 0).collect();
                let ok = arg(1).eq_ignore_ascii_case("PLAIN")
                    && !shared.has(&SieveFault::NoPlain)
                    && parts.len() == 3
                    && parts.get(1) == Some(&shared.user.0.as_bytes())
                    && parts.get(2) == Some(&shared.user.1.as_bytes());
                if ok {
                    authed = true;
                    shared.with(|s| s.logins += 1);
                    send(&mut io, b"OK\r\n").await?;
                } else {
                    send(&mut io, b"NO \"Authentication failed\"\r\n").await?;
                }
            }
            "LOGOUT" => {
                send(&mut io, b"OK \"Bye\"\r\n").await?;
                return Ok(());
            }
            _ if !authed => send(&mut io, b"NO \"Log in first\"\r\n").await?,
            "LISTSCRIPTS" => {
                let mut out = String::new();
                shared.with(|s| {
                    for name in s.scripts.keys() {
                        out.push_str(&quoted(name));
                        if s.active.as_deref() == Some(name) {
                            out.push_str(" ACTIVE");
                        }
                        out.push_str("\r\n");
                    }
                });
                out.push_str("OK\r\n");
                send(&mut io, out.as_bytes()).await?;
            }
            "GETSCRIPT" => {
                let body = shared.with(|s| s.scripts.get(&arg(1)).cloned());
                match body {
                    Some(b) => {
                        let out = format!("{{{}}}\r\n{b}\r\nOK\r\n", b.len());
                        send(&mut io, out.as_bytes()).await?;
                    }
                    None => send(&mut io, b"NO (NONEXISTENT) \"No such script\"\r\n").await?,
                }
            }
            "HAVESPACE" => {
                if shared.has(&SieveFault::NoSpace) {
                    send(&mut io, b"NO (QUOTA/MAXSIZE) \"Too big\"\r\n").await?;
                } else {
                    send(&mut io, b"OK\r\n").await?;
                }
            }
            "CHECKSCRIPT" => {
                if shared.has(&SieveFault::RejectScripts) {
                    send(&mut io, b"NO \"line 1: error\"\r\n").await?;
                } else {
                    send(&mut io, b"OK\r\n").await?;
                }
            }
            "PUTSCRIPT" => {
                if shared.has(&SieveFault::RejectScripts) {
                    send(&mut io, b"NO \"line 1: error\"\r\n").await?;
                } else if shared.has(&SieveFault::NoSpace) {
                    send(&mut io, b"NO (QUOTA/MAXSIZE) \"Too big\"\r\n").await?;
                } else {
                    shared.with(|s| s.scripts.insert(arg(1), arg(2)));
                    send(&mut io, b"OK\r\n").await?;
                }
            }
            "SETACTIVE" => {
                let name = arg(1);
                let exists = shared.with(|s| name.is_empty() || s.scripts.contains_key(&name));
                if shared.has(&SieveFault::RefuseSetActive) || !exists {
                    send(&mut io, b"NO \"Cannot activate\"\r\n").await?;
                } else {
                    shared.with(|s| s.active = (!name.is_empty()).then_some(name));
                    send(&mut io, b"OK\r\n").await?;
                }
            }
            _ => send(&mut io, b"NO \"Unknown command\"\r\n").await?,
        }
    }
    Ok(())
}
