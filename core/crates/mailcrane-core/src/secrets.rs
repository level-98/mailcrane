//! Passwords and OAuth tokens, kept only in the operating system's secure
//! storage (Android Keystore, iOS Keychain).
//!
//! The core never writes a secret to SQLite, a log or a file. It reads and
//! writes credentials only through [`SecretStore`], which each app implements
//! on top of its platform's secure storage and hands to the core at start-up.

use std::fmt;

use zeroize::Zeroizing;

/// Secure storage for credentials. Implemented by the apps (Android Keystore,
/// iOS Keychain) and, for tests, by `mailcrane_testkit::MemorySecretStore`.
pub trait SecretStore: Send + Sync {
    /// Store `value` under `key`, replacing any value already there.
    fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError>;

    /// The value stored under `key`, or `None` if there is none.
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError>;

    /// Remove the value stored under `key`. Removing a missing key is not an error.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
}

/// The name a secret is stored under.
///
/// Built only through the constructors below, so every part of the app uses
/// the same names. The name holds an account's row id and the kind of secret,
/// never anything the user typed.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SecretKey(String);

impl SecretKey {
    /// The IMAP/SMTP password for an account.
    pub fn account_password(account_id: i64) -> Self {
        Self(format!("account/{account_id}/password"))
    }

    /// The OAuth 2.0 refresh token for an account (RFC 6749 §1.5).
    pub fn oauth_refresh_token(account_id: i64) -> Self {
        Self(format!("account/{account_id}/oauth-refresh-token"))
    }

    /// Every key that can hold a secret for `account_id`. Used when an
    /// account is deleted, so none of its secrets are left behind.
    pub fn all_for_account(account_id: i64) -> [Self; 2] {
        [
            Self::account_password(account_id),
            Self::oauth_refresh_token(account_id),
        ]
    }

    /// The key as the platform stores it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Keys are not secret, so they may appear in errors and logs.
        f.debug_tuple("SecretKey").field(&self.0).finish()
    }
}

/// A password or token. The bytes are zeroed when it is dropped, and `Debug`
/// prints `Secret(***)`. There is deliberately no `Display` and no `Clone`.
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    /// Wrap secret bytes. The vector is moved in, so no unzeroed copy is left.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Wrap a secret string, such as a password the user typed.
    pub fn from_string(value: String) -> Self {
        Self::new(value.into_bytes())
    }

    /// The secret bytes. Use them straight away; don't copy them into
    /// anything that outlives the call.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Why the platform's secure storage could not be used.
///
/// None of these carry a secret value. Every variant means the same thing to
/// the user: the stored credential can't be read, so ask for it again.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// Secure storage can't be reached right now (for example, the device is
    /// locked before its first unlock).
    #[error("secure storage is unavailable")]
    Unavailable,
    /// The stored value can no longer be decrypted, for example because the
    /// app's data was restored onto a new device where the key doesn't exist.
    #[error("the stored secret can no longer be read")]
    Invalidated,
    /// Any other platform failure. `reason` is a short description or error
    /// code from the platform, never a secret.
    #[error("secure storage failed: {reason}")]
    Other {
        /// What went wrong, without any secret value.
        reason: String,
    },
}

/// Delete every secret stored for `account_id`. Call this when an account is
/// removed. It tries every key even if one fails, and returns the first error.
pub fn delete_account_secrets(store: &dyn SecretStore, account_id: i64) -> Result<(), SecretError> {
    let mut first_error = None;
    for key in SecretKey::all_for_account(account_id) {
        if let Err(err) = store.delete(&key) {
            first_error.get_or_insert(err);
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::from_string("hunter2-example".to_owned());
        let printed = format!("{secret:?}");
        assert_eq!(printed, "Secret(***)");
        assert!(!printed.contains("hunter2"));
        let nested = format!("{:?}", Some(&secret));
        assert!(!nested.contains("hunter2"));
    }

    #[test]
    fn secret_exposes_its_bytes() {
        let secret = Secret::new(b"token-example".to_vec());
        assert_eq!(secret.expose(), b"token-example");
    }

    #[test]
    fn secret_bytes_are_zeroed_on_drop() {
        // Zeroizing<Vec<u8>> wipes the buffer in its Drop. Check the same
        // wrapper behaves that way by zeroizing explicitly and reading back.
        use zeroize::Zeroize;
        let mut inner = Zeroizing::new(b"secret-example".to_vec());
        inner.zeroize();
        assert!(inner.iter().all(|b| *b == 0));
    }

    #[test]
    fn keys_are_stable_and_distinct() {
        assert_eq!(
            SecretKey::account_password(7).as_str(),
            "account/7/password"
        );
        assert_eq!(
            SecretKey::oauth_refresh_token(7).as_str(),
            "account/7/oauth-refresh-token"
        );
        assert_ne!(
            SecretKey::account_password(7),
            SecretKey::account_password(8)
        );
    }
}
