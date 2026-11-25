//! Writing and sending a new plain-text message.
//!
//! The apps hand over a [`ComposeDraft`] with recipients exactly as typed.
//! [`validate_recipients`] checks them; [`send`] builds the message with
//! `mailcrane-mime` and sends it straight away over SMTP. M2-06 replaces the
//! direct send with the outbox.
//!
//! Sending is split into [`prepare_send`] (reads the account and password,
//! builds the message; no network) and [`send_prepared`] (talks to the
//! server), so callers that need a `Send` future don't hold the `Store`
//! across an await.

use std::collections::HashSet;
use std::time::Duration;

use mailcrane_mime::{
    build_message, parse_address_list, Address, AddressProblem, BuildError, OutgoingMessage,
    TransferEncoding,
};
use mailcrane_smtp::{Envelope, SmtpAuth, SmtpConfig, SmtpError, SmtpSecurity};
use mailcrane_store::{AccountId, Security, Store};

use crate::secrets::{SecretKey, SecretStore};

pub(crate) mod reply;
pub mod signature;

pub use reply::{prepare_reply, ReplyError, ReplyKind};

/// A message the user is writing. Each recipient entry is one chip or field
/// value as typed, such as `Ann <ann@example.org>`; an entry may also hold
/// several addresses separated by commas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeDraft {
    /// The account to send from.
    pub account_id: AccountId,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    /// For a reply: the Message-ID replied to, without angle brackets.
    pub in_reply_to: Option<String>,
    /// For a reply: the thread's Message-IDs, oldest first.
    pub references: Vec<String>,
}

/// Which recipient field an entry is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

/// An entry that is not a valid address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientError {
    pub field: RecipientField,
    /// Position of the entry in that field's list.
    pub entry: usize,
    pub problem: AddressProblem,
}

/// The checked recipients of a draft.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recipients {
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
}

impl Recipients {
    /// Every address once, for the SMTP envelope. Duplicates are compared
    /// ignoring case.
    fn envelope(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.to
            .iter()
            .chain(&self.cc)
            .chain(&self.bcc)
            .filter(|a| seen.insert(a.email.to_ascii_lowercase()))
            .map(|a| a.email.clone())
            .collect()
    }
}

/// Why a message was not sent, or not to everyone. In every case except
/// [`SendError::RecipientsRejected`]'s partial form (see [`SendReport`]),
/// nothing was sent and the draft is unchanged.
///
/// None of these carry server text or message content, so they are safe to
/// log, apart from the rejected addresses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// Some entries are not valid addresses.
    #[error("some recipients are invalid")]
    InvalidRecipients(Vec<RecipientError>),
    /// To, Cc and Bcc are all empty.
    #[error("no recipients")]
    NoRecipients,
    /// More recipients than one message may have.
    #[error("too many recipients")]
    TooManyRecipients,
    /// The message is too large.
    #[error("message too large")]
    TooLarge,
    /// The account doesn't exist (any more).
    #[error("account not found")]
    AccountNotFound,
    /// The account's own address can't be used as a sender.
    #[error("the account's address is invalid")]
    InvalidSender,
    /// No password is stored for the account, or it can't be read.
    #[error("no stored credentials")]
    MissingCredentials,
    /// The local database failed.
    #[error("storage failure")]
    Storage,
    /// The server could not be reached.
    #[error("cannot connect to the SMTP server")]
    CannotConnect,
    /// TLS failed, including an untrusted certificate.
    #[error("TLS failure")]
    TlsFailure,
    /// STARTTLS was required but not offered. Nothing was sent in clear.
    #[error("the server does not offer STARTTLS")]
    StartTlsNotOffered,
    /// The server refused the login.
    #[error("authentication failed")]
    AuthFailed,
    /// The server took too long.
    #[error("timed out")]
    Timeout,
    /// The server refused every recipient.
    #[error("all recipients were rejected")]
    RecipientsRejected { rejected: Vec<String> },
    /// The server refused the message. `temporary` means trying again later
    /// may work.
    #[error("the server refused the message")]
    Refused { temporary: bool },
    /// Something inside the app failed (for example no random numbers).
    #[error("internal failure")]
    Internal,
}

/// The result of a send that reached at least one recipient.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SendReport {
    /// Recipients the server refused; the message went to everyone else.
    pub rejected: Vec<String>,
}

/// Options for sending. The defaults are what the apps use.
#[derive(Debug, Clone)]
pub struct SendOptions {
    /// Time allowed to connect, and for each command.
    pub timeout: Duration,
    /// Extra trusted root certificates (DER), for tests against the
    /// testkit's servers. This only adds trust; it can't turn checks off.
    pub extra_root_certs: Vec<Vec<u8>>,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            extra_root_certs: Vec::new(),
        }
    }
}

