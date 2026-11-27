//! What a sync sends before it fetches: the device's own changes, in one
//! fixed order, so the app and the tests run the same thing (#459).

use crate::events::EventListener;
use mailcrane_imap::Session;
use mailcrane_store::{AccountId, Store};

/// Sends `account`'s local changes to the server, in this order: queued
/// actions (moves, deletes, flags), drafts (M2-05), copies of sent mail
/// for the Sent folder (M2-07), then snoozes and wakes (M4-06).
///
/// The order matters to snoozes: a move or delete the user made is
/// replayed before any wake runs. A move or delete of a message whose wake
/// is past step 0 is refused when it's made (lead's decision on G6), so a
/// queued one never crosses a wake.
///
/// Each step defers or undoes what the server refuses, so none of them
/// stops the next; a local database failure is reported by the sync that
/// follows, which reads the same database.
pub async fn push_local_changes(
    store: &Store,
    session: &mut Session,
    listener: &dyn EventListener,
    account: AccountId,
    now: i64,
) {
    let _ = crate::actions::replay(store, session, listener, account, now).await;
    let _ = crate::drafts::upload_drafts(store, session, account, now).await;
    let _ = crate::outbox::save_sent_copies(store, session, account, now).await;
    let _ = crate::snooze::sync_snoozes(store, session, listener, account, now).await;
}
