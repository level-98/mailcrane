//! Writing and sending a new message (M2-02), as the apps see it.
//!
//! The apps keep recipients as chips. [`parse_recipients`] turns typed text
//! into chips, [`validate_recipients`] checks a draft, and [`send`] builds
//! and sends it. All address and message rules live in the core.

use mailcrane_core::compose::{self as core, PreparedSend};
use mailcrane_core::secrets::SecretStore;
use mailcrane_mime::AddressProblem;
use mailcrane_store::{AccountId, Store};

use crate::secrets;
use crate::sync::{SyncErrorKind, SyncFailure};

/// An account the user can send from.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountSummary {
    pub id: i64,
    pub email: String,
    /// Empty when the user gave no name.
    pub display_name: String,
}

/// Lists the accounts in the database, oldest first, for the From picker.
#[uniffi::export]
pub fn list_accounts(db_path: String) -> Result<Vec<AccountSummary>, SyncFailure> {
    let storage = || SyncFailure::Failed {
        kind: SyncErrorKind::Storage,
    };
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let accounts = store.list_accounts().map_err(|_| storage())?;
    Ok(accounts
        .into_iter()
        .map(|a| AccountSummary {
            id: a.id.0,
            email: a.details.email,
            display_name: a.details.display_name,
        })
        .collect())
}

/// What is wrong with a typed recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RecipientProblem {
    MissingAt,
    InvalidLocalPart,
    InvalidDomain,
    ControlCharacter,
    NonAscii,
    Malformed,
    TooLong,
    TooMany,
}

impl From<AddressProblem> for RecipientProblem {
    fn from(p: AddressProblem) -> Self {
        match p {
            AddressProblem::MissingAt => Self::MissingAt,
            AddressProblem::InvalidLocalPart => Self::InvalidLocalPart,
            AddressProblem::InvalidDomain => Self::InvalidDomain,
            AddressProblem::ControlCharacter => Self::ControlCharacter,
            AddressProblem::NonAscii => Self::NonAscii,
            AddressProblem::Malformed => Self::Malformed,
            AddressProblem::TooLong => Self::TooLong,
            AddressProblem::TooMany => Self::TooMany,
        }
    }
}

/// One typed recipient, ready to show as a chip.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecipientEntry {
    /// The entry as typed, trimmed. Put this back in the draft unchanged.
    pub text: String,
    /// Display name; empty when there is none or the entry is invalid.
    pub name: String,
    /// The address; empty when the entry is invalid.
    pub email: String,
    /// `None` when the entry is a valid address.
    pub problem: Option<RecipientProblem>,
}

/// Typed text split into finished entries and what is still being typed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecipientInput {
    pub entries: Vec<RecipientEntry>,
    /// Text after the last comma or semicolon. Empty when `whole` was set.
    pub rest: String,
}

/// Splits typed recipient text at commas and semicolons (not those inside
/// quotes or `<…>`). With `whole` set, the text after the last separator is
/// an entry too; use that when the field loses focus or before sending.
#[uniffi::export]
pub fn parse_recipients(text: String, whole: bool) -> RecipientInput {
    let split = mailcrane_mime::split_address_list(&text, whole);
    let entries = split
        .entries
        .into_iter()
        .map(|e| match e.result {
            Ok(a) => RecipientEntry {
                text: e.text,
                name: a.name.unwrap_or_default(),
                email: a.email,
                problem: None,
            },
            Err(p) => RecipientEntry {
                text: e.text,
                name: String::new(),
                email: String::new(),
                problem: Some(p.into()),
            },
        })
        .collect();
    RecipientInput {
        entries,
        rest: split.rest,
    }
}

/// A new message. Recipients are entries as `parse_recipients` returned
/// them (their `text`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ComposeDraft {
    pub account_id: i64,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
}

impl From<ComposeDraft> for core::ComposeDraft {
    fn from(d: ComposeDraft) -> Self {
        Self {
            account_id: AccountId(d.account_id),
            to: d.to,
            cc: d.cc,
            bcc: d.bcc,
            subject: d.subject,
            body_text: d.body_text,
        }
    }
}

/// Which recipient field an entry is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

impl From<core::RecipientField> for RecipientField {
    fn from(f: core::RecipientField) -> Self {
        match f {
            core::RecipientField::To => Self::To,
            core::RecipientField::Cc => Self::Cc,
            core::RecipientField::Bcc => Self::Bcc,
        }
    }
}