/// Checks every recipient entry. Returns the parsed addresses, or every
/// invalid entry, or [`SendError::NoRecipients`] when all fields are empty.
pub fn validate_recipients(draft: &ComposeDraft) -> Result<Recipients, SendError> {
    let mut errors = Vec::new();
    let mut parse = |field, entries: &[String]| {
        let mut out = Vec::new();
        for (entry, text) in entries.iter().enumerate() {
            match parse_address_list(text) {
                Ok(list) => out.extend(list),
                Err(bad) => errors.extend(bad.into_iter().map(|b| RecipientError {
                    field,
                    entry,
                    problem: b.problem,
                })),
            }
        }
        out
    };
    let recipients = Recipients {
        to: parse(RecipientField::To, &draft.to),
        cc: parse(RecipientField::Cc, &draft.cc),
        bcc: parse(RecipientField::Bcc, &draft.bcc),
    };
    if !errors.is_empty() {
        // One error per entry is enough to mark it.
        errors.dedup_by_key(|e| (e.field, e.entry));
        return Err(SendError::InvalidRecipients(errors));
    }
    if recipients.to.is_empty() && recipients.cc.is_empty() && recipients.bcc.is_empty() {
        return Err(SendError::NoRecipients);
    }
    Ok(recipients)
}

/// A built message and where to send it. Holds the password; dropped (and
/// not logged: `Debug` hides it) once sent.
#[derive(Debug)]
pub struct PreparedSend {
    config: SmtpConfig,
    auth: SmtpAuth,
    envelope: Envelope,
    message: Vec<u8>,
}

/// A message built for sending, without credentials: what the outbox keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltMessage {
    pub envelope: Envelope,
    /// The exact bytes to send; never a Bcc header.
    pub raw: Vec<u8>,
    /// Its Message-ID, without angle brackets.
    pub message_id: String,
}

/// Validates the draft, reads the account and builds the message with a
/// fresh Message-ID. Nothing goes over the network; no password is read.
pub fn build_for_sending(store: &Store, draft: &ComposeDraft) -> Result<BuiltMessage, SendError> {
    let recipients = validate_recipients(draft)?;
    let account = store
        .get_account(draft.account_id)
        .map_err(|_| SendError::Storage)?
        .ok_or(SendError::AccountNotFound)?
        .details;
    let from = Address {
        name: Some(account.display_name.trim().to_owned()).filter(|n| !n.is_empty()),
        email: account.email.clone(),
    };
    let envelope = Envelope {
        from: account.email.clone(),
        to: recipients.envelope(),
    };
    let domain = account
        .email
        .rsplit_once('@')
        .map(|(_, d)| d)
        .unwrap_or_default();
    let message_id = mailcrane_mime::new_message_id(domain).map_err(|_| SendError::Internal)?;
    let raw = build_message(&OutgoingMessage {
        from,
        to: recipients.to,
        cc: recipients.cc,
        bcc: recipients.bcc,
        subject: draft.subject.clone(),
        body_text: draft.body_text.clone(),
        // The SMTP client doesn't negotiate 8BITMIME yet, so stay 7-bit.
        encoding: TransferEncoding::QuotedPrintable,
        in_reply_to: draft.in_reply_to.clone(),
        references: draft.references.clone(),
        message_id: Some(message_id.clone()),
        is_draft: false,
    })
    .map_err(|e| match e {
        BuildError::InvalidAddress { .. } => SendError::InvalidSender,
        BuildError::NoRecipients => SendError::NoRecipients,
        BuildError::TooManyRecipients => SendError::TooManyRecipients,
        BuildError::TooLarge => SendError::TooLarge,
        BuildError::Random => SendError::Internal,
    })?;
    Ok(BuiltMessage {
        envelope,
        raw,
        message_id,
    })
}

/// The account's SMTP server and credentials (the password from the secret
/// store, never the database).
pub(crate) fn smtp_settings(
    store: &Store,
    secrets: &dyn SecretStore,
    account_id: AccountId,
    options: &SendOptions,
) -> Result<(SmtpConfig, SmtpAuth), SendError> {
    let account = store
        .get_account(account_id)
        .map_err(|_| SendError::Storage)?
        .ok_or(SendError::AccountNotFound)?
        .details;
    let password = secrets
        .get(&SecretKey::account_password(account_id.0))
        .map_err(|_| SendError::MissingCredentials)?
        .ok_or(SendError::MissingCredentials)?;
    let password =
        std::str::from_utf8(password.expose()).map_err(|_| SendError::MissingCredentials)?;
    let security = match account.smtp_security {
        Security::Tls => SmtpSecurity::ImplicitTls,
        Security::Starttls => SmtpSecurity::StartTls,
    };
    let mut config = SmtpConfig::new(account.smtp_host, account.smtp_port, security);
    config.connect_timeout = options.timeout;
    config.command_timeout = options.timeout;
    config.extra_root_certs = options.extra_root_certs.clone();
    let auth = SmtpAuth::Password {
        username: account.username,
        password: mailcrane_smtp::Secret::new(password),
    };
    Ok((config, auth))
}

