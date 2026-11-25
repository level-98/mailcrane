//! Errors from talking to an IMAP server.
//!
//! The messages are safe to log: they never include server text, which can
//! echo credentials back, or anything the user typed.

use mailcrane_store::StoreError;

/// What went wrong while talking to an IMAP server.
#[derive(Debug, thiserror::Error)]
pub enum ImapError {
    /// A host name or credential contains characters that are not allowed.
    /// Nothing was sent.
    #[error("invalid host name or credential")]
    InvalidInput,
    /// The server could not be reached.
    #[error("cannot connect to the IMAP server")]
    CannotConnect,
    /// The TLS handshake failed, including an untrusted certificate.
    #[error("TLS failure")]
    TlsFailure,
    /// STARTTLS was required but the server does not offer it, or refused
    /// it. Nothing else was sent in clear text.
    #[error("the server does not offer STARTTLS")]
    StartTlsNotOffered,
    /// The server advertises `LOGINDISABLED` even over TLS (RFC 9051 §7.2.2),
    /// and offers no mechanism we support.
    #[error("the server does not allow login")]
    LoginDisabled,
    /// The server refused the username and password.
    #[error("authentication failed")]
    AuthFailed,
    /// The server answered a command with `NO` or `BAD`.
    #[error("the server refused the command")]
    CommandFailed,
    /// The server said `BYE` and is closing the connection.
    #[error("the server closed the session")]
    ServerBye,
    /// The connection closed before a complete response arrived.
    #[error("the connection closed unexpectedly")]
    ConnectionClosed,
    /// The server sent something that is not valid IMAP.
    #[error("malformed response from the server")]
    Protocol,
    /// A response line is longer than the limit.
    #[error("response line too long")]
    LineTooLong,
    /// A literal or a whole response is larger than the limit.
    #[error("response too large")]
    ResponseTooLarge,
    /// A step took longer than its time limit.
    #[error("timed out")]
    Timeout,
    /// Saving to the store failed. Nothing was saved.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

impl ImapError {
    /// The variant's name, e.g. `"CommandFailed"`. Safe to log: a fixed
    /// string that never carries server text or store details.
    pub fn name(&self) -> &'static str {
        match self {
            ImapError::InvalidInput => "InvalidInput",
            ImapError::CannotConnect => "CannotConnect",
            ImapError::TlsFailure => "TlsFailure",
            ImapError::StartTlsNotOffered => "StartTlsNotOffered",
            ImapError::LoginDisabled => "LoginDisabled",
            ImapError::AuthFailed => "AuthFailed",
            ImapError::CommandFailed => "CommandFailed",
            ImapError::ServerBye => "ServerBye",
            ImapError::ConnectionClosed => "ConnectionClosed",
            ImapError::Protocol => "Protocol",
            ImapError::LineTooLong => "LineTooLong",
            ImapError::ResponseTooLarge => "ResponseTooLarge",
            ImapError::Timeout => "Timeout",
            ImapError::Store(_) => "Store",
        }
    }
}
