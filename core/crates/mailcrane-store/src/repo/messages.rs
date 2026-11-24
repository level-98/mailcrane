use super::narrow;
use crate::{
    BodyState, Flags, FolderId, MessageHeader, MessageId, NewAttachment, NewMessageHeader,
    RemoteFlags, Store, StoreError, ThreadId,
};
use rusqlite::{params, OptionalExtension, Row};

const COLS: &str = "id, folder_id, uid, message_id_header, in_reply_to, references_json, \
    thread_id, subject, from_name, from_addr, to_json, cc_json, date, internal_date, size, \
    seen, flagged, answered, draft, deleted, keywords_json, has_attachments, snippet, body_state";

fn flag(r: &Row<'_>, i: usize, column: &'static str) -> Result<bool, StoreError> {
    Ok(narrow::<u8>(r.get(i)?, column)? != 0)
}

fn from_row(r: &Row<'_>) -> Result<MessageHeader, StoreError> {
    let body_state = match r.get::<_, String>(23)?.as_str() {
        "none" => BodyState::None,
        "fetched" => BodyState::Fetched,
        _ => {
            return Err(StoreError::Corrupt {
                column: "body_state",
            })
        }
    };
    let thread: Option<i64> = r.get(6)?;
    Ok(MessageHeader {
        id: MessageId(r.get(0)?),
        details: NewMessageHeader {
            folder_id: FolderId(r.get(1)?),
            uid: narrow(r.get(2)?, "uid")?,
            message_id_header: r.get(3)?,
            in_reply_to: r.get(4)?,
            references_json: r.get(5)?,
            thread_id: thread.map(ThreadId),
            subject: r.get(7)?,
            from_name: r.get(8)?,
            from_addr: r.get(9)?,
            to_json: r.get(10)?,
            cc_json: r.get(11)?,
            date: r.get(12)?,
            internal_date: r.get(13)?,
            size: r.get(14)?,
            flags: Flags {
                seen: flag(r, 15, "seen")?,
                flagged: flag(r, 16, "flagged")?,
                answered: flag(r, 17, "answered")?,
                draft: flag(r, 18, "draft")?,
                deleted: flag(r, 19, "deleted")?,
            },
            keywords_json: r.get(20)?,
            has_attachments: flag(r, 21, "has_attachments")?,
            snippet: r.get(22)?,
        },
        body_state,
    })
}

impl Store {
    /// Inserts the header or, if `(folder, uid)` exists, updates it. The
    /// cached-body state is kept.
    pub fn upsert_message_header(&self, m: &NewMessageHeader) -> Result<MessageId, StoreError> {
        let id: i64 = self.conn.query_row(
            "INSERT INTO messages (folder_id, uid, message_id_header, in_reply_to, \
             references_json, thread_id, subject, from_name, from_addr, to_json, cc_json, date, \
             internal_date, size, seen, flagged, answered, draft, deleted, keywords_json, \
             has_attachments, snippet) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22) \
             ON CONFLICT (folder_id, uid) DO UPDATE SET \
               message_id_header = excluded.message_id_header, in_reply_to = excluded.in_reply_to, \
               references_json = excluded.references_json, thread_id = excluded.thread_id, \
               subject = excluded.subject, from_name = excluded.from_name, \
               from_addr = excluded.from_addr, to_json = excluded.to_json, \
               cc_json = excluded.cc_json, date = excluded.date, \
               internal_date = excluded.internal_date, size = excluded.size, \
               seen = excluded.seen, flagged = excluded.flagged, answered = excluded.answered, \
               draft = excluded.draft, deleted = excluded.deleted, \
               keywords_json = excluded.keywords_json, \
               has_attachments = excluded.has_attachments, snippet = excluded.snippet \
             RETURNING id",
            params![
                m.folder_id.0,
                m.uid,
                m.message_id_header,
                m.in_reply_to,
                m.references_json,
                m.thread_id.map(|t| t.0),
                m.subject,
                m.from_name,
                m.from_addr,
                m.to_json,
                m.cc_json,
                m.date,
                m.internal_date,
                m.size,
                m.flags.seen,
                m.flags.flagged,
                m.flags.answered,
                m.flags.draft,
                m.flags.deleted,
                m.keywords_json,
                m.has_attachments,
                m.snippet
            ],
            |r| r.get(0),
        )?;
        Ok(MessageId(id))
    }

