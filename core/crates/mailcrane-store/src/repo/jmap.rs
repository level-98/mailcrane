//! JMAP accounts (M3-04): how they are stored next to IMAP ones. See
//! `migrations/0014_jmap.sql`.

use crate::{
    AccountId, Flags, FolderId, MessageId, NewAccount, NewMessageHeader, Store, StoreError,
};
use rusqlite::{params, OptionalExtension};

/// The protocol an account speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protocol {
    /// IMAP for mail, SMTP for sending (every account before JMAP).
    Imap,
    /// JMAP, with its session URL (RFC 8620 §2).
    Jmap { session_url: String },
}

/// A message row that is a JMAP email in one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmapRow {
    pub message_id: MessageId,
    pub folder_id: FolderId,
    pub email_id: String,
    pub blob_id: String,
}

impl Store {
    /// Inserts a JMAP account. `a`'s IMAP and SMTP fields aren't used; pass
    /// empty values. Returns the new id.
    pub fn insert_jmap_account(
        &self,
        a: &NewAccount,
        session_url: &str,
    ) -> Result<AccountId, StoreError> {
        self.conn.execute(
            "INSERT INTO accounts (display_name, email, imap_host, imap_port, imap_security, \
             smtp_host, smtp_port, smtp_security, username, protocol, jmap_session_url) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'jmap',?10)",
            params![
                a.display_name,
                a.email,
                a.imap_host,
                a.imap_port,
                a.imap_security.as_str(),
                a.smtp_host,
                a.smtp_port,
                a.smtp_security.as_str(),
                a.username,
                session_url
            ],
        )?;
        Ok(AccountId(self.conn.last_insert_rowid()))
    }

    /// The account's protocol, or `None` if there is no such account.
    pub fn account_protocol(&self, account: AccountId) -> Result<Option<Protocol>, StoreError> {
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT protocol, jmap_session_url FROM accounts WHERE id = ?1",
                params![account.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((p, _)) if p == "imap" => Ok(Some(Protocol::Imap)),
            Some((p, Some(url))) if p == "jmap" => Ok(Some(Protocol::Jmap { session_url: url })),
            Some(_) => Err(StoreError::Corrupt { column: "protocol" }),
        }
    }

