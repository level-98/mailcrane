//! Data for the home-screen widget (M4-11): the Unified Inbox's unread count
//! and newest messages, read-only and quick.

use crate::messages::{marker_color, marker_colors};
use crate::sync::{failed, SyncFailure};
use mailcrane_core::events;
use mailcrane_store::Store;

/// Most messages a widget shows.
const MAX_WIDGET_MESSAGES: u32 = 10;

/// One message in the widget.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WidgetMessage {
    pub id: i64,
    pub account_id: i64,
    /// The account's marker colour (0–7).
    pub account_color: u32,
    /// Sender name, or address if there is no name.
    pub from_name: String,
    pub subject: String,
    /// Unix seconds.
    pub date: i64,
    pub is_seen: bool,
}

/// What the widget shows.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WidgetSnapshot {
    /// Unread messages across every account's inbox.
    pub unread_count: u64,
    /// The newest messages of every inbox, newest first.
    pub messages: Vec<WidgetMessage>,
}

/// The widget's data: the unread count and up to `limit` (at most 10)
/// newest messages of the Unified Inbox. Reads only the database.
#[uniffi::export]
pub fn widget_snapshot(db_path: String, limit: u32) -> Result<WidgetSnapshot, SyncFailure> {
    let storage = |_| failed(events::SyncErrorKind::Storage);
    let store = Store::open(&db_path).map_err(storage)?;
    let unread_count = store.count_unified_unread().map_err(storage)?;
    let rows = store
        .list_unified_inbox(limit.min(MAX_WIDGET_MESSAGES), 0)
        .map_err(storage)?;
    let colors = marker_colors(&store).map_err(storage)?;
    let messages = rows
        .into_iter()
        .map(|(h, account)| {
            let d = h.details;
            let from = d
                .from_name
                .filter(|n| !n.trim().is_empty())
                .or(d.from_addr)
                .unwrap_or_default();
            WidgetMessage {
                id: h.id.0,
                account_id: account.0,
                account_color: marker_color(&colors, account.0),
                from_name: crate::messages::display_line(&from),
                subject: crate::messages::display_line(&d.subject),
                date: d.date,
                is_seen: d.flags.seen,
            }
        })
        .collect();
    Ok(WidgetSnapshot {
        unread_count,
        messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{
        AccountId, Flags, FolderId, FolderRole, NewAccount, NewFolder, NewMessageHeader, Security,
    };

    fn account(store: &Store, email: &str) -> AccountId {
        store
            .insert_account(&NewAccount {
                display_name: "A".into(),
                email: email.into(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 465,
                smtp_security: Security::Tls,
                username: email.into(),
            })
            .expect("account")
    }

    fn inbox(store: &Store, account: AccountId) -> FolderId {
        store
            .upsert_folder(&NewFolder {
                account_id: account,
                remote_name: "INBOX".into(),
                display_name: "Inbox".into(),
                delimiter: None,
                role: Some(FolderRole::Inbox),
                uidvalidity: None,
                uidnext: None,
                highestmodseq: None,
                is_subscribed: true,
                selectable: true,
                virtual_role: None,
            })
            .expect("folder")
    }

    fn message(
        store: &Store,
        folder: FolderId,
        uid: u32,
        date: i64,
        seen: bool,
        name: Option<&str>,
    ) {
        store
            .upsert_message_header(&NewMessageHeader {
                folder_id: folder,
                uid,
                message_id_header: None,
                in_reply_to: None,
                references_json: "[]".into(),
                thread_id: None,
                subject: format!("s{uid}"),
                from_name: name.map(Into::into),
                from_addr: Some("x@example.net".into()),
                to_json: "[]".into(),
                cc_json: "[]".into(),
                date,
                internal_date: date,
                size: 1,
                flags: Flags {
                    seen,
                    ..Flags::default()
                },
                keywords_json: "[]".into(),
                has_attachments: false,
                snippet: String::new(),
            })
            .expect("message");
    }

    #[test]
    fn counts_and_newest_messages_across_accounts_quickly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let a = account(&store, "a@example.org");
        let b = account(&store, "b@example.org");
        let ia = inbox(&store, a);
        let ib = inbox(&store, b);
        for i in 0..2_000u32 {
            message(&store, ia, i + 1, i64::from(i), true, Some("Ana"));
        }
        message(&store, ib, 1, 5_000, false, None);
        message(&store, ia, 9_999, 4_000, false, Some("Bob"));
        drop(store);
        let started = std::time::Instant::now();
        let snap = widget_snapshot(path.clone(), 5).expect("snapshot");
        let took = started.elapsed();
        assert_eq!(snap.unread_count, 2);
        assert_eq!(snap.messages.len(), 5);
        assert_eq!(snap.messages[0].account_id, b.0);
        assert_eq!(
            snap.messages[0].from_name, "x@example.net",
            "address when no name"
        );
        assert_eq!(snap.messages[1].from_name, "Bob");
        assert!(!snap.messages[1].is_seen);
        assert_eq!(
            widget_snapshot(path, 100).expect("snapshot").messages.len(),
            10,
            "capped"
        );
        assert!(took.as_millis() < 200, "{took:?}");
    }
}
