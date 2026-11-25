//! IMAP client: connecting over TLS, logging in, listing folders, syncing
//! and waiting for changes with IDLE.
//!
//! Connections always use TLS: implicit TLS (RFC 8314, port 993) or
//! STARTTLS (RFC 9051 §6.2.1, port 143). There is no plain-text mode and no
//! way to skip certificate checks, and credentials are only sent after TLS
//! is up. Every response is read within size limits and time limits, so a
//! hostile server gets an error, never a panic or a hang.
//!
//! ```no_run
//! # async fn run(store: &mailcrane_store::Store, account: mailcrane_store::AccountId)
//! #     -> Result<(), mailcrane_imap::ImapError> {
//! use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret, Session};
//!
//! let config = ImapConfig::new("imap.example.org", 993, ImapSecurity::ImplicitTls);
//! let mut session = Session::connect(&config, "alice@example.org", &Secret::new("pw")).await?;
//! let folders = session.list_folders().await?;
//! save_folders(store, account, &folders)?;
//! session.logout().await;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod append;
mod b64;
mod command;
mod condstore;
mod connection;
mod error;
mod fetch;
mod idle;
pub mod mutf7;
pub mod parser;
mod quirks;
mod roles;
mod session;
mod store_cmds;
mod tls;

pub use append::{AppendFlags, SearchKey};
pub use condstore::{FetchedFlags, FlagUpdate, QresyncParams, QresyncSelect};
pub use error::ImapError;
pub use fetch::{
    parse_internal_date, FetchedHeaders, SelectInfo, PREFETCH_MAX_OCTETS, SEARCH_WINDOW,
};
pub use idle::IdleOutcome;
pub use parser::{parse_response, Envelope, EnvelopeAddress, FetchData, Limits};
pub use quirks::{detect as detect_quirks, ServerQuirk};
pub use session::{save_folders, ImapConfig, ImapFolder, ImapSecurity, Session};
pub use store_cmds::{ChangeError, SystemFlag};

use std::fmt;

/// A password or token. Never shown in `Debug` output.
#[derive(Clone, PartialEq, Eq)]
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
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-imap");
    }

    #[test]
    fn secret_is_hidden_in_debug() {
        assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(***)");
    }
}
