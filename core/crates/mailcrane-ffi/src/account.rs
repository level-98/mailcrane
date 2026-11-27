//! Adding an account by hand (M1-03), as the apps see it.
//!
//! These types mirror `mailcrane_core::account::setup`. There is deliberately
//! no plain-text security option here either.

use mailcrane_core::account::setup as core;
use mailcrane_core::secrets::{Secret, SecretStore};

use mailcrane_store::Store;

use crate::secrets;

/// How a connection is protected. There is no plain-text option.
///
/// Not called `Security` (Apple's framework) or `Protocol` (an Objective-C
/// runtime class), so the Swift bindings don't clash with system names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionSecurity {
    /// TLS from the first byte.
    Tls,
    /// Upgrade with STARTTLS; fails if the server does not offer it.
    StartTls,
}

/// Which server a setting or an error belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MailServer {
    Imap,
    Smtp,
}

/// One server's address and security.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerConfig {
    pub host: String,
    /// Checked to be 1–65535 by the core.
    pub port: u32,
    pub security: ConnectionSecurity,
}

/// Everything the user types to add an account, except the password.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ManualAccountConfig {
    pub email: String,
    pub display_name: String,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    pub username: String,
}

/// A form field that can be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FieldProblem {
    Empty,
    ContainsSpace,
    HasScheme,
    InvalidCharacters,
    PortOutOfRange,
    MalformedEmail,
}

/// One field-specific error, for showing under that field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FieldError {
    pub field: Field,
    pub problem: FieldProblem,
}

/// Why the account was not added. In every case nothing was saved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum AddAccountError {
    #[error("some fields are invalid")]
    Invalid { errors: Vec<FieldError> },
    #[error("cannot connect to the server")]
    CannotConnect { server: MailServer },
    #[error("TLS failure")]
    TlsFailure { server: MailServer },
    #[error("the server does not offer STARTTLS")]
    StartTlsNotOffered { server: MailServer },
    #[error("the server refused the login")]
    AuthFailed { server: MailServer },
    #[error("the server timed out")]
    Timeout { server: MailServer },
    #[error("the account could not be saved")]
    SaveFailed,
    /// The same account is already in the app (#346). Nothing was saved.
    #[error("this account is already added")]
    AlreadyAdded,
    /// The app has not called `install_secret_store` yet. A bug in the app,
    /// not something the user can fix.
    #[error("the core is not ready")]
    NotReady,
}

impl From<ConnectionSecurity> for core::Security {
    fn from(value: ConnectionSecurity) -> Self {
        match value {
            ConnectionSecurity::Tls => core::Security::Tls,
            ConnectionSecurity::StartTls => core::Security::StartTls,
        }
    }
}

impl From<MailServer> for core::Protocol {
    fn from(value: MailServer) -> Self {
        match value {
            MailServer::Imap => core::Protocol::Imap,
            MailServer::Smtp => core::Protocol::Smtp,
        }
    }
}

impl From<core::Protocol> for MailServer {
    fn from(value: core::Protocol) -> Self {
        match value {
            core::Protocol::Imap => MailServer::Imap,
            core::Protocol::Smtp => MailServer::Smtp,
        }
    }
}

impl From<ServerConfig> for core::ServerConfig {
    fn from(value: ServerConfig) -> Self {
        Self {
            host: value.host,
            port: value.port,
            security: value.security.into(),
        }
    }
}

impl From<ManualAccountConfig> for core::ManualAccountConfig {
    fn from(value: ManualAccountConfig) -> Self {
        Self {
            email: value.email,
            display_name: value.display_name,
            imap: value.imap.into(),
            smtp: value.smtp.into(),
            username: value.username,
        }
    }
}

