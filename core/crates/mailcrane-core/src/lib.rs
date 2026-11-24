//! Accounts, sync scheduling and the public API of the Mailcrane engine.
//!
//! Changes reach the apps as [`events::CoreEvent`]s; the apps do not poll.

#![forbid(unsafe_code)]

pub mod events;
pub mod secrets;
pub mod sync;

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
