//! Message actions (M1-14): read, unread, flag, unflag, archive, delete and
//! move.
//!
//! An action changes the local store at once (and tells the apps), and is
//! queued in the same transaction. [`replay`] later carries the queue out on
//! the server, in order, retrying with back-off; if the server refuses an
//! action for good, the local change is undone and
//! [`CoreEvent::ActionFailed`] is sent.

mod replay;

pub(crate) use replay::{action_kind, report_folder_reset};
pub use replay::{backoff_secs, replay, ReplayReport};

use crate::events::{CoreEvent, EventListener};
use mailcrane_store::{
    AccountId, FolderId, FolderRole, MessageHeader, MessageId, NewOp, OpKind, OpMessage, Store,
    StoreError,
};

/// What the user asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Mark as read (`\Seen`).
    MarkRead,
    /// Mark as unread.
    MarkUnread,
    /// Flag (star) it (`\Flagged`).
    Flag,
    /// Remove the flag.
    Unflag,
    /// Move to the account's Archive folder.
    Archive,
    /// Move to Trash, or delete for good if already in Trash or there is no
    /// Trash folder.
    Delete,
    /// Move to another folder of the same account.
    Move {
        /// The destination.
        to: FolderId,
    },
}

/// Why an action could not be applied. Nothing was changed.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// One of the messages is not in the database.
    #[error("unknown message")]
    UnknownMessage,
    /// The destination folder is not in the database, or belongs to another
    /// account.
    #[error("unknown folder")]
    UnknownFolder,
    /// The destination is a virtual folder, such as Gmail's Starred (#390): a
    /// message is in it because of a flag or the server's own judgement, so
    /// nothing moves into it.
    #[error("not a destination")]
    NotADestination,
    /// The account has no Archive folder, so there is nowhere to archive to.
    #[error("the account has no Archive folder")]
    NoArchiveFolder,
    /// The messages are in different folders. Act on one folder at a time.
    #[error("the messages are in different folders")]
    MixedFolders,
    /// A move of messages from several accounts: folders belong to one
    /// account, so there is no destination for all of them (M4-14).
    #[error("the messages are in different accounts")]
    MixedAccounts,
    /// More than [`MAX_BULK`] messages in one action.
    #[error("too many messages")]
    TooMany,
    /// A move or delete of a message whose wake is past step 0 (lead's
    /// decision on G6, as #431 for re-snoozing): the move back has started
    /// on the server. Nothing was changed; it can be done once the message
    /// lands, a few seconds later.
    #[error("the message is being woken")]
    Waking,
    /// The local database failed.
    #[error("store error: {0}")]
    Store(StoreError),
}

impl From<StoreError> for ActionError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Waking => ActionError::Waking,
            other => ActionError::Store(other),
        }
    }
}

/// What [`apply_action`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The messages' account, whose queue should be replayed.
    pub account: AccountId,
    /// Whether anything was queued (false if nothing needed to change, or
    /// the action cancelled out an earlier one).
    pub queued: bool,
}

/// Applies `action` to `messages`, which must all be in one folder: changes
/// the store, queues the action for the server and sends `MessagesChanged`.
/// Messages already in the wanted state are skipped. On error nothing is
/// changed.
pub fn apply_action(
    store: &Store,
    messages: &[MessageId],
    action: Action,
    now: i64,
    listener: &dyn EventListener,
) -> Result<Option<Applied>, ActionError> {
    let mut headers = Vec::with_capacity(messages.len());
    for id in messages {
        let header = store
            .get_message_header(*id)?
            .ok_or(ActionError::UnknownMessage)?;
        headers.push(header);
    }
    let Some(first) = headers.first() else {
        return Ok(None);
    };
    let folder_id = first.details.folder_id;
    if headers.iter().any(|h| h.details.folder_id != folder_id) {
        return Err(ActionError::MixedFolders);
    }
    let plan = plan(store, folder_id, &headers, action)?;
    let account = plan.account;
    if plan.messages.is_empty() {
        return Ok(Some(Applied {
            account,
            queued: false,
        }));
    }
    let kind = plan.kind;
    let count = plan.messages.len();
    let op = store.queue_op(
        account,
        plan.kind,
        plan.folder,
        plan.dest,
        &plan.messages,
        now,
    )?;
    listener.on_event(CoreEvent::MessagesChanged { folder_id });
    tracing::debug!(
        ?kind,
        count,
        queued = op.is_some(),
        "message action applied"
    );
    Ok(Some(Applied {
        account,
        queued: op.is_some(),
    }))
}