impl From<core::FieldError> for FieldError {
    fn from(value: core::FieldError) -> Self {
        let field = match value.field {
            core::Field::Email => Field::Email,
            core::Field::DisplayName => Field::DisplayName,
            core::Field::ImapHost => Field::ImapHost,
            core::Field::ImapPort => Field::ImapPort,
            core::Field::SmtpHost => Field::SmtpHost,
            core::Field::SmtpPort => Field::SmtpPort,
            core::Field::Username => Field::Username,
            core::Field::Password => Field::Password,
        };
        let problem = match value.problem {
            core::FieldProblem::Empty => FieldProblem::Empty,
            core::FieldProblem::ContainsSpace => FieldProblem::ContainsSpace,
            core::FieldProblem::HasScheme => FieldProblem::HasScheme,
            core::FieldProblem::InvalidCharacters => FieldProblem::InvalidCharacters,
            core::FieldProblem::PortOutOfRange => FieldProblem::PortOutOfRange,
            core::FieldProblem::MalformedEmail => FieldProblem::MalformedEmail,
        };
        Self { field, problem }
    }
}

impl From<core::AddAccountError> for AddAccountError {
    fn from(value: core::AddAccountError) -> Self {
        use core::AddAccountError as E;
        match value {
            E::Invalid(errors) => Self::Invalid {
                errors: errors.into_iter().map(Into::into).collect(),
            },
            E::CannotConnect { server } => Self::CannotConnect {
                server: server.into(),
            },
            E::TlsFailure { server } => Self::TlsFailure {
                server: server.into(),
            },
            E::StartTlsNotOffered { server } => Self::StartTlsNotOffered {
                server: server.into(),
            },
            E::AuthFailed { server } => Self::AuthFailed {
                server: server.into(),
            },
            E::Timeout { server } => Self::Timeout {
                server: server.into(),
            },
            E::SaveFailed => Self::SaveFailed,
            E::AlreadyAdded => Self::AlreadyAdded,
        }
    }
}

/// The usual port for a protocol and security choice: IMAP 993/143, SMTP
/// 465/587. The apps fill the port field with this when security changes.
#[uniffi::export]
pub fn default_port(server: MailServer, security: ConnectionSecurity) -> u16 {
    core::default_port(server.into(), security.into())
}

/// Checks the form without contacting any server. An empty list means every
/// field is fine. The password is checked by `add_account_manual`.
#[uniffi::export]
pub fn validate_manual_account(config: ManualAccountConfig) -> Vec<FieldError> {
    match core::validate(&config.into()) {
        Ok(()) => Vec::new(),
        Err(errors) => errors.into_iter().map(Into::into).collect(),
    }
}

/// Validates the details, logs in to the IMAP server and checks SMTP
/// authentication (each within 30 seconds), and only then saves the account
/// to the database at `db_path` and the password to the app's secure storage.
/// Returns the new account's id. On any failure nothing is saved.
///
/// `db_path` is the same database file the app gives `SyncEngine`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn add_account_manual(
    db_path: String,
    config: ManualAccountConfig,
    password: String,
) -> Result<i64, AddAccountError> {
    crate::worker::on_worker_thread(move || async move {
        // Fail before contacting a server if the app isn't set up.
        let secrets = secrets::secret_store().map_err(|_| AddAccountError::NotReady)?;
        add_with(
            &db_path,
            secrets.as_ref(),
            config,
            password,
            &core::SetupOptions::default(),
        )
        .await
    })
    .await
    .unwrap_or(Err(AddAccountError::NotReady))
}

/// What the user typed for a JMAP account (M3-04).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct JmapAccountSetup {
    pub email: String,
    pub display_name: String,
    /// A domain, or the JMAP session URL.
    pub server: String,
    pub username: String,
}

impl From<JmapAccountSetup> for mailcrane_core::account::jmap::JmapAccountConfig {
    fn from(c: JmapAccountSetup) -> Self {
        Self {
            email: c.email,
            display_name: c.display_name,
            server: c.server,
            username: c.username,
        }
    }
}

