//! Adding an IMAP/SMTP account from server details the user types in.
//!
//! [`add_account_manual`] validates the details, logs in to the IMAP server
//! and checks SMTP authentication, and only then saves the account to the
//! store and the password to [`SecretStore`]. If any step fails, nothing is
//! saved. There is no plain-text option: every connection uses implicit TLS
//! or STARTTLS, and a server without STARTTLS is an error.

use std::time::Duration;

use mailcrane_store::{AccountId, NewAccount, Store};

use crate::secrets::{Secret, SecretKey, SecretStore};

/// How long each server gets to connect, upgrade to TLS and accept the
/// login, before setup gives up with [`AddAccountError::Timeout`].
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// How a connection is protected. There is deliberately no plain-text option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Security {
    /// TLS from the first byte (RFC 8314).
    Tls,
    /// Start in clear text and upgrade with STARTTLS (RFC 9051 §6.2.1 for
    /// IMAP, RFC 3207 for SMTP). Fails if the server does not offer it.
    StartTls,
}

/// Which server a setting or an error belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// The incoming mail server.
    Imap,
    /// The outgoing mail server.
    Smtp,
}

/// One server's address and connection security, as the user typed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Host name or IP address, with no scheme and no port.
    pub host: String,
    /// TCP port. Wider than `u16` so that out-of-range input from the apps
    /// reaches [`validate`] and gets a field error instead of wrapping.
    pub port: u32,
    /// TLS or STARTTLS.
    pub security: Security,
}

/// Everything the user types to add an account by hand, except the password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAccountConfig {
    /// The account's email address.
    pub email: String,
    /// The name shown on sent mail. May be empty.
    pub display_name: String,
    /// The incoming (IMAP) server.
    pub imap: ServerConfig,
    /// The outgoing (SMTP) server.
    pub smtp: ServerConfig,
    /// The login name, used for both servers.
    pub username: String,
}

/// A form field that can be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Email,
    DisplayName,
    ImapHost,
    ImapPort,
    SmtpHost,
    SmtpPort,
    Username,
    Password,
}

/// What is wrong with a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldProblem {
    /// The field is required but empty.
    Empty,
    /// A host name contains a space or other whitespace.
    ContainsSpace,
    /// A host name has a scheme such as `imaps://`.
    HasScheme,
    /// A field contains a control character (such as a line break) or, for a
    /// host, a character that can't appear in a host name.
    InvalidCharacters,
    /// A port is outside 1–65535.
    PortOutOfRange,
    /// An email address doesn't have exactly one `@` and a domain.
    MalformedEmail,
}

/// One field-specific validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldError {
    pub field: Field,
    pub problem: FieldProblem,
}

/// Why an account could not be added. In every case nothing was saved.
///
/// None of these carry server text or anything the user typed, so they are
/// safe to log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddAccountError {
    /// One or more fields are invalid. Nothing was sent to any server.
    #[error("some fields are invalid")]
    Invalid(Vec<FieldError>),
    /// The server could not be reached, or did not talk IMAP/SMTP.
    #[error("cannot connect to the {server:?} server")]
    CannotConnect { server: Protocol },
    /// The TLS handshake failed, including an untrusted certificate.
    #[error("TLS failure with the {server:?} server")]
    TlsFailure { server: Protocol },
    /// STARTTLS was chosen but the server does not offer it. The password was
    /// not sent.
    #[error("the {server:?} server does not offer STARTTLS")]
    StartTlsNotOffered { server: Protocol },
    /// The server refused the username or password.
    #[error("the {server:?} server refused the login")]
    AuthFailed { server: Protocol },
    /// The server did not finish within [`CHECK_TIMEOUT`].
    #[error("the {server:?} server timed out")]
    Timeout { server: Protocol },
    /// Both servers accepted the login, but the account or password could
    /// not be saved. Anything partly saved was removed again.
    #[error("the account could not be saved")]
    SaveFailed,
    /// An account for the same mailbox (incoming server and username or
    /// address) is already in the app (#346). Nothing was saved.
    #[error("this account is already added")]
    AlreadyAdded,
}

