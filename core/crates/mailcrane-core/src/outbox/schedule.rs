//! Scheduled send (M4-07). A scheduled message waits in the outbox until its
//! time (`not_before`). If the account's SMTP server offers `FUTURERELEASE`
//! (RFC 4865) far enough ahead, the message is handed over at once with
//! `HOLDUNTIL`, and the server sends it on time even with the phone off;
//! otherwise this device sends it at the time. The outbox records which, so
//! the app can say so honestly.

use mailcrane_store::{AccountId, OutboxId, ScheduleMode, Store, StoreError};

use crate::compose::{smtp_settings, ComposeDraft, SendError, SendOptions};
use crate::secrets::SecretStore;

use super::{enqueue, error_code, OutboxError};

/// Puts the draft in the outbox to be sent at `send_at` (Unix seconds).
/// Validated and built now, like Send.
pub fn schedule(
    store: &Store,
    draft: &ComposeDraft,
    send_at: i64,
    now: i64,
) -> Result<OutboxId, SendError> {
    let id = enqueue(store, draft, send_at, now)?;
    store
        .set_schedule_mode(id, ScheduleMode::Pending)
        .map_err(|_| SendError::Storage)?;
    Ok(id)
}

/// Moves a scheduled message that hasn't gone out (or to the server) to
/// `send_at`.
pub fn reschedule(store: &Store, id: OutboxId, send_at: i64) -> Result<(), OutboxError> {
    if store.reschedule_outbox(id, send_at)? {
        Ok(())
    } else {
        Err(OutboxError::NotFound)
    }
}

/// For the account's scheduled messages whose sender isn't decided: asks the
/// server whether it can hold them; if so, submits each now with
/// `HOLDUNTIL`, otherwise marks it for this device to send. Connection
/// trouble leaves them undecided, to ask again later. Errors only for a
/// local database failure.
pub async fn decide_scheduled(
    store: &Store,
    secrets: &dyn SecretStore,
    account: AccountId,
    options: &SendOptions,
    now: i64,
) -> Result<(), StoreError> {
    let undecided = store.undecided_scheduled(account)?;
    if undecided.is_empty() {
        return Ok(());
    }
    let Ok((config, auth)) = smtp_settings(store, secrets, account, options) else {
        // No password and so on: the device tries at the time, and that
        // send reports the problem.
        for m in &undecided {
            store.set_schedule_mode(m.id, ScheduleMode::Device)?;
        }
        return Ok(());
    };
    let limit = match mailcrane_smtp::future_release_limit(&config).await {
        Ok(limit) => limit,
        Err(_) => return Ok(()),
    };
    for message in undecided {
        let wait = message.not_before.saturating_sub(now);
        let server_can_hold =
            limit.is_some_and(|max| wait > 0 && u64::try_from(wait).is_ok_and(|w| w <= max));
        if !server_can_hold {
            store.set_schedule_mode(message.id, ScheduleMode::Device)?;
            continue;
        }
        if !store.claim_outbox(message.id)? {
            continue;
        }
        let envelope = mailcrane_smtp::Envelope {
            from: message.envelope_from.clone(),
            to: message.envelope_to.clone(),
        };
        let result = mailcrane_smtp::send_with_hold(
            &config,
            &auth,
            &envelope,
            &message.raw,
            Some(message.not_before),
        )
        .await;
        match result {
            Ok(report) => {
                // The server has it now; `sent_at` records the hand-over.
                store.outbox_sent(message.id, &report.rejected, now)?;
                store.set_schedule_mode(message.id, ScheduleMode::Server)?;
            }
            Err(error) => {
                // Not held after all: this device sends it at the time.
                let error = crate::compose::map_smtp(error);
                tracing::info!(
                    code = error_code(&error),
                    "server can't hold a scheduled message"
                );
                store.requeue_outbox(message.id, message.not_before)?;
                store.set_schedule_mode(message.id, ScheduleMode::Device)?;
            }
        }
    }
    Ok(())
}
