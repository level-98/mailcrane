//! Accounts, sync scheduling and the public API of the Mailcrane engine.

#![forbid(unsafe_code)]

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-core");
    }
}
