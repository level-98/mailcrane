//! JMAP push (M4-27, RFC 8620 §7.2): registering the app's Web Push URL (a
//! UnifiedPush endpoint) with each JMAP account's server, so the server
//! wakes the app when mail changes, with no gateway and no connection of
//! the app's own.
//!
//! For each account, [`register`] keeps one subscription in step with the
//! endpoint:
//!
//! - none yet, or made for another endpoint or key: create a new one (and
//!   destroy the old one on the server, best effort);
//! - made but never verified within [`VERIFY_WITHIN`]: create it again (the
//!   verification push was lost);
//! - verified and expiring within [`REFRESH_BEFORE`]: ask for
//!   [`LIFETIME`] more;
//! - otherwise: nothing to do, and no network.
//!
//! The server's `PushVerification` arrives as a push; [`verify`] answers
//! it. [`teardown`] destroys the subscription and forgets it.

use super::SyncError;
use mailcrane_jmap::push::{
    create_push_subscription, destroy_push_subscription, refresh_push_subscription,
    verify_push_subscription,
};
use mailcrane_jmap::{Auth, ClientConfig, PushKeys, Session};
use mailcrane_store::{AccountId, PushSubscriptionRow, Store};

/// How long a subscription is asked to last.
pub const LIFETIME: i64 = 7 * 24 * 3600;
/// A verified subscription that expires sooner than this is refreshed.
pub const REFRESH_BEFORE: i64 = 2 * 24 * 3600;
/// An unverified subscription older than this is made again.
pub const VERIFY_WITHIN: i64 = 3600;

/// The Web Push target the server should send to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTarget {
    /// The endpoint URL (https).
    pub url: String,
    /// Keys for encrypted pushes (RFC 8291), if the distributor gave them.
    pub keys: Option<PushKeys>,
}

/// What [`register`] did for one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// Made a new subscription; its verification push is on the way.
    Created,
    /// Asked the server to keep the subscription longer.
    Refreshed,
    /// Waiting for the verification push of a recent subscription.
    Waiting,
    /// Already verified and not expiring soon: nothing was sent.
    Unchanged,
}

/// Brings the account's subscription in step with `target`; see the module
/// docs. `session` is the account's JMAP session, `now` unix seconds.
pub async fn register(
    store: &Store,
    account: AccountId,
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    target: &PushTarget,
    now: i64,
) -> Result<Registered, SyncError> {
    let p256dh = target.keys.as_ref().map(|k| k.p256dh.clone());
    let existing = store.push_subscription(account)?;
    if let Some(row) = &existing {
        if row.url == target.url && row.p256dh == p256dh {
            if !row.verified {
                if now.saturating_sub(row.created_at) < VERIFY_WITHIN {
                    return Ok(Registered::Waiting);
                }
            } else if row
                .expires
                .is_some_and(|e| e.saturating_sub(now) < REFRESH_BEFORE)
            {
                let asked = now.saturating_add(LIFETIME);
                let set =
                    refresh_push_subscription(session, auth, config, &row.subscription_id, asked)
                        .await?;
                store.set_push_expires(account, set)?;
                return Ok(Registered::Refreshed);
            } else {
                return Ok(Registered::Unchanged);
            }
        }
    }
    // A new subscription. The old one is no use any more; if the server
    // can't be told, it expires there by itself.
    if let Some(old) = &existing {
        if let Err(e) = destroy_push_subscription(session, auth, config, &old.subscription_id).await
        {
            tracing::info!(account = account.0, kind = ?e, "old push subscription not destroyed");
        }
    }
    let device_client_id = match &existing {
        Some(row) => row.device_client_id.clone(),
        None => new_device_client_id()?,
    };
    let created = create_push_subscription(
        session,
        auth,
        config,
        &device_client_id,
        &target.url,
        target.keys.as_ref(),
        Some(now.saturating_add(LIFETIME)),
    )
    .await?;
    store.save_push_subscription(&PushSubscriptionRow {
        account,
        subscription_id: created.id,
        device_client_id,
        url: target.url.clone(),
        p256dh,
        expires: created.expires,
        verified: false,
        created_at: now,
    })?;
    Ok(Registered::Created)
}

/// Answers the server's `PushVerification` for the account's subscription
/// `subscription_id` with `code`, and records it as verified.
pub async fn verify(
    store: &Store,
    account: AccountId,
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    subscription_id: &str,
    code: &str,
) -> Result<(), SyncError> {
    let ours = store
        .push_subscription(account)?
        .is_some_and(|row| row.subscription_id == subscription_id);
    if !ours {
        return Err(SyncError::UnknownPushSubscription);
    }
    verify_push_subscription(session, auth, config, subscription_id, code).await?;
    store.mark_push_verified(subscription_id)?;
    Ok(())
}

/// Destroys the account's subscription on the server and forgets it. It
/// is forgotten even if the server can't be reached (it then expires
/// there); the server's error is returned for the log.
pub async fn teardown(
    store: &Store,
    account: AccountId,
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
) -> Result<(), SyncError> {
    let Some(row) = store.push_subscription(account)? else {
        return Ok(());
    };
    let destroyed = destroy_push_subscription(session, auth, config, &row.subscription_id).await;
    store.delete_push_subscription(account)?;
    destroyed.map_err(SyncError::from)
}

/// A random `deviceClientId` (RFC 8620 §7.2): 128 bits, hex.
fn new_device_client_id() -> Result<String, SyncError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| SyncError::Random)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
