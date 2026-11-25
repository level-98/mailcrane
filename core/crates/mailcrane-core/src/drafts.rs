//! Drafts (M2-05). Nothing a user types is lost: [`save_draft`] stores it at
//! once, and [`upload_drafts`] puts the newest version in the server's Drafts
//! folder later (retrying with back-off), replacing the previous copy.
//!
//! A draft keeps one `Message-ID` for all its versions. After appending a new
//! version, older copies with that id are removed by UID (and `UID EXPUNGE`
//! where the server has it; never a bare `EXPUNGE`). A copy with that id that
//! this device didn't upload was changed elsewhere: it is kept as a second
//! draft, never silently replaced.

use std::collections::BTreeSet;

use mailcrane_imap::{AppendFlags, ChangeError, ImapError, Session, SystemFlag};
use mailcrane_mime::{
    build_message, new_message_id, parse_address_list, parse_message, Address, OutgoingMessage,
    TransferEncoding,
};
use mailcrane_store::{
    AccountId, Draft, DraftFields, DraftId, FolderId, FolderRole, MessageId, OpKind, OpMessage,
    ServerCopy, Store, StoreError,
};

use crate::actions::backoff_secs;
use crate::compose::reply::format_entry;
use crate::sync::SyncError;

/// Why a draft operation failed. Nothing was changed.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    /// The draft, message or account doesn't exist.
    #[error("not found")]
    NotFound,
    /// The message's body isn't downloaded, so it can't be opened as a draft.
    #[error("the message body is not downloaded")]
    BodyNotDownloaded,
    /// No random numbers for a new Message-ID.
    #[error("internal failure")]
    Internal,
    /// The local database failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

fn drafts_folder(store: &Store, account: AccountId) -> Result<Option<FolderId>, StoreError> {
    Ok(store
        .list_folders(account)?
        .into_iter()
        .find(|f| f.details.role == Some(FolderRole::Drafts))
        .map(|f| f.id))
}

fn fresh_message_id(store: &Store, account: AccountId) -> Result<String, DraftError> {
    let email = store
        .get_account(account)?
        .ok_or(DraftError::NotFound)?
        .details
        .email;
    let domain = email.rsplit_once('@').map(|(_, d)| d).unwrap_or_default();
    new_message_id(domain).map_err(|_| DraftError::Internal)
}

/// Saves what the user typed: a new draft for `account` when `id` is
/// `None`, otherwise a new version of draft `id`. Local and instant; the
/// upload happens later. Returns the draft's id.
pub fn save_draft(
    store: &Store,
    account: AccountId,
    id: Option<DraftId>,
    fields: &DraftFields,
    now: i64,
) -> Result<DraftId, DraftError> {
    match id {
        Some(id) => {
            if store.update_draft(id, fields, now)? {
                Ok(id)
            } else {
                Err(DraftError::NotFound)
            }
        }
        None => {
            let message_id = fresh_message_id(store, account)?;
            Ok(store.create_draft(account, fields, &message_id, now)?)
        }
    }
}

/// The compose fields of a parsed message, for a draft from the server.
fn fields_of(raw: &[u8]) -> DraftFields {
    let parsed = parse_message(raw).unwrap_or_default();
    DraftFields {
        to: parsed.to.iter().map(format_entry).collect(),
        cc: parsed.cc.iter().map(format_entry).collect(),
        bcc: parsed.bcc.iter().map(format_entry).collect(),
        subject: parsed.subject.unwrap_or_default(),
        body: parsed.text_plain.unwrap_or_default(),
        in_reply_to: parsed.in_reply_to.first().cloned(),
        references: parsed.references,
    }
}

