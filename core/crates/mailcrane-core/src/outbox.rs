//! The outbox (M2-06). Pressing Send puts the built message here
//! ([`enqueue`]); it is sent as soon as possible ([`send_due`]), retried
//! after temporary failures, and marked failed (with the reason) after
//! permanent ones or a day of trying. It can then be retried, edited (back
//! into a draft) or deleted.
//!
//! A message is claimed (`sending`) before it goes to the server, so two
//! senders never send it twice. If the app stops mid-send, the message stays
//! `sending`; [`recover_interrupted`] then looks for its Message-ID in the
//! Sent folder. Found: it went out, so it's marked sent. Not found: it is
//! sent again. That can duplicate a message the server accepted but didn't
//! file in Sent (a server that doesn't save sent mail, before M2-07 saves a
//! copy); a lost message would be worse.

mod save_sent;
mod schedule;

pub use save_sent::{save_sent_copies, SaveReport, SENT_FOLDER_NAME};
pub use schedule::{decide_scheduled, reschedule, schedule};

use mailcrane_imap::Session;
use mailcrane_store::{
    AccountId, DraftFields, DraftId, FolderRole, NewOutboxMessage, OutboxId, OutboxMessage,
    OutboxState, Store, StoreError,
};

use crate::compose::{
    build_for_sending, prepared, send_prepared, smtp_settings, ComposeDraft, PreparedSend,
    SendError, SendOptions,
};
use crate::secrets::SecretStore;

/// A message is given up after this long of temporary failures.
pub const GIVE_UP_AFTER_SECS: i64 = 24 * 60 * 60;

/// Seconds to wait after the `attempts`-th failed try: 30 s, 1 min, 2 min,
/// 5 min, then every 15 minutes.
pub fn retry_delay(attempts: u32) -> i64 {
    match attempts {
        0 | 1 => 30,
        2 => 60,
        3 => 120,
        4 => 300,
        _ => 900,
    }
}

/// Whether trying again later may work: no connection, a timeout, TLS
/// trouble (a captive portal looks like that), or a 4xx refusal.
/// Permanent: a 5xx refusal, failed login, every recipient refused, and
/// anything about the message itself.
pub fn is_temporary(error: &SendError) -> bool {
    matches!(
        error,
        SendError::CannotConnect
            | SendError::Timeout
            | SendError::TlsFailure
            | SendError::StartTlsNotOffered
            | SendError::Refused { temporary: true }
    )
}

/// A short code for an error, stored in `last_error`. Never server text.
pub fn error_code(error: &SendError) -> &'static str {
    match error {
        SendError::InvalidRecipients(_) => "invalid_recipients",
        SendError::NoRecipients => "no_recipients",
        SendError::TooManyRecipients => "too_many_recipients",
        SendError::TooLarge => "too_large",
        SendError::AccountNotFound => "account_not_found",
        SendError::InvalidSender => "invalid_sender",
        SendError::MissingCredentials => "missing_credentials",
        SendError::Storage => "storage",
        SendError::CannotConnect => "network",
        SendError::TlsFailure => "tls",
        SendError::StartTlsNotOffered => "starttls",
        SendError::AuthFailed => "auth",
        SendError::Timeout => "timeout",
        SendError::RecipientsRejected { .. } => "recipients_rejected",
        SendError::Refused { temporary: true } => "refused_temporary",
        SendError::Refused { temporary: false } => "refused",
        SendError::Internal => "internal",
    }
}

/// Puts the draft in the outbox: validated and built now (so problems with
/// the message show at once), sent later. `not_before` holds it back until
/// then (0: at once).
pub fn enqueue(
    store: &Store,
    draft: &ComposeDraft,
    not_before: i64,
    now: i64,
) -> Result<OutboxId, SendError> {
    let built = build_for_sending(store, draft)?;
    store
        .insert_outbox(
            &NewOutboxMessage {
                account_id: draft.account_id,
                raw: built.raw,
                envelope_from: built.envelope.from,
                envelope_to: built.envelope.to,
                message_id_header: built.message_id,
                fields: DraftFields {
                    to: draft.to.clone(),
                    cc: draft.cc.clone(),
                    bcc: draft.bcc.clone(),
                    subject: draft.subject.clone(),
                    body: draft.body_text.clone(),
                    in_reply_to: draft.in_reply_to.clone(),
                    references: draft.references.clone(),
                },
                not_before,
                scheduled: false,
            },
            now,
        )
        .map_err(|_| SendError::Storage)
}

