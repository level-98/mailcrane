//! The only API the apps see; the UniFFI boundary.

// UniFFI's generated scaffolding contains `unsafe` code, so `forbid` and
// `deny` cannot be used here. Hand-written code in this crate must still
// contain no `unsafe`.
#![allow(unsafe_code)]

uniffi::setup_scaffolding!();

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// A greeting from the core; proves the Rust to FFI to app chain works.
#[uniffi::export]
pub fn hello() -> String {
    format!("Hello from Mailcrane core {}", mailcrane_core::version())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-ffi");
    }

    #[test]
    fn hello_includes_core_version() {
        assert_eq!(
            hello(),
            format!("Hello from Mailcrane core {}", mailcrane_core::version())
        );
    }
}
