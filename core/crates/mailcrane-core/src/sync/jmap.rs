//! Syncing a JMAP account (M3-04, RFC 8620 and RFC 8621) into the same
//! store tables IMAP uses, so the rest of the app can't tell them apart.
//!
//! - Mailboxes become folders; `Mailbox/get` refreshes them.
//! - A folder's first sync stores its newest emails (`Email/query`, newest
//!   first, then `Email/get`).
//! - After that, one `Email/changes` call per account brings every synced
//!   folder up to date (RFC 8620 §5.2): created and updated emails are
//!   (re)stored in each folder they're now in and removed from the others;
//!   destroyed ones are removed. `cannotCalculateChanges` starts over.
//! - Bodies are the email's blob, downloaded once (RFC 8620 §6.2).

use super::convert::jmap_header_row;
use super::initial::{save_body, SyncConfig, SyncError};
use crate::events::{AccountId, CoreEvent, EventListener, FolderId};
use mailcrane_jmap::{Auth, ClientConfig, JmapEmail, JmapError, MailboxRole, Session};
use mailcrane_store::{BodyState, FolderRole, MessageId, NewFolder, Store};
use std::collections::{BTreeSet, HashMap};

/// The `Email` data type's state key in the store.
const EMAIL: &str = "Email";
/// The `Mailbox` data type's state key in the store.
const MAILBOX: &str = "Mailbox";
/// Ids per `Email/get` call.
const GET_BATCH: usize = 50;
/// Changes asked for per `Email/changes` call.
const MAX_CHANGES: u32 = 256;
/// `Email/changes` pages per sync, so a hostile server can't loop us.
const MAX_CHANGE_PAGES: usize = 200;
/// Deepest mailbox nesting followed when building folder paths.
const MAX_DEPTH: usize = 32;

/// Syncs one JMAP account over its session.
pub struct JmapSyncer<'a> {
    store: &'a Store,
    listener: &'a dyn EventListener,
    account: AccountId,
    session: Session,
    auth: Auth,
    config: ClientConfig,
    sync: SyncConfig,
}

