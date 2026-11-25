//! The outbox (M2-06): messages waiting for the server to accept them.
//!
//! State changes are single guarded statements (`WHERE state = ...`), so
//! two senders racing (the app and a background job) can't both claim one
//! message.

use super::narrow;
use crate::{AccountId, DraftFields, Store, StoreError};
use rusqlite::{params, OptionalExtension, Row};

/// Row id of an outbox message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutboxId(pub i64);

/// Where an outbox message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    /// Waiting to be sent.
    Queued,
    /// Being sent now (or the app stopped while sending).
    Sending,
    /// Accepted by the server.
    Sent,
    /// Refused for good, or retried for too long. Can be retried by hand.
    Failed,
}

impl OutboxState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Result<Self, StoreError> {
        Ok(match text {
            "queued" => Self::Queued,
            "sending" => Self::Sending,
            "sent" => Self::Sent,
            "failed" => Self::Failed,
            _ => return Err(StoreError::Corrupt { column: "state" }),
        })
    }
}

/// A sent message's copy in the Sent folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentCopy {
    /// Not sent yet.
    None,
    /// To upload.
    Pending,
    /// In the Sent folder.
    Saved,
    /// Not needed: the server saves it, or the account says never.
    Skipped,
    /// The upload was refused; the message was sent but isn't in Sent.
    Failed,
}

impl SentCopy {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Saved => "saved",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Result<Self, StoreError> {
        Ok(match text {
            "none" => Self::None,
            "pending" => Self::Pending,
            "saved" => Self::Saved,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            _ => {
                return Err(StoreError::Corrupt {
                    column: "sent_copy",
                })
            }
        })
    }
}

/// Who sends a scheduled message (M4-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleMode {
    /// Not scheduled.
    None,
    /// Scheduled; who sends it isn't known yet.
    Pending,
    /// Held by the server (RFC 4865); goes out on time with the phone off.
    Server,
    /// This device sends it at the time.
    Device,
}

impl ScheduleMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Server => "server",
            Self::Device => "device",
        }
    }

    fn parse(text: &str) -> Result<Self, StoreError> {
        Ok(match text {
            "none" => Self::None,
            "pending" => Self::Pending,
            "server" => Self::Server,
            "device" => Self::Device,
            _ => {
                return Err(StoreError::Corrupt {
                    column: "schedule_mode",
                })
            }
        })
    }
}

/// Whether to save sent messages to the Sent folder, per account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaveSentCopy {
    /// Yes, unless the server does it itself.
    #[default]
    Auto,
    Always,
    Never,
}

impl SaveSentCopy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    fn parse(text: &str) -> Result<Self, StoreError> {
        Ok(match text {
            "auto" => Self::Auto,
            "always" => Self::Always,
            "never" => Self::Never,
            _ => {
                return Err(StoreError::Corrupt {
                    column: "save_sent_copy",
                })
            }
        })
    }
}

/// A message to put in the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutboxMessage {
    pub account_id: AccountId,
    pub raw: Vec<u8>,
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    pub message_id_header: String,
    /// What the user typed, for editing it again.
    pub fields: DraftFields,
    /// Held back until this time (Unix seconds); 0 for at once.
    pub not_before: i64,
    /// A scheduled message (M4-07), rather than one held for undo send.
    pub scheduled: bool,
}

/// A message in the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxMessage {
    pub id: OutboxId,
    pub account_id: AccountId,
    pub raw: Vec<u8>,
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    pub message_id_header: String,
    pub fields: DraftFields,
    pub state: OutboxState,
    pub attempts: u32,
    pub next_attempt_at: i64,
    pub not_before: i64,
    pub last_error: Option<String>,
    pub rejected: Vec<String>,
    pub created_at: i64,
    pub sent_at: Option<i64>,
    /// Its copy in the Sent folder, once sent.
    pub sent_copy: SentCopy,
    pub copy_attempts: u32,
    /// Who sends it, for a scheduled message.
    pub schedule_mode: ScheduleMode,
}

