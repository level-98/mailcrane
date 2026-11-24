//! An in-memory [`SecretStore`] for tests.

use std::collections::HashMap;
use std::sync::Mutex;

use mailcrane_core::secrets::{Secret, SecretError, SecretKey, SecretStore};

/// Keeps secrets in a map, for tests. It can be told to fail, to test how the
/// core handles secure storage that can't be used.
#[derive(Default)]
pub struct MemorySecretStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    values: HashMap<String, Vec<u8>>,
    fail_with: Option<SecretError>,
}

impl MemorySecretStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Make every later call fail with `error`, or work again with `None`.
    pub fn fail_with(&self, error: Option<SecretError>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.fail_with = error;
        }
    }

    /// The keys currently stored, sorted. Values are never exposed this way.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = match self.inner.lock() {
            Ok(inner) => inner.values.keys().cloned().collect(),
            Err(_) => Vec::new(),
        };
        keys.sort();
        keys
    }

    fn with<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> Result<T, SecretError> {
        let mut inner = self.inner.lock().map_err(|_| SecretError::Other {
            reason: "memory store lock poisoned".to_owned(),
        })?;
        if let Some(err) = inner.fail_with.clone() {
            return Err(err);
        }
        Ok(f(&mut inner))
    }
}

impl SecretStore for MemorySecretStore {
    fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError> {
        self.with(|inner| {
            inner
                .values
                .insert(key.as_str().to_owned(), value.expose().to_vec());
        })
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        self.with(|inner| inner.values.get(key.as_str()).cloned().map(Secret::new))
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.with(|inner| {
            inner.values.remove(key.as_str());
        })
    }
}
