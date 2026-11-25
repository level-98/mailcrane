//! Accounts, sync scheduling and the public API of the Mailcrane engine.
//!
//! Changes reach the apps as [`events::CoreEvent`]s; the apps do not poll.

#![forbid(unsafe_code)]

pub mod account;
pub mod actions;
pub mod attachments;
pub mod compose;
pub mod contacts;
pub mod drafts;
pub mod events;
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod links;
pub mod notify;
pub mod oauth;
pub mod outbox;
pub mod phishing;
pub mod rules;
pub mod sasl;
pub mod search;
pub mod secrets;
pub mod snooze;
pub mod sync;
pub mod threading;
pub mod unsubscribe;

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// The version of the Mailcrane core.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_package() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert!(!version().is_empty());
    }

    #[test]
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-core");
    }
}
