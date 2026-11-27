//! Carrying snoozes out on the server (M4-06).
//!
//! # The model (#444)
//!
//! A snooze row is in one of three states, and while it's moving a message
//! its `step` says how far the move got (store migration 0023):
//!
//! | state | step | on the server |
//! |---|---|---|
//! | `to_snooze` | 0 | nothing done yet; the original is where it was |
//! | `to_snooze`/`to_wake` | 1 | target UIDNEXT recorded; a COPY may have happened |
//! | `to_snooze`/`to_wake` | 2 | copied; the original is untouched |
//! | `to_snooze`/`to_wake` | 3 | **committed**: the copy is in the target and the original is gone (MOVE) or flagged `\Deleted` |
//! | `to_snooze`/`to_wake` | 4 | refused after the COPY: the copy is to be removed (rollback) |
//! | `snoozed` | 0 | in Snoozed, with the wake keyword when allowed |
//! | `to_wake` | 0 | due; still in Snoozed with its keyword (a wake changes nothing before its move) |
//!
//! Transitions, per server step: 0 → 1 (UIDNEXT recorded, just before the
//! COPY or MOVE is sent) → 2 (COPY) → 3 (`\Deleted`, or the MOVE's reply)
//! → done. Step ≥ 1 is what tells a later sync that *this* install may have
//! moved the message. A snooze sets its keyword
//! on the original before the COPY; a wake removes it, and clears `\Seen`
//! when asked, on the copy after the move. Then:
//!
//! - a dropped connection at any step: the row stays and the next sync
//!   resumes from the saved step, never repeating a COPY;
//! - a refusal at step 0: a snooze is undone locally; a wake keeps its row
//!   and is retried with back-off;
//! - a refusal at steps 1–2: rollback (step 4), removing the copy, then the
//!   same as a refusal at step 0;
//! - a refusal at step 3, or a missing original after the COPY: the move is
//!   carried forward (finished), since the copy is the message now;
//! - a refusal that lasts ([`GIVE_UP_FAILURES`] syncs and [`GIVE_UP_SECS`],
//!   whichever is later): given up.
//!
//! Snoozing a message again (#431, lead's decision on #455): while its
//! wake is past step 0, the move back has started on the server, so a new
//! snooze would cross it. It is refused (`SnoozeError::Waking`, nothing
//! changed, atomically with the sync in `Store::snooze_messages`); the
//! message shows as waking, and the user snoozes it again once it lands,
//! a few seconds later. At step 0 nothing has moved: the new snooze
//! replaces the wake, and since the message is in Snoozed, only its
//! keyword changes.
//!
//! Invariants, after every completed step:
//!
//! 1. Once a move is finished or undone, the server has the message in
//!    exactly one live (not `\Deleted`) place, except after a given-up
//!    rollback, where a copy can be left. (Mid-move, at step 2, there are
//!    two until the move commits or rolls back.)
//! 2. A snooze keyword is on a message in the Snoozed folder (where other
//!    installs learn wake times from) only while a snooze row for it exists:
//!    a finished wake strips it from the copy and from a flagged original,
//!    and every give-up strips it, so nothing re-learns it. (A refused
//!    snooze is undone with the keyword still on the original in its own
//!    folder, where nothing reads it.)
//! 3. A given-up snooze or wake leaves the message visible on this device,
//!    with no row and no keyword, wherever the server has it.
//! 4. Retries back off, and stop at the give-up rule.
//! 5. A message has at most one snooze row, and no new snooze starts while
//!    a wake is moving it.

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

