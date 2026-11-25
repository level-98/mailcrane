//! The outbox for the apps (M2-06): Send queues the message and tries it at
//! once; the app's background sender calls `send_outbox` until nothing is
//! left, and the outbox screen lists, retries, edits and deletes.

use crate::actions::now;
use crate::compose::{ComposeDraft, SendFailure, SendResult};
use crate::messages::one_line;
use crate::secrets;
use crate::sync::{failed, SyncEngine, SyncFailure};
use mailcrane_core::compose::{self as core, PreparedSend};
use mailcrane_core::events;
use mailcrane_core::outbox::{self, OutboxError, SendOutcome};
use mailcrane_core::secrets::SecretStore;
use mailcrane_store::{AccountId, OutboxId, OutboxState, SentCopy, Store};

/// Where a message in the outbox is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OutboxEntryState {
    /// Waiting to be sent (perhaps after a failed try).
    Queued,
    /// Being sent now.
    Sending,
    /// Refused for good, or retried for a day. Retry, edit or delete it.
    Failed,
    /// Sent, but its copy couldn't be saved to the Sent folder. Retry saves
    /// the copy again (it never resends the message); Delete forgets it.
    NotSaved,
}

/// A message in the outbox, for the outbox screen.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutboxEntry {
    pub id: i64,
    /// Recipients as typed, joined with ", ".
    pub to: String,
    pub subject: String,
    pub state: OutboxEntryState,
    /// The last error as a short code (`network`, `auth`, `refused`, ...);
    /// never server text.
    pub last_error: Option<String>,
    pub attempts: u32,
    /// When it's tried next (Unix seconds), for a queued message.
    pub next_attempt_at: i64,
}

/// Why an outbox call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum OutboxFailure {
    /// The message doesn't exist, or is being sent right now.
    #[error("not found or busy")]
    NotFound,
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

impl From<OutboxError> for OutboxFailure {
    fn from(e: OutboxError) -> Self {
        match e {
            OutboxError::NotFound => Self::NotFound,
            OutboxError::Store(_) => Self::Storage,
        }
    }
}

fn open(db_path: &str) -> Result<Store, OutboxFailure> {
    Store::open(db_path).map_err(|_| OutboxFailure::Storage)
}

fn enqueue_draft(db_path: &str, draft: ComposeDraft) -> Result<OutboxId, SendFailure> {
    let store = Store::open(db_path).map_err(|_| SendFailure::Storage)?;
    let draft: core::ComposeDraft = draft.into();
    Ok(outbox::enqueue(&store, &draft, 0, now())?)
}

/// Claims and prepares a message; in its own function so the `Store` (not
/// `Sync`) is dropped before the send awaits.
fn prepare_one(
    db_path: &str,
    secrets: &dyn SecretStore,
    id: OutboxId,
    options: &core::SendOptions,
) -> Result<Option<PreparedSend>, core::SendError> {
    let store = Store::open(db_path).map_err(|_| core::SendError::Storage)?;
    outbox::prepare(&store, secrets, id, options)
}

fn record_one(
    db_path: &str,
    id: OutboxId,
    result: Result<core::SendReport, core::SendError>,
) -> Result<SendOutcome, core::SendError> {
    let store = Store::open(db_path).map_err(|_| core::SendError::Storage)?;
    outbox::record(&store, id, result, now()).map_err(|_| core::SendError::Storage)
}

fn delete_quietly(db_path: &str, id: OutboxId) {
    if let Ok(store) = Store::open(db_path) {
        let _ = outbox::delete(&store, id);
    }
}

/// Send, as the compose screen calls it: validates and queues the draft,
/// then tries it once.
///
/// - Accepted: `SendResult { rejected, queued: false }`.
/// - Failed for now (offline, busy server): `SendResult { queued: true }`;
///   it stays in the outbox and the background sender retries it.
/// - Invalid or refused for good: the error, and the message is taken out
///   of the outbox again, so the compose screen keeps it to fix and resend.
#[uniffi::export(async_runtime = "tokio")]
pub async fn send_via_outbox(
    db_path: String,
    draft: ComposeDraft,
) -> Result<SendResult, SendFailure> {
    let secrets = secrets::secret_store().map_err(|_| SendFailure::NotReady)?;
    send_via_outbox_with(
        &db_path,
        secrets.as_ref(),
        draft,
        &core::SendOptions::default(),
    )
    .await
}