/// How one send attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// The server accepted it; `rejected` lists recipients it refused.
    Sent { rejected: Vec<String> },
    /// Failed for now; queued again for `retry_at`.
    Queued { error: SendError, retry_at: i64 },
    /// Failed for good (or after a day of trying); left in the outbox as
    /// failed.
    Failed { error: SendError },
}

/// Claims queued message `id` and gets it ready to send. `Ok(None)` if it
/// isn't queued (sent, being sent elsewhere, deleted). A missing password
/// or account is recorded as a failure and returned as `Err`.
pub fn prepare(
    store: &Store,
    secrets: &dyn SecretStore,
    id: OutboxId,
    options: &SendOptions,
) -> Result<Option<PreparedSend>, SendError> {
    let Some(message) = store.get_outbox(id).map_err(|_| SendError::Storage)? else {
        return Ok(None);
    };
    if !store.claim_outbox(id).map_err(|_| SendError::Storage)? {
        return Ok(None);
    }
    match smtp_settings(store, secrets, message.account_id, options) {
        Ok((config, auth)) => Ok(Some(prepared(
            config,
            auth,
            mailcrane_smtp::Envelope {
                from: message.envelope_from,
                to: message.envelope_to,
            },
            message.raw,
        ))),
        Err(error) => {
            let attempts = message.attempts.saturating_add(1);
            store
                .outbox_failed(id, attempts, error_code(&error))
                .map_err(|_| SendError::Storage)?;
            Err(error)
        }
    }
}

/// Records how sending message `id` went and decides what happens next.
pub fn record(
    store: &Store,
    id: OutboxId,
    result: Result<crate::compose::SendReport, SendError>,
    now: i64,
) -> Result<SendOutcome, StoreError> {
    let Some(message) = store.get_outbox(id)? else {
        // Deleted meanwhile; nothing to record.
        return Ok(match result {
            Ok(report) => SendOutcome::Sent {
                rejected: report.rejected,
            },
            Err(error) => SendOutcome::Failed { error },
        });
    };
    let attempts = message.attempts.saturating_add(1);
    Ok(match result {
        Ok(report) => {
            store.outbox_sent(id, &report.rejected, now)?;
            crate::contacts::record_sent(store, &message, now)?;
            SendOutcome::Sent {
                rejected: report.rejected,
            }
        }
        Err(error)
            if is_temporary(&error)
                && now.saturating_sub(message.created_at) < GIVE_UP_AFTER_SECS =>
        {
            let retry_at = now.saturating_add(retry_delay(attempts));
            store.outbox_retry_at(id, attempts, retry_at, error_code(&error))?;
            tracing::info!(attempts, "outbox message deferred");
            SendOutcome::Queued { error, retry_at }
        }
        Err(error) => {
            store.outbox_failed(id, attempts, error_code(&error))?;
            tracing::warn!(attempts, code = error_code(&error), "outbox message failed");
            SendOutcome::Failed { error }
        }
    })
}

/// Sends queued message `id` now (if it's still queued). `Ok(None)` if
/// there was nothing to send.
///
/// The future borrows `store`; the FFI calls [`prepare`], the SMTP send and
/// [`record`] itself to keep its future `Send`.
pub async fn send_one(
    store: &Store,
    secrets: &dyn SecretStore,
    id: OutboxId,
    options: &SendOptions,
    now: i64,
) -> Result<Option<SendOutcome>, StoreError> {
    let prepared = match prepare(store, secrets, id, options) {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(None),
        Err(error) => return Ok(Some(SendOutcome::Failed { error })),
    };
    let result = send_prepared(prepared).await;
    record(store, id, result, now).map(Some)
}

/// What one [`send_due`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OutboxReport {
    pub sent: usize,
    pub failed: usize,
    /// Messages still queued.
    pub remaining: usize,
    /// When the next queued message is due, if any.
    pub next_attempt_at: Option<i64>,
}

