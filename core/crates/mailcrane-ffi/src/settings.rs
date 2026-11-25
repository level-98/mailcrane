//! Per-account settings for the apps: the signature (M2-10).

use mailcrane_core::compose::signature;
use mailcrane_store::{AccountId, SaveSentCopy, SignatureSetting, Store, MAX_SIGNATURE_BYTES};

/// An account's signature, as the settings screen edits it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountSignature {
    /// Plain text; may be empty.
    pub text: String,
    /// Whether it's added to new messages, replies and forwards.
    pub enabled: bool,
}

/// Why a setting couldn't be read or saved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SettingsFailure {
    /// The account doesn't exist.
    #[error("account not found")]
    NotFound,
    /// The signature is longer than the limit (10 KiB).
    #[error("signature too long")]
    TooLong,
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

/// Whether to save sent messages to the Sent folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SaveSentSetting {
    /// Yes, unless the server does it itself (Gmail, Microsoft).
    Automatic,
    /// Always.
    On,
    /// Never.
    Off,
}

/// The account's "Save sent messages to the Sent folder" setting.
#[uniffi::export]
pub fn get_save_sent(db_path: String, account_id: i64) -> Result<SaveSentSetting, SettingsFailure> {
    let value = open(&db_path)?
        .save_sent_copy(AccountId(account_id))
        .map_err(|_| SettingsFailure::Storage)?
        .ok_or(SettingsFailure::NotFound)?;
    Ok(match value {
        SaveSentCopy::Auto => SaveSentSetting::Automatic,
        SaveSentCopy::Always => SaveSentSetting::On,
        SaveSentCopy::Never => SaveSentSetting::Off,
    })
}

/// Changes the account's "Save sent messages to the Sent folder" setting.
#[uniffi::export]
pub fn set_save_sent(
    db_path: String,
    account_id: i64,
    setting: SaveSentSetting,
) -> Result<(), SettingsFailure> {
    let value = match setting {
        SaveSentSetting::Automatic => SaveSentCopy::Auto,
        SaveSentSetting::On => SaveSentCopy::Always,
        SaveSentSetting::Off => SaveSentCopy::Never,
    };
    if open(&db_path)?
        .set_save_sent_copy(AccountId(account_id), value)
        .map_err(|_| SettingsFailure::Storage)?
    {
        Ok(())
    } else {
        Err(SettingsFailure::NotFound)
    }
}

/// What kind of draft a signature goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DraftKind {
    /// A new message: at the end.
    New,
    /// A reply: below the cursor, above the quote.
    Reply,
    /// A forward: above the forwarded message.
    Forward,
}

impl From<DraftKind> for signature::DraftKind {
    fn from(kind: DraftKind) -> Self {
        match kind {
            DraftKind::New => Self::New,
            DraftKind::Reply => Self::Reply,
            DraftKind::Forward => Self::Forward,
        }
    }
}

fn open(db_path: &str) -> Result<Store, SettingsFailure> {
    Store::open(db_path).map_err(|_| SettingsFailure::Storage)
}

/// The account's signature setting.
#[uniffi::export]
pub fn get_signature(
    db_path: String,
    account_id: i64,
) -> Result<AccountSignature, SettingsFailure> {
    let setting = open(&db_path)?
        .get_signature(AccountId(account_id))
        .map_err(|_| SettingsFailure::Storage)?
        .ok_or(SettingsFailure::NotFound)?;
    Ok(AccountSignature {
        text: setting.text,
        enabled: setting.enabled,
    })
}

/// Saves the account's signature setting.
#[uniffi::export]
pub fn set_signature(
    db_path: String,
    account_id: i64,
    signature: AccountSignature,
) -> Result<(), SettingsFailure> {
    if signature.text.len() > MAX_SIGNATURE_BYTES {
        return Err(SettingsFailure::TooLong);
    }
    let saved = open(&db_path)?
        .set_signature(
            AccountId(account_id),
            &SignatureSetting {
                text: signature.text,
                enabled: signature.enabled,
            },
        )
        .map_err(|_| SettingsFailure::Storage)?;
    if saved {
        Ok(())
    } else {
        Err(SettingsFailure::NotFound)
    }
}

