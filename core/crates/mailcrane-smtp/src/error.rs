//! Errors from sending, classified from reply codes (RFC 5321 §4.2.1).

use std::fmt;

/// What went wrong while talking to an SMTP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtpError {
    /// The server could not be reached, closed the connection, or sent
    /// something that is not SMTP.
    CannotConnect,
    /// The TLS handshake failed, including an untrusted certificate.
    TlsFailure,
    /// STARTTLS was required but the server does not offer it. Nothing was
    /// sent in clear text.
    StartTlsNotOffered,
    /// The server refused the username and password.
    AuthFailed,
    /// Every recipient was refused.
    RecipientsRejected { rejected: Vec<String> },
    /// A 4xx reply: try again later.
    Temporary { code: u16, message: String },
    /// A 5xx reply: do not retry unchanged.
    Permanent { code: u16, message: String },
    /// A step took longer than its time limit.
    Timeout,
    /// An address or credential contains characters that are not allowed.
    /// Nothing was sent.
    InvalidInput,
}

impl fmt::Display for SmtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Recipients and server text are deliberately left out of this
        // message so it is safe to log.
        match self {
            Self::CannotConnect => f.write_str("cannot connect to the SMTP server"),
            Self::TlsFailure => f.write_str("TLS failure"),
            Self::StartTlsNotOffered => f.write_str("the server does not offer STARTTLS"),
            Self::AuthFailed => f.write_str("authentication failed"),
            Self::RecipientsRejected { .. } => f.write_str("all recipients were rejected"),
            Self::Temporary { code, .. } => write!(f, "temporary failure ({code})"),
            Self::Permanent { code, .. } => write!(f, "permanent failure ({code})"),
            Self::Timeout => f.write_str("timed out"),
            Self::InvalidInput => f.write_str("invalid address or credential"),
        }
    }
}

impl std::error::Error for SmtpError {}

/// Turns an unexpected reply into the matching error.
pub(crate) fn classify(code: u16, text: &str) -> SmtpError {
    let message: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
    match code {
        400..=499 => SmtpError::Temporary { code, message },
        _ => SmtpError::Permanent { code, message },
    }
}
