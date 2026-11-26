//! Recipient suggestions for the apps (M2-08). The apps merge in device
//! contacts themselves; those never reach the core.

use crate::actions::now;
use mailcrane_core::contacts;
use mailcrane_store::{AccountId, Store};

/// A suggested recipient from past correspondence.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactSuggestion {
    /// Lower-case address.
    pub email: String,
    /// Display name; may be empty.
    pub name: String,
}

/// Why suggestions couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ContactsFailure {
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

/// Up to `limit` (at most 50) people matching `prefix` (start of the name,
/// any word of it, or the start of the address; case and accents ignored),
/// best first, from `account_id`'s mail (`None`: every account's).
#[uniffi::export]
pub fn suggest_contacts(
    db_path: String,
    account_id: Option<i64>,
    prefix: String,
    limit: u32,
) -> Result<Vec<ContactSuggestion>, ContactsFailure> {
    let store = Store::open(&db_path).map_err(|_| ContactsFailure::Storage)?;
    let limit = usize::try_from(limit).unwrap_or(contacts::MAX_SUGGESTIONS);
    let found = contacts::suggest(&store, account_id.map(AccountId), &prefix, limit, now())
        .map_err(|_| ContactsFailure::Storage)?;
    Ok(found
        .into_iter()
        .map(|s| ContactSuggestion {
            email: s.email,
            name: crate::messages::display_line(&s.name),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_prefix_suggests_nothing_and_a_bad_path_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        assert_eq!(suggest_contacts(path, None, "  ".into(), 8), Ok(Vec::new()));
        let bad = dir.path().join("no").join("such").join("m.db");
        assert_eq!(
            suggest_contacts(bad.to_string_lossy().into_owned(), None, "a".into(), 8),
            Err(ContactsFailure::Storage)
        );
    }
}
