//! Snoozed messages (M4-06). See `migrations/0009_snooze.sql` for the states.

use super::narrow;
use crate::{AccountId, FolderId, MessageId, Store, StoreError};
use rusqlite::{params, OptionalExtension, Row};

/// Row id of a snooze.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnoozeId(pub i64);

/// Where a snooze is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozeState {
    /// Snoozed on this device; the server move is still to do.
    ToSnooze,
    /// In the Snoozed folder on the server.
    Snoozed,
    /// Due or unsnoozed; the move back is still to do.
    ToWake,
}

impl SnoozeState {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToSnooze => "to_snooze",
            Self::Snoozed => "snoozed",
            Self::ToWake => "to_wake",
        }
    }

    fn parse(text: &str) -> Result<Self, StoreError> {
        Ok(match text {
            "to_snooze" => Self::ToSnooze,
            "snoozed" => Self::Snoozed,
            "to_wake" => Self::ToWake,
            _ => return Err(StoreError::Corrupt { column: "state" }),
        })
    }
}

/// A new snooze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSnooze {
    pub account_id: AccountId,
    /// The message's `Message-ID` header.
    pub message_id_header: String,
    /// Where it goes back to.
    pub original_folder_id: Option<FolderId>,
    /// The local row to hide until the server move is done, if any.
    pub hide_message: Option<MessageId>,
    /// Wake time, unix seconds UTC.
    pub wake_at: i64,
}

/// A stored snooze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snooze {
    pub id: SnoozeId,
    pub account_id: AccountId,
    pub message_id_header: String,
    pub original_folder_id: Option<FolderId>,
    /// The hidden local row (state `ToSnooze` only).
    pub local_message_id: Option<MessageId>,
    pub wake_at: i64,
    pub state: SnoozeState,
    /// The wake time is in a server keyword, so other installs know it.
    pub on_server: bool,
    /// Mark it unread when it wakes (false when the user unsnoozed it).
    pub mark_unread: bool,
    pub attempts: u32,
    pub next_attempt_at: i64,
    /// How far a move without MOVE got (`migrations/0023_snooze_step.sql`):
    /// 0 when nothing has been done on the server yet.
    pub step: u8,
    /// The target folder's UIDNEXT recorded just before the COPY.
    pub target_uidnext: Option<u32>,
    /// When a rollback (step 4) began, unix seconds.
    pub rollback_since: Option<i64>,
}

/// A stored message with keywords that start with a given prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordedMessage {
    pub id: MessageId,
    pub message_id_header: Option<String>,
    /// The matching keywords.
    pub keywords: Vec<String>,
}

const COLS: &str = "id, account_id, message_id_header, original_folder_id, local_message_id, \
    wake_at, state, on_server, mark_unread, attempts, next_attempt_at, step, target_uidnext, \
    rollback_since";

fn from_row(r: &Row<'_>) -> Result<Snooze, StoreError> {
    Ok(Snooze {
        id: SnoozeId(r.get(0)?),
        account_id: AccountId(r.get(1)?),
        message_id_header: r.get(2)?,
        original_folder_id: r.get::<_, Option<i64>>(3)?.map(FolderId),
        local_message_id: r.get::<_, Option<i64>>(4)?.map(MessageId),
        wake_at: r.get(5)?,
        state: SnoozeState::parse(&r.get::<_, String>(6)?)?,
        on_server: narrow::<u8>(r.get(7)?, "on_server")? != 0,
        mark_unread: narrow::<u8>(r.get(8)?, "mark_unread")? != 0,
        attempts: narrow(r.get(9)?, "attempts")?,
        next_attempt_at: r.get(10)?,
        step: narrow(r.get(11)?, "step")?,
        target_uidnext: r
            .get::<_, Option<i64>>(12)?
            .map(|v| narrow(v, "target_uidnext"))
            .transpose()?,
        rollback_since: r.get(13)?,
    })
}