/// The signature to put into a draft from this account: its text when it's
/// turned on and not empty, otherwise `None`. A missing account or a
/// database failure also gives `None`; a draft without a signature is fine.
#[uniffi::export]
pub fn signature_for_draft(db_path: String, account_id: i64) -> Option<String> {
    let setting = open(&db_path)
        .ok()?
        .get_signature(AccountId(account_id))
        .ok()??;
    (setting.enabled && !setting.text.trim().is_empty()).then_some(setting.text)
}

/// Adds `signature` to `body` (see the core's `apply_signature`): at the end
/// of a new message, between the cursor line and the quote of a reply or
/// forward. `None` or empty adds nothing.
#[uniffi::export]
pub fn apply_signature(body: String, signature: Option<String>, kind: DraftKind) -> String {
    signature::apply_signature(&body, signature.as_deref(), kind.into())
}

/// Swaps an unedited `old` signature in `body` for `new`, for a change of
/// From account. An edited signature is left alone.
#[uniffi::export]
pub fn swap_signature(body: String, old: Option<String>, new: Option<String>) -> String {
    signature::swap_signature(&body, old.as_deref(), new.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{NewAccount, Security};

    fn account(path: &str) -> i64 {
        Store::open(path)
            .expect("store")
            .insert_account(&NewAccount {
                display_name: "Ann".into(),
                email: "ann@example.org".into(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 465,
                smtp_security: Security::Tls,
                username: "ann".into(),
            })
            .expect("account")
            .0
    }

    #[test]
    fn signature_round_trips_and_feeds_drafts() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("s.db").to_string_lossy().into_owned();
        let id = account(&path);
        assert_eq!(
            signature_for_draft(path.clone(), id),
            None,
            "empty by default"
        );

        let sig = AccountSignature {
            text: "Ann".into(),
            enabled: true,
        };
        assert_eq!(set_signature(path.clone(), id, sig.clone()), Ok(()));
        assert_eq!(get_signature(path.clone(), id), Ok(sig));
        assert_eq!(
            signature_for_draft(path.clone(), id).as_deref(),
            Some("Ann")
        );

        let off = AccountSignature {
            text: "Ann".into(),
            enabled: false,
        };
        assert_eq!(set_signature(path.clone(), id, off), Ok(()));
        assert_eq!(signature_for_draft(path.clone(), id), None, "turned off");

        assert_eq!(
            get_signature(path.clone(), 99),
            Err(SettingsFailure::NotFound)
        );
        assert_eq!(
            set_signature(
                path.clone(),
                id,
                AccountSignature {
                    text: "x".repeat(MAX_SIGNATURE_BYTES + 1),
                    enabled: true
                }
            ),
            Err(SettingsFailure::TooLong)
        );
        assert_eq!(
            set_signature(
                path,
                99,
                AccountSignature {
                    text: String::new(),
                    enabled: true
                }
            ),
            Err(SettingsFailure::NotFound)
        );
    }

    #[test]
    fn save_sent_setting_round_trips() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("s.db").to_string_lossy().into_owned();
        let id = account(&path);
        assert_eq!(
            get_save_sent(path.clone(), id),
            Ok(SaveSentSetting::Automatic)
        );
        for setting in [
            SaveSentSetting::Off,
            SaveSentSetting::On,
            SaveSentSetting::Automatic,
        ] {
            assert_eq!(set_save_sent(path.clone(), id, setting), Ok(()));
            assert_eq!(get_save_sent(path.clone(), id), Ok(setting));
        }
        assert_eq!(
            get_save_sent(path.clone(), 99),
            Err(SettingsFailure::NotFound)
        );
        assert_eq!(
            set_save_sent(path, 99, SaveSentSetting::On),
            Err(SettingsFailure::NotFound)
        );
    }

    #[test]
    fn apply_and_swap_delegate_to_the_core() {
        let body = apply_signature("Hi".into(), Some("Ann".into()), DraftKind::New);
        assert_eq!(body, "Hi\n\n-- \nAnn");
        assert_eq!(
            swap_signature(body, Some("Ann".into()), Some("Bob".into())),
            "Hi\n\n-- \nBob"
        );
    }
}
