//! Initial sync of a folder: headers for the newest messages first, older
//! ones on demand, bodies when a message is opened.
//!
//! Every batch of headers is written in one store transaction, so a failure
//! leaves every finished batch stored and no partial one. Running the sync
//! again fetches only what is missing.

use super::convert::header_row;
use crate::events::{AccountId, CoreEvent, EventListener, FolderId, SyncErrorKind};
use mailcrane_imap::{ImapError, SelectInfo, Session};
use mailcrane_mime::Disposition;
use mailcrane_store::{BodyState, MessageId, NewAttachment, Store, StoreError};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// How much to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncConfig {
    /// Headers fetched on the first sync of a folder. Default 200.
    pub initial_count: usize,
    /// Headers fetched and stored per batch. Default 50.
    pub batch_size: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            initial_count: 200,
            batch_size: 50,
        }
    }
}

/// Why a sync step failed. None of the messages include server text.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// Talking to the server failed.
    #[error("IMAP error: {0}")]
    Imap(#[from] ImapError),
    /// Talking to a JMAP server failed (M3-04).
    #[error("JMAP error: {0}")]
    Jmap(#[from] mailcrane_jmap::JmapError),
    /// The local database failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// The folder is not in the database.
    #[error("unknown folder")]
    UnknownFolder,
    /// The message is not in the database.
    #[error("unknown message")]
    UnknownMessage,
    /// The server no longer has the message.
    #[error("the message is gone from the server")]
    MessageGone,
    /// The server renumbered the folder since the message list was fetched.
    /// Sync the folder again.
    #[error("the folder was renumbered by the server")]
    FolderReset,
}

impl SyncError {
    /// The variant behind the kind, e.g. `"CommandFailed"` or
    /// `"UnknownFolder"`. Safe to log: a fixed string, never server text,
    /// message content or store details.
    pub fn detail(&self) -> &'static str {
        match self {
            SyncError::Imap(e) => e.name(),
            SyncError::Jmap(_) => "Jmap",
            SyncError::Store(_) => "Store",
            SyncError::UnknownFolder => "UnknownFolder",
            SyncError::UnknownMessage => "UnknownMessage",
            SyncError::MessageGone => "MessageGone",
            SyncError::FolderReset => "FolderReset",
        }
    }

    /// The kind reported in [`CoreEvent::SyncError`].
    pub fn kind(&self) -> SyncErrorKind {
        match self {
            SyncError::Imap(e) => match e {
                ImapError::CannotConnect | ImapError::ConnectionClosed | ImapError::ServerBye => {
                    SyncErrorKind::Network
                }
                ImapError::TlsFailure | ImapError::StartTlsNotOffered => SyncErrorKind::Tls,
                ImapError::AuthFailed | ImapError::LoginDisabled => SyncErrorKind::Auth,
                ImapError::Timeout => SyncErrorKind::Timeout,
                ImapError::Store(_) => SyncErrorKind::Storage,
                _ => SyncErrorKind::Protocol,
            },
            SyncError::Jmap(e) => {
                use mailcrane_jmap::JmapError as J;
                match e {
                    J::Network => SyncErrorKind::Network,
                    J::Tls => SyncErrorKind::Tls,
                    J::Unauthorized => SyncErrorKind::Auth,
                    J::Timeout => SyncErrorKind::Timeout,
                    _ => SyncErrorKind::Protocol,
                }
            }
            SyncError::Store(_) => SyncErrorKind::Storage,
            SyncError::UnknownFolder | SyncError::UnknownMessage | SyncError::MessageGone => {
                SyncErrorKind::NotFound
            }
            SyncError::FolderReset => SyncErrorKind::FolderReset,
        }
    }
}

/// What [`log_failure`] logs. Every field is a `&'static str` or a fieldless
/// enum, so no server text, message content or credential can get in
/// (AGENTS.md §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureLog {
    /// Where it failed, e.g. `"sync"` or `"connect"`.
    pub stage: &'static str,
    /// The kind the app is told.
    pub kind: SyncErrorKind,
    /// The variant behind the kind, see [`SyncError::detail`].
    pub error: &'static str,
    /// The last IMAP command sent, e.g. `"SELECT"` or `"UID FETCH"`, or
    /// `"none"`.
    pub command: &'static str,
}

impl FailureLog {
    /// The fields for a failed step.
    pub fn new(stage: &'static str, error: &SyncError, command: Option<&'static str>) -> Self {
        Self {
            stage,
            kind: error.kind(),
            error: error.detail(),
            command: command.unwrap_or("none"),
        }
    }
}

/// Logs a failed sync step: where it failed (`stage`), the kind, the
/// variant behind it and the last IMAP command sent. See [`FailureLog`].
pub fn log_failure(stage: &'static str, error: &SyncError, command: Option<&'static str>) {
    let log = FailureLog::new(stage, error, command);
    tracing::warn!(
        stage = log.stage,
        kind = ?log.kind,
        error = log.error,
        command = log.command,
        "sync failed"
    );
}

