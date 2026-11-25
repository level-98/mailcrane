//! JMAP errors. None of them carries a password, token or message content.

/// Why a JMAP call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JmapError {
    /// The URL isn't a usable `https:` URL.
    #[error("invalid URL")]
    InvalidUrl,
    /// The connection failed.
    #[error("network failure")]
    Network,
    /// The certificate didn't check out.
    #[error("TLS failure")]
    Tls,
    /// No answer in time.
    #[error("timed out")]
    Timeout,
    /// The server refused the credentials (HTTP 401 or 403).
    #[error("authentication failed")]
    Unauthorized,
    /// Any other unexpected HTTP status.
    #[error("HTTP status {0}")]
    Http(u16),
    /// The response is larger than allowed.
    #[error("response too large")]
    TooLarge,
    /// The response isn't the JSON we expect.
    #[error("malformed response")]
    Malformed,
    /// The server isn't a JMAP server, or offers no mail account.
    #[error("not a JMAP mail server")]
    NotJmap,
    /// A method call failed with this JMAP error type (RFC 8620 §3.6.2),
    /// such as `cannotCalculateChanges` or `invalidArguments`.
    #[error("method error: {0}")]
    Method(String),
}
