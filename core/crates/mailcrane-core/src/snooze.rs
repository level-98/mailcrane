//! Snooze (M4-06): a message leaves its folder and comes back, unread, at a
//! chosen time.
//!
//! Everything happens locally first, so snoozing works offline:
//! [`snooze`] hides the message and records the wake time. [`sync_snoozes`]
//! (run with the queued actions) then moves it to a `Snoozed` folder on the
//! server, created if missing, and, when the folder's `PERMANENTFLAGS`
//! allow new keywords, sets `$snoozed-until-<UTC YYYYMMDDTHHMMZ>` so other
//! Mailcrane installs learn the wake time ([`learn_from_keywords`]).
//! Without keyword support the wake time stays on this device
//! ([`Snooze::device_only`]).
//!
//! [`wake_due`] runs when the app opens, on a timer while it is open, and in
//! background sync. It returns the messages that woke, for a notification;
//! the next [`sync_snoozes`] moves them back to their folder (or the Inbox
//! if it's gone), removes the keyword and marks them unread.

mod keyword;
mod server;

pub use keyword::{format_keyword, parse_keyword, KEYWORD_PREFIX};
pub use mailcrane_store::{Snooze, SnoozeState};
pub use server::{sync_snoozes, SnoozeReport};

use crate::events::{CoreEvent, EventListener};
use mailcrane_store::{AccountId, Folder, FolderId, MessageId, NewSnooze, Store, StoreError};

/// The folder snoozed messages wait in. An existing folder with this name
/// (any case, at the top level) is used.
pub const SNOOZED_FOLDER_NAME: &str = "Snoozed";

/// Why messages could not be snoozed. Nothing was changed.
#[derive(Debug, thiserror::Error)]
pub enum SnoozeError {
    /// One of the messages is not in the database.
    #[error("unknown message")]
    UnknownMessage,
    /// A message has no `Message-ID` header, which snooze needs to find it
    /// again after moving it.
    #[error("the message has no Message-ID")]
    NoMessageId,
    /// A message is being woken: its move back has started on the server,
    /// so it can't be snoozed again until it lands, a few seconds later
    /// (#431). Nothing was changed.
    #[error("the message is being woken")]
    Waking,
    /// The local database failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// A message that woke, for the notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Woken {
    pub account_id: AccountId,
    /// Its local row in the Snoozed folder, if synced.
    pub message_id: Option<MessageId>,
    pub subject: String,
    pub from: String,
}

/// The account's Snoozed folder, if there is one locally.
pub fn snoozed_folder(store: &Store, account: AccountId) -> Result<Option<Folder>, StoreError> {
    Ok(store
        .list_folders(account)?
        .into_iter()
        .find(is_snoozed_folder))
}

fn is_snoozed_folder(folder: &Folder) -> bool {
    folder
        .details
        .remote_name
        .eq_ignore_ascii_case(SNOOZED_FOLDER_NAME)
}

/// Snoozes `messages` until `wake_at` (unix seconds UTC). They are hidden
/// at once; a message already in the Snoozed folder just gets the new wake
/// time. Sends `MessagesChanged`. Returns the accounts whose server work
/// to run ([`sync_snoozes`]). On error nothing is changed.
pub fn snooze(
    store: &Store,
    messages: &[MessageId],
    wake_at: i64,
    listener: &dyn EventListener,
) -> Result<Vec<AccountId>, SnoozeError> {
    let mut plans = Vec::with_capacity(messages.len());
    for id in messages {
        let header = store
            .get_message_header(*id)?
            .ok_or(SnoozeError::UnknownMessage)?;
        let mid = header
            .details
            .message_id_header
            .clone()
            .filter(|m| !m.trim().is_empty())
            .ok_or(SnoozeError::NoMessageId)?;
        let folder = store
            .get_folder(header.details.folder_id)?
            .ok_or(SnoozeError::UnknownMessage)?;
        let in_snoozed = is_snoozed_folder(&folder);
        plans.push((
            NewSnooze {
                account_id: folder.details.account_id,
                message_id_header: mid,
                original_folder_id: (!in_snoozed).then_some(folder.id),
                hide_message: (!in_snoozed).then_some(*id),
                wake_at,
            },
            folder.id,
        ));
    }
    let all: Vec<NewSnooze> = plans.iter().map(|(plan, _)| plan.clone()).collect();
    if store.snooze_messages(&all)? == mailcrane_store::Snoozing::Waking {
        return Err(SnoozeError::Waking);
    }
    let mut changed: Vec<FolderId> = Vec::new();
    let mut accounts: Vec<AccountId> = Vec::new();
    for (plan, folder) in &plans {
        if !changed.contains(folder) {
            changed.push(*folder);
        }
        if !accounts.contains(&plan.account_id) {
            accounts.push(plan.account_id);
        }
    }
    for folder_id in changed {
        listener.on_event(CoreEvent::MessagesChanged { folder_id });
    }
    tracing::debug!(count = plans.len(), "messages snoozed");
    Ok(accounts)
}