/// Syncs the folders of one account over one connection. Holding the
/// session mutably means only one folder syncs at a time.
pub struct FolderSyncer<'a> {
    pub(super) store: &'a Store,
    pub(super) session: &'a mut Session,
    pub(super) listener: &'a dyn EventListener,
    account: AccountId,
    config: SyncConfig,
}

impl<'a> FolderSyncer<'a> {
    /// A syncer for `account`, using its logged-in `session`.
    pub fn new(
        store: &'a Store,
        session: &'a mut Session,
        listener: &'a dyn EventListener,
        account: AccountId,
        config: SyncConfig,
    ) -> Self {
        Self {
            store,
            session,
            listener,
            account,
            config,
        }
    }

    /// Syncs a folder: records `UIDVALIDITY` and `UIDNEXT`, then fetches
    /// headers for the newest `initial_count` messages that are not stored
    /// yet, newest batch first. If `UIDVALIDITY` changed, the folder's
    /// cached messages are deleted first (RFC 9051 §2.3.1.1). Returns how
    /// many messages were stored. On failure a `SyncError` event is sent.
    pub async fn sync_folder(&mut self, folder: FolderId) -> Result<usize, SyncError> {
        let result = self.sync_folder_inner(folder).await;
        self.report(result)
    }

    /// Fetches headers for the next `count` messages older than the oldest
    /// stored one, newest batch first. Returns how many were stored.
    pub async fn load_older(&mut self, folder: FolderId, count: usize) -> Result<usize, SyncError> {
        let result = self.load_older_inner(folder, count).await;
        self.report(result)
    }

    /// Downloads the whole message once and keeps it for offline reading:
    /// stores the raw bytes, the snippet and the attachment list. Uses
    /// `BODY.PEEK`, so the message is not marked `\Seen` on the server. A
    /// message already fetched is not downloaded again.
    pub async fn fetch_body(&mut self, message: MessageId) -> Result<(), SyncError> {
        let result = self.fetch_body_inner(message).await;
        self.report(result)
    }

    pub(super) fn report<T>(&self, result: Result<T, SyncError>) -> Result<T, SyncError> {
        if let Err(error) = &result {
            log_failure("sync", error, self.session.last_command());
            self.listener.on_event(CoreEvent::SyncError {
                account_id: self.account,
                kind: error.kind(),
            });
        }
        result
    }

    /// Selects the folder and clears its cache if `UIDVALIDITY` changed.
    pub(super) async fn select(
        &mut self,
        folder: FolderId,
    ) -> Result<(String, SelectInfo, bool), SyncError> {
        let row = self
            .store
            .get_folder(folder)?
            .ok_or(SyncError::UnknownFolder)?;
        let info = self.session.select(&row.details.remote_name).await?;
        let changed = self.reset_if_renumbered(folder, row.details.uidvalidity, &info)?;
        Ok((row.details.remote_name, info, changed))
    }

    /// Clears the folder's cache if the server's `UIDVALIDITY` is not the
    /// stored one (RFC 9051 §2.3.1.1). Returns whether it did.
    pub(super) fn reset_if_renumbered(
        &self,
        folder: FolderId,
        stored: Option<i64>,
        info: &SelectInfo,
    ) -> Result<bool, SyncError> {
        let changed = stored != Some(i64::from(info.uidvalidity));
        if changed {
            if stored.is_some() {
                tracing::info!(
                    folder = folder.0,
                    "UIDVALIDITY changed, clearing the folder"
                );
            }
            self.store.reset_folder(
                folder,
                i64::from(info.uidvalidity),
                i64::from(info.uidnext),
            )?;
            self.listener
                .on_event(CoreEvent::FolderChanged { folder_id: folder });
        }
        Ok(changed)
    }

    async fn sync_folder_inner(&mut self, folder: FolderId) -> Result<usize, SyncError> {
        let (_, info, _) = self.select(folder).await?;
        self.fill_newest(folder, &info).await
    }

    /// With the folder selected: fetches the newest `initial_count`
    /// messages that are not stored yet.
    pub(super) async fn fill_newest(
        &mut self,
        folder: FolderId,
        info: &SelectInfo,
    ) -> Result<usize, SyncError> {
        let newest = self.newest_uids(info.uidnext.saturating_sub(1)).await?;
        let stored: HashSet<u32> = self.store.list_uids(folder)?.into_iter().collect();
        let wanted: Vec<u32> = newest
            .into_iter()
            .take(self.config.initial_count)
            .filter(|uid| !stored.contains(uid))
            .collect();
        self.fetch_batches(folder, &wanted).await
    }

    async fn load_older_inner(
        &mut self,
        folder: FolderId,
        count: usize,
    ) -> Result<usize, SyncError> {
        let (_, info, reset) = self.select(folder).await?;
        if reset {
            // The cache was just cleared: start again from the newest.
            return self.sync_folder_inner(folder).await;
        }
        let below = match self.store.oldest_uid(folder)? {
            Some(oldest) => oldest.saturating_sub(1),
            None => info.uidnext.saturating_sub(1),
        };
        let wanted: Vec<u32> = self
            .newest_uids(below)
            .await?
            .into_iter()
            .take(count)
            .collect();
        self.fetch_batches(folder, &wanted).await
    }