/// Finds the JMAP session for the account (RFC 8620 §2.2) and logs in to
/// check the details, then saves the account to the database at `db_path`
/// and the password to secure storage. Returns the new account's id. On any
/// failure nothing is saved. Errors use the same types as manual IMAP
/// setup; `server` is always the incoming server.
#[uniffi::export(async_runtime = "tokio")]
pub async fn add_account_jmap(
    db_path: String,
    setup: JmapAccountSetup,
    password: String,
) -> Result<i64, AddAccountError> {
    crate::worker::on_worker_thread(move || async move {
        let secrets = secrets::secret_store().map_err(|_| AddAccountError::NotReady)?;
        add_jmap_with(
            &db_path,
            secrets.as_ref(),
            setup,
            password,
            &mailcrane_core::account::jmap::JmapSetupOptions::default(),
        )
        .await
    })
    .await
    .unwrap_or(Err(AddAccountError::NotReady))
}

async fn add_jmap_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    setup: JmapAccountSetup,
    password: String,
    options: &mailcrane_core::account::jmap::JmapSetupOptions,
) -> Result<i64, AddAccountError> {
    use mailcrane_core::account::jmap;
    let config: jmap::JmapAccountConfig = setup.into();
    let password = Secret::from_string(password);
    let session_url = jmap::check_jmap_account(&config, &password, options).await?;
    let store = Store::open(db_path).map_err(|_| AddAccountError::SaveFailed)?;
    let id = jmap::save_jmap_account(&store, secrets, &config, &session_url, &password)?;
    Ok(id.0)
}

/// Adds a Google account (M3-02) after "Sign in with Google": logs in to
/// Gmail's IMAP and SMTP with the sign-in's access token (XOAUTH2), saves
/// the account, and keeps the refresh token in secure storage. Returns the
/// new account's id; on any failure nothing is saved. `sign_in_id` is what
/// `oauth_complete` returned.
#[uniffi::export(async_runtime = "tokio")]
pub async fn add_account_google(
    db_path: String,
    sign_in_id: String,
    email: String,
    display_name: String,
) -> Result<i64, AddAccountError> {
    crate::worker::on_worker_thread(move || async move {
        let secrets = secrets::secret_store().map_err(|_| AddAccountError::NotReady)?;
        add_google_with(
            &db_path,
            secrets.as_ref(),
            &sign_in_id,
            &email,
            &display_name,
            &core::SetupOptions::default(),
        )
        .await
    })
    .await
    .unwrap_or(Err(AddAccountError::NotReady))
}

pub(crate) async fn add_google_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    sign_in_id: &str,
    email: &str,
    display_name: &str,
    options: &core::SetupOptions,
) -> Result<i64, AddAccountError> {
    use mailcrane_core::oauth::providers::google;
    let config = google::account_config(email, display_name);
    add_oauth_with(db_path, secrets, sign_in_id, &config, google::ID, options).await
}

/// Adds an account for a finished OAuth sign-in with `config`'s servers.
async fn add_oauth_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    sign_in_id: &str,
    config: &core::ManualAccountConfig,
    provider: &str,
    options: &core::SetupOptions,
) -> Result<i64, AddAccountError> {
    // A sign-in that expired or was never finished: sign in again.
    let token =
        crate::oauth::finished_access_token(sign_in_id).ok_or(AddAccountError::AuthFailed {
            server: MailServer::Imap,
        })?;
    // A duplicate (#346) is refused before Google's servers are contacted.
    // The store is dropped before the await: it isn't `Sync`.
    {
        let store = Store::open(db_path).map_err(|_| AddAccountError::SaveFailed)?;
        core::ensure_not_added(
            &store,
            core::IncomingServer::Imap(config.imap.host.trim()),
            config.username.trim(),
            config.email.trim(),
        )?;
    }
    core::check_oauth_account(config, &token, options).await?;
    let store = Store::open(db_path).map_err(|_| AddAccountError::SaveFailed)?;
    let id = core::save_oauth_account(&store, config, provider)?;
    if crate::oauth::attach_with(secrets, &Store::scope_of(db_path), sign_in_id, id.0).is_err() {
        let _ = store.delete_account(id);
        return Err(AddAccountError::SaveFailed);
    }
    Ok(id.0)
}

