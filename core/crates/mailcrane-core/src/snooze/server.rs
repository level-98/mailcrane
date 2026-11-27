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
//! 5. Every write that finishes, advances or deletes a snooze row is
//!    conditional on the row being exactly as the sync last saw it
//!    (`SnoozeSeen`: state, step, wake time, mark-unread, target UIDNEXT,
//!    copy UID; #459). If the user changed it meanwhile, the write does
//!    nothing, the sync stops there, and the next sync reads the row again
//!    and decides afresh.
//! 6. A message has at most one snooze row. While its wake is past step 0,
//!    no new snooze starts (#431), and no move or delete is accepted
//!    (lead's decision on G6, refused in the same transaction that reads
//!    the row, `StoreError::Waking`): the user acts again once it lands, a
//!    few seconds later. A wake the server refuses goes back to step 0, so
//!    nothing stays locked; one held up by dropped connections stays past
//!    step 0 with back-off and isn't given up (the move couldn't reach
//!    that server either).
//!
//! The copy a move made is cleaned up by its exact UID when the server
//! names it (COPYUID, RFC 4315 §3, `copy_uid`, #450); otherwise by
//! Message-ID at or above the recorded UIDNEXT. `\Seen` is cleared only on
//! a copy surely this install's (a named UID, or step 2 or 3): resuming at
//! step 1, the copy found could be another install's.
//!
//! A sync sends queued actions before snoozes (`push_local_changes`), so a
//! move or delete accepted at step 0 reaches the server before the wake,
//! which then finds Snoozed empty and cleans up nothing that isn't its own.

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
    let work: Vec<Snooze> = store
        .list_snoozes(Some(account))?
        .into_iter()
        .filter(|s| s.state != SnoozeState::Snoozed && s.next_attempt_at <= now)
        .collect();
    sync_snooze_rows(store, session, listener, now, work).await
}

