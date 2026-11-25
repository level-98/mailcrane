//! Drafts (M2-05): saved locally at once, uploaded to the Drafts folder
//! later.

use super::narrow;
use crate::{AccountId, FolderId, Store, StoreError};
use rusqlite::{params, OptionalExtension, Row};

/// Row id of a draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DraftId(pub i64);

/// What the user typed, as the compose screen holds it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DraftFields {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// A stored draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub id: DraftId,
    pub account_id: AccountId,
    pub fields: DraftFields,
    /// Without angle brackets; the same for every version.
    pub message_id_header: String,
    /// Where the newest uploaded copy is.
    pub server_folder_id: Option<FolderId>,
    pub server_uid: Option<u32>,
    /// Changed since the last upload.
    pub dirty: bool,
    pub attempts: u32,
    pub next_attempt_at: i64,
    /// Bumped on every save.
    pub version: i64,
    pub updated_at: i64,
    /// A version changed on another device.
    pub is_conflict: bool,
}

/// Recipient entries one per line; CR and LF inside an entry become spaces.
fn join_lines(entries: &[String]) -> String {
    entries
        .iter()
        .map(|e| e.replace(['\r', '\n'], " "))
        .filter(|e| !e.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

const COLS: &str = "id, account_id, to_list, cc_list, bcc_list, subject, body, in_reply_to, \
    references_list, message_id_header, server_folder_id, server_uid, dirty, attempts, \
    next_attempt_at, version, updated_at, is_conflict";

fn from_row(r: &Row<'_>) -> Result<Draft, StoreError> {
    let folder: Option<i64> = r.get(10)?;
    let uid: Option<i64> = r.get(11)?;
    let references: String = r.get(8)?;
    Ok(Draft {
        id: DraftId(r.get(0)?),
        account_id: AccountId(r.get(1)?),
        fields: DraftFields {
            to: split_lines(&r.get::<_, String>(2)?),
            cc: split_lines(&r.get::<_, String>(3)?),
            bcc: split_lines(&r.get::<_, String>(4)?),
            subject: r.get(5)?,
            body: r.get(6)?,
            in_reply_to: r.get(7)?,
            references: references.split_whitespace().map(str::to_owned).collect(),
        },
        message_id_header: r.get(9)?,
        server_folder_id: folder.map(FolderId),
        server_uid: uid.map(|u| narrow(u, "server_uid")).transpose()?,
        dirty: r.get::<_, i64>(12)? != 0,
        attempts: narrow(r.get(13)?, "attempts")?,
        next_attempt_at: r.get(14)?,
        version: r.get(15)?,
        updated_at: r.get(16)?,
        is_conflict: r.get::<_, i64>(17)? != 0,
    })
}

/// Where an uploaded copy of a draft is on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerCopy {
    pub folder: FolderId,
    pub uid: u32,
}

impl Store {
    /// The local message with `uid` in `folder`, if synced.
    pub fn message_at(
        &self,
        folder: FolderId,
        uid: u32,
    ) -> Result<Option<crate::MessageId>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
                params![folder.0, uid],
                |r| r.get(0),
            )
            .optional()?
            .map(crate::MessageId))
    }

    /// Creates a draft for `account` with `message_id_header` (new and
    /// unique). It is dirty, so the next upload sends it.
    pub fn create_draft(
        &self,
        account: AccountId,
        fields: &DraftFields,
        message_id_header: &str,
        now: i64,
    ) -> Result<DraftId, StoreError> {
        self.conn.execute(
            "INSERT INTO drafts (account_id, to_list, cc_list, bcc_list, subject, body, \
             in_reply_to, references_list, message_id_header, next_attempt_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                account.0,
                join_lines(&fields.to),
                join_lines(&fields.cc),
                join_lines(&fields.bcc),
                fields.subject,
                fields.body,
                fields.in_reply_to,
                fields.references.join(" "),
                message_id_header,
                now
            ],
        )?;
        Ok(DraftId(self.conn.last_insert_rowid()))
    }

    /// Saves new content for a draft and marks it for upload (now, without
    /// back-off: the user is active). Returns whether it exists.
    pub fn update_draft(
        &self,
        id: DraftId,
        fields: &DraftFields,
        now: i64,
    ) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE drafts SET to_list = ?2, cc_list = ?3, bcc_list = ?4, subject = ?5, \
             body = ?6, in_reply_to = ?7, references_list = ?8, dirty = 1, attempts = 0, \
             next_attempt_at = ?9, version = version + 1, updated_at = ?9 WHERE id = ?1",
            params![
                id.0,
                join_lines(&fields.to),
                join_lines(&fields.cc),
                join_lines(&fields.bcc),
                fields.subject,
                fields.body,
                fields.in_reply_to,
                fields.references.join(" "),
                now
            ],
        )?;
        Ok(n > 0)
    }

    /// A draft that came from the server (opened from the Drafts folder, or
    /// a version changed on another device): stored clean, owning `copy`.
    pub fn import_draft(
        &self,
        account: AccountId,
        fields: &DraftFields,
        message_id_header: &str,
        copy: ServerCopy,
        is_conflict: bool,
        now: i64,
    ) -> Result<DraftId, StoreError> {
        let id = self.create_draft(account, fields, message_id_header, now)?;
        self.conn.execute(
            "UPDATE drafts SET server_folder_id = ?2, server_uid = ?3, dirty = 0, \
             is_conflict = ?4 WHERE id = ?1",
            params![id.0, copy.folder.0, copy.uid, is_conflict],
        )?;
        Ok(id)
    }

    pub fn get_draft(&self, id: DraftId) -> Result<Option<Draft>, StoreError> {
        let sql = format!("SELECT {COLS} FROM drafts WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![id.0])?;
        rows.next()?.map(from_row).transpose()
    }

    /// The draft with this Message-ID (without angle brackets), if any.
    pub fn find_draft(&self, message_id_header: &str) -> Result<Option<Draft>, StoreError> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM drafts WHERE message_id_header = ?1",
                params![message_id_header],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.get_draft(DraftId(id)),
            None => Ok(None),
        }
    }

    /// The account's drafts, most recently changed first.
    pub fn list_drafts(&self, account: AccountId) -> Result<Vec<Draft>, StoreError> {
        let sql = format!(
            "SELECT {COLS} FROM drafts WHERE account_id = ?1 ORDER BY updated_at DESC, id DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![account.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }

    /// UIDs in `folder` that belong to drafts other than `except`.
    pub fn draft_uids_in(&self, folder: FolderId, except: DraftId) -> Result<Vec<u32>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT server_uid FROM drafts WHERE server_folder_id = ?1 AND id != ?2 \
             AND server_uid IS NOT NULL",
        )?;
        let mut rows = stmt.query(params![folder.0, except.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(narrow(r.get(0)?, "server_uid")?);
        }
        Ok(out)
    }

    /// Records an upload of `version`: the draft's copy is now `copy` (or
    /// unknown, without UIDPLUS and a failed search). Clears `dirty` only if
    /// nothing was saved since that version was sent.
    pub fn draft_uploaded(
        &self,
        id: DraftId,
        version: i64,
        folder: FolderId,
        uid: Option<u32>,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE drafts SET server_folder_id = ?2, server_uid = ?3, attempts = 0, \
             dirty = CASE WHEN version = ?4 THEN 0 ELSE dirty END WHERE id = ?1",
            params![id.0, folder.0, uid, version],
        )?;
        Ok(())
    }

    /// The upload failed for now: try again at `next_attempt_at`.
    pub fn defer_draft(
        &self,
        id: DraftId,
        attempts: u32,
        next_attempt_at: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE drafts SET attempts = ?2, next_attempt_at = ?3 WHERE id = ?1",
            params![id.0, attempts, next_attempt_at],
        )?;
        Ok(())
    }

    /// Deletes the draft. Returns where its server copy was, so the caller
    /// can delete that too.
    pub fn delete_draft(&self, id: DraftId) -> Result<Option<ServerCopy>, StoreError> {
        let copy = self.get_draft(id)?.and_then(|d| {
            Some(ServerCopy {
                folder: d.server_folder_id?,
                uid: d.server_uid?,
            })
        });
        self.conn
            .execute("DELETE FROM drafts WHERE id = ?1", params![id.0])?;
        Ok(copy)
    }
}