async fn add_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    config: ManualAccountConfig,
    password: String,
    options: &core::SetupOptions,
) -> Result<i64, AddAccountError> {
    let config: core::ManualAccountConfig = config.into();
    let password = Secret::from_string(password);
    // A duplicate is refused before any server is contacted (#346). The
    // store is dropped before the await: it isn't `Sync`.
    {
        let store = Store::open(db_path).map_err(|_| AddAccountError::SaveFailed)?;
        core::ensure_not_added(
            &store,
            core::IncomingServer::Imap(config.imap.host.trim()),
            config.username.trim(),
            config.email.trim(),
        )?;
    }
    core::check_account(&config, &password, options).await?;
    // Opened only after the checks, so the future holds no `Store` (which is
    // not `Sync`) across an await.
    let store = Store::open(db_path).map_err(|_| AddAccountError::SaveFailed)?;
    let id = core::save_account(&store, secrets, &config, &password)?;
    Ok(id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ManualAccountConfig {
        ManualAccountConfig {
            email: "alice@example.org".into(),
            display_name: String::new(),
            imap: ServerConfig {
                host: "imap.example.org".into(),
                port: 993,
                security: ConnectionSecurity::Tls,
            },
            smtp: ServerConfig {
                host: "smtp.example.org".into(),
                port: 587,
                security: ConnectionSecurity::StartTls,
            },
            username: "alice@example.org".into(),
        }
    }

    #[test]
    fn default_ports_cross_the_boundary() {
        assert_eq!(default_port(MailServer::Imap, ConnectionSecurity::Tls), 993);
        assert_eq!(
            default_port(MailServer::Imap, ConnectionSecurity::StartTls),
            143
        );
        assert_eq!(default_port(MailServer::Smtp, ConnectionSecurity::Tls), 465);
        assert_eq!(
            default_port(MailServer::Smtp, ConnectionSecurity::StartTls),
            587
        );
    }

    #[test]
    fn validation_errors_cross_the_boundary() {
        assert!(validate_manual_account(config()).is_empty());
        let mut bad = config();
        bad.imap.host = "imaps://imap.example.org".into();
        bad.smtp.port = 0;
        assert_eq!(
            validate_manual_account(bad),
            vec![
                FieldError {
                    field: Field::ImapHost,
                    problem: FieldProblem::HasScheme
                },
                FieldError {
                    field: Field::SmtpPort,
                    problem: FieldProblem::PortOutOfRange
                },
            ]
        );
    }

    #[test]
    fn core_errors_map_one_to_one() {
        assert_eq!(
            AddAccountError::from(core::AddAccountError::StartTlsNotOffered {
                server: core::Protocol::Smtp
            }),
            AddAccountError::StartTlsNotOffered {
                server: MailServer::Smtp
            }
        );
        assert_eq!(
            AddAccountError::from(core::AddAccountError::SaveFailed),
            AddAccountError::SaveFailed
        );
    }

    #[tokio::test]
    async fn adds_a_jmap_account_end_to_end() {
        use mailcrane_testkit::FakeJmap;
        let server = FakeJmap::builder()
            .user("ana@example.org", "app-password")
            .start()
            .await
            .expect("fake jmap");
        let secrets = mailcrane_testkit::MemorySecretStore::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("mailcrane.sqlite");
        let db_path = db_path.to_str().expect("utf-8 path");
        let options = mailcrane_core::account::jmap::JmapSetupOptions {
            timeout: std::time::Duration::from_secs(10),
            extra_roots: vec![server.root_cert_der().to_vec()],
        };
        let setup = JmapAccountSetup {
            email: "ana@example.org".into(),
            display_name: "Ana".into(),
            server: server.base_url(),
            username: "ana@example.org".into(),
        };

        let wrong = add_jmap_with(db_path, &secrets, setup.clone(), "nope".into(), &options).await;
        assert_eq!(
            wrong,
            Err(AddAccountError::AuthFailed {
                server: MailServer::Imap
            })
        );
        let empty = add_jmap_with(
            db_path,
            &secrets,
            JmapAccountSetup {
                server: String::new(),
                ..setup.clone()
            },
            "x".into(),
            &options,
        )
        .await;
        assert!(matches!(empty, Err(AddAccountError::Invalid { .. })));

        let id = add_jmap_with(db_path, &secrets, setup, "app-password".into(), &options)
            .await
            .expect("account added");
        let store = Store::open(db_path).expect("open");
        assert_eq!(
            store.list_accounts().expect("list").len(),
            1,
            "failed attempts saved nothing"
        );
        assert!(matches!(
            store.account_protocol(mailcrane_store::AccountId(id)),
            Ok(Some(mailcrane_store::Protocol::Jmap { .. }))
        ));
        assert_eq!(secrets.keys(), vec![format!("account/{id}/password")]);
    }

    /// Signs in through the fake token endpoint; returns the sign-in id and
    /// its access token.
    async fn signed_in(server: &mailcrane_testkit::FakeTokenEndpoint) -> (String, String) {
        let provider = crate::oauth::OAuthProviderConfig {
            id: "google".into(),
            authorization_endpoint: "https://auth.example.org/authorize".into(),
            token_endpoint: server.token_url(),
            client_id: "client-1".into(),
            scopes: vec!["https://mail.google.com/".into()],
            redirect_uri: "app.mailcrane:/oauth2redirect".into(),
            extra_auth_params: vec![],
        };
        let start =
            crate::oauth::oauth_begin(provider, Some("ann@gmail.com".into())).expect("begin");
        let url = url::Url::parse(&start.url).expect("url");
        let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        let code = server.authorize(
            &q["code_challenge"],
            "app.mailcrane:/oauth2redirect",
            "client-1",
        );
        let redirect = format!(
            "app.mailcrane:/oauth2redirect?code={code}&state={}",
            q["state"]
        );
        let id = crate::oauth::complete_with(redirect, vec![server.root_cert_der().to_vec()])
            .await
            .expect("signed in");
        let token = crate::oauth::finished_access_token(&id).expect("token");
        (id, token)
    }

    #[tokio::test]
    async fn adds_an_oauth_account_end_to_end() {
        // M3-02: Sign in with Google, then both servers with XOAUTH2.
        use mailcrane_core::secrets::SecretKey;
        use mailcrane_testkit::{FakeImap, FakeSmtp, FakeTokenEndpoint};
        let tokens = FakeTokenEndpoint::start().await.expect("token endpoint");
        let (sign_in, token) = signed_in(&tokens).await;
        let imap = FakeImap::builder()
            .user("ann@gmail.com", "unused")
            .xoauth2_token(token.clone())
            .start()
            .await
            .expect("fake imap");
        let smtp = FakeSmtp::builder().start().await.expect("fake smtp");
        let secrets = mailcrane_testkit::MemorySecretStore::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("mailcrane.sqlite");
        let db_path = db_path.to_str().expect("utf-8 path");
        let mut config =
            mailcrane_core::oauth::providers::google::account_config("ann@gmail.com", "Ann");
        config.imap.host = "localhost".into();
        config.imap.port = u32::from(imap.addr().port());
        config.smtp.host = "localhost".into();
        config.smtp.port = u32::from(smtp.addr().port());
        let options = core::SetupOptions {
            timeout: std::time::Duration::from_secs(10),
            extra_root_certs: vec![imap.root_cert_der().to_vec(), smtp.root_cert_der().to_vec()],
        };

        // A sign-in that doesn't exist: nothing happens.
        assert_eq!(
            add_oauth_with(
                db_path,
                &secrets,
                "signin-nope",
                &config,
                "google",
                &options
            )
            .await,
            Err(AddAccountError::AuthFailed {
                server: MailServer::Imap
            })
        );

        let id = add_oauth_with(db_path, &secrets, &sign_in, &config, "google", &options)
            .await
            .expect("account added");
        let store = Store::open(db_path).expect("open");
        let account = mailcrane_store::AccountId(id);
        assert_eq!(
            store.oauth_provider(account).expect("read").as_deref(),
            Some("google")
        );
        // The refresh token is in secure storage; there is no password.
        assert_eq!(
            secrets.keys(),
            vec![SecretKey::oauth_refresh_token(id).as_str().to_owned()]
        );
        // Sync logs in with the cached access token.
        let credential =
            mailcrane_core::account::credentials::credential(&store, &secrets, account)
                .expect("credential");
        match credential {
            mailcrane_core::account::credentials::Credential::OAuth(t) => {
                assert_eq!(t.as_str(), token)
            }
            other => panic!("expected a token, got {other:?}"),
        }
        // The sign-in is used up.
        assert!(crate::oauth::finished_access_token(&sign_in).is_none());
    }

    #[tokio::test]
    async fn a_refused_token_saves_nothing() {
        use mailcrane_testkit::{FakeImap, FakeSmtp, FakeTokenEndpoint};
        let tokens = FakeTokenEndpoint::start().await.expect("token endpoint");
        let (sign_in, _) = signed_in(&tokens).await;
        let imap = FakeImap::builder()
            .user("ann@gmail.com", "unused")
            .xoauth2_token("some-other-token")
            .start()
            .await
            .expect("fake imap");
        let smtp = FakeSmtp::builder().start().await.expect("fake smtp");
        let secrets = mailcrane_testkit::MemorySecretStore::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("mailcrane.sqlite");
        let db_path = db_path.to_str().expect("utf-8 path");
        let mut config =
            mailcrane_core::oauth::providers::google::account_config("ann@gmail.com", "");
        config.imap.host = "localhost".into();
        config.imap.port = u32::from(imap.addr().port());
        config.smtp.host = "localhost".into();
        config.smtp.port = u32::from(smtp.addr().port());
        let options = core::SetupOptions {
            timeout: std::time::Duration::from_secs(10),
            extra_root_certs: vec![imap.root_cert_der().to_vec(), smtp.root_cert_der().to_vec()],
        };
        assert_eq!(
            add_oauth_with(db_path, &secrets, &sign_in, &config, "google", &options).await,
            Err(AddAccountError::AuthFailed {
                server: MailServer::Imap
            })
        );
        assert!(Store::open(db_path)
            .expect("open")
            .list_accounts()
            .expect("list")
            .is_empty());
        assert!(secrets.keys().is_empty());
    }

    #[tokio::test]
    async fn a_duplicate_is_refused_before_any_server_is_contacted() {
        // #346: the account is already in the database; its servers don't
        // exist, so reaching them would fail with CannotConnect instead.
        let secrets = mailcrane_testkit::MemorySecretStore::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let db = dir.path().join("mail.db").to_string_lossy().into_owned();
        let core_config: core::ManualAccountConfig = config().into();
        let store = Store::open(&db).expect("store");
        core::save_account(
            &store,
            &secrets,
            &core_config,
            &Secret::from_string("pw".into()),
        )
        .expect("first");
        drop(store);
        let got = add_with(
            &db,
            &secrets,
            config(),
            "pw".into(),
            &core::SetupOptions::default(),
        )
        .await;
        assert_eq!(got, Err(AddAccountError::AlreadyAdded));
    }

    #[tokio::test]
    async fn adds_an_account_end_to_end() {
        use mailcrane_testkit::{FakeImap, FakeSmtp};
        let imap = FakeImap::builder()
            .user("alice@example.org", "s3cret-value")
            .start()
            .await
            .expect("fake imap");
        let smtp = FakeSmtp::builder().start().await.expect("fake smtp");
        let secrets = mailcrane_testkit::MemorySecretStore::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("mailcrane.sqlite");
        let db_path = db_path.to_str().expect("utf-8 path");

        let mut config = config();
        config.imap = ServerConfig {
            host: "localhost".into(),
            port: u32::from(imap.starttls_addr().port()),
            security: ConnectionSecurity::StartTls,
        };
        config.smtp = ServerConfig {
            host: "localhost".into(),
            port: u32::from(smtp.addr().port()),
            security: ConnectionSecurity::Tls,
        };
        let options = core::SetupOptions {
            timeout: std::time::Duration::from_secs(10),
            extra_root_certs: vec![imap.root_cert_der().to_vec(), smtp.root_cert_der().to_vec()],
        };

        let wrong = add_with(db_path, &secrets, config.clone(), "wrong".into(), &options).await;
        assert_eq!(
            wrong,
            Err(AddAccountError::AuthFailed {
                server: MailServer::Imap
            })
        );

        let id = add_with(db_path, &secrets, config, "s3cret-value".into(), &options)
            .await
            .expect("account added");
        let accounts = Store::open(db_path)
            .expect("open")
            .list_accounts()
            .expect("list");
        assert_eq!(accounts.len(), 1, "the failed attempt saved nothing");
        assert_eq!(accounts[0].id.0, id);
        assert_eq!(secrets.keys(), vec![format!("account/{id}/password")]);
    }
}

/// Why an account couldn't be removed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RemoveAccountFailure {
    /// No such account.
    #[error("account not found")]
    NotFound,
    /// The secure storage or the database failed; nothing was removed from
    /// the database.
    #[error("storage failure")]
    Storage,
}

