//! New-mail notifications (M4-05): which messages deserve one.
//!
//! The rules live in the store query (`Store::take_new_mail`): in the inbox,
//! unread, not a draft, not from the account's own address, not snoozed,
//! recent, and never notified before. The first look at an account only
//! records what is there. This module adds the policy: how recent, how many
//! at once, and which folder is the inbox.

use mailcrane_store::{Account, FolderId, FolderRole, NewMail, Store, StoreError};

/// Messages that arrived longer ago than this are never notified: an old
/// unread message turning up after a long time offline is not "new mail".
pub const MAX_AGE_SECS: i64 = 2 * 24 * 60 * 60;

/// At most this many notifications per account at a time; the rest are
/// returned by the next call.
pub const MAX_PER_ACCOUNT: u32 = 20;

/// The account's inbox: the folder with the inbox role, otherwise the one
/// named `INBOX` in any case (RFC 9051 §5.1). `None` before its folders are
/// known.
pub fn inbox_of(store: &Store, account: &Account) -> Result<Option<FolderId>, StoreError> {
    let folders = store.list_folders(account.id)?;
    let by_role = folders
        .iter()
        .find(|f| f.details.role == Some(FolderRole::Inbox));
    let by_name = || {
        folders
            .iter()
            .find(|f| f.details.remote_name.eq_ignore_ascii_case("INBOX"))
    };
    Ok(by_role.or_else(by_name).map(|f| f.id))
}

/// New mail to notify for `account` now (unix seconds `now`), each message
/// at most once. Empty until the inbox has been synced at least once.
pub fn take_new_mail(
    store: &Store,
    account: &Account,
    now: i64,
) -> Result<Vec<NewMail>, StoreError> {
    let Some(inbox) = inbox_of(store, account)? else {
        return Ok(Vec::new());
    };
    store.take_new_mail(
        account.id,
        inbox,
        &account.details.email,
        now.saturating_sub(MAX_AGE_SECS),
        now,
        MAX_PER_ACCOUNT,
    )
}
