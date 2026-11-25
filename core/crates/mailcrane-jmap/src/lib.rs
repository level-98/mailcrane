//! JMAP client (RFC 8620 core, RFC 8621 mail), M3-04: session discovery,
//! method calls, mailboxes, emails, actions and sending.
//!
//! Everything goes over HTTPS with the platform's certificate checks, which
//! can't be turned off; tests may add one extra trusted root. Responses are
//! size-limited and parsed into plain types; malformed JSON is an error,
//! never a panic.

#![forbid(unsafe_code)]

pub mod actions;
pub mod email;
mod error;
mod http;
pub mod mailbox;
pub mod request;
pub mod session;
pub mod submission;

pub use actions::{apply, EmailAction, SetOutcome};
pub use email::{
    download_blob, email_changes, get_emails, query_mailbox, EmailChanges, EmailPage, JmapAddress,
    JmapEmail,
};
pub use error::JmapError;
pub use http::{Auth, ClientConfig, MAX_RESPONSE_BYTES};
pub use mailbox::{get_mailboxes, JmapMailbox, MailboxRole};
pub use request::{call, parse_response, MethodCall, MethodResponse};
pub use session::{discover, Session};
pub use submission::{
    get_identities, send_message, upload, Identity, Outgoing, Sent, CAP_SUBMISSION,
};

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
