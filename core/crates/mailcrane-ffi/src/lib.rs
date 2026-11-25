//! The only API the apps see; the UniFFI boundary.

// `deny`, not `forbid`: UniFFI's generated scaffolding contains `unsafe` code
// and allows it locally, which `forbid` would reject. `deny` still rejects any
// `unsafe` in hand-written code here.
#![deny(unsafe_code)]

uniffi::setup_scaffolding!();

pub mod account;
#[cfg(test)]
mod accounts_tests;
pub mod actions;
#[cfg(target_os = "android")]
mod android_tls;
pub mod attachments;
pub mod autoconfig;
pub mod compose;
pub mod contacts;
pub mod drafts;
pub mod folders;
pub mod links;
pub mod messages;
pub mod oauth;
pub mod outbox;
pub mod rules;
pub mod search;
pub mod secrets;
pub mod settings;
pub mod snooze;
pub mod sync;
pub mod threads;
pub mod unsubscribe;
pub mod widget;
mod worker;

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
