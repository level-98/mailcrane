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
use crate::folder_admin::{CreatePlan, FolderAdminError, RenamePlan};
use mailcrane_jmap::{Auth, ClientConfig, JmapEmail, JmapError, MailboxRole, Session};
use mailcrane_store::{BodyState, Folder, FolderRole, MessageId, NewFolder, Store, VirtualRole};
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

    /// Creates the planned folder as a mailbox (M4-26), then stores it the
    /// way [`JmapSyncer::refresh_mailboxes`] would: the path as its remote
    /// name, its own name to show, and the mailbox id. `parent` is the
    /// folder it goes under. Nothing is stored if the server refuses.
    pub async fn create_folder(
        &mut self,
        plan: &CreatePlan,
        parent: Option<FolderId>,
    ) -> Result<FolderId, FolderAdminError> {
        let parent_mailbox = match parent {
            Some(p) => Some(self.mailbox_of(p)?),
            None => None,
        };
        let name = leaf(&plan.remote_name);
        let id = mailcrane_jmap::create_mailbox(
            &self.session,
            &self.auth,
            &self.config,
            name,
            parent_mailbox.as_deref(),
        )
        .await?;
        let folder = self.store.upsert_folder(&NewFolder {
            account_id: self.account,
            remote_name: plan.remote_name.clone(),
            display_name: name.to_owned(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
            virtual_role: None,
        })?;
        self.store.set_jmap_mailbox(folder, &id)?;
        Ok(folder)
    }

    /// Renames or moves `folder` as planned (M4-26): one `Mailbox/set`
    /// update of its name and parent, then the stored paths of it and its
    /// subfolders. Nothing is stored if the server refuses.
    pub async fn rename_folder(
        &mut self,
        folder: FolderId,
        plan: &RenamePlan,
    ) -> Result<(), FolderAdminError> {
        if plan.is_noop() {
            return Ok(());
        }
        let mailbox = self.mailbox_of(folder)?;
        let parent_mailbox = match plan.new_remote.rsplit_once('/') {
            Some((parent_path, _)) => {
                let parent = self
                    .store
                    .list_folders(self.account)?
                    .into_iter()
                    .find(|f| f.details.remote_name == parent_path)
                    .ok_or(FolderAdminError::NotFound)?;
                Some(self.mailbox_of(parent.id)?)
            }
            None => None,
        };
        mailcrane_jmap::update_mailbox(
            &self.session,
            &self.auth,
            &self.config,
            &mailbox,
            leaf(&plan.new_remote),
            parent_mailbox.as_deref(),
        )
        .await?;
        // JMAP folders show their own name, not the path.
        let rows: Vec<_> = plan
            .rows
            .iter()
            .map(|(id, remote, _)| (*id, remote.clone(), leaf(remote).to_owned()))
            .collect();
        self.store.rename_folders(&rows)?;
        Ok(())
    }

    /// Destroys the folder's mailbox (M4-26). The server refuses one that
    /// still holds emails (`HasMessages`); only then is it removed here.
    pub async fn delete_folder(&mut self, folder: &Folder) -> Result<(), FolderAdminError> {
        let mailbox = self.mailbox_of(folder.id)?;
        mailcrane_jmap::destroy_mailbox(&self.session, &self.auth, &self.config, &mailbox).await?;
        self.store.delete_folder(folder.id)?;
        Ok(())
    }

    fn mailbox_of(&self, folder: FolderId) -> Result<String, FolderAdminError> {
        self.store
            .jmap_mailbox(folder)?
            .ok_or(FolderAdminError::NotFound)
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
                // Only while the row is still that message: in the action's
                // folder with its UID. Row ids are reused once a row is
                // deleted, so it could be another email now (#517).
                let same = self
                    .store
                    .get_message_header(m.message_id)?
                    .is_some_and(|h| h.details.folder_id == op.folder_id && h.details.uid == m.uid);
                if !same {
                    continue;
                }
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
            // Ok((refused messages, unanswered emails)) when the server changed
            // at least some of them.
            type Answer = (Vec<MessageId>, Vec<String>);
            let result: Result<Answer, Option<JmapError>> = match &action {
                // The rows are gone locally (a sync removed them): nothing to do.
                _ if ids.is_empty() => Ok((Vec::new(), Vec::new())),
                None => Err(None),
                Some(action) => {
                    match mailcrane_jmap::apply(
                        &self.session,
                        &self.auth,
                        &self.config,
                        &ids,
                        action,
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
                            // RFC 8620 §5.3: every id must be answered, as done
                            // or not. One left out of both is unconfirmed: its
                            // server state is fetched below and applied, never
                            // guessed (#448).
                            let unanswered: Vec<String> = ids
                                .iter()
                                .filter(|id| {
                                    !outcome.done.contains(id)
                                        && !outcome.failed.iter().any(|(f, _)| f == *id)
                                })
                                .cloned()
                                .collect();
                            if !refused.is_empty() && refused.len() == rows.len() {
                                Err(None)
                            } else {
                                Ok((refused, unanswered))
                            }
                        }
                        Err(e) => Err(Some(e)),
                    }
                }
            };
            // The server state of unanswered emails, fetched before the op is
            // completed: a dropped connection here defers the op (its set or
            // destroy can be repeated safely); a refusal undoes the change for
            // those emails, since it can't be confirmed.
            //
            // An email with a later queued op is still fetched and checked,
            // so this op is never counted done for it unconfirmed (that
            // would be #448 again). Only the server's state isn't applied
            // to it, so the later op's local result survives until its own
            // replay settles it (lead's decision on #451).
            let later_op = |email: &String| {
                rows.iter().filter(|(e, _)| e == email).any(|(_, m)| {
                    ops.get(index.saturating_add(1)..)
                        .unwrap_or_default()
                        .iter()
                        .any(|o| o.messages.iter().any(|om| om.message_id == *m))
                })
            };
            let result = match result {
                Ok((mut refused, unanswered)) if !unanswered.is_empty() => {
                    tracing::warn!(
                        unanswered = unanswered.len(),
                        "JMAP server left emails out of its Email/set reply; reconciling"
                    );
                    match self.get_in_batches(&unanswered).await {
                        Ok(emails) => Ok((refused, Some((unanswered, emails)))),
                        Err(e) if transient(&e) => Err(Some(e)),
                        Err(_) => {
                            refused.extend(
                                rows.iter()
                                    .filter(|(email, _)| unanswered.contains(email))
                                    .map(|(_, m)| *m),
                            );
                            // Nothing confirmed at all: the whole op failed.
                            if refused.len() == rows.len() {
                                Err(None)
                            } else {
                                Ok((refused, None))
                            }
                        }
                    }
                }
                Ok((refused, _)) => Ok((refused, None)),
                Err(e) => Err(e),
            };
            match result {
                Ok((mut refused, reconcile)) => {
                    // One op counts as failed at most once in the report.
                    let mut counted_failed = false;
                    // Unconfirmed emails first go back to how they were here
                    // (detached from the op, so completing it can't delete
                    // them), then take the server's state, as Email/changes
                    // would give it. Only then is the op completed. A local
                    // database failure at any point leaves the email shown,
                    // never lost (lead's decision on #451).
                    if let Some((unanswered, emails)) = &reconcile {
                        let later: Vec<&String> =
                            unanswered.iter().filter(|e| later_op(e)).collect();
                        let is_applied = |email: &String| {
                            action
                                .as_ref()
                                .is_some_and(|a| applied(a, emails.iter().find(|e| &e.id == email)))
                        };
                        // An email with a later op stays in this op only when
                        // the server's state confirms the change; otherwise
                        // this op's change is undone for it too.
                        let unconfirmed: Vec<MessageId> = rows
                            .iter()
                            .filter(|(email, _)| {
                                unanswered.contains(email)
                                    && (!later.contains(&email) || !is_applied(email))
                            })
                            .map(|(_, m)| *m)
                            .collect();
                        self.store.fail_op_messages(op.id, &unconfirmed)?;
                        let mut changed: BTreeSet<FolderId> = BTreeSet::new();
                        for id in unanswered.iter().filter(|id| !later.contains(id)) {
                            if !emails.iter().any(|e| &e.id == id) {
                                for row in self.store.jmap_rows(self.account, id)? {
                                    changed.insert(row.folder_id);
                                    self.store.delete_message_rows(&[row.message_id])?;
                                }
                            }
                        }
                        for e in emails.iter().filter(|e| !later.contains(&&e.id)) {
                            self.place(e, &mut changed)?;
                        }
                        changed.insert(op.folder_id);
                        for folder in changed {
                            self.listener
                                .on_event(CoreEvent::MessagesChanged { folder_id: folder });
                        }
                        // The ones the server didn't change are a refusal the
                        // user should hear about, like any other.
                        {
                            let not_applied: Vec<MessageId> = rows
                                .iter()
                                .filter(|(email, _)| {
                                    unanswered.contains(email) && !is_applied(email)
                                })
                                .map(|(_, m)| *m)
                                .collect();
                            if !not_applied.is_empty() {
                                tracing::warn!(
                                    not_applied = not_applied.len(),
                                    "JMAP server didn't apply a change it left out of its reply"
                                );
                                report.failed = report.failed.saturating_add(1);
                                counted_failed = true;
                                self.listener.on_event(CoreEvent::ActionFailed {
                                    account_id: self.account,
                                    action: crate::actions::action_kind(op.kind),
                                    message_ids: not_applied,
                                    reason: crate::events::SyncErrorKind::Protocol,
                                });
                            }
                        }
                        refused.retain(|m| !unconfirmed.contains(m));
                    }
                    if !refused.is_empty() {
                        let undone = self.store.fail_op_messages(op.id, &refused)?;
                        tracing::warn!(
                            refused = undone.len(),
                            "JMAP server refused part of a message action; undone for those"
                        );
                        if !counted_failed {
                            report.failed = report.failed.saturating_add(1);
                        }
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
            let virtual_role = m.role.as_ref().and_then(virtual_role);
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
                virtual_role,
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
        for batch in ids.chunks(self.get_batch()) {
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
            for batch in ids.chunks(self.get_batch()) {
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

    /// How many ids one `Email/get` asks for: [`GET_BATCH`], or fewer when
    /// the server's `maxObjectsInGet` (RFC 8620 §2) is lower.
    fn get_batch(&self) -> usize {
        self.session
            .max_objects_in_get
            .map_or(GET_BATCH, |n| n.min(GET_BATCH))
            .max(1)
    }

    /// `Email/get` of `ids`, in batches of at most the server's
    /// `maxObjectsInGet` (RFC 8620 §2), and never more than [`GET_BATCH`].
    async fn get_in_batches(&self, ids: &[String]) -> Result<Vec<JmapEmail>, JmapError> {
        let mut out = Vec::new();
        for batch in ids.chunks(self.get_batch()) {
            let (_, emails) =
                mailcrane_jmap::get_emails(&self.session, &self.auth, &self.config, batch).await?;
            out.extend(emails);
        }
        Ok(out)
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
            let dropped = self.store.forget_folder(folder)?;
            self.listener
                .on_event(CoreEvent::MessagesChanged { folder_id: folder });
            // Its queued changes were keyed by the rows just forgotten (#517).
            for op in dropped {
                tracing::warn!(
                    folder = folder.0,
                    kind = ?op.kind,
                    "queued action dropped: the folder was forgotten"
                );
                self.listener.on_event(CoreEvent::ActionFailed {
                    account_id: self.account,
                    action: crate::actions::action_kind(op.kind),
                    message_ids: op.message_ids,
                    reason: crate::events::SyncErrorKind::FolderReset,
                });
            }
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
        // Only while the row is still this email: ids are reused (#488).
        super::initial::save_opened_body(
            self.store,
            message,
            Some(super::initial::JmapEmail {
                account: self.account,
                email_id: &row.email_id,
            }),
            &raw,
        )?;
        self.listener.on_event(CoreEvent::MessagesChanged {
            folder_id: header.details.folder_id,
        });
        Ok(())
    }
}

/// Whether a failed call is worth trying again later.
/// Whether the server's state of an email shows `action` carried out.
fn applied(action: &mailcrane_jmap::EmailAction, email: Option<&JmapEmail>) -> bool {
    use mailcrane_jmap::EmailAction;
    match action {
        EmailAction::Destroy => email.is_none(),
        EmailAction::Keyword { keyword, on } => {
            email.is_some_and(|e| e.keywords.iter().any(|k| k == keyword) == *on)
        }
        EmailAction::Move { to, .. } => {
            email.is_some_and(|e| e.mailbox_ids.iter().any(|m| m == to))
        }
    }
}

fn transient(e: &JmapError) -> bool {
    matches!(e, JmapError::Network | JmapError::Timeout)
        || matches!(e, JmapError::Http(code) if *code >= 500)
}

/// A mailbox role as a folder role (RFC 8621 §2, the IANA mailbox roles).
/// `flagged` is virtual ([`virtual_role`], #461) and `important` an
/// ordinary mailbox: neither gets a folder role.
fn folder_role(role: &MailboxRole) -> Option<FolderRole> {
    Some(match *role {
        MailboxRole::Inbox => FolderRole::Inbox,
        MailboxRole::Drafts => FolderRole::Drafts,
        MailboxRole::Sent => FolderRole::Sent,
        MailboxRole::Archive => FolderRole::Archive,
        MailboxRole::Junk => FolderRole::Spam,
        MailboxRole::Trash => FolderRole::Trash,
        MailboxRole::All => FolderRole::All,
        MailboxRole::Flagged | MailboxRole::Important => return None,
    })
}

/// A mailbox role as a virtual folder (#390, #461). Only `flagged`, as
/// IMAP's `\Flagged`: `important` (RFC 8457 §3) can be a real folder, and
/// IMAP counts it as virtual only on Gmail, which has no JMAP.
fn virtual_role(role: &MailboxRole) -> Option<VirtualRole> {
    match *role {
        MailboxRole::Flagged => Some(VirtualRole::Starred),
        _ => None,
    }
}
/// The last path segment: a JMAP mailbox's own name.
fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
