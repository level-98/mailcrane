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

#[test]
fn account_color_is_automatic_until_chosen_and_survives_a_reopen() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("c.db");
    let store = Store::open(&path)?;
    let id = store.insert_account(&account())?;
    assert_eq!(store.get_account_color(id)?, None);
    assert!(store.set_account_color(id, Some(5))?);
    drop(store);
    let store = Store::open(&path)?;
    assert_eq!(store.get_account_color(id)?, Some(5));
    let other = store.insert_account(&account())?;
    assert_eq!(
        store.list_account_colors()?,
        vec![(id, Some(5)), (other, None)]
    );
    assert!(store.set_account_color(id, None)?);
    assert_eq!(store.get_account_color(id)?, None);
    Ok(())
}

#[test]
fn bad_colors_and_unknown_accounts_change_nothing() -> R {
    let store = Store::open_in_memory()?;
    assert_eq!(store.get_account_color(AccountId(9))?, None);
    assert!(!store.set_account_color(AccountId(9), Some(1))?);
    let id = store.insert_account(&account())?;
    assert!(store.set_account_color(id, Some(2))?);
    assert!(store
        .set_account_color(id, Some(ACCOUNT_COLOR_COUNT))
        .is_err());
    assert!(store.set_account_color(id, Some(u32::MAX)).is_err());
    assert_eq!(store.get_account_color(id)?, Some(2));
    Ok(())
}

#[test]
fn provider_authserv_id_is_unset_until_chosen_and_survives_a_reopen() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("a.db");
    let store = Store::open(&path)?;
    let id = store.insert_account(&account())?;
    assert_eq!(store.get_provider_authserv_id(id)?, None);
    assert!(store.set_provider_authserv_id(id, Some("mx2.example.org"))?);
    drop(store);
    let store = Store::open(&path)?;
    assert_eq!(
        store.get_provider_authserv_id(id)?.as_deref(),
        Some("mx2.example.org")
    );
    assert!(store.set_provider_authserv_id(id, None)?);
    assert_eq!(store.get_provider_authserv_id(id)?, None);
    Ok(())
}

#[test]
fn bad_authserv_ids_and_unknown_accounts_change_nothing() -> R {
    let store = Store::open_in_memory()?;
    assert_eq!(store.get_provider_authserv_id(AccountId(9))?, None);
    assert!(!store.set_provider_authserv_id(AccountId(9), Some("mx.example.org"))?);
    let id = store.insert_account(&account())?;
    assert!(store.set_provider_authserv_id(id, Some("mx.example.org"))?);
    let too_long = format!("{}.example.org", "a".repeat(250));
    for bad in [
        "",
        "localhost",
        "MX.example.org",
        "mx..example.org",
        ".example.org",
        "example.org.",
        "mx example.org",
        "mx.example.org;",
        "mx.пример.рф",
        too_long.as_str(),
    ] {
        assert!(
            store.set_provider_authserv_id(id, Some(bad)).is_err(),
            "{bad:?}"
        );
        assert!(!is_valid_authserv_id(bad), "{bad:?}");
    }
    assert_eq!(
        store.get_provider_authserv_id(id)?.as_deref(),
        Some("mx.example.org")
    );
    assert!(is_valid_authserv_id("mx_1-a.example.co.uk"));
    Ok(())
}
