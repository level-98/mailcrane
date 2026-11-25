//! Local-only accounts (#289): stored with a marker the core checks before
//! any connection.

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn account(email: &str) -> NewAccount {
    NewAccount {
        display_name: "Fixtures".into(),
        email: email.into(),
        imap_host: String::new(),
        imap_port: 0,
        imap_security: Security::Tls,
        smtp_host: String::new(),
        smtp_port: 0,
        smtp_security: Security::Tls,
        username: String::new(),
    }
}

#[test]
fn local_accounts_are_marked_and_found() -> R {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path().join("m.db"))?;
    let remote = store.insert_account(&account("a@example.org"))?;
    let local = store.insert_local_account(&account("fixtures@example.org"))?;

    assert!(!store.is_local_account(remote)?);
    assert!(store.is_local_account(local)?);
    assert!(!store.is_local_account(AccountId(999))?);

    assert_eq!(
        store.find_local_account("fixtures@example.org")?,
        Some(local)
    );
    // An ordinary account with the same address is not the local one.
    assert_eq!(store.find_local_account("a@example.org")?, None);
    // Both are listed like any account.
    assert_eq!(store.list_accounts()?.len(), 2);
    Ok(())
}

#[test]
fn existing_accounts_stay_remote_after_the_migration() -> R {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path().join("m.db"))?;
    let id = store.insert_account(&account("b@example.org"))?;
    assert!(!store.is_local_account(id)?);
    Ok(())
}