/// The op that carries `action` out on `headers`, all in `folder_id`,
/// leaving out messages already in the wanted state. Checks everything and
/// changes nothing.
fn plan(
    store: &Store,
    folder_id: FolderId,
    headers: &[MessageHeader],
    action: Action,
) -> Result<NewOp, ActionError> {
    let folder = store
        .get_folder(folder_id)?
        .ok_or(ActionError::UnknownFolder)?;
    let account = folder.details.account_id;
    let role_folder = |role: FolderRole| -> Result<Option<FolderId>, ActionError> {
        Ok(store
            .list_folders(account)?
            .into_iter()
            .find(|f| f.details.role == Some(role))
            .map(|f| f.id))
    };

    let (kind, dest) = match action {
        Action::MarkRead => (OpKind::MarkRead, None),
        Action::MarkUnread => (OpKind::MarkUnread, None),
        Action::Flag => (OpKind::Flag, None),
        Action::Unflag => (OpKind::Unflag, None),
        Action::Archive => {
            // Gmail has no Archive folder: archiving removes the Inbox label,
            // which a move to All Mail (`\All`, RFC 6154) does (M3-02).
            let archive = match role_folder(FolderRole::Archive)? {
                Some(archive) => archive,
                None => role_folder(FolderRole::All)?.ok_or(ActionError::NoArchiveFolder)?,
            };
            (OpKind::Move, Some(archive))
        }
        Action::Delete => match role_folder(FolderRole::Trash)? {
            Some(trash) if trash != folder_id => (OpKind::Move, Some(trash)),
            _ => (OpKind::Expunge, None),
        },
        Action::Move { to } => {
            let target = store.get_folder(to)?.ok_or(ActionError::UnknownFolder)?;
            if target.details.account_id != account {
                return Err(ActionError::UnknownFolder);
            }
            if target.details.virtual_role.is_some() {
                return Err(ActionError::NotADestination);
            }
            (OpKind::Move, Some(to))
        }
    };

    let messages: Vec<OpMessage> = headers
        .iter()
        .filter(|h| {
            let f = h.details.flags;
            match kind {
                OpKind::MarkRead => !f.seen,
                OpKind::MarkUnread => f.seen,
                OpKind::Flag => !f.flagged,
                OpKind::Unflag => f.flagged,
                OpKind::Move => dest != Some(folder_id),
                OpKind::Expunge => true,
            }
        })
        .map(|h| OpMessage {
            message_id: h.id,
            uid: h.details.uid,
        })
        .collect();
    Ok(NewOp {
        account,
        kind,
        folder: folder_id,
        dest,
        messages,
    })
}

/// The most messages one multi-select action takes.
pub const MAX_BULK: usize = 10_000;

/// What [`apply_bulk_action`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkApplied {
    /// Accounts with something queued, each once, whose queues to replay.
    pub accounts: Vec<AccountId>,
    /// The messages that changed (for undo). Messages already in the wanted
    /// state are not in it.
    pub messages: Vec<MessageId>,
    /// Whether any message was deleted for good rather than moved to Trash.
    pub deleted_for_good: bool,
    /// Messages left as they were because a wake is moving them (#544).
    pub skipped_waking: Vec<MessageId>,
}