/// Removes an account (M4-01): its password and tokens from secure storage
/// first, then everything stored for it (folders, messages, drafts, outbox,
/// rules…, by the database's cascades). The app stops its sync jobs.
#[uniffi::export]
pub fn remove_account(db_path: String, account_id: i64) -> Result<(), RemoveAccountFailure> {
    let secrets = crate::secrets::secret_store().map_err(|_| RemoveAccountFailure::Storage)?;
    // Telling a JMAP server to stop pushing is network work: never on the
    // caller's stack (#462). If the thread doesn't start, nothing was
    // removed. (A panic part-way, which the code never does on purpose,
    // could leave it half done; it also comes back as `Storage`.)
    crate::worker::on_job_thread(|| remove_account_with(&db_path, secrets.as_ref(), account_id))
        .unwrap_or(Err(RemoveAccountFailure::Storage))
}

/// [`remove_account`] with the secret store given, so tests needn't install
/// the process-wide one (which parallel tests share).
pub(crate) fn remove_account_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    account_id: i64,
) -> Result<(), RemoveAccountFailure> {
    let store = mailcrane_store::Store::open(db_path).map_err(|_| RemoveAccountFailure::Storage)?;
    let id = mailcrane_store::AccountId(account_id);
    if store
        .get_account(id)
        .map_err(|_| RemoveAccountFailure::Storage)?
        .is_none()
    {
        return Err(RemoveAccountFailure::NotFound);
    }
    // While its password is still there: stop the server pushing to this
    // phone for an account it no longer has (M4-27). Best effort; a
    // subscription the server isn't told about expires there.
    destroy_push_subscription(&store, id, None);
    mailcrane_core::secrets::delete_account_secrets(secrets, account_id)
        .map_err(|_| RemoveAccountFailure::Storage)?;
    // An OAuth account's access token (M3-02) goes from memory too: this
    // database's only, never another's account with the same ID (#434).
    mailcrane_core::oauth::token_cache().forget(store.cache_scope(), account_id);
    match store.delete_account(id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(RemoveAccountFailure::NotFound),
        Err(_) => Err(RemoveAccountFailure::Storage),
    }
}

/// Destroys the account's JMAP push subscription on its server, if it has
/// one, within a few seconds; failures are only logged. `extra_root` is a
/// test server's certificate.
pub(crate) fn destroy_push_subscription(
    store: &mailcrane_store::Store,
    id: mailcrane_store::AccountId,
    extra_root: Option<Vec<u8>>,
) {
    let Ok(Some(_)) = store.push_subscription(id) else {
        return;
    };
    let Ok(Some(mailcrane_store::Protocol::Jmap { session_url })) = store.account_protocol(id)
    else {
        return;
    };
    let Ok(auth) = crate::sync::jmap_auth(store, id) else {
        return;
    };
    let config = mailcrane_jmap::ClientConfig {
        timeout: std::time::Duration::from_secs(5),
        extra_roots: extra_root.into_iter().collect(),
    };
    let Ok(runtime) = crate::sync::runtime() else {
        return;
    };
    let done = runtime.block_on(async {
        let session = mailcrane_jmap::discover(&session_url, &auth, &config).await?;
        mailcrane_core::sync::push::teardown(store, id, &session, &auth, &config).await
    });
    if let Err(e) = done {
        tracing::info!(account = id.0, kind = ?e.kind(), "push subscription left to expire");
    }
}