/// A rollback that keeps failing is given up after this many failed syncs
/// and this long, whichever comes later (lead's decision on #432): the
/// message is shown again where the server has it, since a duplicate there
/// is a lesser harm than a message that stays hidden for good.
/// The same rule ends a wake the server keeps refusing (#444).
const GIVE_UP_FAILURES: u32 = 5;
const GIVE_UP_SECS: i64 = 24 * 3600;

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
        // `None`: a refused move was rolled back on the server. Each move
        // runs boxed, so this loop's future stays small however much a move
        // holds across its awaits.
        let result = if s.step == STEP_ROLLBACK {
            Box::pin(roll_back(store, session, s)).await.map(|()| None)
        } else if s.state == SnoozeState::ToSnooze {
            Box::pin(snooze_one(store, session, s)).await.map(Some)
        } else {
            Box::pin(wake_one(store, session, s)).await.map(Some)
        };
        let result = match result {
            // Refused after the COPY started: remove the copy, then undo.
            Err(e) if !e.is_transient() && !matches!(e, ChangeError::Imap(ImapError::Store(_))) => {
                let step = store
                    .get_snooze_by_id(s.id)?
                    .map_or(STEP_NONE, |fresh| fresh.step);
                if step == STEP_FLAGGED_DELETED {
                    // The copy is in the target and the original flagged
                    // \Deleted: carry the move forward, don't undo it.
                    tracing::warn!(snooze = s.id.0, state = ?s.state, "refused after the original was flagged: finishing the move");
                    // The row as stored now: this sync may have recorded
                    // the target's UIDNEXT on it.
                    let fresh = store.get_snooze_by_id(s.id)?.unwrap_or_else(|| s.clone());
                    carry_forward(store, session, &fresh).await.map(Some)
                } else if matches!(step, STEP_COPY_STARTED | STEP_COPIED | STEP_ROLLBACK) {
                    tracing::warn!(snooze = s.id.0, state = ?s.state, step, "refused after the copy: rolling back");
                    store.start_snooze_rollback(s.id, now)?;
                    let fresh = store.get_snooze_by_id(s.id)?.unwrap_or_else(|| s.clone());
                    roll_back(store, session, &fresh).await.map(|()| None)
                } else {
                    Err(e)
                }
            }
            other => other,
        };
        match result {
            // Changed by the user during this sync before anything was
            // sent (`start_move`): not counted; the next sync carries the
            // row out as it is now.
            Ok(Some(changed))
                if changed.is_empty()
                    && store
                        .get_snooze_by_id(s.id)?
                        .is_some_and(|now| now.state != s.state) => {}
            Ok(Some(changed)) => {
                match s.state {
                    SnoozeState::ToSnooze => report.snoozed += 1,
                    _ => report.woken += 1,
                }
                for folder_id in changed {
                    listener.on_event(CoreEvent::MessagesChanged { folder_id });
                }
            }
            Ok(None) if s.state == SnoozeState::ToWake => {
                tracing::warn!(snooze = s.id.0, "wake rolled back on the server");
                let outcome = refused_wake(store, session, s, now, listener).await;
                tally(store, s, now, outcome, &mut report)?;
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
                report.remaining = report
                    .remaining
                    .saturating_add(work.len().saturating_sub(index));
                break;
            }
            Err(_) if store.get_snooze_by_id(s.id)?.map(|f| f.step) == Some(STEP_ROLLBACK) => {
                let fresh = store.get_snooze_by_id(s.id)?.unwrap_or_else(|| s.clone());
                let failures = fresh.attempts.saturating_add(1);
                if give_up_due(&fresh, now) {
                    tracing::warn!(
                        snooze = s.id.0,
                        failures,
                        "rollback given up: the message is shown again"
                    );
                    let outcome = give_up(store, session, &fresh, listener)
                        .await
                        .map(|()| true);
                    tally(store, s, now, outcome, &mut report)?;
                } else {
                    // Keep the row for the next sync rather than leave a
                    // copy behind silently.
                    tracing::warn!(
                        snooze = s.id.0,
                        failures,
                        "rollback refused: retried on the next sync"
                    );
                    store.defer_snooze(s.id, now.saturating_add(backoff_secs(failures)))?;
                    report.remaining = report.remaining.saturating_add(1);
                }
            }
            Err(_) if s.state == SnoozeState::ToWake => {
                // Nothing was changed on the server before the move (#436),
                // so the message is still snoozed there, keyword and all.
                tracing::warn!(snooze = s.id.0, "wake refused by the server");
                let outcome = refused_wake(store, session, s, now, listener).await;
                tally(store, s, now, outcome, &mut report)?;
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

#[cfg(test)]
mod race_tests;

fn store_err(e: StoreError) -> ChangeError {
    ChangeError::Imap(ImapError::Store(e))
}

/// Refused at step 3 (committed): the copy is in the target and the
/// original gone or flagged `\Deleted`, so the move is done in all but the
/// expunge. For a wake, the copy is cleaned and the original's keyword
/// removed, as far as the server allows (invariant 2). Then it is recorded
/// as done, as the end of `snooze_one` / `wake_one` would.
async fn carry_forward(
    store: &Store,
    session: &mut Session,
    s: &Snooze,
) -> Result<Vec<FolderId>, ChangeError> {
    let snoozed = snoozed_folder(store, s.account_id).map_err(store_err)?;
    if s.state == SnoozeState::ToWake {
        if let Some(folder) = &snoozed {
            best_effort(s, strip_original_keywords(session, s, folder).await)?;
        }
        if let Ok(dest) = wake_destination(store, s) {
            best_effort(s, unsnooze_copy(session, s, &dest).await)?;
        }
    }
    match s.state {
        SnoozeState::ToWake => {
            if let Some(folder) = &snoozed {
                if let Some(id) = store
                    .find_message_by_header(folder.id, &s.message_id_header)
                    .map_err(store_err)?
                {
                    store.delete_message_row(id).map_err(store_err)?;
                }
            }
            store.delete_snooze(s.id, false).map_err(store_err)?;
        }
        // Whether the wake keyword reached the server isn't known here, so
        // the wake time is kept on this device.
        _ => store.finish_snooze(s.id, false).map_err(store_err)?,
    }
    Ok(snoozed
        .map(|f| f.id)
        .into_iter()
        .chain(s.original_folder_id)
        .collect())
}

/// A wake the server refused (or that was rolled back): the message is
/// still in Snoozed with its keyword (#436). The row stays, back at step 0
/// and deferred with back-off, and the message is shown in Snoozed until
/// the wake gets through.
fn keep_wake(
    store: &Store,
    s: &Snooze,
    now: i64,
    listener: &dyn EventListener,
) -> Result<(), StoreError> {
    store.reset_snooze_step(s.id, now)?;
    let attempts = s.attempts.saturating_add(1);
    store.defer_snooze(s.id, now.saturating_add(backoff_secs(attempts)))?;
    if let Some(folder) = snoozed_folder(store, s.account_id)? {
        if let Some(id) = store.find_message_by_header(folder.id, &s.message_id_header)? {
            store.set_message_hidden(id, false)?;
            listener.on_event(CoreEvent::MessagesChanged {
                folder_id: folder.id,
            });
        }
    }
    Ok(())
}

/// Counts what a refused wake or a give-up came to. A dropped connection
/// on the way (the server calls of a give-up) leaves the row as it was,
/// deferred with back-off, like any other transient failure (invariant 4).
fn tally(
    store: &Store,
    s: &Snooze,
    now: i64,
    outcome: Result<bool, ChangeError>,
    report: &mut SnoozeReport,
) -> Result<(), SyncError> {
    match outcome {
        Ok(true) => report.failed += 1,
        Ok(false) => report.remaining = report.remaining.saturating_add(1),
        Err(ChangeError::Imap(ImapError::Store(e))) => return Err(SyncError::Store(e)),
        Err(_) => {
            let attempts = s.attempts.saturating_add(1);
            store.defer_snooze(s.id, now.saturating_add(backoff_secs(attempts)))?;
            report.remaining = report.remaining.saturating_add(1);
        }
    }
    Ok(())
}

/// Whether a run of refusals has lasted long enough to give up: at least
/// [`GIVE_UP_FAILURES`] syncs and [`GIVE_UP_SECS`] since it began.
fn give_up_due(s: &Snooze, now: i64) -> bool {
    let failures = s.attempts.saturating_add(1);
    let since = s.rollback_since.unwrap_or(now);
    failures >= GIVE_UP_FAILURES && now.saturating_sub(since) >= GIVE_UP_SECS
}

/// A wake the server refused (or rolled back): kept and retried with
/// back-off, or given up once [`give_up_due`]. Returns whether it was given
/// up.
async fn refused_wake(
    store: &Store,
    session: &mut Session,
    s: &Snooze,
    now: i64,
    listener: &dyn EventListener,
) -> Result<bool, ChangeError> {
    let fresh = store
        .get_snooze_by_id(s.id)
        .map_err(store_err)?
        .unwrap_or_else(|| s.clone());
    if give_up_due(&fresh, now) {
        tracing::warn!(
            snooze = s.id.0,
            "wake given up: the message is shown in Snoozed"
        );
        give_up(store, session, &fresh, listener).await?;
        return Ok(true);
    }
    keep_wake(store, &fresh, now, listener).map_err(store_err)?;
    Ok(false)
}

/// Gives a snooze or wake up (invariant 3): strips the snooze keyword from
/// whatever the server holds of it, as far as the server allows, so no
/// install re-learns it, then shows the message again with no row.
async fn give_up(
    store: &Store,
    session: &mut Session,
    s: &Snooze,
    listener: &dyn EventListener,
) -> Result<(), ChangeError> {
    if let Some(folder) = snoozed_folder(store, s.account_id).map_err(store_err)? {
        let stripped = match s.state {
            // The original is still in Snoozed, and a copy may sit in the
            // destination (a rollback that couldn't remove it).
            SnoozeState::ToWake => {
                let first = strip_original_keywords(session, s, &folder).await;
                if let Ok(dest) = wake_destination(store, s) {
                    best_effort(s, unsnooze_copy(session, s, &dest).await)?;
                }
                first
            }
            // A copy in Snoozed that couldn't be removed.
            _ => fix_copy_keyword(session, s, &folder, None).await,
        };
        best_effort(s, stripped)?;
    }
    undo(store, s, listener).map_err(store_err)
}

/// A server change that is wanted but not required: a refusal is logged
/// (by snooze id only) and ignored; a dropped connection or a local
/// database failure still comes back as an error.
fn best_effort(s: &Snooze, result: Result<(), ChangeError>) -> Result<(), ChangeError> {
    match result {
        Err(e) if e.is_transient() || matches!(e, ChangeError::Imap(ImapError::Store(_))) => Err(e),
        Err(_) => {
            tracing::warn!(snooze = s.id.0, "a cleanup step was refused by the server");
            Ok(())
        }
        Ok(()) => Ok(()),
    }
}

/// Removes every snooze keyword from the messages in Snoozed with this
/// snooze's Message-ID: a wake's original, flagged `\Deleted` but not yet
/// expunged, or one a give-up leaves behind (invariant 2).
async fn strip_original_keywords(
    session: &mut Session,
    s: &Snooze,
    snoozed: &Folder,
) -> Result<(), ChangeError> {
    let info = session.select(&snoozed.details.remote_name).await?;
    if !info.custom_keywords {
        return Ok(());
    }
    for uid in session.uid_search_message_id(&s.message_id_header).await? {
        strip_keywords(session, uid).await?;
    }
    Ok(())
}

/// Removes every snooze keyword from `uid` in the selected folder.
async fn strip_keywords(session: &mut Session, uid: u32) -> Result<(), ChangeError> {
    let fetched = session.uid_fetch_flags(uid, uid, None).await?;
    let keywords: Vec<String> = fetched
        .updates
        .into_iter()
        .filter(|u| u.uid == uid)
        .flat_map(|u| u.flags)
        .filter(|f| f.starts_with(KEYWORD_PREFIX))
        .collect();
    for k in &keywords {
        session.uid_store_keyword(&[uid], k, false).await?;
    }
    Ok(())
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

/// Step 1, if the row is still what this sync read: false if the user
/// changed it meanwhile (snoozed a waking message again at step 0, or
/// unsnoozed it), and then nothing is sent; the next sync carries out the
/// row as it is now.
fn start_move(store: &Store, s: &Snooze, uidnext: u32) -> Result<bool, ChangeError> {
    let started = store
        .start_snooze_move(s.id, s.state, uidnext)
        .map_err(store_err)?;
    if !started {
        tracing::info!(
            snooze = s.id.0,
            "changed during the sync: left for the next one"
        );
    }
    Ok(started)
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
) -> Result<bool, ChangeError> {
    // MOVE only for a fresh move: one the COPY fallback started (on a node
    // without MOVE) must be resumed, or the earlier copy would stay.
    if session.supports_move() && s.step == STEP_NONE {
        // RFC 6851: atomic. The target's UIDNEXT and step 1 are recorded
        // first, so a later sync that finds the original gone knows this
        // install may have moved it, and where to find the copy (#444).
        let target = session.select(to).await?;
        if !start_move(store, s, target.uidnext)? {
            return Ok(false);
        }
        session.select(from).await?;
        return session.uid_move(uids, to).await.map(|()| true);
    }
    if s.step < STEP_COPIED {
        // The target's UIDNEXT tells our copy from older messages.
        let target = session.select(to).await?;
        // A COPY sent before a dropped connection may have happened.
        let done = s.step == STEP_COPY_STARTED && !find_copies(session, s).await?.is_empty();
        session.select(from).await?;
        if !done {
            if !start_move(store, s, target.uidnext)? {
                return Ok(false);
            }
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
            // The copy is in the target and the original flagged \Deleted,
            // so the move is carried forward (as at step 3, lead's decision
            // on #432): clients hide \Deleted, and without UIDPLUS that's
            // where every move ends anyway. Expunging it ourselves later
            // would need a plain EXPUNGE, which removes other clients'
            // \Deleted mail too.
            Err(_) => tracing::warn!(snooze = s.id.0, "UID EXPUNGE refused after the move"),
        }
    }
    Ok(true)
}

/// On the copy in `snoozed`: removes every snooze keyword but `new`, and
/// adds `new`. Only copies made by this move are touched (`find_copies`).
async fn fix_copy_keyword(
    session: &mut Session,
    s: &Snooze,
    snoozed: &Folder,
    new: Option<&str>,
) -> Result<(), ChangeError> {
    session.select(&snoozed.details.remote_name).await?;
    for uid in find_copies(session, s).await? {
        let fetched = session.uid_fetch_flags(uid, uid, None).await?;
        let flags: Vec<String> = fetched
            .updates
            .into_iter()
            .filter(|u| u.uid == uid)
            .flat_map(|u| u.flags)
            .collect();
        for k in flags
            .iter()
            .filter(|k| k.starts_with(KEYWORD_PREFIX) && Some(k.as_str()) != new)
        {
            session.uid_store_keyword(&[uid], k, false).await?;
        }
        if let Some(k) = new {
            if !flags.iter().any(|f| f == k) {
                session.uid_store_keyword(&[uid], k, true).await?;
            }
        }
    }
    Ok(())
}

/// After a wake's move: on the copy in `dest`, removes every snooze keyword
/// and, when the snooze asks for it, `\Seen` (#436). Only copies made by
/// this wake are touched (`find_copies`).
async fn unsnooze_copy(
    session: &mut Session,
    s: &Snooze,
    dest: &Folder,
) -> Result<(), ChangeError> {
    let info = session.select(&dest.details.remote_name).await?;
    for uid in find_copies(session, s).await? {
        if info.custom_keywords {
            let fetched = session.uid_fetch_flags(uid, uid, None).await?;
            let keywords: Vec<String> = fetched
                .updates
                .into_iter()
                .filter(|u| u.uid == uid)
                .flat_map(|u| u.flags)
                .filter(|f| f.starts_with(KEYWORD_PREFIX))
                .collect();
            for k in &keywords {
                session.uid_store_keyword(&[uid], k, false).await?;
            }
        }
        if s.mark_unread {
            session
                .uid_store_flag(&[uid], SystemFlag::Seen, false)
                .await?;
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
    match session.select(&target.details.remote_name).await {
        // The target folder is gone (deleted on another device): the copy
        // went with it.
        Err(ImapError::CommandFailed) => return Ok(()),
        other => {
            other?;
        }
    }
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
        // The original is gone. After the COPY started, look for this
        // move's copy first: if it's there, it is the message now, and the
        // move is carried forward. Refusing would roll back and lose it.
        let copied = s.step >= STEP_COPIED
            || (s.step == STEP_COPY_STARTED && {
                session.select(&snoozed.details.remote_name).await?;
                !find_copies(session, s).await?.is_empty()
            });
        if copied {
            // Its keyword gets the current wake time, as after any resumed
            // move (#436).
            if info.custom_keywords {
                fix_copy_keyword(session, s, &snoozed, new.as_deref()).await?;
            }
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
        let moved = move_uids(
            store,
            session,
            s,
            &uids,
            &source.details.remote_name,
            &snoozed.details.remote_name,
        )
        .await?;
        if !moved {
            return Ok(Vec::new());
        }
        if s.step != STEP_NONE && info.custom_keywords {
            // A resumed move: the wake time may have changed since the
            // keyword went on before the COPY (#432). Put the current one
            // on the copy.
            fix_copy_keyword(session, s, &snoozed, new.as_deref()).await?;
        }
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
    session.select(&snoozed.details.remote_name).await?;
    let uids = newest(
        s,
        session.uid_search_message_id(&s.message_id_header).await?,
    );
    // A committed move (step 3) whose original is still there, flagged
    // \Deleted (no UIDPLUS, or the expunge refused): only the clean-up is
    // left.
    if uids.is_empty() || s.step >= STEP_FLAGGED_DELETED {
        // Moved before a dropped connection (this install recorded where
        // to), or already woken by another install, or moved away by the
        // user. In the first case the copy still needs its keyword and
        // \Seen changes. A missing destination doesn't stop this.
        let mut changed = vec![snoozed.id];
        // Only a move this install started (step ≥ 1) is cleaned up: an
        // empty Snoozed with no step of ours means another install woke it,
        // or the user moved it, and its copy isn't ours to change.
        if s.target_uidnext.is_some() && s.step >= STEP_COPY_STARTED {
            best_effort(s, strip_original_keywords(session, s, &snoozed).await)?;
            if let Ok(dest) = wake_destination(store, s) {
                best_effort(s, unsnooze_copy(session, s, &dest).await)?;
                changed.push(dest.id);
            }
        }
        if let Some(id) = local {
            store.delete_message_row(id).map_err(store_err)?;
        }
        store.delete_snooze(s.id, false).map_err(store_err)?;
        return Ok(changed);
    }
    let dest = wake_destination(store, s)?;
    // Nothing on the server changes before the move (#436): the keyword
    // removal and the \Seen change are made on the copy in `dest` after
    // it, so a refused or dropped wake leaves the message as it was. The
    // copy is found by Message-ID at or above `dest`'s UIDNEXT, recorded
    // here first (for MOVE too).
    // `move_uids` records the destination's UIDNEXT and step 1 before it
    // sends the MOVE or COPY; the copy is found at or above that UIDNEXT.
    let moved = move_uids(
        store,
        session,
        s,
        &uids,
        &snoozed.details.remote_name,
        &dest.details.remote_name,
    )
    .await?;
    if !moved {
        return Ok(Vec::new());
    }
    let s = store
        .get_snooze_by_id(s.id)
        .map_err(store_err)?
        .unwrap_or_else(|| s.clone());
    // Committed: from here every exit finishes the wake (step 3).
    set_step(store, &s, STEP_FLAGGED_DELETED, None)?;
    // A flagged original that wasn't expunged must not keep its keyword
    // (invariant 2).
    best_effort(&s, strip_original_keywords(session, &s, &snoozed).await)?;
    unsnooze_copy(session, &s, &dest).await?;
    if let Some(id) = local {
        store.delete_message_row(id).map_err(store_err)?;
    }
    store.delete_snooze(s.id, false).map_err(store_err)?;
    Ok(vec![snoozed.id, dest.id])
}
