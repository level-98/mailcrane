//! Snooze for the apps (M4-06). Snoozing and unsnoozing change the store at
//! once; the server moves happen in [`SyncEngine::replay_actions`] and before
//! every folder sync. Call [`SyncEngine::wake_due_snoozes`] when the app
//! opens, on a timer while it is open, and in background sync, then post a
//! notification for what it returns and call `replay_actions` for each
//! account.

use crate::actions::now;
use crate::sync::{ListenerAdapter, SyncEngine};
use mailcrane_core::snooze::{self, SnoozeError};
use mailcrane_store::{MessageId, Snooze, SnoozeState};

/// Why snoozing failed. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SnoozeFailure {
    /// A message doesn't exist.
    #[error("not found")]
    NotFound,
    /// A message has no Message-ID header, so it can't be found again after
    /// moving it.
    #[error("the message has no Message-ID")]
    NoMessageId,
    /// A message is coming back from snooze right now: try again in a
    /// moment, once it has landed (#431). Nothing was changed.
    #[error("the message is being woken")]
    Waking,
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

impl From<SnoozeError> for SnoozeFailure {
    fn from(error: SnoozeError) -> Self {
        match error {
            SnoozeError::UnknownMessage => Self::NotFound,
            SnoozeError::NoMessageId => Self::NoMessageId,
            SnoozeError::Waking => Self::Waking,
            SnoozeError::Store(_) => Self::Storage,
        }
    }
}

/// A message's snooze, for the message view.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SnoozeInfo {
    /// Wake time, unix seconds UTC.
    pub wake_at: i64,
    /// The wake time is known to this device only: the server can't store
    /// it, so other installs won't wake it.
    pub device_only: bool,
    /// It is being moved back on the server right now (a wake past its
    /// first step): it can't be snoozed again until it lands (#431). Not
    /// set for a wake that hasn't started or is waiting to retry.
    pub waking: bool,
}

impl From<&Snooze> for SnoozeInfo {
    fn from(s: &Snooze) -> Self {
        Self {
            wake_at: s.wake_at,
            device_only: s.device_only(),
            waking: s.state == SnoozeState::ToWake && s.step > 0,
        }
    }
}

/// A message that woke, for a notification.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WokenMessage {
    /// Its account (replay that account's queue next).
    pub account_id: i64,
    /// Its row in the Snoozed folder, if it was synced.
    pub message_id: Option<i64>,
    /// Subject (may be empty).
    pub subject: String,
    /// Sender name or address (may be empty).
    pub from: String,
}

#[uniffi::export]
impl SyncEngine {
    /// Snoozes messages until `wake_at` (unix seconds UTC). They leave their
    /// folder at once (a `MessagesChanged` event follows); then call
    /// `replay_actions` for the account. A message already in the Snoozed
    /// folder gets the new wake time. Returns the accounts to replay.
    pub fn snooze_messages(
        &self,
        message_ids: Vec<i64>,
        wake_at: i64,
    ) -> Result<Vec<i64>, SnoozeFailure> {
        let store = self.open_store().map_err(|_| SnoozeFailure::Storage)?;
        let ids: Vec<MessageId> = message_ids.into_iter().map(MessageId).collect();
        let listener = ListenerAdapter(self.listener.clone());
        let accounts = snooze::snooze(&store, &ids, wake_at, &listener)?;
        Ok(accounts.into_iter().map(|a| a.0).collect())
    }

    /// Brings snoozed messages back now, without marking them unread.
    /// Returns the accounts to replay (empty if none was snoozed).
    pub fn unsnooze_messages(&self, message_ids: Vec<i64>) -> Result<Vec<i64>, SnoozeFailure> {
        let store = self.open_store().map_err(|_| SnoozeFailure::Storage)?;
        let ids: Vec<MessageId> = message_ids.into_iter().map(MessageId).collect();
        let listener = ListenerAdapter(self.listener.clone());
        let accounts = snooze::unsnooze(&store, &ids, now(), &listener)?;
        Ok(accounts.into_iter().map(|a| a.0).collect())
    }

    /// Wakes every snooze that is due (including ones made on other
    /// installs, learnt from the server) and returns them for a
    /// notification. Each is returned once.
    pub fn wake_due_snoozes(&self) -> Result<Vec<WokenMessage>, SnoozeFailure> {
        let store = self.open_store().map_err(|_| SnoozeFailure::Storage)?;
        let listener = ListenerAdapter(self.listener.clone());
        let woken =
            snooze::wake_due(&store, now(), &listener).map_err(|_| SnoozeFailure::Storage)?;
        Ok(woken
            .into_iter()
            .map(|w| WokenMessage {
                account_id: w.account_id.0,
                message_id: w.message_id.map(|m| m.0),
                subject: crate::messages::display_line(&w.subject),
                from: crate::messages::display_line(&w.from),
            })
            .collect())
    }

