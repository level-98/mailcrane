//! Carrying snoozes out on the server (M4-06).

use super::{format_keyword, snoozed_folder, KEYWORD_PREFIX, SNOOZED_FOLDER_NAME};
use crate::actions::backoff_secs;
use crate::events::{CoreEvent, EventListener};
use crate::sync::SyncError;
use mailcrane_imap::{save_folders, ChangeError, ImapError, Session, SystemFlag};
use mailcrane_store::{
    AccountId, Folder, FolderId, FolderRole, Snooze, SnoozeState, Store, StoreError,
};

/// How far a move without MOVE got, saved on the snooze row after each
/// server step (store migration 0023), so a retry resumes instead of copying
/// again (#423), as `actions/replay.rs` does for pending moves.
const STEP_NONE: u8 = 0;
/// The target's UIDNEXT is recorded; the COPY may or may not have happened.
const STEP_COPY_STARTED: u8 = 1;
const STEP_COPIED: u8 = 2;
const STEP_FLAGGED_DELETED: u8 = 3;
/// Refused after the COPY: the copy must be removed from the target, then
/// the snooze undone. Retried on every sync until it works, never dropped.
const STEP_ROLLBACK: u8 = 4;

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
        if s.state == SnoozeState::Snoozed {
            continue;
        }
        // `None`: a refused move was rolled back on the server.
        let result = if s.step == STEP_ROLLBACK {
            roll_back(store, session, s).await.map(|()| None)
        } else if s.state == SnoozeState::ToSnooze {
            snooze_one(store, session, s).await.map(Some)
        } else {
            wake_one(store, session, s).await.map(Some)
        };
        let result = match result {
            // Refused after the COPY started: remove the copy, then undo.
            Err(e) if !e.is_transient() && !matches!(e, ChangeError::Imap(ImapError::Store(_))) => {
                let step = store
                    .get_snooze_by_id(s.id)?
                    .map_or(STEP_NONE, |fresh| fresh.step);
                if matches!(step, STEP_COPY_STARTED | STEP_COPIED | STEP_ROLLBACK) {
                    tracing::warn!(snooze = s.id.0, state = ?s.state, step, "refused after the copy: rolling back");
                    store.set_snooze_step(s.id, STEP_ROLLBACK, None)?;
                    let fresh = store.get_snooze_by_id(s.id)?.unwrap_or_else(|| s.clone());
                    roll_back(store, session, &fresh).await.map(|()| None)
                } else {
                    Err(e)
                }
            }
            other => other,
        };
        match result {
            Ok(Some(changed)) => {
                match s.state {
                    SnoozeState::ToSnooze => report.snoozed += 1,
                    _ => report.woken += 1,
                }
                for folder_id in changed {
                    listener.on_event(CoreEvent::MessagesChanged { folder_id });
                }
            }
            Ok(None) => {
                tracing::warn!(snooze = s.id.0, state = ?s.state, "rolled back on the server");
                undo(store, s, listener)?;
                report.failed += 1;
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
            Err(_) if store.get_snooze_by_id(s.id)?.map(|f| f.step) == Some(STEP_ROLLBACK) => {
                // Even the rollback was refused: keep the row for the next
                // sync rather than leave a copy behind silently.
                tracing::warn!(
                    snooze = s.id.0,
                    "rollback refused: retried on the next sync"
                );
                let attempts = s.attempts.saturating_add(1);
                store.defer_snooze(s.id, now.saturating_add(backoff_secs(attempts)))?;
                report.remaining = report.remaining.saturating_add(1);
            }
            Err(_) => {
                tracing::warn!(snooze = s.id.0, state = ?s.state, "snooze refused by the server");
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

fn set_step(store: &Store, s: &Snooze, step: u8, uidnext: Option<u32>) -> Result<(), ChangeError> {
    store
        .set_snooze_step(s.id, step, uidnext)
        .map_err(store_err)
}

/// UIDs of copies of `s` in the selected folder made by this move: the
/// message's Message-ID, at or above the UIDNEXT recorded before the COPY.
/// An older message with the same Message-ID is never included.
async fn find_copies(session: &mut Session, s: &Snooze) -> Result<Vec<u32>, ChangeError> {
    let Some(floor) = s.target_uidnext else {
        return Ok(Vec::new());
    };
    Ok(session
        .uid_search_message_id(&s.message_id_header)
        .await?
        .into_iter()
        .filter(|uid| *uid >= floor)
        .collect())
}

/// Moves `uids` (in `from`, selected) to `to`: `UID MOVE`, which is atomic,
/// or without it `COPY` + `\Deleted` + `UID EXPUNGE` (RFC 6851 §3.3) with
/// the step saved after each command, so a retry never copies twice (#423).
/// Leaves `from` selected.
async fn move_uids(
    store: &Store,
    session: &mut Session,
    s: &Snooze,
    uids: &[u32],
    from: &str,
    to: &str,
) -> Result<(), ChangeError> {
    if session.supports_move() {
        return session.uid_move(uids, to).await;
    }
    if s.step < STEP_COPIED {
        // The target's UIDNEXT tells our copy from older messages.
        let target = session.select(to).await?;
        // A COPY sent before a dropped connection may have happened.
        let done = s.step == STEP_COPY_STARTED && !find_copies(session, s).await?.is_empty();
        session.select(from).await?;
        if !done {
            set_step(store, s, STEP_COPY_STARTED, Some(target.uidnext))?;
            session.uid_copy(uids, to).await?;
        }
        set_step(store, s, STEP_COPIED, None)?;
    }
    if s.step < STEP_FLAGGED_DELETED {
        session
            .uid_store_flag(uids, SystemFlag::Deleted, true)
            .await?;
        set_step(store, s, STEP_FLAGGED_DELETED, None)?;
    }
    if session.supports_uid_expunge() {
        match session.uid_expunge(uids).await {
            Ok(()) => {}
            Err(e) if e.is_transient() => return Err(e),
            // The original is already flagged \Deleted, which clients hide;
            // without UIDPLUS that's where every move ends anyway.
            Err(_) => tracing::warn!(snooze = s.id.0, "UID EXPUNGE refused after the move"),
        }
    }
    Ok(())
}

/// Where a wake puts the message back: its original folder, else the Inbox.
fn wake_destination(store: &Store, s: &Snooze) -> Result<Folder, ChangeError> {
    let folders = store.list_folders(s.account_id).map_err(store_err)?;
    folders
        .iter()
        .find(|f| Some(f.id) == s.original_folder_id)
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
        .cloned()
        .ok_or(ChangeError::Refused { transient: false })
}

/// The server refused after the COPY (decision on #423): remove the copy
/// from the target, so the server matches what `undo` restores locally.
/// The original was never flagged, so it is still where it was.
async fn roll_back(store: &Store, session: &mut Session, s: &Snooze) -> Result<(), ChangeError> {
    let target = match s.state {
        SnoozeState::ToWake => wake_destination(store, s)?,
        _ => match snoozed_folder(store, s.account_id).map_err(store_err)? {
            Some(f) => f,
            None => return Ok(()),
        },
    };
    session.select(&target.details.remote_name).await?;
    let copies = find_copies(session, s).await?;
    if !copies.is_empty() {
        session
            .uid_store_flag(&copies, SystemFlag::Deleted, true)
            .await?;
        if session.supports_uid_expunge() {
            session.uid_expunge(&copies).await?;
        }
    }
    Ok(())
}

/// The newest of several UIDs with the same Message-ID (two copies left by
/// an older client, say): only that one is moved; the others are left
/// where they are and logged by count only (#423).
fn newest(s: &Snooze, uids: Vec<u32>) -> Vec<u32> {
    if uids.len() > 1 {
        tracing::warn!(
            snooze = s.id.0,
            extra = uids.len() - 1,
            "several messages share this Message-ID: moving the newest"
        );
    }
    uids.into_iter().max().into_iter().collect()
}

async fn snooze_one(
    store: &Store,
    session: &mut Session,
    s: &Snooze,
) -> Result<Vec<FolderId>, ChangeError> {
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
        None => newest(
            s,
            session.uid_search_message_id(&s.message_id_header).await?,
        ),
    };
    let new = format_keyword(s.wake_at);
    if uids.is_empty() {
        if s.step >= STEP_COPIED {
            // The original is gone after the COPY (expunged before the
            // connection dropped, or by another client): the copy is the
            // message now, so the move is done. Rolling back would lose it.
            store
                .finish_snooze(s.id, info.custom_keywords && new.is_some())
                .map_err(store_err)?;
            return Ok(vec![source.id, snoozed.id]);
        }
        // The message is gone from the server.
        return Err(ChangeError::Refused { transient: false });
    }
    // Keywords go on before the COPY, so the copy carries them; a resumed
    // move doesn't set them again.
    if info.custom_keywords && s.step == STEP_NONE {
        let old = old_keywords(store, &source, s)?;
        for k in old.iter().filter(|k| Some(*k) != new.as_ref()) {
            session.uid_store_keyword(&uids, k, false).await?;
        }
        if let Some(k) = &new {
            session.uid_store_keyword(&uids, k, true).await?;
        }
    }
    if source.id != snoozed.id {
        move_uids(
            store,
            session,
            s,
            &uids,
            &source.details.remote_name,
            &snoozed.details.remote_name,
        )
        .await?;
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
) -> Result<Vec<FolderId>, ChangeError> {
    let Some(snoozed) = snoozed_folder(store, s.account_id).map_err(store_err)? else {
        // No Snoozed folder: nothing to move back.
        store.delete_snooze(s.id, false).map_err(store_err)?;
        return Ok(Vec::new());
    };
    let local = store
        .find_message_by_header(snoozed.id, &s.message_id_header)
        .map_err(store_err)?;
    let info = session.select(&snoozed.details.remote_name).await?;
    let uids = newest(
        s,
        session.uid_search_message_id(&s.message_id_header).await?,
    );
    if uids.is_empty() {
        // Already woken (by another install) or moved away by the user.
        if let Some(id) = local {
            store.delete_message_row(id).map_err(store_err)?;
        }
        store.delete_snooze(s.id, false).map_err(store_err)?;
        return Ok(vec![snoozed.id]);
    }
    // Keyword and \Seen changes go on before the COPY; a resumed move
    // doesn't repeat them.
    if info.custom_keywords && s.step == STEP_NONE {
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
    if s.mark_unread && s.step == STEP_NONE {
        session
            .uid_store_flag(&uids, SystemFlag::Seen, false)
            .await?;
    }
    let dest = wake_destination(store, s)?;
    move_uids(
        store,
        session,
        s,
        &uids,
        &snoozed.details.remote_name,
        &dest.details.remote_name,
    )
    .await?;
    if let Some(id) = local {
        store.delete_message_row(id).map_err(store_err)?;
    }
    store.delete_snooze(s.id, false).map_err(store_err)?;
    Ok(vec![snoozed.id, dest.id])
}
