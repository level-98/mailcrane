//! JMAP push subscriptions for the app (M4-27, RFC 8620 §7.2): register the
//! UnifiedPush endpoint with every JMAP account, answer the servers'
//! verification pushes, and remove the subscriptions again. The work is in
//! `mailcrane_core::sync::push`; this runs it per account.
//!
//! Push bodies are hostile input from the network: they're parsed only in
//! the core (`mailcrane_jmap::push::parse_push`, size-limited) and never
//! logged. Logs name account ids and outcomes only, never the endpoint.

use super::jmap::jmap_auth;
use super::*;
use mailcrane_core::sync::push::{self as core_push, PushTarget};
use mailcrane_jmap::{Auth, ClientConfig, PushKeys, PushObject};
use mailcrane_store::Protocol;

/// What a push body asks for, so the app can route it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PushBody {
    /// A JMAP server's verification: answer it with
    /// [`SyncEngine::answer_jmap_push`], promptly.
    Verification,
    /// Anything else (a JMAP `StateChange`, a gateway's empty wake-up, or
    /// something unreadable): check for mail.
    Wake,
}

/// Sorts a push body without any I/O; cheap enough for the main thread.
#[uniffi::export]
pub fn classify_push(body: Vec<u8>) -> PushBody {
    match mailcrane_jmap::parse_push(&body) {
        Ok(PushObject::Verification { .. }) => PushBody::Verification,
        _ => PushBody::Wake,
    }
}

#[uniffi::export]
impl SyncEngine {
    /// Registers `endpoint` (an https Web Push URL) with every JMAP
    /// account, creating, refreshing or replacing subscriptions as needed
    /// (see `mailcrane_core::sync::push`). `p256dh` and `auth` are the
    /// RFC 8291 keys, both or neither. Call it when the endpoint changes
    /// and about once a day. Returns how many JMAP accounts have a
    /// subscription afterwards; an account that fails is logged and
    /// skipped. Blocking.
    pub fn register_jmap_push(
        &self,
        endpoint: String,
        p256dh: Option<String>,
        auth: Option<String>,
    ) -> Result<u32, SyncFailure> {
        let keys = match (p256dh, auth) {
            (Some(p256dh), Some(auth)) => Some(PushKeys { p256dh, auth }),
            (None, None) => None,
            _ => return Err(failed(events::SyncErrorKind::Protocol)),
        };
        let target = PushTarget {
            url: endpoint,
            keys,
        };
        let store = self.open_store()?;
        let mut subscribed = 0u32;
        for (account, session_url) in jmap_accounts(&store)? {
            let now = unix_now();
            let done = self
                .jmap_context(&store, account)
                .and_then(|(auth, config, rt)| {
                    rt.block_on(async {
                        let session =
                            mailcrane_jmap::discover(&session_url, &auth, &config).await?;
                        core_push::register(&store, account, &session, &auth, &config, &target, now)
                            .await
                    })
                    .map_err(|e| e.kind())
                });
            match done {
                Ok(outcome) => {
                    tracing::info!(account = account.0, ?outcome, "jmap push registered");
                    subscribed = subscribed.saturating_add(1);
                }
                Err(kind) => tracing::warn!(account = account.0, ?kind, "jmap push not registered"),
            }
        }
        Ok(subscribed)
    }

    /// Answers a JMAP server's verification push (RFC 8620 §7.2.2).
    /// Returns whether `body` was a verification for one of this app's
    /// subscriptions and was answered. Blocking.
    pub fn answer_jmap_push(&self, body: Vec<u8>) -> bool {
        let Ok(PushObject::Verification {
            subscription_id,
            code,
        }) = mailcrane_jmap::parse_push(&body)
        else {
            return false;
        };
        let Ok(store) = self.open_store() else {
            return false;
        };
        let Ok(Some(account)) = store.push_subscription_account(&subscription_id) else {
            tracing::info!("push verification for an unknown subscription");
            return false;
        };
        let Ok(Some(Protocol::Jmap { session_url })) = store.account_protocol(account) else {
            return false;
        };
        let answered = self
            .jmap_context(&store, account)
            .and_then(|(auth, config, rt)| {
                rt.block_on(async {
                    let session = mailcrane_jmap::discover(&session_url, &auth, &config).await?;
                    core_push::verify(
                        &store,
                        account,
                        &session,
                        &auth,
                        &config,
                        &subscription_id,
                        &code,
                    )
                    .await
                })
                .map_err(|e| e.kind())
            });
        match answered {
            Ok(()) => true,
            Err(kind) => {
                tracing::warn!(account = account.0, ?kind, "jmap push not verified");
                false
            }
        }
    }

    /// Destroys every JMAP push subscription and forgets them: push
    /// wake-ups were turned off. Ones the server can't be told about are
    /// forgotten anyway and expire there. Blocking.
    pub fn unregister_jmap_push(&self) {
        let Ok(store) = self.open_store() else {
            return;
        };
        let Ok(rows) = store.push_subscriptions() else {
            return;
        };
        for row in rows {
            let account = row.account;
            let session_url = match store.account_protocol(account) {
                Ok(Some(Protocol::Jmap { session_url })) => session_url,
                _ => {
                    let _ = store.delete_push_subscription(account);
                    continue;
                }
            };
            let done = self
                .jmap_context(&store, account)
                .and_then(|(auth, config, rt)| {
                    rt.block_on(async {
                        let session =
                            mailcrane_jmap::discover(&session_url, &auth, &config).await?;
                        core_push::teardown(&store, account, &session, &auth, &config).await
                    })
                    .map_err(|e| e.kind())
                });
            if let Err(kind) = done {
                tracing::info!(account = account.0, ?kind, "jmap push removed locally only");
                let _ = store.delete_push_subscription(account);
            }
        }
    }
}

impl SyncEngine {
    /// What talking to the account's JMAP server needs: its credentials,
    /// the client settings and a runtime. No write lock: push work writes
    /// only the account's subscription row.
    fn jmap_context(
        &self,
        store: &Store,
        account: AccountId,
    ) -> Result<(Auth, ClientConfig, tokio::runtime::Runtime), events::SyncErrorKind> {
        let auth = jmap_auth(store, account)?;
        let mut config = ClientConfig::default();
        if let Some(der) = self.extra_root_cert() {
            config.extra_roots.push(der);
        }
        Ok((auth, config, runtime()?))
    }
}

/// Every JMAP account with its session URL.
fn jmap_accounts(store: &Store) -> Result<Vec<(AccountId, String)>, SyncFailure> {
    let accounts = store
        .list_accounts()
        .map_err(|_| failed(events::SyncErrorKind::Storage))?;
    Ok(accounts
        .into_iter()
        .filter_map(|a| match store.account_protocol(a.id) {
            Ok(Some(Protocol::Jmap { session_url })) => Some((a.id, session_url)),
            _ => None,
        })
        .collect())
}