fn lines(entries: &[String]) -> String {
    entries
        .iter()
        .map(|e| e.replace(['\r', '\n'], " "))
        .filter(|e| !e.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn split(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

const COLS: &str = "id, account_id, raw, envelope_from, envelope_to, message_id_header, \
    to_list, cc_list, bcc_list, subject, body, in_reply_to, references_list, state, attempts, \
    next_attempt_at, not_before, last_error, rejected, created_at, sent_at, sent_copy, \
    copy_attempts, schedule_mode";

fn from_row(r: &Row<'_>) -> Result<OutboxMessage, StoreError> {
    let references: String = r.get(12)?;
    Ok(OutboxMessage {
        id: OutboxId(r.get(0)?),
        account_id: AccountId(r.get(1)?),
        raw: r.get(2)?,
        envelope_from: r.get(3)?,
        envelope_to: split(&r.get::<_, String>(4)?),
        message_id_header: r.get(5)?,
        fields: DraftFields {
            to: split(&r.get::<_, String>(6)?),
            cc: split(&r.get::<_, String>(7)?),
            bcc: split(&r.get::<_, String>(8)?),
            subject: r.get(9)?,
            body: r.get(10)?,
            in_reply_to: r.get(11)?,
            references: references.split_whitespace().map(str::to_owned).collect(),
        },
        state: OutboxState::parse(&r.get::<_, String>(13)?)?,
        attempts: narrow(r.get(14)?, "attempts")?,
        next_attempt_at: r.get(15)?,
        not_before: r.get(16)?,
        last_error: r.get(17)?,
        rejected: split(&r.get::<_, String>(18)?),
        created_at: r.get(19)?,
        sent_at: r.get(20)?,
        sent_copy: SentCopy::parse(&r.get::<_, String>(21)?)?,
        copy_attempts: narrow(r.get(22)?, "copy_attempts")?,
        schedule_mode: ScheduleMode::parse(&r.get::<_, String>(23)?)?,
    })
}

impl Store {
    /// Puts a message in the outbox, queued and due at `now` (or at its
    /// `not_before`, if later).
    pub fn insert_outbox(&self, m: &NewOutboxMessage, now: i64) -> Result<OutboxId, StoreError> {
        let f = &m.fields;
        self.conn.execute(
            "INSERT INTO outbox (account_id, raw, envelope_from, envelope_to, message_id_header, \
             to_list, cc_list, bcc_list, subject, body, in_reply_to, references_list, \
             next_attempt_at, not_before, created_at, schedule_mode) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                m.account_id.0,
                m.raw,
                m.envelope_from,
                lines(&m.envelope_to),
                m.message_id_header,
                lines(&f.to),
                lines(&f.cc),
                lines(&f.bcc),
                f.subject,
                f.body,
                f.in_reply_to,
                f.references.join(" "),
                now.max(m.not_before),
                m.not_before,
                now,
                if m.scheduled { "pending" } else { "none" }
            ],
        )?;
        Ok(OutboxId(self.conn.last_insert_rowid()))
    }

    pub fn get_outbox(&self, id: OutboxId) -> Result<Option<OutboxMessage>, StoreError> {
        let sql = format!("SELECT {COLS} FROM outbox WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![id.0])?;
        rows.next()?.map(from_row).transpose()
    }

    fn outbox_where(
        &self,
        filter: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        let sql = format!("SELECT {COLS} FROM outbox WHERE {filter} ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(args)?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }

    /// The account's messages that need attention: queued, sending and
    /// failed, sent ones whose copy in Sent failed, and ones the server
    /// holds until a time still to come. Oldest first.
    pub fn list_outbox(&self, account: AccountId) -> Result<Vec<OutboxMessage>, StoreError> {
        self.outbox_where(
            "account_id = ?1 AND (state != 'sent' OR sent_copy = 'failed' \
             OR (schedule_mode = 'server' AND not_before > CAST(strftime('%s', 'now') AS INTEGER)))",
            &[&account.0],
        )
    }

    /// Scheduled, queued messages of the account whose sender isn't decided
    /// yet.
    pub fn undecided_scheduled(
        &self,
        account: AccountId,
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        self.outbox_where(
            "account_id = ?1 AND state = 'queued' AND schedule_mode = 'pending'",
            &[&account.0],
        )
    }

    /// Records who sends a scheduled message.
    pub fn set_schedule_mode(&self, id: OutboxId, mode: ScheduleMode) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE outbox SET schedule_mode = ?2 WHERE id = ?1",
            params![id.0, mode.as_str()],
        )?;
        Ok(())
    }

    /// Moves a queued scheduled message to `send_at` (and lets the server
    /// question be asked again). Returns false if it isn't queued, or the
    /// server already holds it.
    pub fn reschedule_outbox(&self, id: OutboxId, send_at: i64) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE outbox SET not_before = ?2, next_attempt_at = ?2, schedule_mode = 'pending' \
             WHERE id = ?1 AND state = 'queued' AND schedule_mode != 'none'",
            params![id.0, send_at],
        )?;
        Ok(n > 0)
    }

    /// Sent messages of the account whose copy is due to be uploaded.
    pub fn pending_sent_copies(
        &self,
        account: AccountId,
        now: i64,
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        self.outbox_where(
            "account_id = ?1 AND state = 'sent' AND sent_copy = 'pending' AND copy_next_attempt_at <= ?2",
            &[&account.0, &now],
        )
    }

    /// How many sent messages of the account still have a copy to upload.
    pub fn count_pending_sent_copies(&self, account: AccountId) -> Result<u64, StoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM outbox WHERE account_id = ?1 AND state = 'sent' \
             AND sent_copy = 'pending'",
            params![account.0],
            |r| r.get(0),
        )?;
        narrow(n, "count")
    }

    /// Records where a sent message's copy stands.
    pub fn set_sent_copy(&self, id: OutboxId, copy: SentCopy) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE outbox SET sent_copy = ?2, copy_next_attempt_at = 0 WHERE id = ?1",
            params![id.0, copy.as_str()],
        )?;
        Ok(())
    }

    /// The copy upload failed for now: try again at `next_attempt_at`.
    pub fn defer_sent_copy(
        &self,
        id: OutboxId,
        attempts: u32,
        next_attempt_at: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE outbox SET copy_attempts = ?2, copy_next_attempt_at = ?3 WHERE id = ?1",
            params![id.0, attempts, next_attempt_at],
        )?;
        Ok(())
    }

    /// Tries a failed copy again. Returns whether there was one.
    pub fn retry_sent_copy(&self, id: OutboxId) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE outbox SET sent_copy = 'pending', copy_attempts = 0, copy_next_attempt_at = 0 \
             WHERE id = ?1 AND sent_copy = 'failed'",
            params![id.0],
        )?;
        Ok(n > 0)
    }

    /// The account's setting for saving sent messages.
    pub fn save_sent_copy(&self, account: AccountId) -> Result<Option<SaveSentCopy>, StoreError> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT save_sent_copy FROM accounts WHERE id = ?1",
                params![account.0],
                |r| r.get(0),
            )
            .optional()?;
        text.map(|t| SaveSentCopy::parse(&t)).transpose()
    }

    /// Changes the account's setting. Returns whether the account exists.
    pub fn set_save_sent_copy(
        &self,
        account: AccountId,
        value: SaveSentCopy,
    ) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE accounts SET save_sent_copy = ?2 WHERE id = ?1",
            params![account.0, value.as_str()],
        )?;
        Ok(n > 0)
    }

    /// Queued messages of the account that are due by `now`, oldest first.
    pub fn due_outbox(
        &self,
        account: AccountId,
        now: i64,
    ) -> Result<Vec<OutboxMessage>, StoreError> {
        self.outbox_where(
            "account_id = ?1 AND state = 'queued' AND next_attempt_at <= ?2 AND not_before <= ?2",
            &[&account.0, &now],
        )
    }

    /// Messages of the account left in `sending` (the app stopped mid-send).
    pub fn interrupted_outbox(&self, account: AccountId) -> Result<Vec<OutboxMessage>, StoreError> {
        self.outbox_where("account_id = ?1 AND state = 'sending'", &[&account.0])
    }

    /// Accounts with messages still to send (queued or interrupted).
    pub fn outbox_accounts(&self) -> Result<Vec<AccountId>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT account_id FROM outbox WHERE state IN ('queued', 'sending') ORDER BY account_id",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(AccountId(r.get(0)?));
        }
        Ok(out)
    }

    /// The earliest time a queued message of the account is due, if any.
    pub fn next_outbox_attempt(&self, account: AccountId) -> Result<Option<i64>, StoreError> {
        Ok(self.conn.query_row(
            "SELECT min(max(next_attempt_at, not_before)) FROM outbox \
             WHERE account_id = ?1 AND state = 'queued'",
            params![account.0],
            |r| r.get(0),
        )?)
    }

    /// Claims a queued message for sending. Returns false if it isn't queued
    /// (someone else claimed it, or it was deleted): don't send it then.
    pub fn claim_outbox(&self, id: OutboxId) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE outbox SET state = 'sending' WHERE id = ?1 AND state = 'queued'",
            params![id.0],
        )?;
        Ok(n > 0)
    }

    /// The server accepted it; `rejected` lists recipients it refused.
    pub fn outbox_sent(
        &self,
        id: OutboxId,
        rejected: &[String],
        now: i64,
    ) -> Result<(), StoreError> {
        // The copy for the Sent folder is due at once (M2-07).
        self.conn.execute(
            "UPDATE outbox SET state = 'sent', rejected = ?2, sent_at = ?3, last_error = NULL, \
             sent_copy = 'pending', copy_next_attempt_at = ?3 WHERE id = ?1",
            params![id.0, lines(rejected), now],
        )?;
        Ok(())
    }

    /// It failed for now: queued again for `next_attempt_at`.
    pub fn outbox_retry_at(
        &self,
        id: OutboxId,
        attempts: u32,
        next_attempt_at: i64,
        error: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE outbox SET state = 'queued', attempts = ?2, next_attempt_at = ?3, \
             last_error = ?4 WHERE id = ?1",
            params![id.0, attempts, next_attempt_at, error],
        )?;
        Ok(())
    }

    /// It failed for good (or for too long).
    pub fn outbox_failed(
        &self,
        id: OutboxId,
        attempts: u32,
        error: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE outbox SET state = 'failed', attempts = ?2, last_error = ?3 WHERE id = ?1",
            params![id.0, attempts, error],
        )?;
        Ok(())
    }

    /// Queues a failed or interrupted message again, due now. Returns
    /// whether it was failed or sending.
    pub fn requeue_outbox(&self, id: OutboxId, now: i64) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE outbox SET state = 'queued', attempts = 0, next_attempt_at = ?2 \
             WHERE id = ?1 AND state IN ('failed', 'sending')",
            params![id.0, now],
        )?;
        Ok(n > 0)
    }

    /// Removes a message only if it is still queued (sending hasn't
    /// started). Returns it, or `None` if it's being sent, was sent, failed
    /// or doesn't exist. For undo send (M4-08).
    pub fn take_queued_outbox(&self, id: OutboxId) -> Result<Option<OutboxMessage>, StoreError> {
        let Some(message) = self.get_outbox(id)? else {
            return Ok(None);
        };
        let n = self.conn.execute(
            "DELETE FROM outbox WHERE id = ?1 AND state = 'queued'",
            params![id.0],
        )?;
        Ok((n > 0).then_some(message))
    }

    /// Removes a message that isn't being sent. Returns it, or `None` if it
    /// doesn't exist or is being sent right now.
    pub fn take_outbox(&self, id: OutboxId) -> Result<Option<OutboxMessage>, StoreError> {
        let Some(message) = self.get_outbox(id)? else {
            return Ok(None);
        };
        let n = self.conn.execute(
            "DELETE FROM outbox WHERE id = ?1 AND state != 'sending'",
            params![id.0],
        )?;
        Ok((n > 0).then_some(message))
    }

    /// Sets the state directly; for tests and recovery only.
    pub fn set_outbox_state(&self, id: OutboxId, state: OutboxState) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE outbox SET state = ?2 WHERE id = ?1",
            params![id.0, state.as_str()],
        )?;
        Ok(())
    }
}
