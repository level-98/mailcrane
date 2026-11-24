//! Fake IMAP, SMTP and JMAP servers for tests, able to misbehave on purpose.

#![forbid(unsafe_code)]

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-testkit");
    }
}