    /// Records that `folder` is the JMAP mailbox `mailbox_id`.
    pub fn set_jmap_mailbox(&self, folder: FolderId, mailbox_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO jmap_mailboxes (folder_id, mailbox_id) VALUES (?1, ?2) \
             ON CONFLICT (folder_id) DO UPDATE SET mailbox_id = excluded.mailbox_id",
            params![folder.0, mailbox_id],
        )?;
        Ok(())
    }

    /// The JMAP mailbox of `folder`, if it is one.
    pub fn jmap_mailbox(&self, folder: FolderId) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT mailbox_id FROM jmap_mailboxes WHERE folder_id = ?1",
                params![folder.0],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Every folder of the account that is a JMAP mailbox: (folder, mailbox id).
    pub fn jmap_mailboxes(
        &self,
        account: AccountId,
    ) -> Result<Vec<(FolderId, String)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT j.folder_id, j.mailbox_id FROM jmap_mailboxes j \
             JOIN folders f ON f.id = j.folder_id WHERE f.account_id = ?1 ORDER BY j.folder_id",
        )?;
        let rows = stmt.query_map(params![account.0], |r| Ok((FolderId(r.get(0)?), r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The stored state string for a data type ("Email", "Mailbox").
    pub fn jmap_state(&self, account: AccountId, kind: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT state FROM jmap_state WHERE account_id = ?1 AND type = ?2",
                params![account.0, kind],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_jmap_state(
        &self,
        account: AccountId,
        kind: &str,
        state: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO jmap_state (account_id, type, state) VALUES (?1, ?2, ?3) \
             ON CONFLICT (account_id, type) DO UPDATE SET state = excluded.state",
            params![account.0, kind, state],
        )?;
        Ok(())
    }

    pub fn clear_jmap_state(&self, account: AccountId, kind: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM jmap_state WHERE account_id = ?1 AND type = ?2",
            params![account.0, kind],
        )?;
        Ok(())
    }

    /// Stores a JMAP email in `header.folder_id`: inserts a row with the
    /// folder's next per-folder number if it isn't there yet, or else
    /// updates only its flags and keywords (so threading and the cached body
    /// are kept). `header.uid` is ignored. One transaction.
    pub fn upsert_jmap_email(
        &self,
        header: &NewMessageHeader,
        email_id: &str,
        blob_id: &str,
    ) -> Result<MessageId, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT m.id FROM messages m JOIN jmap_emails j ON j.message_id = m.id \
                 WHERE m.folder_id = ?1 AND j.email_id = ?2",
                params![header.folder_id.0, email_id],
                |r| r.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing {
            let f: &Flags = &header.flags;
            tx.execute(
                "UPDATE messages SET seen = ?2, flagged = ?3, answered = ?4, draft = ?5, \
                 deleted = ?6, keywords_json = ?7 WHERE id = ?1",
                params![
                    id,
                    f.seen,
                    f.flagged,
                    f.answered,
                    f.draft,
                    f.deleted,
                    header.keywords_json
                ],
            )?;
            MessageId(id)
        } else {
            let uid: i64 = tx.query_row(
                "SELECT COALESCE(uidnext, 1) FROM folders WHERE id = ?1",
                params![header.folder_id.0],
                |r| r.get(0),
            )?;
            let uid32 =
                u32::try_from(uid).map_err(|_| StoreError::Corrupt { column: "uidnext" })?;
            tx.execute(
                "UPDATE folders SET uidnext = ?2, uidvalidity = COALESCE(uidvalidity, 1) WHERE id = ?1",
                params![header.folder_id.0, uid.saturating_add(1)],
            )?;
            let row = NewMessageHeader {
                uid: uid32,
                ..header.clone()
            };
            // Same statement as an IMAP insert, through the store's own upsert.
            let id = Self::upsert_header_in(&tx, &row)?;
            tx.execute(
                "INSERT INTO jmap_emails (message_id, email_id, blob_id) VALUES (?1, ?2, ?3)",
                params![id.0, email_id, blob_id],
            )?;
            id
        };
        tx.commit()?;
        Ok(id)
    }

    /// Every row of the JMAP email in the account's folders.
    pub fn jmap_rows(
        &self,
        account: AccountId,
        email_id: &str,
    ) -> Result<Vec<JmapRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.folder_id, j.email_id, j.blob_id FROM jmap_emails j \
             JOIN messages m ON m.id = j.message_id JOIN folders f ON f.id = m.folder_id \
             WHERE f.account_id = ?1 AND j.email_id = ?2",
        )?;
        let rows = stmt.query_map(params![account.0, email_id], |r| {
            Ok(JmapRow {
                message_id: MessageId(r.get(0)?),
                folder_id: FolderId(r.get(1)?),
                email_id: r.get(2)?,
                blob_id: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The JMAP email a message row is, if any.
    pub fn jmap_row(&self, message: MessageId) -> Result<Option<JmapRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT m.id, m.folder_id, j.email_id, j.blob_id FROM jmap_emails j \
                 JOIN messages m ON m.id = j.message_id WHERE m.id = ?1",
                params![message.0],
                |r| {
                    Ok(JmapRow {
                        message_id: MessageId(r.get(0)?),
                        folder_id: FolderId(r.get(1)?),
                        email_id: r.get(2)?,
                        blob_id: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Deletes message rows (with their bodies, by cascade). Returns how many.
    pub fn delete_message_rows(&self, ids: &[MessageId]) -> Result<usize, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let mut n = 0usize;
        for id in ids {
            n = n.saturating_add(tx.execute("DELETE FROM messages WHERE id = ?1", params![id.0])?);
        }
        tx.commit()?;
        Ok(n)
    }

    /// How many messages the folder holds locally.
    pub fn count_folder_messages(&self, folder: FolderId) -> Result<u64, StoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE folder_id = ?1",
            params![folder.0],
            |r| r.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Forgets the folder's local copy: deletes its messages and clears its
    /// sync state, so the next sync starts over. For a JMAP server that
    /// can't say what changed (`cannotCalculateChanges`, RFC 8620 §5.2).
    pub fn forget_folder(&self, folder: FolderId) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM messages WHERE folder_id = ?1",
            params![folder.0],
        )?;
        tx.execute(
            "UPDATE folders SET uidvalidity = NULL, uidnext = NULL, highestmodseq = NULL WHERE id = ?1",
            params![folder.0],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Marks a folder as synced once, for an empty JMAP mailbox (a folder
    /// with messages is marked by its first row).
    pub fn mark_folder_synced(&self, folder: FolderId) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE folders SET uidvalidity = COALESCE(uidvalidity, 1), uidnext = COALESCE(uidnext, 1)              WHERE id = ?1",
            params![folder.0],
        )?;
        Ok(())
    }
}
