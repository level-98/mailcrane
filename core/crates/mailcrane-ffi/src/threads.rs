//! Conversations for the apps (M4-02).

use crate::messages::{name_line, one_line, MessageSummary};
use crate::sync::{failed, SyncFailure};
use mailcrane_core::events;
use mailcrane_core::threading;
use mailcrane_store::{FolderId, Store, ThreadId};

/// One row of the conversation list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadSummary {
    pub thread_id: i64,
    /// The subject of the newest message in the folder.
    pub subject: String,
    /// Senders, newest first, without repeats (at most 4).
    pub participants: Vec<String>,
    /// Messages of the conversation in this folder.
    pub message_count: u32,
    pub unread_count: u32,
    /// Newest date, unix seconds.
    pub last_date: i64,
    pub has_attachments: bool,
    pub is_flagged: bool,
    /// The newest message in the folder.
    pub newest_message_id: i64,
}

/// Most rows returned at once.
const MAX_THREADS: u32 = 500;

/// The conversations with messages in `folder_id`, newest first, at most
/// `limit` (capped at 500) after skipping `offset`. Messages not yet put in
/// a conversation are threaded first. Reads only the database.
#[uniffi::export]
pub fn list_threads(
    db_path: String,
    folder_id: i64,
    offset: u32,
    limit: u32,
) -> Result<Vec<ThreadSummary>, SyncFailure> {
    let storage = |_| failed(events::SyncErrorKind::Storage);
    let store = Store::open(&db_path).map_err(storage)?;
    // Mail stored before threading existed, or by a sync that was cut off.
    threading::assign_new(&store, None).map_err(storage)?;
    let rows = store
        .list_threads(FolderId(folder_id), offset, limit.min(MAX_THREADS))
        .map_err(storage)?;
    Ok(rows
        .into_iter()
        .map(|t| ThreadSummary {
            thread_id: t.thread_id.0,
            subject: one_line(&t.subject),
            participants: t.participants.iter().map(|p| name_line(p)).collect(),
            message_count: t.message_count,
            unread_count: t.unread_count,
            last_date: t.last_date,
            has_attachments: t.has_attachments,
            is_flagged: t.is_flagged,
            newest_message_id: t.newest_message_id.0,
        })
        .collect())
}

/// Every message of a conversation across folders (Sent included, Trash and
/// Spam left out), oldest first.
#[uniffi::export]
pub fn list_thread_messages(
    db_path: String,
    thread_id: i64,
) -> Result<Vec<MessageSummary>, SyncFailure> {
    let storage = |_| failed(events::SyncErrorKind::Storage);
    let store = Store::open(&db_path).map_err(storage)?;
    let rows = store
        .thread_messages(ThreadId(thread_id))
        .map_err(storage)?;
    Ok(rows.into_iter().map(MessageSummary::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{Flags, NewAccount, NewFolder, NewMessageHeader, Security};

    fn header(folder: FolderId, uid: u32, id: &str, refs: &str, subject: &str) -> NewMessageHeader {
        NewMessageHeader {
            folder_id: folder,
            uid,
            message_id_header: Some(id.into()),
            in_reply_to: None,
            references_json: refs.into(),
            thread_id: None,
            subject: subject.into(),
            from_name: Some(format!("P{uid}")),
            from_addr: Some(format!("p{uid}@example.net")),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date: 1_000 + i64::from(uid),
            internal_date: 1_000,
            size: 1,
            flags: Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
        }
    }

    #[test]
    fn existing_mail_is_threaded_when_listed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let account = store
            .insert_account(&NewAccount {
                display_name: "A".into(),
                email: "a@example.org".into(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 465,
                smtp_security: Security::Tls,
                username: "a@example.org".into(),
            })
            .expect("account");
        let folder = store
            .upsert_folder(&NewFolder {
                account_id: account,
                remote_name: "INBOX".into(),
                display_name: "Inbox".into(),
                delimiter: None,
                role: None,
                uidvalidity: None,
                uidnext: None,
                highestmodseq: None,
                is_subscribed: true,
                selectable: true,
            })
            .expect("folder");
        store
            .upsert_message_header(&header(folder, 1, "a@x", "[]", "Plan"))
            .expect("m");
        store
            .upsert_message_header(&header(folder, 2, "b@x", "[\"a@x\"]", "Re: Plan"))
            .expect("m");
        store
            .upsert_message_header(&header(folder, 3, "c@x", "[]", "Other"))
            .expect("m");
        drop(store);
        let threads = list_threads(path.clone(), folder.0, 0, 50).expect("threads");
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].subject, "Other");
        assert_eq!(threads[1].message_count, 2);
        assert_eq!(threads[1].participants, ["P2", "P1"]);
        let messages = list_thread_messages(path, threads[1].thread_id).expect("messages");
        assert_eq!(messages.len(), 2);
    }
}
