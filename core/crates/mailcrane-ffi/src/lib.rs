//! The only API the apps see; the UniFFI boundary.

// M0-05 will allow the unsafe code that UniFFI generates.
#![deny(unsafe_code)]

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-ffi");
    }
}