/// Brings snoozed `messages` back now, without marking them unread.
/// Returns the accounts of the ones that were snoozed, whose server work to
/// run ([`sync_snoozes`]).
pub fn unsnooze(
    store: &Store,
    messages: &[MessageId],
    now: i64,
    listener: &dyn EventListener,
) -> Result<Vec<AccountId>, SnoozeError> {
    let mut accounts: Vec<AccountId> = Vec::new();
    for id in messages {
        let header = store
            .get_message_header(*id)?
            .ok_or(SnoozeError::UnknownMessage)?;
        let Some(mid) = header.details.message_id_header.as_deref() else {
            continue;
        };
        let folder = store
            .get_folder(header.details.folder_id)?
            .ok_or(SnoozeError::UnknownMessage)?;
        if store.request_unsnooze(folder.details.account_id, mid, now)? {
            if !accounts.contains(&folder.details.account_id) {
                accounts.push(folder.details.account_id);
            }
            if is_snoozed_folder(&folder) {
                store.set_message_hidden(*id, true)?;
            }
            listener.on_event(CoreEvent::MessagesChanged {
                folder_id: folder.id,
            });
        }
    }
    Ok(accounts)
}

/// Records wake times that other installs put in keywords on messages in
/// the account's Snoozed folder (as last synced). Returns how many were new.
pub fn learn_from_keywords(store: &Store, account: AccountId) -> Result<usize, StoreError> {
    let Some(folder) = snoozed_folder(store, account)? else {
        return Ok(0);
    };
    let mut learnt = 0;
    for m in store.messages_with_keyword_prefix(folder.id, KEYWORD_PREFIX)? {
        let Some(mid) = m.message_id_header.as_deref() else {
            continue;
        };
        // Several keywords (two installs racing): the earliest wins.
        let Some(wake_at) = m.keywords.iter().filter_map(|k| parse_keyword(k)).min() else {
            continue;
        };
        if store.learn_snooze(account, mid, wake_at)? {
            learnt += 1;
        }
    }
    Ok(learnt)
}

/// Wakes every snooze due at `now` (after learning wake times from the
/// server's keywords). Their messages are hidden from the Snoozed folder
/// until [`sync_snoozes`] moves them back. Returns them, for a notification.
pub fn wake_due(
    store: &Store,
    now: i64,
    listener: &dyn EventListener,
) -> Result<Vec<Woken>, StoreError> {
    for account in store.list_accounts()? {
        learn_from_keywords(store, account.id)?;
    }
    let mut woken = Vec::new();
    for s in store.take_due_snoozes(now)? {
        let folder = snoozed_folder(store, s.account_id)?;
        let row = match &folder {
            Some(f) => store.find_message_by_header(f.id, &s.message_id_header)?,
            None => None,
        };
        let header = match row {
            Some(id) => store.get_message_header(id)?,
            None => None,
        };
        if let (Some(id), Some(f)) = (row, &folder) {
            store.set_message_hidden(id, true)?;
            listener.on_event(CoreEvent::MessagesChanged { folder_id: f.id });
        }
        let (subject, from) = header
            .map(|h| {
                let from = h
                    .details
                    .from_name
                    .or(h.details.from_addr)
                    .unwrap_or_default();
                (h.details.subject, from)
            })
            .unwrap_or_default();
        woken.push(Woken {
            account_id: s.account_id,
            message_id: row,
            subject,
            from,
        });
    }
    Ok(woken)
}

/// The account's snoozes (`None`: every account's), soonest first.
pub fn list_snoozed(store: &Store, account: Option<AccountId>) -> Result<Vec<Snooze>, StoreError> {
    store.list_snoozes(account)
}
