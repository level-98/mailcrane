//! Server-side rules (M4-23): a ManageSieve client (RFC 5804) and a small
//! Sieve script writer (RFC 5228) for Mailcrane's rules.
//!
//! The client only ever talks over TLS: it connects in clear text on port
//! 4190, requires `STARTTLS` (RFC 5804 §2.2) and refuses to go on without
//! it. It logs in with `AUTHENTICATE "PLAIN"` (RFC 4616) and nothing else.
//! Every server response goes through [`parse_response`], which has size
//! limits and never panics. See ADR 0004.

#![forbid(unsafe_code)]

mod error;
mod parser;
pub mod script;
mod session;
mod tls;

pub use error::SieveError;
pub use parser::{
    parse_response, parse_response_with, Limits, ParseError, Response, Status, Token,
};
pub use session::{Capabilities, ScriptInfo, Session, SieveConfig, DEFAULT_PORT};

use std::fmt;

/// A password, kept out of `Debug` output.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
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
mod tests {
    use super::*;

    #[test]
    fn secrets_are_not_printed() {
        assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(***)");
    }
}
