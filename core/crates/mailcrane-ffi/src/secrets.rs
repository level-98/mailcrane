//! The secure-storage interface the apps implement (Android Keystore, iOS
//! Keychain) and hand to the core at start-up.

use std::sync::{Arc, RwLock};

use mailcrane_core::secrets::{Secret, SecretError, SecretKey, SecretStore};

/// Secure storage, implemented in Kotlin and Swift.
///
/// Keys are plain names such as `account/1/password`. Values are the secret
/// bytes. Implementations must never log either value.
#[uniffi::export(with_foreign)]
pub trait PlatformSecretStore: Send + Sync {
    /// Store `value` under `key`, replacing any value already there.
    fn put(&self, key: String, value: Vec<u8>) -> Result<(), SecretStoreError>;
    /// The value under `key`, or `null`/`nil` if there is none.
    fn get(&self, key: String) -> Result<Option<Vec<u8>>, SecretStoreError>;
    /// Remove the value under `key`. A missing key is not an error.
    fn delete(&self, key: String) -> Result<(), SecretStoreError>;
}

/// Errors the platform store returns. They never carry a secret value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SecretStoreError {
    /// Secure storage can't be reached right now.
    #[error("secure storage is unavailable")]
    Unavailable,
    /// The stored value can no longer be decrypted (e.g. after a restore).
    #[error("the stored secret can no longer be read")]
    Invalidated,
    /// Any other failure. `reason` must not contain a secret.
    #[error("secure storage failed: {reason}")]
    Other {
        /// What went wrong.
        reason: String,
    },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for SecretStoreError {
    fn from(err: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Other {
            reason: format!("unexpected error from the app: {}", err.reason),
        }
    }
}

impl From<SecretStoreError> for SecretError {
    fn from(err: SecretStoreError) -> Self {
        match err {
            SecretStoreError::Unavailable => SecretError::Unavailable,
            SecretStoreError::Invalidated => SecretError::Invalidated,
            SecretStoreError::Other { reason } => SecretError::Other { reason },
        }
    }
}

/// Adapts the app's store to the core's [`SecretStore`] trait.
struct ForeignSecretStore(Arc<dyn PlatformSecretStore>);

impl SecretStore for ForeignSecretStore {
    fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError> {
        // The copy handed across the FFI is owned and freed by UniFFI; it is
        // not zeroed. The app side wipes its own buffers where it can.
        Ok(self
            .0
            .put(key.as_str().to_owned(), value.expose().to_vec())?)
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        Ok(self.0.get(key.as_str().to_owned())?.map(Secret::new))
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        Ok(self.0.delete(key.as_str().to_owned())?)
    }
}

static SECRET_STORE: RwLock<Option<Arc<dyn SecretStore>>> = RwLock::new(None);

/// Give the core the app's secure storage. Call once at start-up, before any
/// account is used. Calling it again replaces the store.
#[uniffi::export]
pub fn install_secret_store(store: Arc<dyn PlatformSecretStore>) {
    let adapted: Arc<dyn SecretStore> = Arc::new(ForeignSecretStore(store));
    match SECRET_STORE.write() {
        Ok(mut slot) => *slot = Some(adapted),
        // A poisoned lock only means another thread panicked while holding
        // it; the slot itself is still a valid Option, so replace it.
        Err(poisoned) => *poisoned.into_inner() = Some(adapted),
    }
}

/// The store installed by the app, for the rest of the core to use.
/// `Unavailable` if the app hasn't installed one yet.
// Not called outside tests until account setup (M1-03) reads credentials.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn secret_store() -> Result<Arc<dyn SecretStore>, SecretError> {
    let slot = match SECRET_STORE.read() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    slot.clone().ok_or(SecretError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakePlatform(Mutex<HashMap<String, Vec<u8>>>);

    impl PlatformSecretStore for FakePlatform {
        fn put(&self, key: String, value: Vec<u8>) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .map_err(|_| SecretStoreError::Unavailable)?
                .insert(key, value);
            Ok(())
        }
        fn get(&self, key: String) -> Result<Option<Vec<u8>>, SecretStoreError> {
            Ok(self
                .0
                .lock()
                .map_err(|_| SecretStoreError::Unavailable)?
                .get(&key)
                .cloned())
        }
        fn delete(&self, key: String) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .map_err(|_| SecretStoreError::Unavailable)?
                .remove(&key);
            Ok(())
        }
    }

    struct BrokenPlatform;

    impl PlatformSecretStore for BrokenPlatform {
        fn put(&self, _: String, _: Vec<u8>) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Invalidated)
        }
        fn get(&self, _: String) -> Result<Option<Vec<u8>>, SecretStoreError> {
            Err(SecretStoreError::Invalidated)
        }
        fn delete(&self, _: String) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Other {
                reason: "boom".to_owned(),
            })
        }
    }

    #[test]
    fn adapter_round_trips_through_the_platform() -> Result<(), SecretError> {
        let store = ForeignSecretStore(Arc::new(FakePlatform::default()));
        let key = SecretKey::account_password(3);
        store.put(&key, &Secret::new(b"pw".to_vec()))?;
        assert_eq!(
            store.get(&key)?.map(|s| s.expose().to_vec()),
            Some(b"pw".to_vec())
        );
        store.delete(&key)?;
        assert!(store.get(&key)?.is_none());
        Ok(())
    }

    #[test]
    fn adapter_maps_platform_errors() {
        let store = ForeignSecretStore(Arc::new(BrokenPlatform));
        let key = SecretKey::account_password(3);
        assert_eq!(store.get(&key).err(), Some(SecretError::Invalidated));
        assert_eq!(
            store.delete(&key).err(),
            Some(SecretError::Other {
                reason: "boom".to_owned()
            })
        );
    }

    #[test]
    fn installed_store_is_used() -> Result<(), SecretError> {
        install_secret_store(Arc::new(FakePlatform::default()));
        let store = secret_store()?;
        let key = SecretKey::oauth_refresh_token(4);
        store.put(&key, &Secret::new(b"t".to_vec()))?;
        assert!(store.get(&key)?.is_some());
        Ok(())
    }
}
