//! Errors from talking to a ManageSieve server.
//!
//! The messages are safe to log: they never include server text, scripts,
//! or anything the user typed.

/// What went wrong while talking to a ManageSieve server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SieveError {
    /// A host name, credential, script name or script contains characters
    /// that are not allowed. Nothing was sent.
    #[error("invalid input")]
    InvalidInput,
    /// The server could not be reached.
    #[error("cannot connect to the ManageSieve server")]
    CannotConnect,
    /// The TLS handshake failed, including an untrusted certificate.
    #[error("TLS failure")]
    TlsFailure,
    /// The server does not offer STARTTLS, or refused it. Nothing else was
    /// sent in clear text.
    #[error("the server does not offer STARTTLS")]
    StartTlsNotOffered,
    /// The server offers no SASL mechanism we support (only PLAIN).
    #[error("the server offers no supported login mechanism")]
    NoSupportedMechanism,
    /// The server refused the username and password.
    #[error("authentication failed")]
    AuthFailed,
    /// The server answered a command with `NO`.
    #[error("the server refused the command")]
    CommandFailed,
    /// `CHECKSCRIPT` or `PUTSCRIPT` refused the script as invalid.
    #[error("the server refused the script")]
    ScriptRejected,
    /// The script doesn't fit in the user's quota (RFC 5804 §2.3,
    /// `QUOTA` response codes).
    #[error("not enough space on the server")]
    QuotaExceeded,
    /// The server said `BYE` and is closing the connection.
    #[error("the server closed the session")]
    ServerBye,
    /// The connection closed before a complete response arrived.
    #[error("the connection closed unexpectedly")]
    ConnectionClosed,
    /// The server sent something that is not valid ManageSieve.
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
}

impl SieveError {
    /// The variant's name, e.g. `"AuthFailed"`. Safe to log.
    pub fn name(&self) -> &'static str {
        match self {
            SieveError::InvalidInput => "InvalidInput",
            SieveError::CannotConnect => "CannotConnect",
            SieveError::TlsFailure => "TlsFailure",
            SieveError::StartTlsNotOffered => "StartTlsNotOffered",
            SieveError::NoSupportedMechanism => "NoSupportedMechanism",
            SieveError::AuthFailed => "AuthFailed",
            SieveError::CommandFailed => "CommandFailed",
            SieveError::ScriptRejected => "ScriptRejected",
            SieveError::QuotaExceeded => "QuotaExceeded",
            SieveError::ServerBye => "ServerBye",
            SieveError::ConnectionClosed => "ConnectionClosed",
            SieveError::Protocol => "Protocol",
            SieveError::LineTooLong => "LineTooLong",
            SieveError::ResponseTooLarge => "ResponseTooLarge",
            SieveError::Timeout => "Timeout",
        }
    }

    /// Whether the server itself is missing or unusable for server rules
    /// (as opposed to a passing network problem). Such a result is cached
    /// so the app doesn't try again on every sync.
    pub fn means_unavailable(&self) -> bool {
        matches!(
            self,
            SieveError::CannotConnect
                | SieveError::TlsFailure
                | SieveError::StartTlsNotOffered
                | SieveError::NoSupportedMechanism
                | SieveError::AuthFailed
                | SieveError::Protocol
        )
    }
}