/// An entry of the draft that is not a valid address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RecipientError {
    pub field: RecipientField,
    /// Position of the entry in that field's list.
    pub entry: u32,
    pub problem: RecipientProblem,
}

impl From<core::RecipientError> for RecipientError {
    fn from(e: core::RecipientError) -> Self {
        Self {
            field: e.field.into(),
            entry: u32::try_from(e.entry).unwrap_or(u32::MAX),
            problem: e.problem.into(),
        }
    }
}

/// Checks every recipient of the draft. An empty list means none is
/// invalid; whether there is at least one is checked by `send`.
#[uniffi::export]
pub fn validate_recipients(draft: ComposeDraft) -> Vec<RecipientError> {
    match core::validate_recipients(&draft.into()) {
        Err(core::SendError::InvalidRecipients(errors)) => {
            errors.into_iter().map(Into::into).collect()
        }
        _ => Vec::new(),
    }
}

/// The message was sent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SendResult {
    /// Recipients the server refused; everyone else got the message.
    pub rejected: Vec<String>,
}

/// Why the message was not sent. Nothing was sent in any of these cases.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SendFailure {
    #[error("some recipients are invalid")]
    InvalidRecipients { errors: Vec<RecipientError> },
    #[error("no recipients")]
    NoRecipients,
    #[error("too many recipients")]
    TooManyRecipients,
    #[error("message too large")]
    TooLarge,
    #[error("account not found")]
    AccountNotFound,
    #[error("the account's address is invalid")]
    InvalidSender,
    #[error("no stored credentials")]
    MissingCredentials,
    #[error("storage failure")]
    Storage,
    #[error("cannot connect to the server")]
    CannotConnect,
    #[error("TLS failure")]
    TlsFailure,
    #[error("the server does not offer STARTTLS")]
    StartTlsNotOffered,
    #[error("authentication failed")]
    AuthFailed,
    #[error("timed out")]
    Timeout,
    #[error("all recipients were rejected")]
    RecipientsRejected { rejected: Vec<String> },
    #[error("the server refused the message")]
    Refused { temporary: bool },
    #[error("internal failure")]
    Internal,
    /// The app has not called `install_secret_store` yet.
    #[error("the core is not ready")]
    NotReady,
}

impl From<core::SendError> for SendFailure {
    fn from(e: core::SendError) -> Self {
        use core::SendError as E;
        match e {
            E::InvalidRecipients(errors) => Self::InvalidRecipients {
                errors: errors.into_iter().map(Into::into).collect(),
            },
            E::NoRecipients => Self::NoRecipients,
            E::TooManyRecipients => Self::TooManyRecipients,
            E::TooLarge => Self::TooLarge,
            E::AccountNotFound => Self::AccountNotFound,
            E::InvalidSender => Self::InvalidSender,
            E::MissingCredentials => Self::MissingCredentials,
            E::Storage => Self::Storage,
            E::CannotConnect => Self::CannotConnect,
            E::TlsFailure => Self::TlsFailure,
            E::StartTlsNotOffered => Self::StartTlsNotOffered,
            E::AuthFailed => Self::AuthFailed,
            E::Timeout => Self::Timeout,
            E::RecipientsRejected { rejected } => Self::RecipientsRejected { rejected },
            E::Refused { temporary } => Self::Refused { temporary },
            E::Internal => Self::Internal,
        }
    }
}

/// Validates the draft, builds the message and sends it now from the
/// draft's account in the database at `db_path`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn send(db_path: String, draft: ComposeDraft) -> Result<SendResult, SendFailure> {
    let secrets = secrets::secret_store().map_err(|_| SendFailure::NotReady)?;
    send_with(
        &db_path,
        secrets.as_ref(),
        draft,
        &core::SendOptions::default(),
    )
    .await
}

async fn send_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    draft: ComposeDraft,
    options: &core::SendOptions,
) -> Result<SendResult, SendFailure> {
    let prepared = prepare(db_path, secrets, draft, options)?;
    let report = core::send_prepared(prepared).await?;
    Ok(SendResult {
        rejected: report.rejected,
    })
}

