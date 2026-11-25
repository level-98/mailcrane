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

/// How many marker colours there are (M4-20). A chosen colour is an index
/// below this; the apps hold the palette.
pub const ACCOUNT_COLOR_COUNT: u32 = 8;

impl Store {
    /// The colour the user chose for the account, or `None` when it is
    /// automatic or the account doesn't exist.
    pub fn get_account_color(&self, account: AccountId) -> Result<Option<u32>, StoreError> {
        let color: Option<Option<i64>> = self
            .conn
            .query_row(
                "SELECT color FROM accounts WHERE id = ?1",
                params![account.0],
                |r| r.get(0),
            )
            .optional()?;
        // A value outside the palette (a damaged file) counts as automatic.
        Ok(color
            .flatten()
            .and_then(|c| u32::try_from(c).ok())
            .filter(|c| *c < ACCOUNT_COLOR_COUNT))
    }

    /// Every account's chosen colour, `None` where it is automatic, by
    /// account id.
    pub fn list_account_colors(&self) -> Result<Vec<(AccountId, Option<u32>)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, color FROM accounts ORDER BY id")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let color = r
                .get::<_, Option<i64>>(1)?
                .and_then(|c| u32::try_from(c).ok())
                .filter(|c| *c < ACCOUNT_COLOR_COUNT);
            out.push((AccountId(r.get(0)?), color));
        }
        Ok(out)
    }

    /// Chooses the account's colour, or makes it automatic again with
    /// `None`. Returns whether the account exists. A colour of
    /// [`ACCOUNT_COLOR_COUNT`] or more is refused and nothing changes.
    pub fn set_account_color(
        &self,
        account: AccountId,
        color: Option<u32>,
    ) -> Result<bool, StoreError> {
        if color.is_some_and(|c| c >= ACCOUNT_COLOR_COUNT) {
            return Err(StoreError::Invalid { field: "color" });
        }
        let n = self.conn.execute(
            "UPDATE accounts SET color = ?2 WHERE id = ?1",
            params![account.0, color],
        )?;
        Ok(n > 0)
    }
}
