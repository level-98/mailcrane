//! Connecting, logging in, listing folders and logging out.

use crate::command::{self, Arg, Tags};
use crate::connection::{connect_tcp, within, Connection, Io};
use crate::error::ImapError;
use crate::parser::{Code, Condition, Limits, RespText, Response, StatusKind, Untagged};
use crate::{b64, mutf7, roles, tls, Secret};
use mailcrane_store::{AccountId, FolderId, FolderRole, NewFolder, Store};
use std::collections::HashMap;
use std::time::Duration;

/// How the connection is protected. There is no plain-text option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImapSecurity {
    /// TLS from the first byte (RFC 8314), usually port 993.
    ImplicitTls,
    /// Start in clear text and upgrade with STARTTLS (RFC 9051 §6.2.1),
    /// usually port 143. Fails if the server does not offer it.
    StartTls,
}

/// Where to connect, and the limits to apply.
#[derive(Debug, Clone)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub security: ImapSecurity,
    /// Time allowed for the TCP connection. Default 30 s.
    pub connect_timeout: Duration,
    /// Time allowed for each command, from sending it to its tagged
    /// response, and for the TLS handshake. Default 60 s.
    pub command_timeout: Duration,
    /// Response size limits.
    pub limits: Limits,
    extra_root_certs: Vec<Vec<u8>>,
}

impl ImapConfig {
    /// A config with the default timeouts and limits.
    pub fn new(host: impl Into<String>, port: u16, security: ImapSecurity) -> Self {
        Self {
            host: host.into(),
            port,
            security,
            connect_timeout: Duration::from_secs(30),
            command_timeout: Duration::from_secs(60),
            limits: Limits::default(),
            extra_root_certs: Vec::new(),
        }
    }

    /// Trusts one more root certificate (DER) on top of the platform's.
    /// For tests against the testkit's servers. It only adds trust; it
    /// cannot turn certificate checking off. Ignored on Android, where the
    /// platform verifier does not support extra roots.
    pub fn with_extra_root_cert(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.extra_root_certs.push(der.into());
        self
    }
}

/// A folder as listed by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapFolder {
    /// Name exactly as the server sent it; use this in commands.
    pub raw_name: String,
    /// Name decoded from modified UTF-7, for display. Falls back to the raw
    /// name if it is not valid modified UTF-7.
    pub name: String,
    /// Hierarchy delimiter, if the server has one.
    pub delimiter: Option<char>,
    /// `LIST` attributes, e.g. `\HasChildren` or `\Sent`.
    pub attributes: Vec<String>,
    /// The folder's role, from SPECIAL-USE or its name.
    pub role: Option<FolderRole>,
}

/// An authenticated IMAP session over TLS.
pub struct Session {
    pub(crate) conn: Connection,
    caps: Vec<String>,
    pub(crate) tags: Tags,
    pub(crate) command_timeout: Duration,
    host: String,
    /// Whether `ENABLE QRESYNC` succeeded (RFC 7162 §3.2.3).
    pub(crate) qresync_enabled: bool,
    /// The last command sent, as a fixed label (see [`command_label`]).
    pub(crate) last_command: Option<&'static str>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("host", &self.host)
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

/// The untagged responses and the tagged result of one command.
pub(crate) struct Done {
    pub(crate) untagged: Vec<Untagged>,
    pub(crate) status: StatusKind,
    pub(crate) text: RespText,
}

fn valid_credential(value: &str) -> bool {
    !value.is_empty() && !value.contains(['\0', '\r', '\n'])
}

pub(crate) fn has_cap(caps: &[String], name: &str) -> bool {
    caps.iter().any(|c| c.eq_ignore_ascii_case(name))
}

fn caps_from_code(text: &RespText) -> Option<Vec<String>> {
    match &text.code {
        Some(Code::Capability(caps)) => Some(caps.clone()),
        _ => None,
    }
}

impl Session {
    /// Connects over TLS and logs in.
    ///
    /// Credentials are only ever sent after the TLS handshake has
    /// succeeded. With [`ImapSecurity::StartTls`], a server that does not
    /// offer STARTTLS fails with [`ImapError::StartTlsNotOffered`].
    pub async fn connect(
        config: &ImapConfig,
        username: &str,
        password: &Secret,
    ) -> Result<Session, ImapError> {
        if config.host.is_empty() || config.host.chars().any(|c| c.is_control() || c == ' ') {
            return Err(ImapError::InvalidInput);
        }
        if !valid_credential(username) || !valid_credential(password.expose()) {
            return Err(ImapError::InvalidInput);
        }
        let host = config.host.as_str();
        let timeout = config.command_timeout;
        let tcp = connect_tcp(host, config.port, config.connect_timeout).await?;
        tracing::debug!(host, port = config.port, "IMAP connected");

        let (conn, greeting) = match config.security {
            ImapSecurity::ImplicitTls => {
                let tls = tls::handshake(host, &config.extra_root_certs, timeout, tcp).await?;
                let mut conn = Connection::new(Box::new(tls), config.limits);
                let greeting = read_greeting(&mut conn, timeout).await?;
                (conn, greeting)
            }
            ImapSecurity::StartTls => {
                let conn = Connection::new(Box::new(tcp), config.limits);
                start_tls(conn, config).await?
            }
        };

        let mut session = Session {
            conn,
            caps: Vec::new(),
            tags: Tags::default(),
            command_timeout: timeout,
            host: config.host.clone(),
            qresync_enabled: false,
            last_command: None,
        };
        let (condition, greeting_text) = greeting;
        // RFC 9051 §6.2.1: after STARTTLS, forget anything learned before;
        // start_tls returns no greeting caps for that reason.
        session.caps = match caps_from_code(&greeting_text) {
            Some(caps) => caps,
            None => session.capability().await?,
        };
        if condition == Condition::PreAuth {
            // RFC 9051 §7.1.4: already authenticated, over TLS.
            return Ok(session);
        }
        session.login(username, password).await?;
        tracing::debug!(host, "IMAP logged in");
        Ok(session)
    }