    /// Upserts every header in one transaction: either all are saved or, on
    /// error, none are. Returns the row ids in the same order.
    pub fn upsert_message_headers(
        &self,
        headers: &[NewMessageHeader],
    ) -> Result<Vec<MessageId>, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = Vec::with_capacity(headers.len());
        for h in headers {
            ids.push(self.upsert_message_header(h)?);
        }
        tx.commit()?;
        Ok(ids)
    }

    /// The UIDs stored for the folder, ascending.
    pub fn list_uids(&self, folder: FolderId) -> Result<Vec<u32>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT uid FROM messages WHERE folder_id = ?1 ORDER BY uid")?;
        let mut rows = stmt.query(params![folder.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(narrow(r.get(0)?, "uid")?);
        }
        Ok(out)
    }

    /// The lowest UID stored for the folder, if it has any messages.
    pub fn oldest_uid(&self, folder: FolderId) -> Result<Option<u32>, StoreError> {
        let uid: Option<i64> = self.conn.query_row(
            "SELECT min(uid) FROM messages WHERE folder_id = ?1",
            params![folder.0],
            |r| r.get(0),
        )?;
        uid.map(|u| narrow(u, "uid")).transpose()
    }

    /// The highest UID stored for the folder, if it has any messages.
    pub fn newest_uid(&self, folder: FolderId) -> Result<Option<u32>, StoreError> {
        let uid: Option<i64> = self.conn.query_row(
            "SELECT max(uid) FROM messages WHERE folder_id = ?1",
            params![folder.0],
            |r| r.get(0),
        )?;
        uid.map(|u| narrow(u, "uid")).transpose()
    }

    /// Applies what the server reported for messages already stored, in
    /// one transaction: deletes the `expunged` UIDs (with their bodies and
    /// attachment metadata) and sets the flags in `flags`. UIDs that are not
    /// stored are ignored. Returns how many messages were deleted or had
    /// their flags actually changed. On error nothing is changed.
    pub fn apply_remote_changes(
        &self,
        folder: FolderId,
        expunged: &[u32],
        flags: &[RemoteFlags],
    ) -> Result<usize, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let mut changed = 0usize;
        {
            let mut delete =
                tx.prepare("DELETE FROM messages WHERE folder_id = ?1 AND uid = ?2")?;
            for uid in expunged {
                changed = changed.saturating_add(delete.execute(params![folder.0, uid])?);
            }
            let mut update = tx.prepare(
                "UPDATE messages SET seen = ?3, flagged = ?4, answered = ?5, draft = ?6,                  deleted = ?7, keywords_json = ?8                  WHERE folder_id = ?1 AND uid = ?2 AND (seen IS NOT ?3 OR flagged IS NOT ?4                  OR answered IS NOT ?5 OR draft IS NOT ?6 OR deleted IS NOT ?7                  OR keywords_json IS NOT ?8)",
            )?;
            for f in flags {
                changed = changed.saturating_add(update.execute(params![
                    folder.0,
                    f.uid,
                    f.flags.seen,
                    f.flags.flagged,
                    f.flags.answered,
                    f.flags.draft,
                    f.flags.deleted,
                    f.keywords_json
                ])?);
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Returns the header, or `None` if it doesn't exist.
    pub fn get_message_header(&self, id: MessageId) -> Result<Option<MessageHeader>, StoreError> {
        let sql = format!("SELECT {COLS} FROM messages WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![id.0])?;
        rows.next()?.map(from_row).transpose()
    }

    /// Sets the system flags. Returns whether the message exists.
    pub fn set_flags(&self, id: MessageId, f: Flags) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE messages SET seen=?2, flagged=?3, answered=?4, draft=?5, deleted=?6 \
             WHERE id=?1",
            params![id.0, f.seen, f.flagged, f.answered, f.draft, f.deleted],
        )?;
        Ok(n > 0)
    }

    /// Lists a folder's messages by `date DESC, id DESC`. Pages made with the
    /// same `limit` and consecutive `offset`s have no duplicates or gaps.
    pub fn list_messages(
        &self,
        folder: FolderId,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MessageHeader>, StoreError> {
        let sql = format!(
            "SELECT {COLS} FROM messages WHERE folder_id = ?1 \
             ORDER BY date DESC, id DESC LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![folder.0, limit, offset])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }

    /// Stores the raw message and marks the body as fetched, atomically.
    /// Returns whether the message exists; if not, nothing is stored.
    pub fn store_body(
        &self,
        id: MessageId,
        raw: &[u8],
        fetched_at: i64,
    ) -> Result<bool, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE messages SET body_state = 'fetched' WHERE id = ?1",
            params![id.0],
        )?;
        if n == 0 {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO message_bodies (message_id, raw, fetched_at) VALUES (?1,?2,?3) \
             ON CONFLICT (message_id) DO UPDATE SET raw = excluded.raw, \
             fetched_at = excluded.fetched_at",
            params![id.0, raw, fetched_at],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Like [`Store::store_body`], and in the same transaction sets the
    /// snippet and `has_attachments` and replaces the message's attachment
    /// metadata. Returns whether the message exists; if not, nothing is
    /// stored. On error nothing is stored.
    pub fn store_parsed_body(
        &self,
        id: MessageId,
        raw: &[u8],
        fetched_at: i64,
        snippet: &str,
        attachments: &[NewAttachment],
    ) -> Result<bool, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE messages SET body_state = 'fetched', snippet = ?2, has_attachments = ?3 \
             WHERE id = ?1",
            params![id.0, snippet, !attachments.is_empty()],
        )?;
        if n == 0 {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO message_bodies (message_id, raw, fetched_at) VALUES (?1,?2,?3) \
             ON CONFLICT (message_id) DO UPDATE SET raw = excluded.raw, \
             fetched_at = excluded.fetched_at",
            params![id.0, raw, fetched_at],
        )?;
        tx.execute(
            "DELETE FROM attachments WHERE message_id = ?1",
            params![id.0],
        )?;
        for a in attachments {
            tx.execute(
                "INSERT INTO attachments (message_id, part_id, filename, mime_type, size, \
                 content_id, is_inline) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    id.0,
                    a.part_id,
                    a.filename,
                    a.mime_type,
                    a.size,
                    a.content_id,
                    a.is_inline
                ],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Returns the cached raw message, if any.
    pub fn get_body(&self, id: MessageId) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT raw FROM message_bodies WHERE message_id = ?1",
                params![id.0],
                |r| r.get(0),
            )
            .optional()?)
    }
}