/// In its own function so the `Store` (not `Sync`) is dropped before the
/// send awaits, keeping the future `Send`.
fn prepare(
    db_path: &str,
    secrets: &dyn SecretStore,
    draft: ComposeDraft,
    options: &core::SendOptions,
) -> Result<PreparedSend, SendFailure> {
    let draft: core::ComposeDraft = draft.into();
    // Bad recipients are reported before touching the database.
    core::validate_recipients(&draft)?;
    let store = Store::open(db_path).map_err(|_| SendFailure::Storage)?;
    Ok(core::prepare_send(&store, secrets, &draft, options)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_core::secrets::{Secret, SecretKey};
    use mailcrane_store::{NewAccount, Security};

    fn draft(account_id: i64) -> ComposeDraft {
        ComposeDraft {
            account_id,
            to: vec!["Bob <bob@example.net>".into()],
            cc: vec![],
            bcc: vec!["carol@example.com".into()],
            subject: "Hi".into(),
            body_text: "Hello".into(),
            in_reply_to: None,
            references: vec![],
        }
    }

    #[test]
    fn parse_recipients_makes_chips_and_keeps_the_rest() {
        let got = parse_recipients("Ann <ann@example.org>, nope; \"Doe, J".into(), false);
        assert_eq!(
            got.entries,
            vec![
                RecipientEntry {
                    text: "Ann <ann@example.org>".into(),
                    name: "Ann".into(),
                    email: "ann@example.org".into(),
                    problem: None,
                },
                RecipientEntry {
                    text: "nope".into(),
                    name: String::new(),
                    email: String::new(),
                    problem: Some(RecipientProblem::MissingAt),
                },
            ]
        );
        assert_eq!(got.rest, "\"Doe, J");
        let whole = parse_recipients("x@example.org".into(), true);
        assert_eq!(whole.entries.len(), 1);
        assert!(whole.rest.is_empty());
    }

    #[test]
    fn validate_marks_bad_entries() {
        let mut d = draft(1);
        d.cc = vec!["ok@example.org".into(), "bad@".into()];
        assert_eq!(
            validate_recipients(d),
            vec![RecipientError {
                field: RecipientField::Cc,
                entry: 1,
                problem: RecipientProblem::InvalidDomain
            }]
        );
        assert!(validate_recipients(draft(1)).is_empty());
    }

    #[tokio::test]
    async fn sends_end_to_end_and_lists_accounts() {
        let smtp = mailcrane_testkit::FakeSmtp::builder()
            .start()
            .await
            .expect("fake smtp");
        let secrets = mailcrane_testkit::MemorySecretStore::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("mailcrane.sqlite");
        let db_path = db_path.to_str().expect("utf-8 path").to_owned();
        let id = {
            let store = Store::open(&db_path).expect("open");
            store
                .insert_account(&NewAccount {
                    display_name: "Ann".into(),
                    email: "ann@example.org".into(),
                    imap_host: "localhost".into(),
                    imap_port: 993,
                    imap_security: Security::Tls,
                    smtp_host: "localhost".into(),
                    smtp_port: smtp.addr().port(),
                    smtp_security: Security::Tls,
                    username: "ann@example.org".into(),
                })
                .expect("account")
        };
        secrets
            .put(
                &SecretKey::account_password(id.0),
                &Secret::from_string("s3cret-value".into()),
            )
            .expect("password");

        assert_eq!(
            list_accounts(db_path.clone()).expect("list"),
            vec![AccountSummary {
                id: id.0,
                email: "ann@example.org".into(),
                display_name: "Ann".into(),
            }]
        );

        let options = core::SendOptions {
            timeout: std::time::Duration::from_secs(5),
            extra_root_certs: vec![smtp.root_cert_der().to_vec()],
        };
        let mut bad = draft(id.0);
        bad.to = vec!["nope".into()];
        assert!(matches!(
            send_with(&db_path, &secrets, bad, &options).await,
            Err(SendFailure::InvalidRecipients { .. })
        ));

        let sent = send_with(&db_path, &secrets, draft(id.0), &options)
            .await
            .expect("sent");
        assert!(sent.rejected.is_empty());
        let msgs = smtp.sent().await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].envelope_to,
            vec!["bob@example.net", "carol@example.com"]
        );
    }

    #[test]
    fn send_errors_map_one_to_one() {
        assert_eq!(
            SendFailure::from(core::SendError::Refused { temporary: true }),
            SendFailure::Refused { temporary: true }
        );
        assert_eq!(
            SendFailure::from(core::SendError::MissingCredentials),
            SendFailure::MissingCredentials
        );
    }
}
