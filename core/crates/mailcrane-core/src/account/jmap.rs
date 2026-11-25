//! Adding a JMAP account (M3-04): find and check its session, then save it.
//! Like IMAP setup, nothing is saved unless the server accepted the login.

use super::setup::{
    email_problem, password_problem, AddAccountError, Field, FieldError, FieldProblem, Protocol,
};
use crate::secrets::{Secret, SecretKey, SecretStore};
use mailcrane_jmap::{Auth, ClientConfig, JmapError};
use mailcrane_store::{AccountId, NewAccount, Security, Store};
use std::time::Duration;

/// What the user typed for a JMAP account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmapAccountConfig {
    pub email: String,
    pub display_name: String,
    /// A domain (`example.org`, for `https://example.org/.well-known/jmap`,
    /// RFC 8620 §2.2) or the session URL itself.
    pub server: String,
    pub username: String,
}

/// Options for the check; tests add their own root certificate.
#[derive(Debug, Clone)]
pub struct JmapSetupOptions {
    pub timeout: Duration,
    pub extra_roots: Vec<Vec<u8>>,
}

impl Default for JmapSetupOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            extra_roots: Vec::new(),
        }
    }
}

/// Checks the form, then fetches the session with the credentials (which
/// proves the login works and the server offers mail). Returns the session
/// URL to save. Nothing is stored.
pub async fn check_jmap_account(
    config: &JmapAccountConfig,
    password: &Secret,
    options: &JmapSetupOptions,
) -> Result<String, AddAccountError> {
    let mut errors = Vec::new();
    if let Some(problem) = email_problem(&config.email) {
        errors.push(FieldError {
            field: Field::Email,
            problem,
        });
    }
    let server = config.server.trim();
    let url = mailcrane_jmap::session::session_url(server);
    if server.is_empty() {
        errors.push(FieldError {
            field: Field::ImapHost,
            problem: FieldProblem::Empty,
        });
    } else if url.is_err() {
        errors.push(FieldError {
            field: Field::ImapHost,
            problem: FieldProblem::InvalidCharacters,
        });
    }
    if config.username.trim().is_empty() {
        errors.push(FieldError {
            field: Field::Username,
            problem: FieldProblem::Empty,
        });
    }
    if let Some(problem) = password_problem(password) {
        errors.push(FieldError {
            field: Field::Password,
            problem,
        });
    }
    let password_text = std::str::from_utf8(password.expose()).map_err(|_| {
        AddAccountError::Invalid(vec![FieldError {
            field: Field::Password,
            problem: FieldProblem::InvalidCharacters,
        }])
    })?;
    if !errors.is_empty() {
        return Err(AddAccountError::Invalid(errors));
    }
    let url = url.map_err(|_| AddAccountError::CannotConnect {
        server: Protocol::Imap,
    })?;
    let auth = Auth::Basic {
        username: config.username.trim().to_owned(),
        password: password_text.to_owned(),
    };
    let client = ClientConfig {
        timeout: options.timeout,
        extra_roots: options.extra_roots.clone(),
    };
    mailcrane_jmap::discover(url.as_str(), &auth, &client)
        .await
        .map_err(map_error)?;
    Ok(url.to_string())
}

/// Saves a checked account: its details to the store, the password only to
/// `secrets`. If saving the password fails, the account row is removed.
pub fn save_jmap_account(
    store: &Store,
    secrets: &dyn SecretStore,
    config: &JmapAccountConfig,
    session_url: &str,
    password: &Secret,
) -> Result<AccountId, AddAccountError> {
    // A JMAP account's IMAP/SMTP columns are unused (migration 0014).
    let account = NewAccount {
        display_name: config.display_name.trim().to_owned(),
        email: config.email.trim().to_owned(),
        imap_host: String::new(),
        imap_port: 0,
        imap_security: Security::Tls,
        smtp_host: String::new(),
        smtp_port: 0,
        smtp_security: Security::Tls,
        username: config.username.trim().to_owned(),
    };
    let id = store
        .insert_jmap_account(&account, session_url)
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

/// JMAP errors as setup errors. The "incoming server" stands for the JMAP
/// server, which does both.
fn map_error(e: JmapError) -> AddAccountError {
    let server = Protocol::Imap;
    match e {
        JmapError::Unauthorized => AddAccountError::AuthFailed { server },
        JmapError::Tls => AddAccountError::TlsFailure { server },
        JmapError::Timeout => AddAccountError::Timeout { server },
        _ => AddAccountError::CannotConnect { server },
    }
}
