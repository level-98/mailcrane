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
    /// A push verification names a subscription this app doesn't have
    /// (M4-27).
    #[error("unknown push subscription")]
    UnknownPushSubscription,
    /// The system's random number source failed.
    #[error("no random numbers")]
    Random,
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
            SyncError::UnknownPushSubscription => "UnknownPushSubscription",
            SyncError::Random => "Random",
        }
    }

    /// The local database error behind this one, if any.
    pub fn store_error(&self) -> Option<&StoreError> {
        match self {
            SyncError::Store(e) | SyncError::Imap(ImapError::Store(e)) => Some(e),
            _ => None,
        }
    }

    /// The kind reported in [`CoreEvent::SyncError`].
    pub fn kind(&self) -> SyncErrorKind {
        match self {
            SyncError::Store(e) | SyncError::Imap(ImapError::Store(e)) if e.is_busy() => {
                SyncErrorKind::Busy
            }
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
            SyncError::UnknownPushSubscription => SyncErrorKind::NotFound,
            SyncError::Random => SyncErrorKind::Protocol,
        }
    }
}

/// What [`log_failure`] logs. Every field is a `&'static str`, a fieldless
/// enum or a number, so no server text, message content, stored value or
/// credential can get in (AGENTS.md §4.4).
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
    /// For a local database failure, the [`StoreError`] variant, e.g.
    /// `"Sqlite"`; otherwise `"none"`.
    pub store: &'static str,
    /// The column or field a `Corrupt` or `Invalid` store error names, or
    /// `"none"`.
    pub field: &'static str,
    /// The SQLite extended result code's name (e.g.
    /// `"SQLITE_CONSTRAINT_UNIQUE"`) or the rusqlite error kind, or
    /// `"none"`.
    pub sqlite: &'static str,
    /// The SQLite extended result code as a number, or 0.
    pub sqlite_code: i32,
}

impl FailureLog {
    /// The fields for a failed step.
    pub fn new(stage: &'static str, error: &SyncError, command: Option<&'static str>) -> Self {
        let store = error.store_error();
        Self {
            stage,
            kind: error.kind(),
            error: error.detail(),
            command: command.unwrap_or("none"),
            store: store.map_or("none", StoreError::name),
            field: store.and_then(StoreError::field).unwrap_or("none"),
            sqlite: store.and_then(StoreError::sqlite_detail).unwrap_or("none"),
            sqlite_code: store.and_then(StoreError::sqlite_code).unwrap_or(0),
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
        store = log.store,
        field = log.field,
        sqlite = log.sqlite,
        sqlite_code = log.sqlite_code,
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
            let dropped = self
                .store
                .reset_folder_reporting(
                    folder,
                    i64::from(info.uidvalidity),
                    i64::from(info.uidnext),
                )?
                .unwrap_or_default();
            self.listener
                .on_event(CoreEvent::FolderChanged { folder_id: folder });
            // Changes queued for this folder can't be applied any more: the
            // server renumbered it, so their UIDs name other messages (#517).
            for op in dropped {
                tracing::warn!(
                    folder = folder.0,
                    kind = ?op.kind,
                    "queued action dropped: the server reset the folder"
                );
                self.listener.on_event(CoreEvent::ActionFailed {
                    account_id: self.account,
                    action: crate::actions::action_kind(op.kind),
                    message_ids: op.message_ids,
                    reason: SyncErrorKind::FolderReset,
                });
            }
        }
        Ok(changed)
    }

    /// Whether the folder can be opened. A `\Noselect` folder only holds
    /// other folders: syncing it stores nothing and sends nothing.
    pub(super) fn selectable(&self, folder: FolderId) -> Result<bool, SyncError> {
        let row = self
            .store
            .get_folder(folder)?
            .ok_or(SyncError::UnknownFolder)?;
        Ok(row.details.selectable)
    }

    async fn sync_folder_inner(&mut self, folder: FolderId) -> Result<usize, SyncError> {
        if !self.selectable(folder)? {
            return Ok(0);
        }
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
        let newest = self
            .newest_uids(
                info,
                info.uidnext.saturating_sub(1),
                self.config.initial_count,
            )
            .await?;
        let stored: HashSet<u32> = self.store.list_uids(folder)?.into_iter().collect();
        let wanted: Vec<u32> = newest
            .into_iter()
            .filter(|uid| !stored.contains(uid))
            .collect();
        self.fetch_batches(folder, &wanted).await
    }

    async fn load_older_inner(
        &mut self,
        folder: FolderId,
        count: usize,
    ) -> Result<usize, SyncError> {
        if !self.selectable(folder)? {
            return Ok(0);
        }
        let (_, info, reset) = self.select(folder).await?;
        if reset {
            // The cache was just cleared: start again from the newest.
            return self.sync_folder_inner(folder).await;
        }
        let below = match self.store.oldest_uid(folder)? {
            Some(oldest) => oldest.saturating_sub(1),
            None => info.uidnext.saturating_sub(1),
        };
        let wanted = self.newest_uids(&info, below, count).await?;
        self.fetch_batches(folder, &wanted).await
    }

