//! Per-account settings.

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn account() -> NewAccount {
    NewAccount {
        display_name: "Ann".into(),
        email: "ann@example.org".into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 465,
        smtp_security: Security::Tls,
        username: "ann".into(),
    }
}

#[test]
fn signature_defaults_saves_and_survives_a_reopen() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("s.db");
    let store = Store::open(&path)?;
    let id = store.insert_account(&account())?;
    assert_eq!(
        store.get_signature(id)?,
        Some(SignatureSetting {
            text: String::new(),
            enabled: true
        })
    );
    let setting = SignatureSetting {
        text: "Ann\nExample Ltd".into(),
        enabled: false,
    };
    assert!(store.set_signature(id, &setting)?);
    drop(store);
    let store = Store::open(&path)?;
    assert_eq!(store.get_signature(id)?, Some(setting));
    Ok(())
}

#[test]
fn unknown_accounts_and_huge_signatures_are_refused() -> R {
    let store = Store::open_in_memory()?;
    assert_eq!(store.get_signature(AccountId(9))?, None);
    assert!(!store.set_signature(AccountId(9), &SignatureSetting::default())?);
    let id = store.insert_account(&account())?;
    let huge = SignatureSetting {
        text: "x".repeat(MAX_SIGNATURE_BYTES + 1),
        enabled: true,
    };
    assert!(store.set_signature(id, &huge).is_err());
    assert_eq!(
        store.get_signature(id)?.map(|s| s.text),
        Some(String::new())
    );
    Ok(())
}
