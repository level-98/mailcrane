//! Reading stored messages, as the apps call it: `list_messages` returns one
//! page of a folder's message list, newest first.

use mailcrane_store::{FolderId, MessageHeader, Store};

use crate::sync::{SyncErrorKind, SyncFailure};

/// The most rows one `list_messages` call returns, whatever the app asks for.
pub const MAX_PAGE: u32 = 500;

/// The longest subject, sender name or snippet passed to the apps, in chars.
/// Longer values are cut; a list row never shows more.
const MAX_TEXT_CHARS: usize = 998;

/// One row of a message list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageSummary {
    /// Row id of the message.
    pub id: i64,
    /// Sender display name; empty when the message has none.
    pub from_name: String,
    /// Sender address; empty when the message has none.
    pub from_addr: String,
    /// Subject; empty when the message has none.
    pub subject: String,
    /// Short preview of the text; empty until known.
    pub snippet: String,
    /// Header date, unix seconds UTC.
    pub date: i64,
    /// Whether the message has been read (`\Seen`).
    pub is_seen: bool,
    /// Whether the message is flagged (`\Flagged`).
    pub is_flagged: bool,
    /// Whether the message has attachments.
    pub has_attachments: bool,
}

/// Header text as one safe line: CR, LF and NUL become spaces, and the
/// result is at most [`MAX_TEXT_CHARS`] chars.
fn one_line(text: &str) -> String {
    text.chars()
        .take(MAX_TEXT_CHARS)
        .map(|c| {
            if matches!(c, '\r' | '\n' | '\0') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

impl From<MessageHeader> for MessageSummary {
    fn from(m: MessageHeader) -> Self {
        let d = m.details;
        Self {
            id: m.id.0,
            from_name: one_line(d.from_name.as_deref().unwrap_or_default()),
            from_addr: one_line(d.from_addr.as_deref().unwrap_or_default()),
            subject: one_line(&d.subject),
            snippet: one_line(&d.snippet),
            date: d.date,
            is_seen: d.flags.seen,
            is_flagged: d.flags.flagged,
            has_attachments: d.has_attachments,
        }
    }
}

/// Lists up to `limit` messages of the folder, newest first, skipping the
/// first `offset`. Pages with the same `limit` and consecutive offsets have
/// no duplicates or gaps. `limit` is capped at [`MAX_PAGE`]. An unknown
/// folder gives an empty list. Reads only the local database.
///
/// `db_path` is the same database file the app gives `SyncEngine`.
#[uniffi::export]
pub fn list_messages(
    db_path: String,
    folder_id: i64,
    offset: u32,
    limit: u32,
) -> Result<Vec<MessageSummary>, SyncFailure> {
    let storage = || SyncFailure::Failed {
        kind: SyncErrorKind::Storage,
    };
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let rows = store
        .list_messages(FolderId(folder_id), limit.min(MAX_PAGE), offset)
        .map_err(|_| storage())?;
    Ok(rows.into_iter().map(MessageSummary::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{Flags, NewAccount, NewFolder, NewMessageHeader, Security};

    fn header(folder: FolderId, uid: u32, date: i64) -> NewMessageHeader {
        NewMessageHeader {
            folder_id: folder,
            uid,
            message_id_header: None,
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: format!("Message {uid}"),
            from_name: Some("Ana".into()),
            from_addr: Some("ana@example.org".into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date,
            internal_date: date,
            size: 100,
            flags: Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: "Hello".into(),
        }
    }

    /// A database with one account and one folder; returns its path and the
    /// folder. The file is removed when the guard drops.
    fn setup() -> (tempfile::TempDir, String, FolderId) {
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
            })
            .expect("folder");
        (dir, path, folder)
    }

    #[test]
    fn lists_newest_first_in_pages_without_gaps() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let rows: Vec<_> = (1..=7)
            .map(|n| header(folder, n, 1_000 + i64::from(n)))
            .collect();
        store.upsert_message_headers(&rows).expect("insert");
        drop(store);

        let first = list_messages(path.clone(), folder.0, 0, 3).expect("page 1");
        let second = list_messages(path.clone(), folder.0, 3, 3).expect("page 2");
        let third = list_messages(path, folder.0, 6, 3).expect("page 3");
        let subjects: Vec<_> = first
            .iter()
            .chain(&second)
            .chain(&third)
            .map(|m| m.subject.as_str())
            .collect();
        assert_eq!(
            subjects,
            [
                "Message 7",
                "Message 6",
                "Message 5",
                "Message 4",
                "Message 3",
                "Message 2",
                "Message 1"
            ]
        );
    }

    #[test]
    fn maps_flags_attachments_and_missing_sender() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let mut m = header(folder, 1, 1_000);
        m.flags = Flags {
            seen: true,
            flagged: true,
            ..Flags::default()
        };
        m.has_attachments = true;
        m.from_name = None;
        m.from_addr = None;
        store.upsert_message_header(&m).expect("insert");
        drop(store);

        let rows = list_messages(path, folder.0, 0, 10).expect("list");
        let row = rows.first().expect("one row");
        assert!(row.is_seen && row.is_flagged && row.has_attachments);
        assert_eq!(row.from_name, "");
        assert_eq!(row.from_addr, "");
        assert_eq!(row.date, 1_000);
    }

    #[test]
    fn header_text_becomes_one_bounded_line() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let mut m = header(folder, 1, 1_000);
        m.subject = format!("Line one\r\nLine two\0{}", "x".repeat(5_000));
        m.from_name = Some("Ana\nExample".into());
        store.upsert_message_header(&m).expect("insert");
        drop(store);

        let rows = list_messages(path, folder.0, 0, 10).expect("list");
        let row = rows.first().expect("one row");
        assert!(row.subject.starts_with("Line one  Line two "));
        assert!(!row.subject.contains(['\r', '\n', '\0']));
        assert_eq!(row.subject.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(row.from_name, "Ana Example");
    }

    #[test]
    fn limit_is_capped_and_unknown_folder_is_empty() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let rows: Vec<_> = (1..=600).map(|n| header(folder, n, i64::from(n))).collect();
        store.upsert_message_headers(&rows).expect("insert");
        drop(store);

        let page = list_messages(path.clone(), folder.0, 0, u32::MAX).expect("list");
        assert_eq!(page.len(), MAX_PAGE as usize);
        assert_eq!(list_messages(path, 9_999, 0, 10), Ok(Vec::new()));
    }

    #[test]
    fn unopenable_database_is_a_storage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory is not a database file.
        let path = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            list_messages(path, 1, 0, 10),
            Err(SyncFailure::Failed {
                kind: SyncErrorKind::Storage
            })
        );
    }
}