/// Sends the account's queued messages that are due by `now`, oldest first.
pub async fn send_due(
    store: &Store,
    secrets: &dyn SecretStore,
    account: AccountId,
    options: &SendOptions,
    now: i64,
) -> Result<OutboxReport, StoreError> {
    let mut report = OutboxReport::default();
    // Scheduled messages the server can hold go to it now (M4-07).
    decide_scheduled(store, secrets, account, options, now).await?;
    for message in store.due_outbox(account, now)? {
        match send_one(store, secrets, message.id, options, now).await? {
            Some(SendOutcome::Sent { .. }) => report.sent += 1,
            Some(SendOutcome::Failed { .. }) => report.failed += 1,
            // The server is unreachable: the rest would fail the same way.
            Some(SendOutcome::Queued { .. }) => break,
            None => {}
        }
    }
    report.remaining = store
        .list_outbox(account)?
        .iter()
        .filter(|m| m.state == OutboxState::Queued)
        .count();
    report.next_attempt_at = store.next_outbox_attempt(account)?;
    Ok(report)
}

/// For messages left `sending` by a stopped app: marks them sent if their
/// Message-ID is in the Sent folder, otherwise queues them again (see the
/// module docs for the trade-off). Returns how many were found sent.
/// Leaves the Sent folder selected.
pub async fn recover_interrupted(
    store: &Store,
    session: &mut Session,
    account: AccountId,
    now: i64,
) -> Result<usize, crate::sync::SyncError> {
    let interrupted = store.interrupted_outbox(account)?;
    if interrupted.is_empty() {
        return Ok(0);
    }
    let sent_folder = store
        .list_folders(account)?
        .into_iter()
        .find(|f| f.details.role == Some(FolderRole::Sent));
    let selected = match &sent_folder {
        Some(folder) => session.select(&folder.details.remote_name).await.is_ok(),
        None => false,
    };
    let mut found = 0;
    for message in interrupted {
        let in_sent = selected
            && session
                .uid_search_message_id(&message.message_id_header)
                .await
                .map(|uids| !uids.is_empty())
                // Can't tell: send again rather than lose it.
                .unwrap_or(false);
        if in_sent {
            store.outbox_sent(message.id, &[], now)?;
            crate::contacts::record_sent(store, &message, now)?;
            found += 1;
        } else {
            store.requeue_outbox(message.id, now)?;
        }
    }
    Ok(found)
}

/// Undo send (M4-08): takes message `id` back out of the outbox if it
/// hasn't started sending, and returns the draft exactly as it was
/// (recipients, subject, body, thread headers). `None` if it's too late.
pub fn cancel_pending(store: &Store, id: OutboxId) -> Result<Option<ComposeDraft>, StoreError> {
    Ok(store.take_queued_outbox(id)?.map(|m| ComposeDraft {
        account_id: m.account_id,
        to: m.fields.to,
        cc: m.fields.cc,
        bcc: m.fields.bcc,
        subject: m.fields.subject,
        body_text: m.fields.body,
        in_reply_to: m.fields.in_reply_to,
        references: m.fields.references,
        // The built message has them; take them back out.
        attachments: mailcrane_mime::attachments_of(&m.raw)
            .unwrap_or_default()
            .into_iter()
            .map(|a| mailcrane_mime::OutgoingAttachment {
                filename: a.filename.unwrap_or_default(),
                mime_type: a.mime_type,
                bytes: a.bytes,
            })
            .collect(),
        forward_attachments_of: None,
        rich: None,
    }))
}

/// Why an outbox call on one message failed.
#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    /// The message doesn't exist, or is being sent right now.
    #[error("not found or busy")]
    NotFound,
    /// The local database failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// Queues a failed message again, due now; for a sent message whose copy
/// in Sent failed, tries the copy again instead (the message is never sent
/// twice for that).
pub fn retry(store: &Store, id: OutboxId, now: i64) -> Result<(), OutboxError> {
    if store.retry_sent_copy(id)? || store.requeue_outbox(id, now)? {
        Ok(())
    } else {
        Err(OutboxError::NotFound)
    }
}

/// Removes a message from the outbox (not while it's being sent).
pub fn delete(store: &Store, id: OutboxId) -> Result<OutboxMessage, OutboxError> {
    store.take_outbox(id)?.ok_or(OutboxError::NotFound)
}

/// Takes a message out of the outbox and makes it a draft again, to edit
/// and send anew. Returns the draft.
pub fn edit(store: &Store, id: OutboxId, now: i64) -> Result<DraftId, OutboxError> {
    let message = store.take_outbox(id)?.ok_or(OutboxError::NotFound)?;
    // A fresh Message-ID: the new version is a new message.
    let draft_id = crate::drafts::save_draft(store, message.account_id, None, &message.fields, now)
        .map_err(|e| match e {
            crate::drafts::DraftError::Store(s) => OutboxError::Store(s),
            _ => OutboxError::NotFound,
        })?;
    Ok(draft_id)
}