/// Opens message `message` (from the Drafts folder) as a draft to edit:
/// the draft already linked to it, or a new one imported from its body.
pub fn open_draft(store: &Store, message: MessageId, now: i64) -> Result<DraftId, DraftError> {
    let header = store
        .get_message_header(message)?
        .ok_or(DraftError::NotFound)?;
    let raw = store.get_body(message)?;
    // The body's own Message-ID is the authority; the header row's is the
    // fallback when the body isn't downloaded yet.
    let known_id = raw
        .as_deref()
        .and_then(|r| parse_message(r).ok())
        .and_then(|p| p.message_id)
        .or(header.details.message_id_header.clone());
    if let Some(existing) = known_id
        .as_deref()
        .map(|id| store.find_draft(id))
        .transpose()?
        .flatten()
    {
        return Ok(existing.id);
    }
    let folder = store
        .get_folder(header.details.folder_id)?
        .ok_or(DraftError::NotFound)?;
    let raw = raw.ok_or(DraftError::BodyNotDownloaded)?;
    let account = folder.details.account_id;
    let message_id = match known_id {
        Some(id) => id,
        None => fresh_message_id(store, account)?,
    };
    Ok(store.import_draft(
        account,
        &fields_of(&raw),
        &message_id,
        ServerCopy {
            folder: folder.id,
            uid: header.details.uid,
        },
        false,
        now,
    )?)
}

/// Deletes a draft (after sending it, or when the user discards it). Its
/// server copy is removed through the action queue, so that works offline
/// too.
pub fn delete_draft(store: &Store, id: DraftId, now: i64) -> Result<(), DraftError> {
    let draft = store.get_draft(id)?.ok_or(DraftError::NotFound)?;
    if let Some(copy) = store.delete_draft(id)? {
        let local = store.message_at(copy.folder, copy.uid)?;
        store.queue_op(
            draft.account_id,
            OpKind::Expunge,
            copy.folder,
            None,
            &[OpMessage {
                message_id: local.unwrap_or(MessageId(0)),
                uid: copy.uid,
            }],
            now,
        )?;
    }
    Ok(())
}

/// What one [`upload_drafts`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UploadReport {
    /// Drafts now on the server in their newest version.
    pub uploaded: usize,
    /// Versions changed on another device, now kept as separate drafts.
    pub conflicts: usize,
    /// Drafts still to upload (not due yet, or failed for now).
    pub remaining: usize,
    /// The account has no Drafts folder; drafts stay on this device.
    pub no_drafts_folder: bool,
}

fn addresses(entries: &[String]) -> Vec<Address> {
    // A draft may hold half-typed entries; only valid ones go into the copy.
    entries
        .iter()
        .filter_map(|e| parse_address_list(e).ok())
        .flatten()
        .collect()
}

fn message_bytes(store: &Store, draft: &Draft) -> Result<Vec<u8>, ChangeError> {
    let store_error = |e| ChangeError::Imap(ImapError::Store(e));
    let account = store
        .get_account(draft.account_id)
        .map_err(store_error)?
        .ok_or(ChangeError::Refused { transient: false })?
        .details;
    let f = &draft.fields;
    build_message(&OutgoingMessage {
        from: Address {
            name: Some(account.display_name.trim().to_owned()).filter(|n| !n.is_empty()),
            email: account.email,
        },
        to: addresses(&f.to),
        cc: addresses(&f.cc),
        // Never written into the message (the builder leaves Bcc out); the
        // local draft keeps it.
        bcc: Vec::new(),
        subject: f.subject.clone(),
        body_text: f.body.clone(),
        encoding: TransferEncoding::QuotedPrintable,
        in_reply_to: f.in_reply_to.clone(),
        references: f.references.clone(),
        message_id: Some(draft.message_id_header.clone()),
        is_draft: true,
    })
    // Too large or unbuildable: no retry will help.
    .map_err(|_| ChangeError::Refused { transient: false })
}

