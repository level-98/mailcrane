//! Per-account settings.

use crate::{AccountId, Store, StoreError};
use rusqlite::{params, OptionalExtension};

/// An account's signature setting.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SignatureSetting {
    /// The signature as plain text; may be empty.
    pub text: String,
    /// Whether to add it to new messages, replies and forwards.
    pub enabled: bool,
}

/// Longest signature kept, in bytes. Longer text is refused, not cut.
pub const MAX_SIGNATURE_BYTES: usize = 10 * 1024;

impl Store {
    /// The account's signature setting, or `None` if the account doesn't exist.
    pub fn get_signature(
        &self,
        account: AccountId,
    ) -> Result<Option<SignatureSetting>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT signature_text, signature_enabled FROM accounts WHERE id = ?1",
                params![account.0],
                |r| {
                    Ok(SignatureSetting {
                        text: r.get(0)?,
                        enabled: r.get::<_, i64>(1)? != 0,
                    })
                },
            )
            .optional()?)
    }

    /// Saves the account's signature setting. Returns whether the account
    /// exists. Text longer than [`MAX_SIGNATURE_BYTES`] is refused.
    pub fn set_signature(
        &self,
        account: AccountId,
        setting: &SignatureSetting,
    ) -> Result<bool, StoreError> {
        if setting.text.len() > MAX_SIGNATURE_BYTES {
            return Err(StoreError::Invalid { field: "signature" });
        }
        let n = self.conn.execute(
            "UPDATE accounts SET signature_text = ?2, signature_enabled = ?3 WHERE id = ?1",
            params![account.0, setting.text, setting.enabled],
        )?;
        Ok(n > 0)
    }
}
