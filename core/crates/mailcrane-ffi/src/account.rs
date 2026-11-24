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