/// A built message with its server and credentials, ready to send.
pub(crate) fn prepared(
    config: SmtpConfig,
    auth: SmtpAuth,
    envelope: Envelope,
    message: Vec<u8>,
) -> PreparedSend {
    PreparedSend {
        config,
        auth,
        envelope,
        message,
    }
}

/// Validates the draft, reads the account and its password, and builds the
/// message. Nothing goes over the network.
pub fn prepare_send(
    store: &Store,
    secrets: &dyn SecretStore,
    draft: &ComposeDraft,
    options: &SendOptions,
) -> Result<PreparedSend, SendError> {
    let built = build_for_sending(store, draft)?;
    let (config, auth) = smtp_settings(store, secrets, draft.account_id, options)?;
    Ok(prepared(config, auth, built.envelope, built.raw))
}

/// Sends a prepared message.
pub async fn send_prepared(prepared: PreparedSend) -> Result<SendReport, SendError> {
    let report = mailcrane_smtp::send(
        &prepared.config,
        &prepared.auth,
        &prepared.envelope,
        &prepared.message,
    )
    .await
    .map_err(map_smtp)?;
    if !report.rejected.is_empty() {
        tracing::info!(
            rejected = report.rejected.len(),
            "sent, some recipients refused"
        );
    }
    Ok(SendReport {
        rejected: report.rejected,
    })
}

/// Validates, builds and sends the draft now. On any error nothing was
/// sent, except that [`SendReport::rejected`] lists recipients the server
/// refused while accepting the others.
///
/// The future borrows `store`, which is not `Sync`; callers that need a
/// `Send` future call [`prepare_send`] and [`send_prepared`] themselves.
pub async fn send(
    store: &Store,
    secrets: &dyn SecretStore,
    draft: &ComposeDraft,
    options: &SendOptions,
) -> Result<SendReport, SendError> {
    let prepared = prepare_send(store, secrets, draft, options)?;
    send_prepared(prepared).await
}

pub(crate) fn map_smtp(err: SmtpError) -> SendError {
    match err {
        SmtpError::CannotConnect => SendError::CannotConnect,
        SmtpError::TlsFailure => SendError::TlsFailure,
        SmtpError::StartTlsNotOffered => SendError::StartTlsNotOffered,
        SmtpError::AuthFailed => SendError::AuthFailed,
        SmtpError::Timeout => SendError::Timeout,
        SmtpError::RecipientsRejected { rejected } => SendError::RecipientsRejected { rejected },
        SmtpError::Temporary { .. } => SendError::Refused { temporary: true },
        SmtpError::Permanent { .. } => SendError::Refused { temporary: false },
        // Addresses were checked by `validate_recipients`, so this means the
        // account's own address or username is unusable.
        SmtpError::InvalidInput => SendError::InvalidSender,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(to: &[&str], cc: &[&str], bcc: &[&str]) -> ComposeDraft {
        let v = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect();
        ComposeDraft {
            account_id: AccountId(1),
            to: v(to),
            cc: v(cc),
            bcc: v(bcc),
            subject: "Hi".into(),
            body_text: "Body".into(),
            in_reply_to: None,
            references: vec![],
        }
    }

    #[test]
    fn validate_reports_each_bad_entry_with_its_field() {
        let d = draft(
            &["Ann <ann@example.org>", "nope"],
            &["x@example.org, y@"],
            &["ok@example.net"],
        );
        assert_eq!(
            validate_recipients(&d),
            Err(SendError::InvalidRecipients(vec![
                RecipientError {
                    field: RecipientField::To,
                    entry: 1,
                    problem: AddressProblem::MissingAt
                },
                RecipientError {
                    field: RecipientField::Cc,
                    entry: 0,
                    problem: AddressProblem::InvalidDomain
                },
            ]))
        );
    }

    #[test]
    fn validate_needs_a_recipient() {
        assert_eq!(
            validate_recipients(&draft(&[], &[" "], &[])),
            Err(SendError::NoRecipients)
        );
        let ok = validate_recipients(&draft(&[], &[], &["b@example.org"])).expect("valid");
        assert_eq!(ok.bcc.len(), 1);
    }

    #[test]
    fn envelope_has_each_address_once() {
        let d = draft(
            &["a@example.org, B@example.org"],
            &["b@example.org"],
            &["c@example.org", "A@EXAMPLE.ORG"],
        );
        let r = validate_recipients(&d).expect("valid");
        assert_eq!(
            r.envelope(),
            vec!["a@example.org", "B@example.org", "c@example.org"]
        );
    }
}