impl Store {
    /// Snoozes a message: hides `hide_message` and records the snooze, in
    /// one transaction. Snoozing a message that is already snoozed changes
    /// its wake time (the server keyword is updated on the next run).
    pub fn snooze_message(&self, s: &NewSnooze) -> Result<SnoozeId, StoreError> {
        let tx = self.write_tx()?;
        if let Some(message) = s.hide_message {
            tx.execute(
                "UPDATE messages SET hidden = 1 WHERE id = ?1",
                params![message.0],
            )?;
        }
        tx.execute(
            "INSERT INTO snoozes (account_id, message_id_header, original_folder_id, \
             local_message_id, wake_at, state) VALUES (?1, ?2, ?3, ?4, ?5, 'to_snooze') \
             ON CONFLICT (account_id, message_id_header) DO UPDATE SET \
             wake_at = excluded.wake_at, state = 'to_snooze', mark_unread = 1, \
             local_message_id = COALESCE(excluded.local_message_id, local_message_id), \
             original_folder_id = COALESCE(original_folder_id, excluded.original_folder_id), \
             attempts = 0, next_attempt_at = 0, \
             step = CASE WHEN state = 'to_snooze' THEN step ELSE 0 END, \
             target_uidnext = CASE WHEN state = 'to_snooze' THEN target_uidnext ELSE NULL END, \
             rollback_since = CASE WHEN state = 'to_snooze' THEN rollback_since ELSE NULL END",
            params![
                s.account_id.0,
                s.message_id_header,
                s.original_folder_id.map(|f| f.0),
                s.hide_message.map(|m| m.0),
                s.wake_at
            ],
        )?;
        let id = tx.query_row(
            "SELECT id FROM snoozes WHERE account_id = ?1 AND message_id_header = ?2",
            params![s.account_id.0, s.message_id_header],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(SnoozeId(id))
    }

    /// Records a snooze another install made (learnt from the server
    /// keyword). Does nothing if this message already has one.
    pub fn learn_snooze(
        &self,
        account: AccountId,
        message_id_header: &str,
        wake_at: i64,
    ) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO snoozes (account_id, message_id_header, wake_at, state, \
             on_server) VALUES (?1, ?2, ?3, 'snoozed', 1)",
            params![account.0, message_id_header, wake_at],
        )?;
        Ok(n > 0)
    }

    /// The snooze of a message, by `Message-ID`.
    pub fn get_snooze(
        &self,
        account: AccountId,
        message_id_header: &str,
    ) -> Result<Option<Snooze>, StoreError> {
        let sql =
            format!("SELECT {COLS} FROM snoozes WHERE account_id = ?1 AND message_id_header = ?2");
        self.conn
            .query_row(&sql, params![account.0, message_id_header], |r| {
                Ok(from_row(r))
            })
            .optional()?
            .transpose()
    }

    /// All snoozes of an account (`None`: of every account), soonest first.
    pub fn list_snoozes(&self, account: Option<AccountId>) -> Result<Vec<Snooze>, StoreError> {
        let sql = format!(
            "SELECT {COLS} FROM snoozes WHERE ?1 IS NULL OR account_id = ?1 \
             ORDER BY wake_at, id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account.map(|a| a.0)], |r| Ok(from_row(r)))?;
        rows.map(|r| r?).collect()
    }

    /// Turns every snooze due at `now` from `Snoozed` into `ToWake` and
    /// returns them. Each is returned once.
    pub fn take_due_snoozes(&self, now: i64) -> Result<Vec<Snooze>, StoreError> {
        let tx = self.write_tx()?;
        let due = {
            let sql = format!(
                "SELECT {COLS} FROM snoozes WHERE state = 'snoozed' AND wake_at <= ?1 \
                 ORDER BY wake_at, id"
            );
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt.query_map(params![now], |r| Ok(from_row(r)))?;
            rows.map(|r| r?).collect::<Result<Vec<_>, _>>()?
        };
        tx.execute(
            "UPDATE snoozes SET state = 'to_wake', attempts = 0, next_attempt_at = 0, \
             step = 0, target_uidnext = NULL, rollback_since = NULL \
             WHERE state = 'snoozed' AND wake_at <= ?1",
            params![now],
        )?;
        tx.commit()?;
        Ok(due)
    }

    /// Unsnoozes: the message goes back now, without being marked unread.
    /// Returns false if it wasn't snoozed.
    pub fn request_unsnooze(
        &self,
        account: AccountId,
        message_id_header: &str,
        now: i64,
    ) -> Result<bool, StoreError> {
        let tx = self.write_tx()?;
        let existing: Option<(String, Option<i64>, i64)> = tx
            .query_row(
                "SELECT state, local_message_id, step FROM snoozes \
                 WHERE account_id = ?1 AND message_id_header = ?2",
                params![account.0, message_id_header],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((state, local, step)) = existing else {
            return Ok(false);
        };
        if state == "to_snooze" && step > 0 {
            // The server move has started (#423): dropping the row now would
            // leave a copy in the Snoozed folder. Let it finish (or roll
            // back), then wake it at once, without marking it unread, and
            // without waiting out the back-off.
            tx.execute(
                "UPDATE snoozes SET wake_at = ?3, mark_unread = 0, attempts = 0, \
                 next_attempt_at = 0 WHERE account_id = ?1 AND message_id_header = ?2",
                params![account.0, message_id_header, now],
            )?;
            // A rollback (step 4) ends with the message where it was: show
            // it now rather than when the server finally allows it.
            if step == 4 {
                if let Some(m) = local {
                    tx.execute("UPDATE messages SET hidden = 0 WHERE id = ?1", params![m])?;
                }
            }
        } else if state == "to_snooze" {
            // Not on the server yet: just show it again.
            if let Some(m) = local {
                tx.execute("UPDATE messages SET hidden = 0 WHERE id = ?1", params![m])?;
            }
            tx.execute(
                "DELETE FROM snoozes WHERE account_id = ?1 AND message_id_header = ?2",
                params![account.0, message_id_header],
            )?;
        } else {
            // A wake already under way keeps its step, so it resumes rather
            // than copying again (#423).
            tx.execute(
                "UPDATE snoozes SET mark_unread = 0, wake_at = ?3, attempts = 0, \
                 next_attempt_at = 0, \
                 step = CASE WHEN state = 'to_wake' THEN step ELSE 0 END, \
                 target_uidnext = CASE WHEN state = 'to_wake' THEN target_uidnext ELSE NULL END, \
                 rollback_since = CASE WHEN state = 'to_wake' THEN rollback_since ELSE NULL END, \
                 state = 'to_wake' \
                 WHERE account_id = ?1 AND message_id_header = ?2",
                params![account.0, message_id_header, now],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// The server move to the Snoozed folder is done: the hidden local row
    /// is deleted (the next sync of the Snoozed folder brings it back).
    pub fn finish_snooze(&self, id: SnoozeId, on_server: bool) -> Result<(), StoreError> {
        let tx = self.write_tx()?;
        tx.execute(
            "DELETE FROM messages WHERE hidden = 1 AND id = \
             (SELECT local_message_id FROM snoozes WHERE id = ?1)",
            params![id.0],
        )?;
        tx.execute(
            "UPDATE snoozes SET state = 'snoozed', on_server = ?2, local_message_id = NULL, \
             attempts = 0, next_attempt_at = 0, step = 0, target_uidnext = NULL, \
             rollback_since = NULL WHERE id = ?1 AND state = 'to_snooze'",
            params![id.0, on_server],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Drops a snooze. With `unhide`, its hidden local row is shown again
    /// (the server refused the move for good).
    pub fn delete_snooze(&self, id: SnoozeId, unhide: bool) -> Result<(), StoreError> {
        let tx = self.write_tx()?;
        if unhide {
            tx.execute(
                "UPDATE messages SET hidden = 0 WHERE id = \
                 (SELECT local_message_id FROM snoozes WHERE id = ?1)",
                params![id.0],
            )?;
        }
        tx.execute("DELETE FROM snoozes WHERE id = ?1", params![id.0])?;
        tx.commit()?;
        Ok(())
    }

    /// Records how far a move without MOVE got (#423). `target_uidnext` is
    /// kept unless a new value is given.
    pub fn set_snooze_step(
        &self,
        id: SnoozeId,
        step: u8,
        target_uidnext: Option<u32>,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE snoozes SET step = ?2, target_uidnext = COALESCE(?3, target_uidnext) \
             WHERE id = ?1",
            params![id.0, step, target_uidnext],
        )?;
        Ok(())
    }

    /// The server refused after the COPY: step 4, the copy is to be
    /// removed. `rollback_since` keeps the time of the first refusal.
    pub fn start_snooze_rollback(&self, id: SnoozeId, now: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE snoozes SET step = 4, rollback_since = COALESCE(rollback_since, ?2) \
             WHERE id = ?1",
            params![id.0, now],
        )?;
        Ok(())
    }

    /// The snooze with this id, as stored now.
    pub fn get_snooze_by_id(&self, id: SnoozeId) -> Result<Option<Snooze>, StoreError> {
        let sql = format!("SELECT {COLS} FROM snoozes WHERE id = ?1");
        self.conn
            .query_row(&sql, params![id.0], |r| Ok(from_row(r)))
            .optional()?
            .transpose()
    }

    /// Tries a snooze again at `next_attempt_at`.
    pub fn defer_snooze(&self, id: SnoozeId, next_attempt_at: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE snoozes SET attempts = attempts + 1, next_attempt_at = ?2 WHERE id = ?1",
            params![id.0, next_attempt_at],
        )?;
        Ok(())
    }

    /// Hides or shows a message row (used while it is being woken).
    pub fn set_message_hidden(&self, id: MessageId, hidden: bool) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE messages SET hidden = ?2 WHERE id = ?1",
            params![id.0, hidden],
        )?;
        Ok(())
    }

    /// Deletes a message row (it moved away on the server).
    pub fn delete_message_row(&self, id: MessageId) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM messages WHERE id = ?1", params![id.0])?;
        Ok(())
    }

    /// Messages in `folder` with a keyword starting with `prefix`, and the
    /// matching keywords.
    pub fn messages_with_keyword_prefix(
        &self,
        folder: FolderId,
        prefix: &str,
    ) -> Result<Vec<KeywordedMessage>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.message_id_header, j.value FROM messages m, \
             json_each(m.keywords_json) j \
             WHERE m.folder_id = ?1 AND substr(j.value, 1, length(?2)) = ?2 ORDER BY m.id",
        )?;
        let rows = stmt.query_map(params![folder.0, prefix], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out: Vec<KeywordedMessage> = Vec::new();
        for row in rows {
            let (id, mid, keyword) = row?;
            match out.last_mut() {
                Some(last) if last.id.0 == id => last.keywords.push(keyword),
                _ => out.push(KeywordedMessage {
                    id: MessageId(id),
                    message_id_header: mid,
                    keywords: vec![keyword],
                }),
            }
        }
        Ok(out)
    }

    /// The local row of the message with this `Message-ID` in `folder`.
    pub fn find_message_by_header(
        &self,
        folder: FolderId,
        message_id_header: &str,
    ) -> Result<Option<MessageId>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM messages WHERE folder_id = ?1 AND message_id_header = ?2 \
                 ORDER BY id LIMIT 1",
                params![folder.0, message_id_header],
                |r| r.get(0),
            )
            .optional()?
            .map(MessageId))
    }
}

/// The states, for callers that filter.
impl Snooze {
    /// Whether the wake time is known to this device only.
    pub fn device_only(&self) -> bool {
        self.state == SnoozeState::Snoozed && !self.on_server
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_names_round_trip() {
        for s in [
            SnoozeState::ToSnooze,
            SnoozeState::Snoozed,
            SnoozeState::ToWake,
        ] {
            assert_eq!(SnoozeState::parse(s.as_str()).expect("parses"), s);
        }
        assert!(SnoozeState::parse("other").is_err());
    }
}