    /// UIDs from `high` down to 1 that exist, newest first.
    async fn newest_uids(&mut self, high: u32) -> Result<Vec<u32>, SyncError> {
        let mut uids = self.session.uid_search_range(1, high).await?;
        uids.reverse();
        Ok(uids)
    }

    /// Fetches and stores `uids` one batch at a time, in the order given.
    pub(super) async fn fetch_batches(
        &mut self,
        folder: FolderId,
        uids: &[u32],
    ) -> Result<usize, SyncError> {
        let mut total = 0usize;
        for batch in uids.chunks(self.config.batch_size.max(1)) {
            let fetched = self.session.uid_fetch_headers(batch).await?;
            for uid in &fetched.skipped {
                // Only the UID is logged, never what the server sent.
                tracing::warn!(uid = ?uid, "skipped a message with an unreadable envelope or structure");
            }
            let mut rows = Vec::with_capacity(fetched.messages.len());
            for data in &fetched.messages {
                match header_row(folder, data) {
                    Some(row) => rows.push(row),
                    None => {
                        tracing::warn!(uid = ?data.uid, "skipped a message with missing fields")
                    }
                }
            }
            // Oldest first within the batch, so that of two messages with the
            // same date the newer one gets the higher row id and sorts first.
            rows.sort_by_key(|r| r.uid);
            if rows.is_empty() {
                continue;
            }
            self.store.upsert_message_headers(&rows)?;
            // List-Id, for rules (M4-09).
            let list_ids: Vec<(u32, Option<String>)> = fetched
                .messages
                .iter()
                .filter_map(|d| {
                    let uid = d.uid?;
                    let list_id = d
                        .header_fields
                        .as_deref()
                        .and_then(|h| mailcrane_mime::parse_message(h).ok())
                        .and_then(|p| p.list_id);
                    list_id.map(|l| (uid, Some(l)))
                })
                .collect();
            if !list_ids.is_empty() {
                self.store.set_list_ids(folder, &list_ids)?;
            }
            total = total.saturating_add(rows.len());
            self.listener
                .on_event(CoreEvent::MessagesChanged { folder_id: folder });
        }
        Ok(total)
    }

    async fn fetch_body_inner(&mut self, message: MessageId) -> Result<(), SyncError> {
        let header = self
            .store
            .get_message_header(message)?
            .ok_or(SyncError::UnknownMessage)?;
        if header.body_state == BodyState::Fetched {
            return Ok(());
        }
        let folder = header.details.folder_id;
        let row = self
            .store
            .get_folder(folder)?
            .ok_or(SyncError::UnknownFolder)?;
        let info = self.session.select(&row.details.remote_name).await?;
        if row.details.uidvalidity != Some(i64::from(info.uidvalidity)) {
            // The UID may now mean another message: store nothing.
            return Err(SyncError::FolderReset);
        }
        let raw = self
            .session
            .uid_fetch_body(header.details.uid)
            .await?
            .ok_or(SyncError::MessageGone)?;
        save_body(self.store, message, &raw)?;
        self.listener
            .on_event(CoreEvent::MessagesChanged { folder_id: folder });
        Ok(())
    }
}

/// Stores a downloaded message: the raw bytes, its snippet and attachment
/// list, and its text for search. A message that can't be parsed is still
/// kept, so it can be opened as source. Shared by IMAP and JMAP.
pub(crate) fn save_body(store: &Store, message: MessageId, raw: &[u8]) -> Result<(), SyncError> {
    let (snippet, attachments, body_text) = match mailcrane_mime::parse_message(raw) {
        Ok(parsed) => {
            let attachments: Vec<NewAttachment> = parsed
                .attachments
                .into_iter()
                .map(|a| NewAttachment {
                    part_id: a.part_id,
                    filename: a.filename,
                    mime_type: a.mime_type,
                    size: i64::try_from(a.size).unwrap_or(i64::MAX),
                    content_id: a.content_id,
                    is_inline: a.disposition == Disposition::Inline,
                })
                .collect();
            // Plain text, or text derived from the HTML: indexed for search.
            (
                parsed.snippet,
                attachments,
                parsed.text_plain.unwrap_or_default(),
            )
        }
        Err(error) => {
            tracing::warn!(message = message.0, %error, "message could not be parsed");
            (String::new(), Vec::new(), String::new())
        }
    };
    let fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0);
    if !store.store_parsed_body(message, raw, fetched_at, &snippet, &attachments)? {
        return Err(SyncError::UnknownMessage);
    }
    store.index_body_text(message, &body_text)?;
    Ok(())
}

#[cfg(test)]
#[path = "initial_tests.rs"]
mod tests;
