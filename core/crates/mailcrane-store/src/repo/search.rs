//! Running a search (M4-03). The core builds the FTS5 expression (every
//! user term quoted) and the filters; this runs them with bound parameters.

use super::messages::{from_row, one_copy_each};
use crate::{AccountId, FolderId, MessageHeader, MessageId, Store, StoreError};
use rusqlite::{params, types::Value};

/// What to search for. Every field narrows the results; an empty filter
/// matches everything in scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFilter {
    /// The account to search.
    pub account: AccountId,
    /// Only this folder, if set.
    pub folder: Option<FolderId>,
    /// Only folders whose name contains this (case-insensitive), for `in:`.
    pub folder_name: Option<String>,
    /// An FTS5 `MATCH` expression built from quoted terms only.
    pub fts: Option<String>,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachment: bool,
    /// Header date at or after this (Unix seconds).
    pub after: Option<i64>,
    /// Header date before this (Unix seconds).
    pub before: Option<i64>,
    pub limit: u32,
}

impl Store {
    /// The messages matching `filter`, newest first, leaving out hidden ones.
    /// A message stored in several of the searched folders comes once (see
    /// `one_copy_each` in `repo/messages.rs`), and `limit` counts messages,
    /// not copies.
    pub fn search_messages(&self, filter: &SearchFilter) -> Result<Vec<MessageHeader>, StoreError> {
        let mut sql = String::from(
            "SELECT m.*, {RANK} FROM messages m JOIN folders f ON f.id = m.folder_id \
             WHERE f.account_id = ?1 AND m.hidden = 0",
        );
        let mut args: Vec<Value> = vec![Value::Integer(filter.account.0)];
        let mut bind = |sql: &mut String, clause: &str, value: Value| {
            args.push(value);
            sql.push_str(&clause.replace('?', &format!("?{}", args.len())));
        };
        if let Some(folder) = filter.folder {
            bind(&mut sql, " AND m.folder_id = ?", Value::Integer(folder.0));
        }
        if let Some(name) = &filter.folder_name {
            bind(
                &mut sql,
                " AND lower(f.display_name) LIKE '%' || lower(?) || '%'",
                Value::Text(name.clone()),
            );
        }
        if let Some(fts) = &filter.fts {
            bind(
                &mut sql,
                " AND m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)",
                Value::Text(fts.clone()),
            );
        }
        if filter.unread {
            sql.push_str(" AND m.seen = 0");
        }
        if filter.flagged {
            sql.push_str(" AND m.flagged = 1");
        }
        if filter.has_attachment {
            sql.push_str(" AND m.has_attachments = 1");
        }
        if let Some(after) = filter.after {
            bind(&mut sql, " AND m.date >= ?", Value::Integer(after));
        }
        if let Some(before) = filter.before {
            bind(&mut sql, " AND m.date < ?", Value::Integer(before));
        }
        let mut tail = String::new();
        bind(
            &mut tail,
            "ORDER BY m.date DESC, m.id DESC LIMIT ?",
            Value::Integer(i64::from(filter.limit)),
        );
        let sql = one_copy_each(&sql, &tail);
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args))?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }

    /// Indexes a message's body text for search (plain text, or text from
    /// its HTML). Returns whether the message exists.
    pub fn index_body_text(&self, id: MessageId, text: &str) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE messages_fts SET body_text = ?2 WHERE rowid = ?1",
            params![id.0, text],
        )?;
        Ok(n > 0)
    }
}
