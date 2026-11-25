//! Carrying snoozes out on the server (M4-06).

use super::{format_keyword, snoozed_folder, KEYWORD_PREFIX, SNOOZED_FOLDER_NAME};
use crate::actions::backoff_secs;
use crate::events::{CoreEvent, EventListener};
use crate::sync::SyncError;
use mailcrane_imap::{save_folders, ChangeError, ImapError, Session, SystemFlag};
use mailcrane_store::{AccountId, Folder, FolderRole, Snooze, SnoozeState, Store, StoreError};

/// What [`sync_snoozes`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SnoozeReport {
    /// Messages moved to the Snoozed folder.
    pub snoozed: usize,
    /// Messages moved back.
    pub woken: usize,
    /// Snoozes the server refused for good (the message is shown again).
    pub failed: usize,
    /// Still to do (deferred with back-off).
    pub remaining: usize,
}

/// Moves newly snoozed messages of `account` to the Snoozed folder (with the
/// wake-time keyword when allowed) and woken ones back, unread. Failures
/// that may pass are retried later with back-off; refusals undo the snooze
/// locally. Only a local database failure comes back as an error.
pub async fn sync_snoozes(
    store: &Store,
    session: &mut Session,
    listener: &dyn EventListener,
    account: AccountId,
    now: i64,
) -> Result<SnoozeReport, SyncError> {
    let mut report = SnoozeReport::default();
    let work: Vec<Snooze> = store
        .list_snoozes(Some(account))?
        .into_iter()
        .filter(|s| s.state != SnoozeState::Snoozed && s.next_attempt_at <= now)
        .collect();
    if work.is_empty() {
        return Ok(report);
    }
    for (index, s) in work.iter().enumerate() {
        let result = match s.state {
            SnoozeState::ToSnooze => snooze_one(store, session, s).await,
            SnoozeState::ToWake => wake_one(store, session, s).await,
            SnoozeState::Snoozed => continue,
        };
        match result {
            Ok(changed) => {
                match s.state {
                    SnoozeState::ToSnooze => report.snoozed += 1,
                    _ => report.woken += 1,
                }
                for folder_id in changed {
                    listener.on_event(CoreEvent::MessagesChanged { folder_id });
                }
            }
            Err(ChangeError::Imap(ImapError::Store(e))) => return Err(SyncError::Store(e)),
            Err(e) if e.is_transient() => {
                for later in work.get(index..).unwrap_or_default() {
                    let attempts = later.attempts.saturating_add(1);
                    store.defer_snooze(later.id, now.saturating_add(backoff_secs(attempts)))?;
                }
                report.remaining = work.len().saturating_sub(index);
                break;
            }
            Err(_) => {
                tracing::warn!(state = ?s.state, "snooze refused by the server");
                undo(store, s, listener)?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

fn store_err(e: StoreError) -> ChangeError {
    ChangeError::Imap(ImapError::Store(e))
}

/// The server refused for good: show the message again where it is.
fn undo(store: &Store, s: &Snooze, listener: &dyn EventListener) -> Result<(), StoreError> {
    if s.state == SnoozeState::ToWake {
        if let Some(folder) = snoozed_folder(store, s.account_id)? {
            if let Some(id) = store.find_message_by_header(folder.id, &s.message_id_header)? {
                store.set_message_hidden(id, false)?;
                listener.on_event(CoreEvent::MessagesChanged {
                    folder_id: folder.id,
                });
            }
        }
        store.delete_snooze(s.id, false)
    } else {
        if let Some(folder) = s.original_folder_id {
            listener.on_event(CoreEvent::MessagesChanged { folder_id: folder });
        }
        store.delete_snooze(s.id, true)
    }
}

/// The Snoozed folder, created on the server if missing.
async fn ensure_snoozed_folder(
    store: &Store,
    session: &mut Session,
    account: AccountId,
) -> Result<Folder, ChangeError> {
    if let Some(f) = snoozed_folder(store, account).map_err(store_err)? {
        return Ok(f);
    }
    session.create_folder(SNOOZED_FOLDER_NAME).await?;
    let listed = session.list_folders().await?;
    save_folders(store, account, &listed).map_err(|e| match e {
        ImapError::Store(e) => store_err(e),
        other => ChangeError::Imap(other),
    })?;
    snoozed_folder(store, account)
        .map_err(store_err)?
        // Created but not listed: give up on this one.
        .ok_or(ChangeError::Refused { transient: false })
}

/// Keywords to remove: every snooze keyword stored locally for this message
/// in `folder`, plus the one for its current wake time.
fn old_keywords(store: &Store, folder: &Folder, s: &Snooze) -> Result<Vec<String>, ChangeError> {
    let mut out: Vec<String> = store
        .messages_with_keyword_prefix(folder.id, KEYWORD_PREFIX)
        .map_err(store_err)?
        .into_iter()
        .filter(|m| m.message_id_header.as_deref() == Some(s.message_id_header.as_str()))
        .flat_map(|m| m.keywords)
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// `UID MOVE`, or `COPY` + `\Deleted` + `UID EXPUNGE` without MOVE
/// (RFC 6851 §3.3).
async fn move_uids(session: &mut Session, uids: &[u32], to: &str) -> Result<(), ChangeError> {
    if session.supports_move() {
        return session.uid_move(uids, to).await;
    }
    session.uid_copy(uids, to).await?;
    session
        .uid_store_flag(uids, SystemFlag::Deleted, true)
        .await?;
    if session.supports_uid_expunge() {
        session.uid_expunge(uids).await?;
    }
    Ok(())
}

async fn snooze_one(
    store: &Store,
    session: &mut Session,
    s: &Snooze,
) -> Result<Vec<mailcrane_store::FolderId>, ChangeError> {
    let snoozed = ensure_snoozed_folder(store, session, s.account_id).await?;
    // Where the message is now: its hidden row's folder, or (re-snoozing)
    // the Snoozed folder.
    let local = match s.local_message_id {
        Some(id) => store.get_message_header(id).map_err(store_err)?,
        None => None,
    };
    let source = match &local {
        Some(h) => store
            .get_folder(h.details.folder_id)
            .map_err(store_err)?
            .ok_or(ChangeError::Refused { transient: false })?,
        None if s.original_folder_id.is_some() && s.local_message_id.is_some() => {
            // The hidden row was removed by a sync: look in the original.
            let id = s.original_folder_id.unwrap_or(snoozed.id);
            store
                .get_folder(id)
                .map_err(store_err)?
                .ok_or(ChangeError::Refused { transient: false })?
        }
        None => snoozed.clone(),
    };
    let info = session.select(&source.details.remote_name).await?;
    let uids = match &local {
        Some(h) => vec![h.details.uid],
        None => session.uid_search_message_id(&s.message_id_header).await?,
    };
    if uids.is_empty() {
        // The message is gone from the server.
        return Err(ChangeError::Refused { transient: false });
    }
    let old = old_keywords(store, &source, s)?;
    let new = format_keyword(s.wake_at);
    if info.custom_keywords {
        for k in old.iter().filter(|k| Some(*k) != new.as_ref()) {
            session.uid_store_keyword(&uids, k, false).await?;
        }
        if let Some(k) = &new {
            session.uid_store_keyword(&uids, k, true).await?;
        }
    }
    if source.id != snoozed.id {
        move_uids(session, &uids, &snoozed.details.remote_name).await?;
    }
    store
        .finish_snooze(s.id, info.custom_keywords && new.is_some())
        .map_err(store_err)?;
    Ok(vec![source.id, snoozed.id])
}

async fn wake_one(
    store: &Store,
    session: &mut Session,
    s: &Snooze,
) -> Result<Vec<mailcrane_store::FolderId>, ChangeError> {
    let Some(snoozed) = snoozed_folder(store, s.account_id).map_err(store_err)? else {
        // No Snoozed folder: nothing to move back.
        store.delete_snooze(s.id, false).map_err(store_err)?;
        return Ok(Vec::new());
    };
    let local = store
        .find_message_by_header(snoozed.id, &s.message_id_header)
        .map_err(store_err)?;
    let info = session.select(&snoozed.details.remote_name).await?;
    let uids = session.uid_search_message_id(&s.message_id_header).await?;
    if uids.is_empty() {
        // Already woken (by another install) or moved away by the user.
        if let Some(id) = local {
            store.delete_message_row(id).map_err(store_err)?;
        }
        store.delete_snooze(s.id, false).map_err(store_err)?;
        return Ok(vec![snoozed.id]);
    }
    if info.custom_keywords {
        let mut old = old_keywords(store, &snoozed, s)?;
        if let Some(k) = format_keyword(s.wake_at) {
            if !old.contains(&k) {
                old.push(k);
            }
        }
        for k in &old {
            session.uid_store_keyword(&uids, k, false).await?;
        }
    }
    if s.mark_unread {
        session
            .uid_store_flag(&uids, SystemFlag::Seen, false)
            .await?;
    }
    let folders = store.list_folders(s.account_id).map_err(store_err)?;
    let dest = s
        .original_folder_id
        .and_then(|id| folders.iter().find(|f| f.id == id))
        .or_else(|| {
            folders
                .iter()
                .find(|f| f.details.role == Some(FolderRole::Inbox))
        })
        .or_else(|| {
            folders
                .iter()
                .find(|f| f.details.remote_name.eq_ignore_ascii_case("INBOX"))
        })
        .ok_or(ChangeError::Refused { transient: false })?;
    move_uids(session, &uids, &dest.details.remote_name).await?;
    if let Some(id) = local {
        store.delete_message_row(id).map_err(store_err)?;
    }
    store.delete_snooze(s.id, false).map_err(store_err)?;
    Ok(vec![snoozed.id, dest.id])
}