/// Multi-select (M4-14): applies `action` to `messages`, which may be in
/// several folders and accounts. Every folder's group is checked first,
/// then all are applied and queued in one transaction, one op per folder;
/// on any error nothing changes. A move or delete leaves out the messages a
/// wake is moving and reports them in [`BulkApplied::skipped_waking`]; only
/// a selection that is all waking fails, with [`ActionError::Waking`]
/// (#544). Sends `MessagesChanged` for each folder that changed. A move
/// needs messages of one account.
pub fn apply_bulk_action(
    store: &Store,
    messages: &[MessageId],
    action: Action,
    now: i64,
    listener: &dyn EventListener,
) -> Result<BulkApplied, ActionError> {
    if messages.len() > MAX_BULK {
        return Err(ActionError::TooMany);
    }
    let groups = group_by_folder(store, messages)?;
    // Before planning: a move's destination fits only one account's groups.
    if matches!(action, Action::Move { .. }) {
        let mut account = None;
        for (folder, _) in &groups {
            let owner = store
                .get_folder(*folder)?
                .ok_or(ActionError::UnknownFolder)?
                .details
                .account_id;
            if account.is_some_and(|a| a != owner) {
                return Err(ActionError::MixedAccounts);
            }
            account = Some(owner);
        }
    }
    let mut ops = Vec::with_capacity(groups.len());
    for (folder, headers) in &groups {
        ops.push(plan(store, *folder, headers, action)?);
    }
    ops.retain(|op| !op.messages.is_empty());
    let mut applied = BulkApplied::default();
    if ops.is_empty() {
        return Ok(applied);
    }
    let queued = store.queue_ops(&ops, now)?;
    let skipped: std::collections::HashSet<MessageId> =
        queued.skipped_waking.iter().copied().collect();
    for (op, id) in ops.iter().zip(&queued.ids) {
        let before = applied.messages.len();
        applied.messages.extend(
            op.messages
                .iter()
                .map(|m| m.message_id)
                .filter(|m| !skipped.contains(m)),
        );
        if applied.messages.len() == before {
            // Every message of this folder was waking: nothing changed here.
            continue;
        }
        if op.kind == OpKind::Expunge {
            applied.deleted_for_good = true;
        }
        if id.is_some() && !applied.accounts.contains(&op.account) {
            applied.accounts.push(op.account);
        }
        listener.on_event(CoreEvent::MessagesChanged {
            folder_id: op.folder,
        });
    }
    applied.skipped_waking = queued.skipped_waking;
    tracing::debug!(
        folders = ops.len(),
        count = applied.messages.len(),
        skipped = applied.skipped_waking.len(),
        "bulk message action applied"
    );
    Ok(applied)
}

/// The messages grouped by folder, in the order the folders first appear;
/// a message listed twice counts once.
fn group_by_folder(
    store: &Store,
    messages: &[MessageId],
) -> Result<Vec<(FolderId, Vec<MessageHeader>)>, ActionError> {
    let mut groups: Vec<(FolderId, Vec<MessageHeader>)> = Vec::new();
    let mut seen = std::collections::HashSet::with_capacity(messages.len());
    for id in messages {
        if !seen.insert(*id) {
            continue;
        }
        let header = store
            .get_message_header(*id)?
            .ok_or(ActionError::UnknownMessage)?;
        let folder = header.details.folder_id;
        match groups.iter_mut().find(|(f, _)| *f == folder) {
            Some((_, group)) => group.push(header),
            None => groups.push((folder, vec![header])),
        }
    }
    Ok(groups)
}

/// How many of `messages` a Delete would delete for good rather than move
/// to Trash: those already in Trash, and those in an account without one.
/// The apps ask first when this isn't 0 (M4-14). Changes nothing.
pub fn count_deleted_for_good(store: &Store, messages: &[MessageId]) -> Result<u32, ActionError> {
    if messages.len() > MAX_BULK {
        return Err(ActionError::TooMany);
    }
    let mut count: u32 = 0;
    for (folder, headers) in group_by_folder(store, messages)? {
        let op = plan(store, folder, &headers, Action::Delete)?;
        if op.kind == OpKind::Expunge {
            let n = u32::try_from(op.messages.len()).unwrap_or(u32::MAX);
            count = count.saturating_add(n);
        }
    }
    Ok(count)
}

/// What [`undo_bulk`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BulkUndo {
    /// Messages put back.
    pub undone: u32,
    /// Messages whose action had already reached the server.
    pub too_late: u32,
}

/// Undo for a multi-select action: takes back the queued action of every
/// one of `messages` that hasn't been tried yet, in one transaction.
pub fn undo_bulk(
    store: &Store,
    messages: &[MessageId],
    listener: &dyn EventListener,
) -> Result<BulkUndo, ActionError> {
    if messages.len() > MAX_BULK {
        return Err(ActionError::TooMany);
    }
    let report = store.cancel_queued_many(messages)?;
    for folder_id in report.folders {
        listener.on_event(CoreEvent::MessagesChanged { folder_id });
    }
    Ok(BulkUndo {
        undone: report.undone,
        too_late: report.too_late,
    })
}

/// Undoes the newest queued action on `message` if it hasn't been sent to
/// the server yet (for "Archived · Undo"). Sends `MessagesChanged`. Returns
/// whether anything was undone; if not, it's too late.
pub fn undo(
    store: &Store,
    message: MessageId,
    listener: &dyn EventListener,
) -> Result<bool, ActionError> {
    match store.cancel_queued(message)? {
        Some(folder_id) => {
            listener.on_event(CoreEvent::MessagesChanged { folder_id });
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests;
