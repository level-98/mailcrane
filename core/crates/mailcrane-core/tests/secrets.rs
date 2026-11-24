//! `SecretStore` behaviour, checked against the testkit's in-memory store.

use mailcrane_core::secrets::{
    delete_account_secrets, Secret, SecretError, SecretKey, SecretStore,
};
use mailcrane_testkit::MemorySecretStore;

#[test]
fn secrets_round_trip() -> Result<(), SecretError> {
    let store = MemorySecretStore::new();
    let key = SecretKey::account_password(1);

    store.put(&key, &Secret::from_string("pw-example".to_owned()))?;
    let got = store.get(&key)?.map(|s| s.expose().to_vec());
    assert_eq!(got.as_deref(), Some(&b"pw-example"[..]));

    store.delete(&key)?;
    assert!(store.get(&key)?.is_none());
    // Deleting again is not an error.
    store.delete(&key)?;
    Ok(())
}

#[test]
fn secrets_deleting_an_account_removes_all_its_keys() -> Result<(), SecretError> {
    let store = MemorySecretStore::new();
    for key in SecretKey::all_for_account(1) {
        store.put(&key, &Secret::new(b"a".to_vec()))?;
    }
    store.put(&SecretKey::account_password(2), &Secret::new(b"b".to_vec()))?;

    delete_account_secrets(&store, 1)?;

    assert_eq!(store.keys(), vec!["account/2/password".to_owned()]);
    Ok(())
}

#[test]
fn secrets_platform_errors_come_back_as_secret_error() {
    let store = MemorySecretStore::new();
    store.fail_with(Some(SecretError::Invalidated));
    let key = SecretKey::account_password(1);

    assert_eq!(store.get(&key).err(), Some(SecretError::Invalidated));
    assert_eq!(
        delete_account_secrets(&store, 1),
        Err(SecretError::Invalidated)
    );
}