/// [`sync_snoozes`] on `work`, the rows as the caller read them. The fault
/// table (#459) reads them, lets the user change one, then hands the old
/// copy here: the race between a sync's read and its writes.
pub(crate) async fn sync_snooze_rows(
    store: &Store,
    session: &mut Session,
    listener: &dyn EventListener,
    now: i64,
    work: Vec<Snooze>,
) -> Result<SnoozeReport, SyncError> {
    let mut report = SnoozeReport::default();
    if work.is_empty() {
        return Ok(report);
    }
    for (index, s) in work.iter().enumerate() {
        if s.state == SnoozeState::Snoozed {
            continue;
        }
        // The row as this sync expects it, kept up to date with its own
        // writes: every write that finishes, advances or deletes it
        // requires the stored row to match, or changes nothing
        // (invariant 5, #459).
        let mut cur = s.clone();
        // `None`: a refused move was rolled back on the server. Each move
        // runs boxed, so this loop's future stays small however much a move
        // holds across its awaits.
        let result = if cur.step == STEP_ROLLBACK {
            Box::pin(roll_back(store, session, &cur))
                .await
                .map(|()| None)
        } else if cur.state == SnoozeState::ToSnooze {
            Box::pin(snooze_one(store, session, &mut cur))
                .await
                .map(Some)
        } else {
            Box::pin(wake_one(store, session, &mut cur)).await.map(Some)
        };
        let result = match result {
            // Refused after the COPY started: remove the copy, then undo.
            Err(e) if !e.is_transient() && !matches!(e, ChangeError::Imap(ImapError::Store(_))) => {
                if cur.step == STEP_FLAGGED_DELETED {
                    // The copy is in the target and the original flagged
                    // \Deleted: carry the move forward, don't undo it.
                    tracing::warn!(snooze = s.id.0, state = ?s.state, "refused after the original was flagged: finishing the move");
                    Box::pin(carry_forward(store, session, &cur))
                        .await
                        .map(Some)
                } else if matches!(cur.step, STEP_COPY_STARTED | STEP_COPIED | STEP_ROLLBACK) {
                    tracing::warn!(snooze = s.id.0, state = ?s.state, step = cur.step, "refused after the copy: rolling back");
                    if store.start_snooze_rollback_if(&cur.seen(), now)? {
                        cur.step = STEP_ROLLBACK;
                        Box::pin(roll_back(store, session, &cur))
                            .await
                            .map(|()| None)
                    } else {
                        // Changed by the user meanwhile: the next sync
                        // reads it again and decides.
                        Ok(Some(Vec::new()))
                    }
                } else {
                    Err(e)
                }
            }
            other => other,
        };
        match result {
            // Changed by the user during this sync (a guarded write didn't
            // apply, or nothing was sent): not counted; the next sync
            // carries out the row as it is now. A row that's gone counts
            // as changed too (review of #455).
            Ok(Some(changed))
                if changed.is_empty()
                    && store
                        .get_snooze_by_id(s.id)?
                        .is_none_or(|now| now.seen() != cur.seen()) => {}
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
                let outcome = Box::pin(refused_wake(store, session, &cur, now, listener)).await;
                tally(store, &cur, now, outcome, &mut report)?;
            }
            Ok(None) => {
                tracing::warn!(snooze = s.id.0, state = ?s.state, "rolled back on the server");
                undo(store, &cur, listener)?;
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
            Err(_) if cur.step == STEP_ROLLBACK => {
                // As stored now, for its failure count; if the user changed
                // it meanwhile, the next sync decides.
                let Some(fresh) = store
                    .get_snooze_by_id(s.id)?
                    .filter(|f| f.seen() == cur.seen())
                else {
                    continue;
                };
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
                    tally(store, &cur, now, outcome, &mut report)?;
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
                let outcome = Box::pin(refused_wake(store, session, &cur, now, listener)).await;
                tally(store, &cur, now, outcome, &mut report)?;
            }
            Err(_) => {
                tracing::warn!(snooze = s.id.0, state = ?s.state, "snooze refused by the server");
                undo(store, &cur, listener)?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod fault_tests;
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
            best_effort(s, Box::pin(unsnooze_copy(session, s, &dest)).await)?;
        }
    }
    let done = match s.state {
        SnoozeState::ToWake => {
            let local = match &snoozed {
                Some(folder) => store
                    .find_message_by_header(folder.id, &s.message_id_header)
                    .map_err(store_err)?,
                None => None,
            };
            store
                .delete_snooze_if(&s.seen(), false, local)
                .map_err(store_err)?
        }
        // Whether the wake keyword reached the server isn't known here, so
        // the wake time is kept on this device.
        _ => store
            .finish_snooze_if(&s.seen(), false)
            .map_err(store_err)?,
    };
    if !done {
        // Changed by the user meanwhile: the next sync decides.
        return Ok(Vec::new());
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
    if !store.reset_snooze_step_if(&s.seen(), now)? {
        // Changed by the user meanwhile: the next sync decides.
        return Ok(());
    }
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
    // As stored now, for its failure count and when the run began; if the
    // user changed it meanwhile, the next sync decides.
    let Some(fresh) = store
        .get_snooze_by_id(s.id)
        .map_err(store_err)?
        .filter(|f| f.seen() == s.seen())
    else {
        return Ok(false);
    };
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
                    best_effort(s, Box::pin(unsnooze_copy(session, s, &dest)).await)?;
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
        if !store.delete_snooze_if(&s.seen(), false, None)? {
            // Changed by the user meanwhile: the next sync decides.
            return Ok(());
        }
        if let Some(folder) = snoozed_folder(store, s.account_id)? {
            if let Some(id) = store.find_message_by_header(folder.id, &s.message_id_header)? {
                store.set_message_hidden(id, false)?;
                listener.on_event(CoreEvent::MessagesChanged {
                    folder_id: folder.id,
                });
            }
        }
    } else {
        if !store.delete_snooze_if(&s.seen(), true, None)? {
            return Ok(());
        }
        if let Some(folder) = s.original_folder_id {
            listener.on_event(CoreEvent::MessagesChanged { folder_id: folder });
        }
    }
    Ok(())
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

/// Step 1, if the row is still as this sync last saw it: false if the
/// user changed it meanwhile (snoozed a waking message again at step 0,
/// unsnoozed it, read it), and then nothing is sent; the next sync carries
/// out the row as it is now. On success `s` follows the write.
fn start_move(store: &Store, s: &mut Snooze, uidnext: u32) -> Result<bool, ChangeError> {
    let started = store
        .start_snooze_move_if(&s.seen(), uidnext)
        .map_err(store_err)?;
    if started {
        s.step = STEP_COPY_STARTED;
        s.target_uidnext = Some(uidnext);
        s.copy_uid = None;
    } else {
        tracing::info!(
            snooze = s.id.0,
            "changed during the sync: left for the next one"
        );
    }
    Ok(started)
}

/// Records `step`, and the copy's UID when the server named it (COPYUID,
/// #450), if the row is still as this sync last saw it (#459). False: the
/// user changed it meanwhile, so this sync stops here and the next one
/// reads it again and decides. On success `s` follows the write.
fn advance(
    store: &Store,
    s: &mut Snooze,
    step: u8,
    copy_uid: Option<u32>,
) -> Result<bool, ChangeError> {
    let done = store
        .set_snooze_step_if(&s.seen(), step, None, copy_uid)
        .map_err(store_err)?;
    if done {
        s.step = step;
        if copy_uid.is_some() {
            s.copy_uid = copy_uid;
        }
    } else {
        tracing::info!(
            snooze = s.id.0,
            step,
            "changed during the sync: left for the next one"
        );
    }
    Ok(done)
}

/// Whether the row is still as this sync last saw it, checked before a
/// server change that no store write guards (a snooze's keyword STORE
/// before its COPY, #459).
fn still(store: &Store, s: &Snooze) -> Result<bool, ChangeError> {
    Ok(store
        .get_snooze_by_id(s.id)
        .map_err(store_err)?
        .is_some_and(|now| now.seen() == s.seen()))
}

/// Of `uids` in the selected folder, those not flagged `\Deleted` (#459):
/// a woken original whose expunge was refused, or that a server without
/// UID EXPUNGE kept, is no message to move or snooze.
async fn live_uids(session: &mut Session, uids: Vec<u32>) -> Result<Vec<u32>, ChangeError> {
    let mut live = Vec::with_capacity(uids.len());
    for uid in uids {
        let fetched = session.uid_fetch_flags(uid, uid, None).await?;
        let deleted = fetched
            .updates
            .iter()
            .filter(|u| u.uid == uid)
            .any(|u| u.flags.iter().any(|f| f.eq_ignore_ascii_case("\\Deleted")));
        if !deleted {
            live.push(uid);
        }
    }
    Ok(live)
}

/// Whether the copy [`find_copies`] finds for `s` is surely this
/// install's: the server named it (COPYUID), or this install's COPY or
/// MOVE got its reply (step 2 or 3). At step 1 the reply never came, so a
/// copy at or above the recorded UIDNEXT could be another install's (#450).
fn copy_is_ours(s: &Snooze) -> bool {
    s.copy_uid.is_some() || matches!(s.step, STEP_COPIED | STEP_FLAGGED_DELETED)
}

/// UIDs of copies of `s` in the selected folder made by this move: the
/// message's Message-ID, at or above the UIDNEXT recorded before the COPY.
/// An older message with the same Message-ID is never included.
async fn find_copies(session: &mut Session, s: &Snooze) -> Result<Vec<u32>, ChangeError> {
    // The server named the copy (COPYUID, #450): that one, and nothing a
    // search could also match.
    if let Some(uid) = s.copy_uid {
        return Ok(vec![uid]);
    }
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
    s: &mut Snooze,
    uids: &[u32],
    from: &str,
    to: &str,
) -> Result<bool, ChangeError> {
    if session.supports_move() && s.step <= STEP_COPY_STARTED {
        // RFC 6851: atomic. The target's UIDNEXT and step 1 are recorded
        // first, so a later sync that finds the original gone knows this
        // install may have moved it, and where to find the copy (#444).
        let target = session.select(to).await?;
        // At step 1 with the original still here, MOVE never happened (it's
        // atomic), so it is sent again rather than degraded to a COPY
        // (#459). Unless a COPY from the fallback (a node without MOVE)
        // already made a copy: then that move is resumed below.
        let copied = s.step == STEP_COPY_STARTED && !find_copies(session, s).await?.is_empty();
        if !copied {
            if !start_move(store, s, target.uidnext)? {
                return Ok(false);
            }
            session.select(from).await?;
            let copy = session.uid_move_returning(uids, to).await?;
            return advance(store, s, STEP_FLAGGED_DELETED, copy);
        }
        session.select(from).await?;
    }
    if s.step < STEP_COPIED {
        // The target's UIDNEXT tells our copy from older messages.
        let target = session.select(to).await?;
        // A COPY sent before a dropped connection may have happened.
        let done = s.step == STEP_COPY_STARTED && !find_copies(session, s).await?.is_empty();
        session.select(from).await?;
        let mut copy = None;
        if !done {
            if !start_move(store, s, target.uidnext)? {
                return Ok(false);
            }
            copy = session.uid_copy_returning(uids, to).await?;
        }
        if !advance(store, s, STEP_COPIED, copy)? {
            return Ok(false);
        }
    }
    if s.step < STEP_FLAGGED_DELETED {
        session
            .uid_store_flag(uids, SystemFlag::Deleted, true)
            .await?;
        if !advance(store, s, STEP_FLAGGED_DELETED, None)? {
            return Ok(false);
        }
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
    // `\Seen` only on a copy surely this install's (#450): at step 1 the
    // one found could be another install's, which the user may have read.
    let mark_unread = s.mark_unread && copy_is_ours(s);
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
        if mark_unread {
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
    s: &mut Snooze,
) -> Result<Vec<FolderId>, ChangeError> {
    // A move this install started earlier, resumed now.
    let resumed = s.step != STEP_NONE;
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
        // Only live copies: a woken original left flagged \Deleted is no
        // message to snooze, and must not hide the live one (#459).
        None => {
            let found = session.uid_search_message_id(&s.message_id_header).await?;
            newest(s, live_uids(session, found).await?)
        }
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
            if !store
                .finish_snooze_if(&s.seen(), info.custom_keywords && new.is_some())
                .map_err(store_err)?
            {
                return Ok(Vec::new());
            }
            return Ok(vec![source.id, snoozed.id]);
        }
        // The message is gone from the server.
        return Err(ChangeError::Refused { transient: false });
    }
    // Keywords go on before the COPY, so the copy carries them; a resumed
    // move doesn't set them again. Not if the user changed the row
    // meanwhile (#459).
    if info.custom_keywords && s.step == STEP_NONE {
        if !still(store, s)? {
            return Ok(Vec::new());
        }
        let old = old_keywords(store, &source, s)?;
        for k in old.iter().filter(|k| Some(*k) != new.as_ref()) {
            session.uid_store_keyword(&uids, k, false).await?;
        }
        if let Some(k) = &new {
            session.uid_store_keyword(&uids, k, true).await?;
        }
    }
    if source.id != snoozed.id {
        let moved = Box::pin(move_uids(
            store,
            session,
            s,
            &uids,
            &source.details.remote_name,
            &snoozed.details.remote_name,
        ))
        .await?;
        if !moved {
            return Ok(Vec::new());
        }
        if resumed && info.custom_keywords {
            // A resumed move: the wake time may have changed since the
            // keyword went on before the COPY (#432). Put the current one
            // on the copy.
            fix_copy_keyword(session, s, &snoozed, new.as_deref()).await?;
        }
    }
    if !store
        .finish_snooze_if(&s.seen(), info.custom_keywords && new.is_some())
        .map_err(store_err)?
    {
        return Ok(Vec::new());
    }
    Ok(vec![source.id, snoozed.id])
}

async fn wake_one(
    store: &Store,
    session: &mut Session,
    s: &mut Snooze,
) -> Result<Vec<FolderId>, ChangeError> {
    let Some(snoozed) = snoozed_folder(store, s.account_id).map_err(store_err)? else {
        // No Snoozed folder: nothing to move back.
        store
            .delete_snooze_if(&s.seen(), false, None)
            .map_err(store_err)?;
        return Ok(Vec::new());
    };
    let local = store
        .find_message_by_header(snoozed.id, &s.message_id_header)
        .map_err(store_err)?;
    session.select(&snoozed.details.remote_name).await?;
    // Only live copies: an original a server kept flagged \Deleted is no
    // message to move (#459).
    let found = session.uid_search_message_id(&s.message_id_header).await?;
    let live = live_uids(session, found.clone()).await?;
    // Originals this install flagged \Deleted (its move got past the COPY)
    // that are still there: a sync stopped by the guard right after
    // flagging (#459) left them. Expunged below, with UIDPLUS.
    let flagged: Vec<u32> = if s.step >= STEP_COPIED {
        found
            .into_iter()
            .filter(|uid| !live.contains(uid))
            .collect()
    } else {
        Vec::new()
    };
    let uids = newest(s, live);
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
        // or the user moved it, and its copy isn't ours to change. At step
        // 1 `unsnooze_copy` leaves \Seen alone (#450).
        if s.target_uidnext.is_some() && s.step >= STEP_COPY_STARTED {
            best_effort(s, strip_original_keywords(session, s, &snoozed).await)?;
            if !flagged.is_empty() && session.supports_uid_expunge() {
                session.select(&snoozed.details.remote_name).await?;
                best_effort(s, session.uid_expunge(&flagged).await)?;
            }
            if let Ok(dest) = wake_destination(store, s) {
                best_effort(s, Box::pin(unsnooze_copy(session, s, &dest)).await)?;
                changed.push(dest.id);
            }
        }
        if !store
            .delete_snooze_if(&s.seen(), false, local)
            .map_err(store_err)?
        {
            return Ok(Vec::new());
        }
        return Ok(changed);
    }
    let dest = wake_destination(store, s)?;
    // Nothing on the server changes before the move (#436): the keyword
    // removal and the \Seen change are made on the copy in `dest` after
    // it, so a refused or dropped wake leaves the message as it was.
    // `move_uids` records the destination's UIDNEXT and step 1 before it
    // sends the MOVE or COPY, and step 3, with the copy's UID when the
    // server names it, once the move is committed.
    let moved = Box::pin(move_uids(
        store,
        session,
        s,
        &uids,
        &snoozed.details.remote_name,
        &dest.details.remote_name,
    ))
    .await?;
    if !moved {
        return Ok(Vec::new());
    }
    // Committed: from here every exit finishes the wake (step 3).
    // A flagged original that wasn't expunged must not keep its keyword
    // (invariant 2).
    best_effort(s, strip_original_keywords(session, s, &snoozed).await)?;
    Box::pin(unsnooze_copy(session, s, &dest)).await?;
    if !store
        .delete_snooze_if(&s.seen(), false, local)
        .map_err(store_err)?
    {
        return Ok(Vec::new());
    }
    Ok(vec![snoozed.id, dest.id])
}
