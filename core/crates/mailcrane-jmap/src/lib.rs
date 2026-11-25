//! JMAP client (RFC 8620 core, RFC 8621 mail), part 1 of M3-04: session
//! discovery, method calls and mailboxes.
//!
//! Everything goes over HTTPS with the platform's certificate checks, which
//! can't be turned off; tests may add one extra trusted root. Responses are
//! size-limited and parsed into plain types; malformed JSON is an error,
//! never a panic.

#![forbid(unsafe_code)]

mod error;
mod http;
pub mod mailbox;
pub mod request;
pub mod session;

pub use error::JmapError;
pub use http::{Auth, ClientConfig, MAX_RESPONSE_BYTES};
pub use mailbox::{get_mailboxes, JmapMailbox, MailboxRole};
pub use request::{call, parse_response, MethodCall, MethodResponse};
pub use session::{discover, Session};

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// The core capability (RFC 8620 §2).
pub const CAP_CORE: &str = "urn:ietf:params:jmap:core";
/// The mail capability (RFC 8621 §1.3.1).
pub const CAP_MAIL: &str = "urn:ietf:params:jmap:mail";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-jmap");
    }
}
