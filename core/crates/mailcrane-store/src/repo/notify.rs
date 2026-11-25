//! New-mail notifications (M4-05): which inbox messages to notify, each at
//! most once. See `migrations/0013_notifications.sql`.

use crate::{AccountId, FolderId, MessageId, Store, StoreError};
use rusqlite::params;

/// A message to notify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMail {
    pub message_id: MessageId,
    pub account_id: AccountId,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    /// When it arrived on the server, unix seconds UTC.
    pub internal_date: i64,
}

/// How long `notified` keeps a key: 90 days.
const KEEP_SECS: i64 = 90 * 24 * 60 * 60;

/// The key a message is remembered by: its Message-ID, else its folder and UID.
const KEY: &str = "COALESCE(m.message_id_header, 'uid:' || m.folder_id || ':' || m.uid)";

impl Store {
    /// The inbox messages to notify now, oldest first, at most `limit`;
    /// each is recorded so it is never returned again.
    ///
    /// A message qualifies if it is in `inbox`, unread, not deleted, not a
    /// draft, not from `own_address`, not hidden by a snooze, arrived at or
    /// after `since` (unix seconds), and was not notified before.
    ///
    /// The first call for an account only records the qualifying messages
    /// already there and returns nothing, so adding an account doesn't
    /// notify its whole inbox. It does nothing until `inbox` has been synced
    /// once (it has a UIDVALIDITY), so that first look sees the real inbox
    /// rather than an empty one.
    ///
    /// One transaction: on failure nothing is recorded.
    pub fn take_new_mail(
        &self,
        account: AccountId,
        inbox: FolderId,
        own_address: &str,
        since: i64,
        now: i64,
        limit: u32,
    ) -> Result<Vec<NewMail>, StoreError> {
        let tx = self.write_tx()?;
        let synced: Option<Option<i64>> = tx
            .query_row(
                "SELECT uidvalidity FROM folders WHERE id = ?1 AND account_id = ?2",
                params![inbox.0, account.0],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })?;
        if !matches!(synced, Some(Some(_))) {
            return Ok(Vec::new());
        }
        let primed: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM notify_accounts WHERE account_id = ?1)",
            params![account.0],
            |r| r.get(0),
        )?;
        let condition = format!(
            "m.folder_id = ?1 AND m.seen = 0 AND m.deleted = 0 AND m.draft = 0 \
             AND m.internal_date >= ?2 \
             AND lower(COALESCE(m.from_addr, '')) <> lower(?3) \
             AND NOT EXISTS (SELECT 1 FROM snoozes s WHERE s.local_message_id = m.id) \
             AND NOT EXISTS (SELECT 1 FROM notified n WHERE n.account_id = ?4 AND n.key = {KEY})"
        );
        let found = if primed {
            let mut stmt = tx.prepare(&format!(
                "SELECT m.id, m.subject, m.from_name, m.from_addr, m.internal_date, {KEY} \
                 FROM messages m WHERE {condition} \
                 ORDER BY m.internal_date ASC, m.id ASC LIMIT ?5"
            ))?;
            let rows = stmt.query_map(
                params![inbox.0, since, own_address, account.0, i64::from(limit)],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )?;
            let mut found = Vec::new();
            for row in rows {
                let (id, subject, from_name, from_addr, internal_date, key) = row?;
                found.push((
                    NewMail {
                        message_id: MessageId(id),
                        account_id: account,
                        subject,
                        from_name,
                        from_addr,
                        internal_date,
                    },
                    key,
                ));
            }
            found
        } else {
            Vec::new()
        };
        if primed {
            for (_, key) in &found {
                tx.execute(
                    "INSERT OR IGNORE INTO notified (account_id, key, notified_at) VALUES (?1, ?2, ?3)",
                    params![account.0, key, now],
                )?;
            }
        } else {
            // First look: remember everything that would qualify, notify nothing.
            tx.execute(
                &format!(
                    "INSERT OR IGNORE INTO notified (account_id, key, notified_at) \
                     SELECT ?4, {KEY}, ?5 FROM messages m WHERE {condition}"
                ),
                params![inbox.0, since, own_address, account.0, now],
            )?;
            tx.execute(
                "INSERT INTO notify_accounts (account_id, primed_at) VALUES (?1, ?2)",
                params![account.0, now],
            )?;
        }
        tx.execute(
            "DELETE FROM notified WHERE notified_at < ?1",
            params![now.saturating_sub(KEEP_SECS)],
        )?;
        tx.commit()?;
        Ok(found.into_iter().map(|(mail, _)| mail).collect())
    }
}