    /// A logged-in session over `io`, for tests that script the server.
    #[cfg(test)]
    pub(crate) fn from_io(io: Box<dyn Io>, caps: &[&str], command_timeout: Duration) -> Session {
        Session {
            conn: Connection::new(io, Limits::default()),
            caps: caps.iter().map(|c| (*c).to_string()).collect(),
            tags: Tags::default(),
            command_timeout,
            host: "test.example.org".into(),
            qresync_enabled: false,
            last_command: None,
        }
    }

    /// The server's capabilities, as last reported.
    pub fn capabilities(&self) -> &[String] {
        &self.caps
    }

    /// The host name connected to.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Sends a command and collects its responses within the command
    /// timeout.
    async fn run(&mut self, name: &str, args: &[Arg<'_>]) -> Result<Done, ImapError> {
        self.last_command = Some(command_label(name, args));
        let cmd = command::build(self.tags.next(), name, args)?;
        let limit = self.command_timeout;
        within(limit, exchange(&mut self.conn, cmd.tag, cmd.segments)).await?
    }

    pub(crate) async fn run_command(
        &mut self,
        name: &str,
        args: &[Arg<'_>],
    ) -> Result<Done, ImapError> {
        self.run(name, args).await
    }

    /// The last command sent on this session as a fixed label, e.g.
    /// `"SELECT"` or `"UID FETCH"`, for logging which step failed. Never
    /// holds arguments or server text.
    pub fn last_command(&self) -> Option<&'static str> {
        self.last_command
    }

    /// `CAPABILITY` (RFC 9051 §6.1.1).
    async fn capability(&mut self) -> Result<Vec<String>, ImapError> {
        let done = self.run("CAPABILITY", &[]).await?;
        expect_ok(&done)?;
        capabilities_in(&done).ok_or(ImapError::Protocol)
    }

    /// `AUTHENTICATE PLAIN` if offered (RFC 9051 §6.2.2, RFC 4616),
    /// otherwise `LOGIN` (§6.2.3) unless `LOGINDISABLED` is advertised.
    async fn login(&mut self, username: &str, password: &Secret) -> Result<(), ImapError> {
        let done = if has_cap(&self.caps, "AUTH=PLAIN") {
            self.authenticate_plain(username, password).await?
        } else if has_cap(&self.caps, "LOGINDISABLED") {
            return Err(ImapError::LoginDisabled);
        } else {
            self.run(
                "LOGIN",
                &[
                    Arg::Str(username.as_bytes()),
                    Arg::Str(password.expose().as_bytes()),
                ],
            )
            .await?
        };
        if done.status != StatusKind::Ok {
            return Err(ImapError::AuthFailed);
        }
        // Capabilities may change after login (RFC 9051 §6.2.2).
        self.caps = match caps_from_code(&done.text).or_else(|| capabilities_in(&done)) {
            Some(caps) => caps,
            None => self.capability().await?,
        };
        Ok(())
    }

    async fn authenticate_plain(
        &mut self,
        username: &str,
        password: &Secret,
    ) -> Result<Done, ImapError> {
        self.last_command = Some("AUTHENTICATE");
        let tag = self.tags.next();
        let start = format!("{tag} AUTHENTICATE PLAIN\r\n").into_bytes();
        // RFC 4616 §2: empty authzid, NUL, authcid, NUL, passwd.
        let mut payload = Vec::with_capacity(username.len() + password.expose().len() + 2);
        payload.push(0);
        payload.extend_from_slice(username.as_bytes());
        payload.push(0);
        payload.extend_from_slice(password.expose().as_bytes());
        let response = format!("{}\r\n", b64::encode(&payload)).into_bytes();
        let limit = self.command_timeout;
        within(limit, exchange(&mut self.conn, tag, vec![start, response])).await?
    }

    /// Lists every folder (`LIST "" "*"`, RFC 9051 §6.3.9), asking for
    /// SPECIAL-USE attributes when the server supports them (RFC 6154 §5.1),
    /// and works out each folder's role.
    pub async fn list_folders(&mut self) -> Result<Vec<ImapFolder>, ImapError> {
        let special_use = has_cap(&self.caps, "SPECIAL-USE")
            && (has_cap(&self.caps, "LIST-EXTENDED") || has_cap(&self.caps, "IMAP4rev2"));
        let mut args = vec![Arg::Str(b""), Arg::Str(b"*")];
        if special_use {
            args.push(Arg::Raw("RETURN (SPECIAL-USE)"));
        }
        let done = self.run("LIST", &args).await?;
        expect_ok(&done)?;

        let mut folders: Vec<ImapFolder> = Vec::new();
        let mut skipped = 0usize;
        for untagged in done.untagged {
            let Untagged::List(item) = untagged else {
                continue;
            };
            // RFC 5258 §3.4: a folder that does not exist.
            if item
                .attributes
                .iter()
                .any(|a| a.eq_ignore_ascii_case("\\NonExistent"))
            {
                continue;
            }
            let Ok(raw_name) = String::from_utf8(item.name) else {
                skipped += 1;
                continue;
            };
            if folders.iter().any(|f| f.raw_name == raw_name) {
                continue;
            }
            // RFC 9051 §5.1: INBOX is case-insensitive.
            let name = if raw_name.eq_ignore_ascii_case("INBOX") {
                "INBOX".to_string()
            } else {
                mutf7::decode(&raw_name).unwrap_or_else(|_| raw_name.clone())
            };
            folders.push(ImapFolder {
                raw_name,
                name,
                delimiter: item.delimiter,
                attributes: item.attributes,
                role: None,
            });
        }
        if skipped > 0 {
            tracing::warn!(skipped, "skipped folders whose names are not UTF-8");
        }

        let delimiters: Vec<Option<String>> = folders
            .iter()
            .map(|f| f.delimiter.map(String::from))
            .collect();
        let candidates: Vec<roles::Candidate<'_>> = folders
            .iter()
            .zip(&delimiters)
            .map(|(f, d)| roles::Candidate {
                name: &f.name,
                delimiter: d.as_deref(),
                attributes: &f.attributes,
            })
            .collect();
        let assigned = roles::assign(&candidates);
        for (folder, role) in folders.iter_mut().zip(assigned) {
            folder.role = role;
        }
        tracing::debug!(host = %self.host, count = folders.len(), "IMAP folders listed");
        Ok(folders)
    }

    /// Logs out and closes the connection, ignoring errors (RFC 9051 §6.1.3).
    pub async fn logout(mut self) {
        let _ = within(Duration::from_secs(5), async {
            let _ = self.run("LOGOUT", &[]).await;
            self.conn.shutdown().await;
        })
        .await;
    }
}

/// Saves listed folders to the store in one transaction, keyed by their
/// raw names. Sync state already stored (UIDVALIDITY, UIDNEXT,
/// HIGHESTMODSEQ, subscription) is kept. Returns the folder ids in the same
/// order as `folders`. On error nothing is saved.
pub fn save_folders(
    store: &Store,
    account: AccountId,
    folders: &[ImapFolder],
) -> Result<Vec<FolderId>, ImapError> {
    let existing: HashMap<String, NewFolder> = store
        .list_folders(account)?
        .into_iter()
        .map(|f| (f.details.remote_name.clone(), f.details))
        .collect();
    let rows: Vec<NewFolder> = folders
        .iter()
        .map(|f| {
            let old = existing.get(&f.raw_name);
            NewFolder {
                account_id: account,
                remote_name: f.raw_name.clone(),
                display_name: f.name.clone(),
                delimiter: f.delimiter.map(String::from),
                role: f.role,
                uidvalidity: old.and_then(|o| o.uidvalidity),
                uidnext: old.and_then(|o| o.uidnext),
                highestmodseq: old.and_then(|o| o.highestmodseq),
                is_subscribed: old.is_none_or(|o| o.is_subscribed),
                selectable: !f
                    .attributes
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case("\\Noselect")),
            }
        })
        .collect();
    Ok(store.upsert_folders(&rows)?)
}

/// A fixed label for a command, safe to log: the command name, and for `UID`
/// its subcommand, e.g. `"UID FETCH"`. Never an argument. A name not in the
/// list becomes `"OTHER"`, so nothing that came from data can reach a log.
pub(crate) fn command_label(name: &str, args: &[Arg<'_>]) -> &'static str {
    const NAMES: &[&str] = &[
        "APPEND",
        "AUTHENTICATE",
        "CAPABILITY",
        "CLOSE",
        "CREATE",
        "DELETE",
        "ENABLE",
        "EXAMINE",
        "EXPUNGE",
        "FETCH",
        "ID",
        "IDLE",
        "LIST",
        "LOGIN",
        "LOGOUT",
        "NAMESPACE",
        "NOOP",
        "RENAME",
        "SEARCH",
        "SELECT",
        "STARTTLS",
        "STATUS",
        "STORE",
        "UNSELECT",
    ];
    const UID: &[&str] = &[
        "UID COPY",
        "UID EXPUNGE",
        "UID FETCH",
        "UID MOVE",
        "UID SEARCH",
        "UID STORE",
    ];
    if name.eq_ignore_ascii_case("UID") {
        let Some(Arg::Raw(sub)) = args.first() else {
            return "UID";
        };
        return UID
            .iter()
            .find(|label| {
                label
                    .strip_prefix("UID ")
                    .is_some_and(|l| l.eq_ignore_ascii_case(sub))
            })
            .copied()
            .unwrap_or("UID");
    }
    NAMES
        .iter()
        .find(|label| label.eq_ignore_ascii_case(name))
        .copied()
        .unwrap_or("OTHER")
}

pub(crate) fn expect_ok(done: &Done) -> Result<(), ImapError> {
    if done.status == StatusKind::Ok {
        Ok(())
    } else {
        Err(ImapError::CommandFailed)
    }
}

fn capabilities_in(done: &Done) -> Option<Vec<String>> {
    done.untagged.iter().find_map(|u| match u {
        Untagged::Capability(caps) => Some(caps.clone()),
        _ => None,
    })
}

/// Reads the server greeting (RFC 9051 §7.1).
async fn read_greeting(
    conn: &mut Connection,
    limit: Duration,
) -> Result<(Condition, RespText), ImapError> {
    match within(limit, conn.read_response()).await?? {
        Response::Untagged(Untagged::Condition { condition, text }) => match condition {
            Condition::Ok | Condition::PreAuth => Ok((condition, text)),
            Condition::Bye => Err(ImapError::ServerBye),
            Condition::No | Condition::Bad => Err(ImapError::Protocol),
        },
        _ => Err(ImapError::Protocol),
    }
}

/// The STARTTLS upgrade (RFC 9051 §6.2.1). Nothing but `CAPABILITY` and
/// `STARTTLS` is sent in clear text.
async fn start_tls(
    mut conn: Connection,
    config: &ImapConfig,
) -> Result<(Connection, (Condition, RespText)), ImapError> {
    let limit = config.command_timeout;
    let (condition, greeting) = read_greeting(&mut conn, limit).await?;
    if condition == Condition::PreAuth {
        // Already authenticated in clear text: STARTTLS is no longer
        // allowed, so we cannot secure this connection.
        return Err(ImapError::StartTlsNotOffered);
    }
    let mut tags = Tags::default();
    let caps = match caps_from_code(&greeting) {
        Some(caps) => caps,
        None => {
            let cmd = command::build(tags.next(), "CAPABILITY", &[])?;
            let done = within(limit, exchange(&mut conn, cmd.tag, cmd.segments)).await??;
            expect_ok(&done)?;
            capabilities_in(&done).ok_or(ImapError::Protocol)?
        }
    };
    if !has_cap(&caps, "STARTTLS") {
        return Err(ImapError::StartTlsNotOffered);
    }
    let cmd = command::build(tags.next(), "STARTTLS", &[])?;
    let done = within(limit, exchange(&mut conn, cmd.tag, cmd.segments)).await??;
    if done.status != StatusKind::Ok {
        return Err(ImapError::StartTlsNotOffered);
    }
    // Bytes after the tagged OK were sent before TLS and could have been
    // injected by someone in the middle (CVE-2011-0411 class); refuse them.
    if conn.has_buffered() {
        return Err(ImapError::TlsFailure);
    }
    let io: Box<dyn Io> = conn.into_io();
    let tls = tls::handshake(&config.host, &config.extra_root_certs, limit, io).await?;
    let conn = Connection::new(Box::new(tls), config.limits);
    // The greeting's text and code are from before TLS: drop them.
    let empty = RespText {
        code: None,
        text: String::new(),
    };
    Ok((conn, (Condition::Ok, empty)))
}

/// Sends `segments`, waiting for a `+` continuation between them, and
/// collects responses until the one tagged `tag`.
async fn exchange(
    conn: &mut Connection,
    tag: String,
    segments: Vec<Vec<u8>>,
) -> Result<Done, ImapError> {
    let mut untagged = Vec::new();
    let mut segments = segments.into_iter();
    if let Some(first) = segments.next() {
        conn.write(&first).await?;
    }
    for segment in segments {
        loop {
            match conn.read_response().await? {
                Response::Continuation { .. } => break,
                Response::Untagged(u) => push_untagged(&mut untagged, u)?,
                // The server refused before we sent the rest.
                Response::Tagged {
                    tag: t,
                    status,
                    text,
                } if t == tag => {
                    return Ok(Done {
                        untagged,
                        status,
                        text,
                    })
                }
                Response::Tagged { .. } => return Err(ImapError::Protocol),
            }
        }
        conn.write(&segment).await?;
    }
    loop {
        match conn.read_response().await? {
            Response::Untagged(u) => push_untagged(&mut untagged, u)?,
            Response::Tagged {
                tag: t,
                status,
                text,
            } if t == tag => {
                return Ok(Done {
                    untagged,
                    status,
                    text,
                })
            }
            Response::Tagged { .. } | Response::Continuation { .. } => {
                return Err(ImapError::Protocol)
            }
        }
    }
}

fn push_untagged(untagged: &mut Vec<Untagged>, u: Untagged) -> Result<(), ImapError> {
    if let Untagged::Condition {
        condition: Condition::Bye,
        ..
    } = u
    {
        return Err(ImapError::ServerBye);
    }
    // A hostile server could send untagged data forever; each response is
    // already size-limited, and this caps how many we keep per command.
    if untagged.len() >= 100_000 {
        return Err(ImapError::ResponseTooLarge);
    }
    untagged.push(u);
    Ok(())
}

#[cfg(test)]
mod label_tests {
    use super::command_label;
    use crate::command::Arg;
    use crate::ImapError;

    #[test]
    fn command_labels_are_names_and_uid_subcommands_only() {
        assert_eq!(command_label("SELECT", &[Arg::Str(b"INBOX")]), "SELECT");
        assert_eq!(command_label("select", &[]), "SELECT");
        assert_eq!(
            command_label("UID", &[Arg::Raw("FETCH"), Arg::Built("1:*")]),
            "UID FETCH"
        );
        assert_eq!(command_label("UID", &[Arg::Raw("EXPUNGE")]), "UID EXPUNGE");
        // A subcommand we don't know, or none, is just "UID".
        assert_eq!(command_label("UID", &[Arg::Raw("THREAD")]), "UID");
        assert_eq!(command_label("UID", &[Arg::Str(b"FETCH")]), "UID");
        assert_eq!(command_label("UID", &[]), "UID");
    }

    #[test]
    fn an_unknown_command_name_never_reaches_the_label() {
        // Whatever a caller passes, the label is one of our fixed strings.
        for name in ["X-SECRET hunter2", "alice@example.org", "", "SELECT INBOX"] {
            assert_eq!(command_label(name, &[]), "OTHER");
        }
    }

    #[test]
    fn error_names_are_fixed_identifiers() {
        let errors = [
            ImapError::InvalidInput,
            ImapError::CannotConnect,
            ImapError::TlsFailure,
            ImapError::StartTlsNotOffered,
            ImapError::LoginDisabled,
            ImapError::AuthFailed,
            ImapError::CommandFailed,
            ImapError::ServerBye,
            ImapError::ConnectionClosed,
            ImapError::Protocol,
            ImapError::LineTooLong,
            ImapError::ResponseTooLarge,
            ImapError::Timeout,
        ];
        for error in errors {
            let name = error.name();
            assert!(!name.is_empty());
            assert!(name.chars().all(|c| c.is_ascii_alphabetic()), "{name}");
            assert_eq!(format!("{error:?}"), name);
        }
    }
}