    /// The snooze of a message, if it has one.
    pub fn snooze_info(&self, message_id: i64) -> Result<Option<SnoozeInfo>, SnoozeFailure> {
        let store = self.open_store().map_err(|_| SnoozeFailure::Storage)?;
        let storage = |_| SnoozeFailure::Storage;
        let Some(header) = store
            .get_message_header(MessageId(message_id))
            .map_err(storage)?
        else {
            return Err(SnoozeFailure::NotFound);
        };
        let Some(mid) = header.details.message_id_header else {
            return Ok(None);
        };
        let Some(folder) = store
            .get_folder(header.details.folder_id)
            .map_err(storage)?
        else {
            return Err(SnoozeFailure::NotFound);
        };
        let snooze = store
            .get_snooze(folder.details.account_id, &mid)
            .map_err(storage)?;
        Ok(snooze.as_ref().map(SnoozeInfo::from))
    }

    /// The earliest wake time of any snooze still waiting, for scheduling
    /// the next check (unix seconds UTC), or `None`.
    pub fn next_snooze_wake(&self) -> Result<Option<i64>, SnoozeFailure> {
        let store = self.open_store().map_err(|_| SnoozeFailure::Storage)?;
        let all = snooze::list_snoozed(&store, None).map_err(|_| SnoozeFailure::Storage)?;
        Ok(all
            .iter()
            .filter(|s| s.state != SnoozeState::ToWake)
            .map(|s| s.wake_at)
            .min())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{CoreEvent, CoreEventListener};
    use mailcrane_store::{Flags, NewAccount, NewFolder, NewMessageHeader, Security, Store};

    struct Quiet;
    impl CoreEventListener for Quiet {
        fn on_event(&self, _event: CoreEvent) {}
    }

    fn setup(message_id: Option<&str>) -> (tempfile::TempDir, std::sync::Arc<SyncEngine>, i64) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let account = store
            .insert_account(&NewAccount {
                display_name: "Alice".into(),
                email: "alice@example.org".into(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 587,
                smtp_security: Security::Starttls,
                username: "alice@example.org".into(),
            })
            .expect("account");
        let folder = store
            .upsert_folder(&NewFolder {
                account_id: account,
                remote_name: "INBOX".into(),
                display_name: "Inbox".into(),
                delimiter: Some("/".into()),
                role: None,
                uidvalidity: None,
                uidnext: None,
                highestmodseq: None,
                is_subscribed: true,
                selectable: true,
                virtual_role: None,
            })
            .expect("folder");
        let id = store
            .upsert_message_header(&NewMessageHeader {
                folder_id: folder,
                uid: 1,
                message_id_header: message_id.map(Into::into),
                in_reply_to: None,
                references_json: "[]".into(),
                thread_id: None,
                subject: "Lunch".into(),
                from_name: Some("Ana".into()),
                from_addr: Some("ana@example.org".into()),
                to_json: "[]".into(),
                cc_json: "[]".into(),
                date: 1_000,
                internal_date: 1_000,
                size: 100,
                flags: Flags::default(),
                keywords_json: "[]".into(),
                has_attachments: false,
                snippet: "Hello".into(),
            })
            .expect("message");
        let engine = SyncEngine::new(path, std::sync::Arc::new(Quiet));
        (dir, engine, id.0)
    }

    #[test]
    fn snooze_and_unsnooze_through_the_engine() {
        let (_dir, engine, id) = setup(Some("lunch@example.net"));
        assert_eq!(engine.snooze_info(id), Ok(None));
        assert_eq!(engine.next_snooze_wake(), Ok(None));
        let wake = 4_000_000_000;
        assert_eq!(
            engine.snooze_messages(vec![id], wake).map(|a| a.len()),
            Ok(1)
        );
        assert_eq!(
            engine.snooze_info(id),
            Ok(Some(SnoozeInfo {
                wake_at: wake,
                device_only: false,
                waking: false,
            }))
        );
        assert_eq!(engine.next_snooze_wake(), Ok(Some(wake)));
        // Not on the server yet, so nothing can wake.
        assert_eq!(engine.wake_due_snoozes(), Ok(Vec::new()));
        assert_eq!(engine.unsnooze_messages(vec![id]).map(|a| a.len()), Ok(1));
        assert_eq!(engine.snooze_info(id), Ok(None));
        assert_eq!(engine.unsnooze_messages(vec![id]), Ok(Vec::new()));
    }

    #[test]
    fn errors_cross_the_boundary() {
        let (_dir, engine, id) = setup(None);
        assert_eq!(
            engine.snooze_messages(vec![id], 1),
            Err(SnoozeFailure::NoMessageId)
        );
        assert_eq!(
            engine.snooze_messages(vec![id + 100], 1),
            Err(SnoozeFailure::NotFound)
        );
        assert_eq!(engine.snooze_info(id + 100), Err(SnoozeFailure::NotFound));
    }
}