/// Uploads the account's changed drafts that are due by `now` to its
/// Drafts folder. Errors only for a local database failure.
pub async fn upload_drafts(
    store: &Store,
    session: &mut Session,
    account: AccountId,
    now: i64,
) -> Result<UploadReport, SyncError> {
    let mut report = UploadReport::default();
    let pending: Vec<Draft> = store
        .list_drafts(account)?
        .into_iter()
        .filter(|d| d.dirty)
        .collect();
    if pending.is_empty() {
        return Ok(report);
    }
    let Some(folder) = drafts_folder(store, account)? else {
        report.no_drafts_folder = true;
        report.remaining = pending.len();
        return Ok(report);
    };
    let raw_name = store
        .get_folder(folder)?
        .map(|f| f.details.remote_name)
        .ok_or(SyncError::UnknownFolder)?;
    for (index, draft) in pending.iter().enumerate() {
        if draft.next_attempt_at > now {
            report.remaining = report.remaining.saturating_add(1);
            continue;
        }
        match upload_one(store, session, draft, folder, &raw_name, now).await {
            Ok(conflicts) => {
                report.uploaded = report.uploaded.saturating_add(1);
                report.conflicts = report.conflicts.saturating_add(conflicts);
            }
            Err(error) => {
                if let ChangeError::Imap(ImapError::Store(e)) = error {
                    return Err(SyncError::Store(e));
                }
                // A draft is never dropped: a permanent refusal waits as long
                // as possible and tries again, in case the folder comes back.
                let attempts = if error.is_transient() {
                    draft.attempts.saturating_add(1)
                } else {
                    u32::MAX
                };
                store.defer_draft(
                    draft.id,
                    attempts,
                    now.saturating_add(backoff_secs(attempts)),
                )?;
                tracing::info!(attempts, "draft upload deferred");
                report.remaining = report
                    .remaining
                    .saturating_add(pending.len().saturating_sub(index));
                break;
            }
        }
    }
    Ok(report)
}

/// Uploads one draft. Returns how many versions changed elsewhere were
/// kept as conflict drafts.
async fn upload_one(
    store: &Store,
    session: &mut Session,
    draft: &Draft,
    folder: FolderId,
    raw_name: &str,
    now: i64,
) -> Result<usize, ChangeError> {
    let store_error = |e| ChangeError::Imap(ImapError::Store(e));
    let message = message_bytes(store, draft)?;
    session.select(raw_name).await?;
    let ours = draft
        .server_uid
        .filter(|_| draft.server_folder_id == Some(folder));
    let others: BTreeSet<u32> = store
        .draft_uids_in(folder, draft.id)
        .map_err(store_error)?
        .into_iter()
        .collect();
    let before: BTreeSet<u32> = session
        .uid_search_message_id(&draft.message_id_header)
        .await?
        .into_iter()
        .filter(|uid| !others.contains(uid))
        .collect();

    // Copies with our id that we didn't upload. A draft that was never on
    // the server can only find leftovers of its own interrupted uploads.
    let foreign: Vec<u32> = if draft.server_uid.is_some() {
        before
            .iter()
            .copied()
            .filter(|uid| Some(*uid) != ours)
            .collect()
    } else {
        Vec::new()
    };
    for uid in &foreign {
        let raw = session.uid_fetch_body(*uid).await?.unwrap_or_default();
        let id = new_message_id(
            draft
                .message_id_header
                .rsplit_once('@')
                .map(|(_, d)| d)
                .unwrap_or_default(),
        )
        .map_err(|_| ChangeError::Refused { transient: true })?;
        store
            .import_draft(
                draft.account_id,
                &fields_of(&raw),
                &id,
                ServerCopy { folder, uid: *uid },
                true,
                now,
            )
            .map_err(store_error)?;
    }

    let appended = session
        .append(raw_name, AppendFlags::Draft, &message)
        .await?;
    let new_uid = match appended {
        Some(uid) => Some(uid),
        // Without UIDPLUS: the new copy is the one that wasn't there before.
        None => session
            .uid_search_message_id(&draft.message_id_header)
            .await?
            .into_iter()
            .filter(|uid| !before.contains(uid) && !others.contains(uid))
            .max(),
    };

    let stale: Vec<u32> = before
        .iter()
        .copied()
        .chain(ours)
        .filter(|uid| Some(*uid) != new_uid && !foreign.contains(uid) && !others.contains(uid))
        .collect::<BTreeSet<u32>>()
        .into_iter()
        .collect();
    if !stale.is_empty() {
        session
            .uid_store_flag(&stale, SystemFlag::Deleted, true)
            .await?;
        if session.supports_uid_expunge() {
            session.uid_expunge(&stale).await?;
        }
    }
    store
        .draft_uploaded(draft.id, draft.version, folder, new_uid)
        .map_err(store_error)?;
    Ok(foreign.len())
}

#[cfg(test)]
mod tests;
