//! Saving sent messages to the Sent folder (M2-07).
//!
//! After the outbox marks a message sent, its exact bytes (already without
//! `Bcc`) are appended to the Sent folder with `\Seen`, retried with back-off
//! like other queued work. Servers that file sent mail themselves
//! ([`ServerQuirk::AutoSavesSent`]) are skipped under the `auto` setting.
//! A failed copy is never a reason to send the message again: the message
//! stays sent, with its copy marked failed for the user to retry.

use mailcrane_imap::{save_folders, AppendFlags, ImapError, ServerQuirk, Session};
use mailcrane_store::{AccountId, FolderRole, OutboxMessage, SaveSentCopy, SentCopy, Store};

use crate::actions::backoff_secs;
use crate::sync::SyncError;

/// The name used when the server has no Sent folder at all.
pub const SENT_FOLDER_NAME: &str = "Sent";

/// What one [`save_sent_copies`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SaveReport {
    /// Copies now in the Sent folder.
    pub saved: usize,
    /// Copies not needed (the server saves them, or the setting is never).
    pub skipped: usize,
    /// Copies refused for good.
    pub failed: usize,
    /// Copies still to upload.
    pub remaining: usize,
}

/// The Sent folder's server name, creating `Sent` if the account has none.
async fn sent_folder(
    store: &Store,
    session: &mut Session,
    account: AccountId,
) -> Result<Option<String>, SyncError> {
    let find = |store: &Store| -> Result<Option<String>, SyncError> {
        Ok(store
            .list_folders(account)?
            .into_iter()
            .find(|f| f.details.role == Some(FolderRole::Sent))
            .map(|f| f.details.remote_name))
    };
    if let Some(name) = find(store)? {
        return Ok(Some(name));
    }
    if session.create_folder(SENT_FOLDER_NAME).await.is_err() {
        return Ok(None);
    }
    // List again so the new folder is saved, with its role from its name.
    let listed = session.list_folders().await?;
    save_folders(store, account, &listed)?;
    find(store)
}

/// Uploads the account's pending sent copies that are due by `now`.
/// Errors only for a local database failure or a failed folder listing.
pub async fn save_sent_copies(
    store: &Store,
    session: &mut Session,
    account: AccountId,
    now: i64,
) -> Result<SaveReport, SyncError> {
    let mut report = SaveReport::default();
    let pending = store.pending_sent_copies(account, now)?;
    if pending.is_empty() {
        return Ok(report);
    }
    let setting = store
        .save_sent_copy(account)?
        .ok_or(SyncError::UnknownFolder)?;
    let skip = match setting {
        SaveSentCopy::Never => true,
        SaveSentCopy::Always => false,
        SaveSentCopy::Auto => session.quirks().contains(&ServerQuirk::AutoSavesSent),
    };
    if skip {
        for m in &pending {
            store.set_sent_copy(m.id, SentCopy::Skipped)?;
        }
        report.skipped = pending.len();
        return Ok(report);
    }
    let Some(folder) = sent_folder(store, session, account).await? else {
        defer_all(store, &pending, now)?;
        report.remaining = pending.len();
        return Ok(report);
    };
    match session.select(&folder).await {
        Ok(_) => {}
        // The folder is gone on the server: these copies can't be saved.
        Err(ImapError::CommandFailed) => {
            for m in &pending {
                store.set_sent_copy(m.id, SentCopy::Failed)?;
            }
            report.failed = pending.len();
            return Ok(report);
        }
        Err(ImapError::Store(e)) => return Err(SyncError::Store(e)),
        // Connection trouble: try again later.
        Err(_) => {
            defer_all(store, &pending, now)?;
            report.remaining = pending.len();
            return Ok(report);
        }
    }
    for (index, message) in pending.iter().enumerate() {
        match save_one(session, &folder, message).await {
            Ok(()) => {
                store.set_sent_copy(message.id, SentCopy::Saved)?;
                report.saved += 1;
            }
            Err(error) if error.is_transient() => {
                defer_all(store, pending.get(index..).unwrap_or_default(), now)?;
                report.remaining = pending.len().saturating_sub(index);
                break;
            }
            Err(mailcrane_imap::ChangeError::Imap(ImapError::Store(e))) => {
                return Err(SyncError::Store(e));
            }
            Err(_) => {
                store.set_sent_copy(message.id, SentCopy::Failed)?;
                tracing::warn!("copy of a sent message refused by the Sent folder");
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Appends one copy, unless it's there already (a retry after the copy was
/// appended but not recorded must not make a second one).
async fn save_one(
    session: &mut Session,
    folder: &str,
    message: &OutboxMessage,
) -> Result<(), mailcrane_imap::ChangeError> {
    let existing = session
        .uid_search_message_id(&message.message_id_header)
        .await?;
    if existing.is_empty() {
        session
            .append(folder, AppendFlags::Seen, &message.raw)
            .await?;
    }
    Ok(())
}

fn defer_all(store: &Store, messages: &[OutboxMessage], now: i64) -> Result<(), SyncError> {
    for m in messages {
        let attempts = m.copy_attempts.saturating_add(1);
        store.defer_sent_copy(m.id, attempts, now.saturating_add(backoff_secs(attempts)))?;
    }
    Ok(())
}