pub(crate) async fn send_via_outbox_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    draft: ComposeDraft,
    options: &core::SendOptions,
) -> Result<SendResult, SendFailure> {
    let id = enqueue_draft(db_path, draft)?;
    let prepared = match prepare_one(db_path, secrets, id, options) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return Ok(SendResult {
                rejected: Vec::new(),
                queued: true,
            })
        }
        Err(e) => {
            delete_quietly(db_path, id);
            return Err(e.into());
        }
    };
    let result = core::send_prepared(prepared).await;
    match record_one(db_path, id, result)? {
        SendOutcome::Sent { rejected } => Ok(SendResult {
            rejected,
            queued: false,
        }),
        SendOutcome::Queued { .. } => Ok(SendResult {
            rejected: Vec::new(),
            queued: true,
        }),
        SendOutcome::Failed { error } => {
            delete_quietly(db_path, id);
            Err(error.into())
        }
    }
}

/// Undo send (M4-08): validates and queues the draft to go out after
/// `delay_secs` seconds (0: at once, for the background sender). Returns the
/// outbox id for `cancel_pending`. Invalid drafts fail here, as with Send.
#[uniffi::export]
pub fn enqueue_with_delay(
    db_path: String,
    draft: ComposeDraft,
    delay_secs: u32,
) -> Result<i64, SendFailure> {
    let store = Store::open(&db_path).map_err(|_| SendFailure::Storage)?;
    let draft: core::ComposeDraft = draft.into();
    let now = now();
    let not_before = now.saturating_add(i64::from(delay_secs));
    Ok(outbox::enqueue(&store, &draft, not_before, now)?.0)
}

/// Undo send: the draft back, if the message hasn't started sending; then
/// it is no longer in the outbox. `None` means it's too late ("Already
/// sent").
#[uniffi::export]
pub fn cancel_pending(
    db_path: String,
    outbox_id: i64,
) -> Result<Option<ComposeDraft>, OutboxFailure> {
    let store = open(&db_path)?;
    let draft =
        outbox::cancel_pending(&store, OutboxId(outbox_id)).map_err(|_| OutboxFailure::Storage)?;
    Ok(draft.map(|d| ComposeDraft {
        account_id: d.account_id.0,
        to: d.to,
        cc: d.cc,
        bcc: d.bcc,
        subject: d.subject,
        body_text: d.body_text,
        in_reply_to: d.in_reply_to,
        references: d.references,
    }))
}

/// What the background sender should do next.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutboxStatus {
    /// Messages still queued.
    pub remaining: u32,
    /// When the next one is due (Unix seconds), if any.
    pub next_attempt_at: Option<i64>,
}

fn due_ids(db_path: &str, account: AccountId) -> Result<Vec<OutboxId>, SyncFailure> {
    let store = Store::open(db_path).map_err(|_| failed(events::SyncErrorKind::Storage))?;
    Ok(store
        .due_outbox(account, now())
        .map_err(|_| failed(events::SyncErrorKind::Storage))?
        .into_iter()
        .map(|m| m.id)
        .collect())
}

fn status(db_path: &str, account: AccountId) -> Result<OutboxStatus, SyncFailure> {
    let storage = |_| failed(events::SyncErrorKind::Storage);
    let store = Store::open(db_path).map_err(storage)?;
    let remaining = store
        .list_outbox(account)
        .map_err(storage)?
        .iter()
        .filter(|m| m.state == OutboxState::Queued)
        .count();
    Ok(OutboxStatus {
        remaining: u32::try_from(remaining).unwrap_or(u32::MAX),
        next_attempt_at: store.next_outbox_attempt(account).map_err(storage)?,
    })
}

/// Sends the account's queued messages that are due now, oldest first,
/// stopping at the first that fails for now. Call it on start-up, when the
/// network comes back, and at `next_attempt_at`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn send_outbox(db_path: String, account_id: i64) -> Result<OutboxStatus, SyncFailure> {
    let secrets = secrets::secret_store().map_err(|_| failed(events::SyncErrorKind::Auth))?;
    send_outbox_with(
        &db_path,
        secrets.as_ref(),
        AccountId(account_id),
        &core::SendOptions::default(),
    )
    .await
}

