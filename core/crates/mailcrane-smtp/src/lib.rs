//! Sending messages over SMTP.
//!
//! Connections always use TLS: implicit TLS (RFC 8314, port 465) or STARTTLS
//! (RFC 3207, port 587). There is no plain-text mode and no way to skip
//! certificate checks. Credentials are only sent after TLS is up.
//!
//! This is a small client on `tokio-rustls` rather than `lettre`, because
//! `lettre` fails the whole send when any one recipient is refused, and we
//! must send to the accepted ones and report the rest.

#![forbid(unsafe_code)]

mod client;
mod data;
mod envelope;
mod error;

pub use client::{
    check_login, future_release_limit, send, send_with_hold, SendReport, SmtpAuth, SmtpConfig,
    SmtpSecurity,
};
pub use envelope::Envelope;
pub use error::SmtpError;

use std::fmt;

/// A password or token. Never shown in `Debug` output, and wiped from
/// memory when dropped (#521). `Clone` stays for callers that keep a login
/// to send again; each copy is wiped on drop too.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(zeroize::Zeroizing<String>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(zeroize::Zeroizing::new(value.into()))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests;