/// The server an account reads its mail from, as stored.
#[derive(Debug, Clone, Copy)]
pub enum IncomingServer<'a> {
    /// An IMAP host name.
    Imap(&'a str),
    /// A JMAP session URL.
    Jmap(&'a str),
}

/// Host names and URLs compared ignoring case and a trailing dot.
fn same_server(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches('.').to_lowercase();
    norm(a) == norm(b)
}

/// Fails with [`AddAccountError::AlreadyAdded`] if `store` already has an
/// account for this mailbox: the same incoming server and the same username
/// or address, ignoring case (#346). Local-only accounts don't count.
pub fn ensure_not_added(
    store: &Store,
    server: IncomingServer<'_>,
    username: &str,
    email: &str,
) -> Result<(), AddAccountError> {
    let same = |a: &str, b: &str| {
        !a.trim().is_empty() && a.trim().to_lowercase() == b.trim().to_lowercase()
    };
    let accounts = store
        .list_accounts()
        .map_err(|_| AddAccountError::SaveFailed)?;
    for account in accounts {
        let d = &account.details;
        if !(same(&d.username, username) || same(&d.email, email)) {
            continue;
        }
        if store
            .is_local_account(account.id)
            .map_err(|_| AddAccountError::SaveFailed)?
        {
            continue;
        }
        let protocol = store
            .account_protocol(account.id)
            .map_err(|_| AddAccountError::SaveFailed)?;
        let matches = match (server, protocol) {
            (IncomingServer::Imap(host), Some(mailcrane_store::Protocol::Imap)) => {
                same_server(&d.imap_host, host)
            }
            (IncomingServer::Jmap(url), Some(mailcrane_store::Protocol::Jmap { session_url })) => {
                same_server(&session_url, url)
            }
            _ => false,
        };
        if matches {
            return Err(AddAccountError::AlreadyAdded);
        }
    }
    Ok(())
}

/// The usual port for a protocol and security choice: IMAP 993 (TLS) or 143
/// (STARTTLS), SMTP 465 (TLS, RFC 8314) or 587 (STARTTLS, RFC 6409).
pub fn default_port(protocol: Protocol, security: Security) -> u16 {
    match (protocol, security) {
        (Protocol::Imap, Security::Tls) => 993,
        (Protocol::Imap, Security::StartTls) => 143,
        (Protocol::Smtp, Security::Tls) => 465,
        (Protocol::Smtp, Security::StartTls) => 587,
    }
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn check_host(host: &str, field: Field, errors: &mut Vec<FieldError>) {
    let problem = if host.is_empty() {
        Some(FieldProblem::Empty)
    } else if host.chars().any(char::is_whitespace) {
        Some(FieldProblem::ContainsSpace)
    } else if host.contains("://") {
        Some(FieldProblem::HasScheme)
    } else if has_control(host) || host.contains(['/', '@', '?', '#']) {
        Some(FieldProblem::InvalidCharacters)
    } else {
        None
    };
    if let Some(problem) = problem {
        errors.push(FieldError { field, problem });
    }
}

fn check_port(port: u32, field: Field, errors: &mut Vec<FieldError>) {
    if !(1..=65535).contains(&port) {
        errors.push(FieldError {
            field,
            problem: FieldProblem::PortOutOfRange,
        });
    }
}

pub(crate) fn email_problem(email: &str) -> Option<FieldProblem> {
    if email.is_empty() {
        return Some(FieldProblem::Empty);
    }
    if has_control(email) {
        return Some(FieldProblem::InvalidCharacters);
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Some(FieldProblem::MalformedEmail);
    };
    let bad_domain = domain.is_empty()
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..");
    if local.is_empty() || bad_domain || email.chars().any(char::is_whitespace) {
        return Some(FieldProblem::MalformedEmail);
    }
    None
}

/// Checks the form before anything is sent. Returns every problem found, so
/// the apps can mark all the wrong fields at once.
pub fn validate(config: &ManualAccountConfig) -> Result<(), Vec<FieldError>> {
    let mut errors = Vec::new();
    if let Some(problem) = email_problem(&config.email) {
        errors.push(FieldError {
            field: Field::Email,
            problem,
        });
    }
    // The display name goes into From: headers, so line breaks would allow
    // header injection.
    if has_control(&config.display_name) {
        errors.push(FieldError {
            field: Field::DisplayName,
            problem: FieldProblem::InvalidCharacters,
        });
    }
    check_host(&config.imap.host, Field::ImapHost, &mut errors);
    check_port(config.imap.port, Field::ImapPort, &mut errors);
    check_host(&config.smtp.host, Field::SmtpHost, &mut errors);
    check_port(config.smtp.port, Field::SmtpPort, &mut errors);
    if config.username.is_empty() {
        errors.push(FieldError {
            field: Field::Username,
            problem: FieldProblem::Empty,
        });
    } else if has_control(&config.username) {
        errors.push(FieldError {
            field: Field::Username,
            problem: FieldProblem::InvalidCharacters,
        });
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub(crate) fn password_problem(password: &Secret) -> Option<FieldProblem> {
    let bytes = password.expose();
    if bytes.is_empty() {
        Some(FieldProblem::Empty)
    } else if std::str::from_utf8(bytes).is_err()
        || bytes.iter().any(|b| matches!(b, 0 | b'\r' | b'\n'))
    {
        Some(FieldProblem::InvalidCharacters)
    } else {
        None
    }
}

/// Options for [`check_account`] and [`add_account_manual_with`]. The
/// defaults are what the apps use.
#[derive(Debug, Clone)]
pub struct SetupOptions {
    /// Time allowed per server. Default [`CHECK_TIMEOUT`].
    pub timeout: Duration,
    /// Extra trusted root certificates (DER), for tests against the testkit's
    /// servers. This only adds trust; it cannot turn certificate checks off.
    pub extra_root_certs: Vec<Vec<u8>>,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            timeout: CHECK_TIMEOUT,
            extra_root_certs: Vec::new(),
        }
    }
}

fn map_imap(err: mailcrane_imap::ImapError) -> AddAccountError {
    use mailcrane_imap::ImapError as E;
    let server = Protocol::Imap;
    match err {
        E::TlsFailure => AddAccountError::TlsFailure { server },
        E::StartTlsNotOffered => AddAccountError::StartTlsNotOffered { server },
        E::AuthFailed | E::LoginDisabled => AddAccountError::AuthFailed { server },
        E::Timeout => AddAccountError::Timeout { server },
        // `InvalidInput` can't happen after `validate`; the rest mean the
        // server is not a working IMAP server at that address.
        E::InvalidInput
        | E::CannotConnect
        | E::CommandFailed
        | E::ServerBye
        | E::ConnectionClosed
        | E::Protocol
        | E::LineTooLong
        | E::ResponseTooLarge
        | E::Store(_) => AddAccountError::CannotConnect { server },
    }
}

fn map_smtp(err: mailcrane_smtp::SmtpError) -> AddAccountError {
    use mailcrane_smtp::SmtpError as E;
    let server = Protocol::Smtp;
    match err {
        E::TlsFailure => AddAccountError::TlsFailure { server },
        E::StartTlsNotOffered => AddAccountError::StartTlsNotOffered { server },
        E::AuthFailed => AddAccountError::AuthFailed { server },
        E::Timeout => AddAccountError::Timeout { server },
        E::CannotConnect
        | E::InvalidInput
        | E::RecipientsRejected { .. }
        | E::Temporary { .. }
        | E::Permanent { .. } => AddAccountError::CannotConnect { server },
    }
}

/// What a check logs in with.
#[derive(Clone, Copy)]
enum LoginWith<'a> {
    Password(&'a str),
    /// An OAuth access token, for XOAUTH2 (M3-02).
    Token(&'a str),
}

async fn check_imap(
    server: &ServerConfig,
    port: u16,
    username: &str,
    login: LoginWith<'_>,
    options: &SetupOptions,
) -> Result<(), AddAccountError> {
    let security = match server.security {
        Security::Tls => mailcrane_imap::ImapSecurity::ImplicitTls,
        Security::StartTls => mailcrane_imap::ImapSecurity::StartTls,
    };
    let mut config = mailcrane_imap::ImapConfig::new(server.host.clone(), port, security);
    config.connect_timeout = options.timeout;
    config.command_timeout = options.timeout;
    for der in &options.extra_root_certs {
        config = config.with_extra_root_cert(der.clone());
    }
    let auth = match login {
        LoginWith::Password(p) => {
            mailcrane_imap::ImapAuth::Password(mailcrane_imap::Secret::new(p))
        }
        LoginWith::Token(t) => mailcrane_imap::ImapAuth::XOAuth2(mailcrane_imap::Secret::new(t)),
    };
    let attempt = async {
        let session = mailcrane_imap::Session::connect_auth(&config, username, &auth).await?;
        session.logout().await;
        Ok::<(), mailcrane_imap::ImapError>(())
    };
    match tokio::time::timeout(options.timeout, attempt).await {
        Ok(result) => result.map_err(map_imap),
        Err(_) => Err(AddAccountError::Timeout {
            server: Protocol::Imap,
        }),
    }
}

async fn check_smtp(
    server: &ServerConfig,
    port: u16,
    username: &str,
    login: LoginWith<'_>,
    options: &SetupOptions,
) -> Result<(), AddAccountError> {
    let security = match server.security {
        Security::Tls => mailcrane_smtp::SmtpSecurity::ImplicitTls,
        Security::StartTls => mailcrane_smtp::SmtpSecurity::StartTls,
    };
    let mut config = mailcrane_smtp::SmtpConfig::new(server.host.clone(), port, security);
    config.connect_timeout = options.timeout;
    config.command_timeout = options.timeout;
    config.extra_root_certs = options.extra_root_certs.clone();
    let auth = match login {
        LoginWith::Password(p) => mailcrane_smtp::SmtpAuth::Password {
            username: username.to_owned(),
            password: mailcrane_smtp::Secret::new(p),
        },
        LoginWith::Token(t) => mailcrane_smtp::SmtpAuth::XOAuth2 {
            username: username.to_owned(),
            token: mailcrane_smtp::Secret::new(t),
        },
    };
    match tokio::time::timeout(options.timeout, mailcrane_smtp::check_login(&config, &auth)).await {
        Ok(result) => result.map_err(map_smtp),
        Err(_) => Err(AddAccountError::Timeout {
            server: Protocol::Smtp,
        }),
    }
}

/// The checked ports, once [`validate`] has passed.
fn ports(config: &ManualAccountConfig) -> Result<(u16, u16), AddAccountError> {
    let port = |value: u32, field| {
        u16::try_from(value).map_err(|_| {
            AddAccountError::Invalid(vec![FieldError {
                field,
                problem: FieldProblem::PortOutOfRange,
            }])
        })
    };
    Ok((
        port(config.imap.port, Field::ImapPort)?,
        port(config.smtp.port, Field::SmtpPort)?,
    ))
}

/// Validates the details and logs in to both servers, without saving
/// anything. Both servers are checked at the same time, each within
/// `options.timeout`. If both fail, the IMAP error is returned.
pub async fn check_account(
    config: &ManualAccountConfig,
    password: &Secret,
    options: &SetupOptions,
) -> Result<(), AddAccountError> {
    let mut errors = validate(config).err().unwrap_or_default();
    if let Some(problem) = password_problem(password) {
        errors.push(FieldError {
            field: Field::Password,
            problem,
        });
    }
    if !errors.is_empty() {
        return Err(AddAccountError::Invalid(errors));
    }
    let (imap_port, smtp_port) = ports(config)?;
    let password = std::str::from_utf8(password.expose()).map_err(|_| {
        AddAccountError::Invalid(vec![FieldError {
            field: Field::Password,
            problem: FieldProblem::InvalidCharacters,
        }])
    })?;
    let login = LoginWith::Password(password);
    let (imap, smtp) = tokio::join!(
        check_imap(&config.imap, imap_port, &config.username, login, options),
        check_smtp(&config.smtp, smtp_port, &config.username, login, options),
    );
    imap?;
    smtp
}

/// [`check_account`] for an account that signs in with OAuth (M3-02): both
/// servers with XOAUTH2 and `access_token`, without saving anything.
pub async fn check_oauth_account(
    config: &ManualAccountConfig,
    access_token: &str,
    options: &SetupOptions,
) -> Result<(), AddAccountError> {
    validate(config).map_err(AddAccountError::Invalid)?;
    let (imap_port, smtp_port) = ports(config)?;
    let login = LoginWith::Token(access_token);
    let (imap, smtp) = tokio::join!(
        check_imap(&config.imap, imap_port, &config.username, login, options),
        check_smtp(&config.smtp, smtp_port, &config.username, login, options),
    );
    imap?;
    smtp
}

/// Saves a checked account that signs in with OAuth `provider` (such as
/// `"google"`): its details only. No password is stored; the caller keeps
/// the refresh token in secure storage and removes the account if that fails.
pub fn save_oauth_account(
    store: &Store,
    config: &ManualAccountConfig,
    provider: &str,
) -> Result<AccountId, AddAccountError> {
    let (imap_port, smtp_port) = ports(config)?;
    let account = NewAccount {
        display_name: config.display_name.trim().to_owned(),
        email: config.email.trim().to_owned(),
        imap_host: config.imap.host.trim().to_owned(),
        imap_port,
        imap_security: store_security(config.imap.security),
        smtp_host: config.smtp.host.trim().to_owned(),
        smtp_port,
        smtp_security: store_security(config.smtp.security),
        username: config.username.trim().to_owned(),
    };
    let id = store
        .insert_account(&account)
        .map_err(|_| AddAccountError::SaveFailed)?;
    match store.set_oauth_provider(id, Some(provider)) {
        Ok(true) => Ok(id),
        _ => {
            let _ = store.delete_account(id);
            Err(AddAccountError::SaveFailed)
        }
    }
}

fn store_security(security: Security) -> mailcrane_store::Security {
    match security {
        Security::Tls => mailcrane_store::Security::Tls,
        Security::StartTls => mailcrane_store::Security::Starttls,
    }
}

/// Saves an account that [`check_account`] accepted: the details to the
/// store, the password only to `secrets`. If saving the password fails, the
/// account row is removed again, so nothing is left behind.
pub fn save_account(
    store: &Store,
    secrets: &dyn SecretStore,
    config: &ManualAccountConfig,
    password: &Secret,
) -> Result<AccountId, AddAccountError> {
    validate(config).map_err(AddAccountError::Invalid)?;
    let (imap_port, smtp_port) = ports(config)?;
    let account = NewAccount {
        display_name: config.display_name.clone(),
        email: config.email.clone(),
        imap_host: config.imap.host.clone(),
        imap_port,
        imap_security: store_security(config.imap.security),
        smtp_host: config.smtp.host.clone(),
        smtp_port,
        smtp_security: store_security(config.smtp.security),
        username: config.username.clone(),
    };
    // Checked again here, right before saving, whichever way the caller came.
    ensure_not_added(
        store,
        IncomingServer::Imap(&account.imap_host),
        &account.username,
        &account.email,
    )?;
    let id = store
        .insert_account(&account)
        .map_err(|_| AddAccountError::SaveFailed)?;
    if secrets
        .put(&SecretKey::account_password(id.0), password)
        .is_err()
    {
        if store.delete_account(id).is_err() {
            tracing::warn!(
                account = id.0,
                "could not remove account after secret store failure"
            );
        }
        return Err(AddAccountError::SaveFailed);
    }
    Ok(id)
}

/// Adds an account from manually typed details, with the default options:
/// validates, checks the IMAP login and SMTP authentication, then saves.
/// On any failure nothing is saved.
///
/// The future borrows `store`, which is not `Sync`; callers that need a
/// `Send` future (such as the FFI) call [`check_account`] and then
/// [`save_account`] themselves.
pub async fn add_account_manual(
    store: &Store,
    secrets: &dyn SecretStore,
    config: &ManualAccountConfig,
    password: Secret,
) -> Result<AccountId, AddAccountError> {
    add_account_manual_with(store, secrets, config, password, &SetupOptions::default()).await
}

/// [`add_account_manual`] with explicit options.
pub async fn add_account_manual_with(
    store: &Store,
    secrets: &dyn SecretStore,
    config: &ManualAccountConfig,
    password: Secret,
    options: &SetupOptions,
) -> Result<AccountId, AddAccountError> {
    // Before any login, so a duplicate is refused without contacting a server.
    ensure_not_added(
        store,
        IncomingServer::Imap(config.imap.host.trim()),
        config.username.trim(),
        config.email.trim(),
    )?;
    check_account(config, &password, options).await?;
    save_account(store, secrets, config, &password)
}

#[cfg(test)]
mod tests;