impl<'a> JmapSyncer<'a> {
    /// Fetches the session (RFC 8620 §2) and returns a syncer for it.
    pub async fn connect(
        store: &'a Store,
        listener: &'a dyn EventListener,
        account: AccountId,
        session_url: &str,
        auth: Auth,
        config: ClientConfig,
        sync: SyncConfig,
    ) -> Result<JmapSyncer<'a>, SyncError> {
        let session = mailcrane_jmap::discover(session_url, &auth, &config).await?;
        Ok(Self {
            store,
            listener,
            account,
            session,
            auth,
            config,
            sync,
        })
    }

    /// Lists the account's mailboxes and saves them as folders, keeping each
    /// folder's sync state. Returns the inbox, if there is one.
    pub async fn refresh_mailboxes(&mut self) -> Result<Option<FolderId>, SyncError> {
        let result = self.refresh_inner().await;
        self.report(result)
    }

    /// Brings the folder up to date: the account's changes since the last
    /// sync, and the newest emails if the folder was never synced. Returns
    /// how many rows were stored or changed.
    pub async fn sync_folder(&mut self, folder: FolderId) -> Result<usize, SyncError> {
        let result = self.sync_inner(folder).await;
        self.report(result)
    }

    /// Stores up to `count` emails older than the ones the folder has.
    pub async fn load_older(&mut self, folder: FolderId, count: usize) -> Result<usize, SyncError> {
        let result = self.older_inner(folder, count).await;
        self.report(result)
    }

    /// Downloads the message once and keeps it for offline reading. JMAP
    /// downloads never change `$seen`.
    pub async fn fetch_body(&mut self, message: MessageId) -> Result<(), SyncError> {
        let result = self.body_inner(message).await;
        self.report(result)
    }

    /// Downloads one batch of bodies ahead of time (M4-25) and saves each
    /// one; see [`super::prefetch`]. Blob downloads never change `$seen`.
    /// An email the server no longer has, or one over `max_size` bytes, is
    /// left as it was. Sends no events: the caller reports the batch.
    pub async fn prefetch_batch(
        &mut self,
        batch: &[super::prefetch::Candidate],
        max_size: u32,
    ) -> Result<super::prefetch::BatchOutcome, SyncError> {
        let limit = usize::try_from(max_size).unwrap_or(usize::MAX);
        let mut outcome = super::prefetch::BatchOutcome {
            saved: 0,
            tried: batch.iter().map(|c| c.id).collect(),
        };
        for candidate in batch {
            let Some(row) = self.store.jmap_row(candidate.id)? else {
                continue;
            };
            let raw = match mailcrane_jmap::download_blob(
                &self.session,
                &self.auth,
                &self.config,
                &row.blob_id,
            )
            .await
            {
                Ok(raw) => raw,
                Err(JmapError::Http(404)) => continue,
                Err(e) => return Err(e.into()),
            };
            if raw.len() > limit {
                continue;
            }
            match save_body(self.store, candidate.id, &raw) {
                Ok(()) => outcome.saved = outcome.saved.saturating_add(1),
                Err(SyncError::UnknownMessage) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(outcome)
    }

    /// Carries the account's queued message actions out on the server,
    /// oldest first (RFC 8621 §4.6 `Email/set`): read and flag states as
    /// keywords, moves as `mailboxIds`, deletion for good as `destroy`. Stops
    /// at the first that fails for now; one the server refuses is undone
    /// locally. Errors only for a local database failure, like IMAP replay.
    pub async fn replay_actions(
        &mut self,
        now: i64,
    ) -> Result<crate::actions::ReplayReport, SyncError> {
        use crate::actions::{backoff_secs, ReplayReport};
        use mailcrane_jmap::EmailAction;
        use mailcrane_store::OpKind;
        let ops = self.store.pending_ops(self.account)?;
        let mut report = ReplayReport::default();
        for (index, op) in ops.iter().enumerate() {
            if op.next_attempt_at > now {
                report.remaining = ops.len().saturating_sub(index);
                break;
            }
            // Each local row with its JMAP email, so a refusal per email can be
            // undone per message.
            let mut rows: Vec<(String, MessageId)> = Vec::new();
            for m in &op.messages {
                if let Some(row) = self.store.jmap_row(m.message_id)? {
                    rows.push((row.email_id, m.message_id));
                }
            }
            let ids: Vec<String> = rows.iter().map(|(e, _)| e.clone()).collect();
            let action = match op.kind {
                OpKind::MarkRead => Some(EmailAction::Keyword {
                    keyword: "$seen".into(),
                    on: true,
                }),
                OpKind::MarkUnread => Some(EmailAction::Keyword {
                    keyword: "$seen".into(),
                    on: false,
                }),
                OpKind::Flag => Some(EmailAction::Keyword {
                    keyword: "$flagged".into(),
                    on: true,
                }),
                OpKind::Unflag => Some(EmailAction::Keyword {
                    keyword: "$flagged".into(),
                    on: false,
                }),
                OpKind::Expunge => Some(EmailAction::Destroy),
                OpKind::Move => {
                    let from = self.store.jmap_mailbox(op.folder_id)?;
                    let to = match op.dest_folder_id {
                        Some(dest) => self.store.jmap_mailbox(dest)?,
                        None => None,
                    };
                    from.zip(to)
                        .map(|(from, to)| EmailAction::Move { from, to })
                }
            };
            let expunged = matches!(op.kind, OpKind::Move | OpKind::Expunge);
            // Ok(refused messages) when the server changed at least some of them.
            let result: Result<Vec<MessageId>, Option<JmapError>> = match action {
                // The rows are gone locally (a sync removed them): nothing to do.
                _ if ids.is_empty() => Ok(Vec::new()),
                None => Err(None),
                Some(action) => {
                    match mailcrane_jmap::apply(
                        &self.session,
                        &self.auth,
                        &self.config,
                        &ids,
                        &action,
                    )
                    .await
                    {
                        Ok(outcome) => {
                            // Already gone counts as done (a destroy repeated after a lost
                            // reply); anything else refused is undone for those emails only.
                            // Their server state never changed, so Email/changes would never
                            // correct an optimistic local change on them.
                            let refused: Vec<MessageId> = rows
                                .iter()
                                .filter(|(email, _)| {
                                    outcome
                                        .failed
                                        .iter()
                                        .any(|(id, kind)| id == email && kind != "notFound")
                                })
                                .map(|(_, m)| *m)
                                .collect();
                            if !refused.is_empty() && refused.len() == rows.len() {
                                Err(None)
                            } else {
                                Ok(refused)
                            }
                        }
                        Err(e) => Err(Some(e)),
                    }
                }
            };
            match result {
                Ok(refused) => {
                    if !refused.is_empty() {
                        let undone = self.store.fail_op_messages(op.id, &refused)?;
                        tracing::warn!(
                            refused = undone.len(),
                            "JMAP server refused part of a message action; undone for those"
                        );
                        report.failed = report.failed.saturating_add(1);
                        self.listener.on_event(CoreEvent::MessagesChanged {
                            folder_id: op.folder_id,
                        });
                        self.listener.on_event(CoreEvent::ActionFailed {
                            account_id: self.account,
                            action: crate::actions::action_kind(op.kind),
                            message_ids: undone,
                            reason: crate::events::SyncErrorKind::Protocol,
                        });
                    }
                    self.store.complete_op(op.id, expunged)?;
                    report.done = report.done.saturating_add(1);
                }
                Err(Some(e)) if transient(&e) => {
                    let attempts = op.attempts.saturating_add(1);
                    let next = now.saturating_add(backoff_secs(attempts));
                    self.store.defer_op(op.id, attempts, next, "network")?;
                    report.remaining = ops.len().saturating_sub(index);
                    break;
                }
                Err(e) => {
                    let reverted = self.store.fail_op(op.id)?;
                    tracing::warn!(kind = ?op.kind, "JMAP server refused a message action; undone");
                    report.failed = report.failed.saturating_add(1);
                    self.listener.on_event(CoreEvent::MessagesChanged {
                        folder_id: op.folder_id,
                    });
                    self.listener.on_event(CoreEvent::ActionFailed {
                        account_id: self.account,
                        action: crate::actions::action_kind(op.kind),
                        message_ids: reverted,
                        reason: e.map_or(crate::events::SyncErrorKind::Protocol, |e| {
                            SyncError::from(e).kind()
                        }),
                    });
                }
            }
        }
        Ok(report)
    }

    fn report<T>(&self, result: Result<T, SyncError>) -> Result<T, SyncError> {
        if let Err(error) = &result {
            tracing::warn!(kind = ?error.kind(), "JMAP sync failed");
            self.listener.on_event(CoreEvent::SyncError {
                account_id: self.account,
                kind: error.kind(),
            });
        }
        result
    }

    async fn refresh_inner(&mut self) -> Result<Option<FolderId>, SyncError> {
        let (state, mailboxes) =
            mailcrane_jmap::get_mailboxes(&self.session, &self.auth, &self.config).await?;
        let by_id: HashMap<&str, &mailcrane_jmap::JmapMailbox> =
            mailboxes.iter().map(|m| (m.id.as_str(), m)).collect();
        let existing = self.store.list_folders(self.account)?;
        let mut inbox = None;
        for m in &mailboxes {
            // "Parent/Child", following parents (RFC 8621 §2: parentId).
            let mut parts = vec![m.name.as_str()];
            let mut parent = m.parent_id.as_deref();
            let mut depth = 0;
            while let Some(p) = parent.and_then(|id| by_id.get(id)) {
                depth += 1;
                if depth > MAX_DEPTH {
                    break;
                }
                parts.push(p.name.as_str());
                parent = p.parent_id.as_deref();
            }
            parts.reverse();
            let path = parts.join("/");
            let old = existing.iter().find(|f| f.details.remote_name == path);
            let role = m.role.as_ref().and_then(folder_role);
            let id = self.store.upsert_folder(&NewFolder {
                account_id: self.account,
                remote_name: path,
                display_name: m.name.clone(),
                delimiter: Some("/".into()),
                role,
                uidvalidity: old.and_then(|f| f.details.uidvalidity),
                uidnext: old.and_then(|f| f.details.uidnext),
                highestmodseq: None,
                is_subscribed: true,
                selectable: true,
            })?;
            self.store.set_jmap_mailbox(id, &m.id)?;
            if role == Some(FolderRole::Inbox) {
                inbox = Some(id);
            }
        }
        self.store.set_jmap_state(self.account, MAILBOX, &state)?;
        Ok(inbox)
    }

    async fn sync_inner(&mut self, folder: FolderId) -> Result<usize, SyncError> {
        let mailbox = self
            .store
            .jmap_mailbox(folder)?
            .ok_or(SyncError::UnknownFolder)?;
        let mut stored = 0usize;
        if let Some(since) = self.store.jmap_state(self.account, EMAIL)? {
            stored = stored.saturating_add(self.apply_changes(since).await?);
        }
        let row = self
            .store
            .get_folder(folder)?
            .ok_or(SyncError::UnknownFolder)?;
        if row.details.uidvalidity.is_none() {
            stored = stored.saturating_add(self.first_sync(folder, &mailbox).await?);
        }
        Ok(stored)
    }

    /// The folder's newest emails, for its first sync.
    async fn first_sync(&mut self, folder: FolderId, mailbox: &str) -> Result<usize, SyncError> {
        let limit = u32::try_from(self.sync.initial_count).unwrap_or(u32::MAX);
        let page = mailcrane_jmap::query_mailbox(
            &self.session,
            &self.auth,
            &self.config,
            mailbox,
            0,
            limit,
        )
        .await?;
        let stored = self.store_into(folder, &page.ids).await?;
        self.store.mark_folder_synced(folder)?;
        Ok(stored)
    }

    async fn older_inner(&mut self, folder: FolderId, count: usize) -> Result<usize, SyncError> {
        let mailbox = self
            .store
            .jmap_mailbox(folder)?
            .ok_or(SyncError::UnknownFolder)?;
        let position = self.store.count_folder_messages(folder)?;
        let limit = u32::try_from(count).unwrap_or(u32::MAX);
        let page = mailcrane_jmap::query_mailbox(
            &self.session,
            &self.auth,
            &self.config,
            &mailbox,
            position,
            limit,
        )
        .await?;
        self.store_into(folder, &page.ids).await
    }

    /// Fetches `ids` and stores those in `folder`'s mailbox into it. The
    /// first fetch of an account also records where `Email/changes` starts.
    async fn store_into(&mut self, folder: FolderId, ids: &[String]) -> Result<usize, SyncError> {
        let mut stored = 0usize;
        for batch in ids.chunks(GET_BATCH) {
            let (state, emails) =
                mailcrane_jmap::get_emails(&self.session, &self.auth, &self.config, batch).await?;
            if self.store.jmap_state(self.account, EMAIL)?.is_none() && !state.is_empty() {
                self.store.set_jmap_state(self.account, EMAIL, &state)?;
            }
            for e in &emails {
                self.store
                    .upsert_jmap_email(&jmap_header_row(folder, e), &e.id, &e.blob_id)?;
                stored = stored.saturating_add(1);
            }
            self.listener
                .on_event(CoreEvent::MessagesChanged { folder_id: folder });
        }
        Ok(stored)
    }

    /// `Email/changes` from `since` until the server has no more (RFC 8620
    /// §5.2), applied to every folder that has been synced.
    async fn apply_changes(&mut self, mut since: String) -> Result<usize, SyncError> {
        let mut touched = 0usize;
        for _ in 0..MAX_CHANGE_PAGES {
            let changes = match mailcrane_jmap::email_changes(
                &self.session,
                &self.auth,
                &self.config,
                &since,
                MAX_CHANGES,
            )
            .await
            {
                Ok(c) => c,
                Err(JmapError::Method(kind)) if kind == "cannotCalculateChanges" => {
                    // RFC 8620 §5.2: the client must start over for this type.
                    tracing::info!("JMAP server can't calculate changes; starting over");
                    self.start_over()?;
                    return Ok(touched);
                }
                Err(e) => return Err(e.into()),
            };
            let mut changed: BTreeSet<FolderId> = BTreeSet::new();
            for id in &changes.destroyed {
                for row in self.store.jmap_rows(self.account, id)? {
                    changed.insert(row.folder_id);
                    self.store.delete_message_rows(&[row.message_id])?;
                    touched = touched.saturating_add(1);
                }
            }
            let mut ids: Vec<String> = changes.created.clone();
            ids.extend(changes.updated.iter().cloned());
            ids.sort();
            ids.dedup();
            for batch in ids.chunks(GET_BATCH) {
                let (_, emails) =
                    mailcrane_jmap::get_emails(&self.session, &self.auth, &self.config, batch)
                        .await?;
                // Ids the server no longer returns are gone.
                for id in batch {
                    if !emails.iter().any(|e| &e.id == id) {
                        for row in self.store.jmap_rows(self.account, id)? {
                            changed.insert(row.folder_id);
                            self.store.delete_message_rows(&[row.message_id])?;
                        }
                    }
                }
                for e in &emails {
                    touched = touched.saturating_add(self.place(e, &mut changed)?);
                }
            }
            self.store
                .set_jmap_state(self.account, EMAIL, &changes.new_state)?;
            for folder in changed {
                self.listener
                    .on_event(CoreEvent::MessagesChanged { folder_id: folder });
            }
            if !changes.has_more_changes || changes.new_state == since {
                break;
            }
            since = changes.new_state;
        }
        Ok(touched)
    }

    /// Stores the email in each synced folder it's now in, and removes it
    /// from the folders it left.
    fn place(&self, e: &JmapEmail, changed: &mut BTreeSet<FolderId>) -> Result<usize, SyncError> {
        let mut n = 0usize;
        let mut wanted = Vec::new();
        for (folder, mailbox) in self.store.jmap_mailboxes(self.account)? {
            if !e.mailbox_ids.iter().any(|m| m == &mailbox) {
                continue;
            }
            let synced = self
                .store
                .get_folder(folder)?
                .is_some_and(|f| f.details.uidvalidity.is_some());
            if synced {
                wanted.push(folder);
            }
        }
        for row in self.store.jmap_rows(self.account, &e.id)? {
            if !wanted.contains(&row.folder_id) {
                self.store.delete_message_rows(&[row.message_id])?;
                changed.insert(row.folder_id);
                n = n.saturating_add(1);
            }
        }
        for folder in wanted {
            self.store
                .upsert_jmap_email(&jmap_header_row(folder, e), &e.id, &e.blob_id)?;
            changed.insert(folder);
            n = n.saturating_add(1);
        }
        Ok(n)
    }

    /// Forgets every folder's emails and the Email state, so each folder
    /// fills again on its next sync.
    fn start_over(&self) -> Result<(), SyncError> {
        for (folder, _) in self.store.jmap_mailboxes(self.account)? {
            self.store.forget_folder(folder)?;
            self.listener
                .on_event(CoreEvent::MessagesChanged { folder_id: folder });
        }
        self.store.clear_jmap_state(self.account, EMAIL)?;
        Ok(())
    }

    async fn body_inner(&mut self, message: MessageId) -> Result<(), SyncError> {
        let header = self
            .store
            .get_message_header(message)?
            .ok_or(SyncError::UnknownMessage)?;
        if header.body_state == BodyState::Fetched {
            return Ok(());
        }
        let row = self
            .store
            .jmap_row(message)?
            .ok_or(SyncError::UnknownMessage)?;
        let raw = match mailcrane_jmap::download_blob(
            &self.session,
            &self.auth,
            &self.config,
            &row.blob_id,
        )
        .await
        {
            Ok(raw) => raw,
            Err(JmapError::Http(404)) => return Err(SyncError::MessageGone),
            Err(e) => return Err(e.into()),
        };
        save_body(self.store, message, &raw)?;
        self.listener.on_event(CoreEvent::MessagesChanged {
            folder_id: header.details.folder_id,
        });
        Ok(())
    }
}

/// Whether a failed call is worth trying again later.
fn transient(e: &JmapError) -> bool {
    matches!(e, JmapError::Network | JmapError::Timeout)
        || matches!(e, JmapError::Http(code) if *code >= 500)
}

/// A mailbox role as a folder role (RFC 8621 §2, the IANA mailbox roles).
fn folder_role(role: &MailboxRole) -> Option<FolderRole> {
    Some(match *role {
        MailboxRole::Inbox => FolderRole::Inbox,
        MailboxRole::Drafts => FolderRole::Drafts,
        MailboxRole::Sent => FolderRole::Sent,
        MailboxRole::Archive => FolderRole::Archive,
        MailboxRole::Junk => FolderRole::Spam,
        MailboxRole::Trash => FolderRole::Trash,
        MailboxRole::All => FolderRole::All,
    })
}