pub(crate) async fn send_outbox_with(
    db_path: &str,
    secrets: &dyn SecretStore,
    account: AccountId,
    options: &core::SendOptions,
) -> Result<OutboxStatus, SyncFailure> {
    let storage = |_| failed(events::SyncErrorKind::Storage);
    for id in due_ids(db_path, account)? {
        let prepared = match prepare_one(db_path, secrets, id, options) {
            Ok(Some(p)) => p,
            // Claimed elsewhere, or failed for good (recorded already).
            Ok(None) | Err(_) => continue,
        };
        let result = core::send_prepared(prepared).await;
        if let SendOutcome::Queued { .. } = record_one(db_path, id, result).map_err(storage)? {
            break;
        }
    }
    status(db_path, account)
}

/// The account's unsent messages (queued, sending, failed), oldest first.
#[uniffi::export]
pub fn list_outbox(db_path: String, account_id: i64) -> Result<Vec<OutboxEntry>, OutboxFailure> {
    let list = open(&db_path)?
        .list_outbox(AccountId(account_id))
        .map_err(|_| OutboxFailure::Storage)?;
    Ok(list
        .into_iter()
        .filter_map(|m| {
            let state = match m.state {
                OutboxState::Queued => OutboxEntryState::Queued,
                OutboxState::Sending => OutboxEntryState::Sending,
                OutboxState::Failed => OutboxEntryState::Failed,
                OutboxState::Sent if m.sent_copy == SentCopy::Failed => OutboxEntryState::NotSaved,
                OutboxState::Sent => return None,
            };
            Some(OutboxEntry {
                id: m.id.0,
                to: one_line(
                    &m.fields
                        .to
                        .iter()
                        .chain(&m.fields.cc)
                        .chain(&m.fields.bcc)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                subject: one_line(&m.fields.subject),
                state,
                last_error: m.last_error,
                attempts: m.attempts,
                next_attempt_at: m.next_attempt_at.max(m.not_before),
            })
        })
        .collect())
}

/// Accounts with messages still to send, for the background sender.
#[uniffi::export]
pub fn outbox_accounts(db_path: String) -> Result<Vec<i64>, OutboxFailure> {
    Ok(open(&db_path)?
        .outbox_accounts()
        .map_err(|_| OutboxFailure::Storage)?
        .into_iter()
        .map(|a| a.0)
        .collect())
}

/// Queues a failed message again, to be sent now (for a `NotSaved` one,
/// tries saving its copy to Sent again).
#[uniffi::export]
pub fn retry_outbox(db_path: String, outbox_id: i64) -> Result<(), OutboxFailure> {
    Ok(outbox::retry(&open(&db_path)?, OutboxId(outbox_id), now())?)
}

/// Removes a message from the outbox (not while it's being sent).
#[uniffi::export]
pub fn delete_outbox(db_path: String, outbox_id: i64) -> Result<(), OutboxFailure> {
    outbox::delete(&open(&db_path)?, OutboxId(outbox_id))?;
    Ok(())
}

/// Takes a message out of the outbox and makes it a draft again. Returns
/// the draft id, for the compose screen.
#[uniffi::export]
pub fn edit_outbox(db_path: String, outbox_id: i64) -> Result<i64, OutboxFailure> {
    Ok(outbox::edit(&open(&db_path)?, OutboxId(outbox_id), now())?.0)
}

#[uniffi::export]
impl SyncEngine {
    /// For messages left `sending` when the app stopped: marks them sent if
    /// their Message-ID is in the Sent folder, otherwise queues them again.
    /// Call on start-up, before `send_outbox`. Returns how many were found
    /// sent. Blocking.
    pub fn recover_outbox(&self, account_id: i64) -> Result<u32, SyncFailure> {
        let account = AccountId(account_id);
        let interrupted = {
            let store = self.open_store()?;
            store
                .interrupted_outbox(account)
                .map_err(|_| failed(events::SyncErrorKind::Storage))?
                .len()
        };
        if interrupted == 0 {
            return Ok(0);
        }
        let found = self.with_session(account, |store, session, _listener| {
            Box::pin(
                async move { outbox::recover_interrupted(store, session, account, now()).await },
            )
        })?;
        Ok(u32::try_from(found).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests;
