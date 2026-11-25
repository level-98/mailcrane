//! Conversation threads (M4-02). See `migrations/0011_threading.sql`. The
//! linking rules live in `mailcrane-core`'s `threading`; these are the
//! building blocks it runs inside [`Store::in_transaction`].

use super::messages::{from_row, COLS};
use super::narrow;
use crate::{AccountId, FolderId, MessageHeader, MessageId, Store, StoreError, ThreadId};
use rusqlite::params;

/// A message still to be put in a thread, with what threading needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnthreadedMessage {
    pub id: MessageId,
    pub account_id: AccountId,
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_json: String,
    pub subject: String,
    pub date: i64,
}

/// One row of the conversation list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub thread_id: ThreadId,
    /// The subject of the newest message in the folder.
    pub subject: String,
    /// Senders in the folder, newest first, without repeats (at most 4).
    pub participants: Vec<String>,
    /// Messages of the thread in this folder.
    pub message_count: u32,
    pub unread_count: u32,
    /// Newest date in this folder, unix seconds.
    pub last_date: i64,
    pub has_attachments: bool,
    pub is_flagged: bool,
    /// The newest message in the folder (opened when the thread has one).
    pub newest_message_id: MessageId,
}

impl Store {
    /// Runs `f` in one transaction: committed if it returns `Ok`, rolled
    /// back otherwise. The threading functions below don't open their own
    /// transactions, so they can run inside it; other store functions must
    /// not be called from `f`.
    pub fn in_transaction<T>(
        &self,
        f: impl FnOnce(&Store) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(value) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Messages without a thread (`None`: of every account), oldest row
    /// first.
    pub fn unthreaded_messages(
        &self,
        account: Option<AccountId>,
    ) -> Result<Vec<UnthreadedMessage>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, f.account_id, m.message_id_header, m.in_reply_to, m.references_json, \
             m.subject, m.date FROM messages m JOIN folders f ON f.id = m.folder_id \
             WHERE m.thread_id IS NULL AND (?1 IS NULL OR f.account_id = ?1) ORDER BY m.id",
        )?;
        let rows = stmt.query_map(params![account.map(|a| a.0)], |r| {
            Ok(UnthreadedMessage {
                id: MessageId(r.get(0)?),
                account_id: AccountId(r.get(1)?),
                message_id_header: r.get(2)?,
                in_reply_to: r.get(3)?,
                references_json: r.get(4)?,
                subject: r.get(5)?,
                date: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The threads any of `keys` already belongs to.
    pub fn threads_for_keys(
        &self,
        account: AccountId,
        keys: &[String],
    ) -> Result<Vec<ThreadId>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT thread_id FROM thread_keys WHERE account_id = ?1 AND key = ?2",
        )?;
        let mut out = Vec::new();
        for key in keys {
            let mut rows = stmt.query(params![account.0, key])?;
            while let Some(row) = rows.next()? {
                let id = ThreadId(row.get(0)?);
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    /// Threads of messages with the subject key `subject` and the given
    /// `kind` dated within `[from, to]`.
    pub fn threads_for_subject(
        &self,
        account: AccountId,
        subject: &str,
        kind: u8,
        from: i64,
        to: i64,
    ) -> Result<Vec<ThreadId>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT m.thread_id FROM messages m JOIN folders f ON f.id = m.folder_id \
             WHERE m.thread_subject = ?2 AND m.subject_kind = ?3 AND m.date BETWEEN ?4 AND ?5 \
             AND f.account_id = ?1 AND m.thread_id IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![account.0, subject, kind, from, to], |r| {
            Ok(ThreadId(r.get(0)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A new, empty thread.
    pub fn create_thread(
        &self,
        account: AccountId,
        subject: &str,
        date: i64,
    ) -> Result<ThreadId, StoreError> {
        self.conn.execute(
            "INSERT INTO threads (account_id, subject, last_date) VALUES (?1, ?2, ?3)",
            params![account.0, subject, date],
        )?;
        Ok(ThreadId(self.conn.last_insert_rowid()))
    }

    /// Moves everything of `losers` into `winner` and deletes them.
    pub fn merge_threads(&self, winner: ThreadId, losers: &[ThreadId]) -> Result<(), StoreError> {
        for loser in losers.iter().filter(|l| **l != winner) {
            self.conn.execute(
                "UPDATE messages SET thread_id = ?1 WHERE thread_id = ?2",
                params![winner.0, loser.0],
            )?;
            self.conn.execute(
                "UPDATE thread_keys SET thread_id = ?1 WHERE thread_id = ?2",
                params![winner.0, loser.0],
            )?;
            self.conn.execute(
                "UPDATE threads SET last_date = max(last_date, \
                 (SELECT last_date FROM threads WHERE id = ?2)) WHERE id = ?1",
                params![winner.0, loser.0],
            )?;
            self.conn
                .execute("DELETE FROM threads WHERE id = ?1", params![loser.0])?;
        }
        Ok(())
    }

    /// Records that `keys` belong to `thread`.
    pub fn bind_thread_keys(
        &self,
        account: AccountId,
        keys: &[String],
        thread: ThreadId,
    ) -> Result<(), StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO thread_keys (account_id, key, thread_id) VALUES (?1, ?2, ?3) \
             ON CONFLICT (account_id, key) DO UPDATE SET thread_id = excluded.thread_id",
        )?;
        for key in keys {
            stmt.execute(params![account.0, key, thread.0])?;
        }
        Ok(())
    }

    /// Puts a message in `thread`, with its subject key and kind.
    pub fn set_message_thread(
        &self,
        message: MessageId,
        thread: ThreadId,
        subject_key: &str,
        kind: u8,
        date: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE messages SET thread_id = ?2, thread_subject = ?3, subject_kind = ?4 WHERE id = ?1",
            params![message.0, thread.0, subject_key, kind],
        )?;
        self.conn.execute(
            "UPDATE threads SET last_date = max(last_date, ?2) WHERE id = ?1",
            params![thread.0, date],
        )?;
        Ok(())
    }

    /// Forgets every thread of `account`, for a full rebuild.
    pub fn clear_threads(&self, account: AccountId) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE messages SET thread_id = NULL, thread_subject = NULL, subject_kind = 0 \
             WHERE folder_id IN (SELECT id FROM folders WHERE account_id = ?1)",
            params![account.0],
        )?;
        self.conn.execute(
            "DELETE FROM thread_keys WHERE account_id = ?1",
            params![account.0],
        )?;
        self.conn.execute(
            "DELETE FROM threads WHERE account_id = ?1",
            params![account.0],
        )?;
        Ok(())
    }

    /// The conversations with messages in `folder`, newest first. Hidden
    /// messages (queued moves or deletes) are left out.
    pub fn list_threads(
        &self,
        folder: FolderId,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ThreadSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT thread_id, count(*), sum(seen = 0), max(date), max(has_attachments), \
             max(flagged) FROM messages WHERE folder_id = ?1 AND hidden = 0 \
             AND thread_id IS NOT NULL GROUP BY thread_id \
             ORDER BY max(date) DESC, thread_id DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(params![folder.0, limit, offset], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut people = self.conn.prepare_cached(
            "SELECT id, subject, from_name, from_addr FROM messages \
             WHERE folder_id = ?1 AND thread_id = ?2 AND hidden = 0 ORDER BY date DESC, id DESC",
        )?;
        let mut out = Vec::with_capacity(rows.len());
        for (thread, count, unread, last, attach, flagged) in rows {
            let mut participants: Vec<String> = Vec::new();
            let mut subject = String::new();
            let mut newest = None;
            let mut q = people.query(params![folder.0, thread])?;
            while let Some(r) = q.next()? {
                if newest.is_none() {
                    newest = Some(MessageId(r.get(0)?));
                    subject = r.get(1)?;
                }
                let name: Option<String> = r.get(2)?;
                let addr: Option<String> = r.get(3)?;
                let who = name
                    .filter(|n| !n.trim().is_empty())
                    .or(addr)
                    .unwrap_or_default();
                if !who.is_empty() && !participants.contains(&who) && participants.len() < 4 {
                    participants.push(who);
                }
            }
            let Some(newest) = newest else { continue };
            out.push(ThreadSummary {
                thread_id: ThreadId(thread),
                subject,
                participants,
                message_count: narrow(count, "count")?,
                unread_count: narrow(unread, "unread")?,
                last_date: last,
                has_attachments: attach != 0,
                is_flagged: flagged != 0,
                newest_message_id: newest,
            });
        }
        Ok(out)
    }

    /// Every message of a thread across its account's folders, oldest
    /// first, leaving out hidden ones and those in Trash or Spam.
    pub fn thread_messages(&self, thread: ThreadId) -> Result<Vec<MessageHeader>, StoreError> {
        let cols = COLS
            .split(", ")
            .map(|c| format!("m.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {cols} FROM messages m JOIN folders f ON f.id = m.folder_id \
             WHERE m.thread_id = ?1 AND m.hidden = 0 \
             AND COALESCE(f.role, '') NOT IN ('trash', 'spam') ORDER BY m.date, m.id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![thread.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }
}