    /// The newest `want` UIDs from `high` down to 1 that exist, newest
    /// first. Searched in windows, so a big folder never sends one
    /// over-long answer (issue #282).
    async fn newest_uids(
        &mut self,
        info: &SelectInfo,
        high: u32,
        want: usize,
    ) -> Result<Vec<u32>, SyncError> {
        Ok(self
            .session
            .uid_search_newest(info.exists, 1, high, &[], want)
            .await?)
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
        // A reused connection (#294) that has this folder selected already
        // doesn't select it again.
        let info = self
            .session
            .select_reusing(&row.details.remote_name)
            .await?;
        if row.details.uidvalidity != Some(i64::from(info.uidvalidity)) {
            // The UID may now mean another message: store nothing.
            return Err(SyncError::FolderReset);
        }
        let raw = self
            .session
            .uid_fetch_body(header.details.uid)
            .await?
            .ok_or(SyncError::MessageGone)?;
        save_opened_body(self.store, message, None, &raw)?;
        self.listener
            .on_event(CoreEvent::MessagesChanged { folder_id: folder });
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: for each [`save_opened_body`] on this thread, whether its save
    /// ran with the store's urgent mark.
    pub(crate) static OPENED_SAVES: std::cell::RefCell<Vec<bool>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The JMAP email an opened body belongs to (#488): the save applies only
/// while the row is still that email of that account, since row ids are
/// reused. (IMAP opens don't carry one yet: #503.)
#[derive(Debug, Clone, Copy)]
pub(crate) struct JmapEmail<'a> {
    pub(crate) account: AccountId,
    pub(crate) email_id: &'a str,
}

/// [`save_body`] for a message the user opened (#294, #472, #473). The body
/// is parsed first; only its writes run under the store's urgent mark, made
/// on this thread, so other writers give it the next turn for SQLite's write
/// lock, but not while the opener waits for a connection, downloads or
/// parses (#473, #488 review). It writes only the body and its parts, never
/// the flags, folder or state columns a sync owns, so it can run beside a
/// sync. A message the sync removed meanwhile, or (with `jmap`) a row that
/// is another email now, is dropped quietly: nothing is written (#472, #488).
pub(crate) fn save_opened_body(
    store: &Store,
    message: MessageId,
    jmap: Option<JmapEmail<'_>>,
    raw: &[u8],
) -> Result<(), SyncError> {
    let parsed = parse_body(message, raw);
    let saved = store.urgent(|| {
        #[cfg(test)]
        OPENED_SAVES.with(|s| s.borrow_mut().push(store.urgent_here()));
        store_body_parts(store, message, jmap, raw, &parsed)
    });
    match saved {
        Err(SyncError::UnknownMessage) => {
            tracing::debug!(
                message = message.0,
                "an opened message was removed or replaced before its body was saved"
            );
            Ok(())
        }
        other => other,
    }
}

/// A downloaded message parsed for storing.
pub(crate) struct ParsedBody {
    snippet: String,
    attachments: Vec<NewAttachment>,
    /// Plain text, or text derived from the HTML: indexed for search.
    body_text: String,
}

/// Parses `raw` for storing. A message that can't be parsed is still kept
/// (as source), with no snippet, parts or text. CPU work, no writes.
pub(crate) fn parse_body(message: MessageId, raw: &[u8]) -> ParsedBody {
    match mailcrane_mime::parse_message(raw) {
        Ok(parsed) => ParsedBody {
            attachments: parsed
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
                .collect(),
            snippet: parsed.snippet,
            body_text: parsed.text_plain.unwrap_or_default(),
        },
        Err(error) => {
            tracing::warn!(message = message.0, %error, "message could not be parsed");
            ParsedBody {
                snippet: String::new(),
                attachments: Vec::new(),
                body_text: String::new(),
            }
        }
    }
}

/// Writes a parsed body: the raw bytes, snippet and parts in one
/// transaction, then the search text. `UnknownMessage` if the row is gone
/// (or, with `jmap`, another email now).
fn store_body_parts(
    store: &Store,
    message: MessageId,
    jmap: Option<JmapEmail<'_>>,
    raw: &[u8],
    parsed: &ParsedBody,
) -> Result<(), SyncError> {
    let fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0);
    let stored = match jmap {
        None => store.store_parsed_body(
            message,
            raw,
            fetched_at,
            &parsed.snippet,
            &parsed.attachments,
        )?,
        Some(email) => store.store_parsed_body_for_jmap_email(
            message,
            (email.account, email.email_id),
            raw,
            fetched_at,
            &parsed.snippet,
            &parsed.attachments,
        )?,
    };
    if !stored {
        return Err(SyncError::UnknownMessage);
    }
    store.index_body_text(message, &parsed.body_text)?;
    Ok(())
}

/// Stores a downloaded message: the raw bytes, its snippet and attachment
/// list, and its text for search. A message that can't be parsed is still
/// kept, so it can be opened as source. Shared by IMAP and JMAP.
pub(crate) fn save_body(store: &Store, message: MessageId, raw: &[u8]) -> Result<(), SyncError> {
    let parsed = parse_body(message, raw);
    store_body_parts(store, message, None, raw, &parsed)
}

#[cfg(test)]
#[path = "initial_tests.rs"]
mod tests;
