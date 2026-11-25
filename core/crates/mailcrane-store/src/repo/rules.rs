//! Local rules (M4-09). See `migrations/0012_rules.sql`.

use super::messages::{from_row, COLS};
use super::narrow;
use crate::{AccountId, FolderId, MessageHeader, MessageId, Store, StoreError};
use rusqlite::{params, OptionalExtension};

/// Row id of a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuleId(pub i64);

/// What a rule looks at. Text matches are "contains", ignoring case and
/// accents; never a regular expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleCondition {
    /// The sender's name or address contains the text.
    From(String),
    /// A To or Cc name or address contains the text.
    ToCc(String),
    /// The subject contains the text.
    Subject(String),
    /// The message has an attachment.
    HasAttachment,
    /// The List-Id (RFC 2919) is exactly this (any case).
    ListId(String),
    /// Older than this many days (only meaningful when running a rule on
    /// existing mail; new mail never matches).
    OlderThanDays(u32),
}

/// What a rule does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleAction {
    /// Move to a folder of the same account.
    Move(FolderId),
    MarkRead,
    Flag,
    Archive,
    Delete,
}

/// A rule to save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRule {
    /// `None`: every account.
    pub account_id: Option<AccountId>,
    pub name: String,
    pub enabled: bool,
    /// All conditions must match (`false`: any one).
    pub match_all: bool,
    pub stop_processing: bool,
    pub conditions: Vec<RuleCondition>,
    pub actions: Vec<RuleAction>,
}

/// A stored rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: RuleId,
    pub position: u32,
    pub rule: NewRule,
}

fn condition_parts(c: &RuleCondition) -> (&'static str, String) {
    match c {
        RuleCondition::From(v) => ("from", v.clone()),
        RuleCondition::ToCc(v) => ("to_cc", v.clone()),
        RuleCondition::Subject(v) => ("subject", v.clone()),
        RuleCondition::HasAttachment => ("has_attachment", String::new()),
        RuleCondition::ListId(v) => ("list_id", v.clone()),
        RuleCondition::OlderThanDays(d) => ("older_than_days", d.to_string()),
    }
}

fn condition_from(kind: &str, value: String) -> Result<RuleCondition, StoreError> {
    Ok(match kind {
        "from" => RuleCondition::From(value),
        "to_cc" => RuleCondition::ToCc(value),
        "subject" => RuleCondition::Subject(value),
        "has_attachment" => RuleCondition::HasAttachment,
        "list_id" => RuleCondition::ListId(value),
        "older_than_days" => RuleCondition::OlderThanDays(
            value
                .parse()
                .map_err(|_| StoreError::Corrupt { column: "value" })?,
        ),
        _ => return Err(StoreError::Corrupt { column: "kind" }),
    })
}

fn action_parts(a: &RuleAction) -> (&'static str, Option<i64>) {
    match a {
        RuleAction::Move(f) => ("move", Some(f.0)),
        RuleAction::MarkRead => ("mark_read", None),
        RuleAction::Flag => ("flag", None),
        RuleAction::Archive => ("archive", None),
        RuleAction::Delete => ("delete", None),
    }
}

fn action_from(kind: &str, folder: Option<i64>) -> Result<RuleAction, StoreError> {
    Ok(match (kind, folder) {
        ("move", Some(f)) => RuleAction::Move(FolderId(f)),
        ("mark_read", _) => RuleAction::MarkRead,
        ("flag", _) => RuleAction::Flag,
        ("archive", _) => RuleAction::Archive,
        ("delete", _) => RuleAction::Delete,
        _ => return Err(StoreError::Corrupt { column: "kind" }),
    })
}

impl Store {
    fn write_rule_parts(
        tx: &rusqlite::Transaction<'_>,
        id: RuleId,
        rule: &NewRule,
    ) -> Result<(), StoreError> {
        tx.execute(
            "DELETE FROM rule_conditions WHERE rule_id = ?1",
            params![id.0],
        )?;
        tx.execute("DELETE FROM rule_actions WHERE rule_id = ?1", params![id.0])?;
        for (i, c) in rule.conditions.iter().enumerate() {
            let (kind, value) = condition_parts(c);
            tx.execute(
                "INSERT INTO rule_conditions (rule_id, position, kind, value) VALUES (?1, ?2, ?3, ?4)",
                params![id.0, i64::try_from(i).unwrap_or(i64::MAX), kind, value],
            )?;
        }
        for (i, a) in rule.actions.iter().enumerate() {
            let (kind, folder) = action_parts(a);
            tx.execute(
                "INSERT INTO rule_actions (rule_id, position, kind, folder_id) VALUES (?1, ?2, ?3, ?4)",
                params![id.0, i64::try_from(i).unwrap_or(i64::MAX), kind, folder],
            )?;
        }
        Ok(())
    }

    /// Saves a new rule at the end of the list.
    pub fn insert_rule(&self, rule: &NewRule) -> Result<RuleId, StoreError> {
        let tx = self.write_tx()?;
        tx.execute(
            "INSERT INTO rules (account_id, name, position, enabled, match_all, stop_processing) \
             VALUES (?1, ?2, (SELECT COALESCE(max(position), -1) + 1 FROM rules), ?3, ?4, ?5)",
            params![
                rule.account_id.map(|a| a.0),
                rule.name,
                rule.enabled,
                rule.match_all,
                rule.stop_processing
            ],
        )?;
        let id = RuleId(tx.last_insert_rowid());
        Self::write_rule_parts(&tx, id, rule)?;
        tx.commit()?;
        Ok(id)
    }

    /// Replaces a rule's settings, keeping its place. Returns false if it
    /// doesn't exist.
    pub fn update_rule(&self, id: RuleId, rule: &NewRule) -> Result<bool, StoreError> {
        let tx = self.write_tx()?;
        let n = tx.execute(
            "UPDATE rules SET account_id = ?2, name = ?3, enabled = ?4, match_all = ?5, \
             stop_processing = ?6 WHERE id = ?1",
            params![
                id.0,
                rule.account_id.map(|a| a.0),
                rule.name,
                rule.enabled,
                rule.match_all,
                rule.stop_processing
            ],
        )?;
        if n == 0 {
            return Ok(false);
        }
        Self::write_rule_parts(&tx, id, rule)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn delete_rule(&self, id: RuleId) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM rules WHERE id = ?1", params![id.0])?;
        Ok(())
    }

    pub fn set_rule_enabled(&self, id: RuleId, enabled: bool) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE rules SET enabled = ?2 WHERE id = ?1",
            params![id.0, enabled],
        )?;
        Ok(())
    }

    /// Moves a rule to `index` in the list (0 = first), shifting the others.
    pub fn move_rule(&self, id: RuleId, index: u32) -> Result<(), StoreError> {
        let tx = self.write_tx()?;
        let mut ids: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM rules ORDER BY position, id")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let Some(from) = ids.iter().position(|r| *r == id.0) else {
            return Ok(());
        };
        let moved = ids.remove(from);
        let to = usize::try_from(index).unwrap_or(usize::MAX).min(ids.len());
        ids.insert(to, moved);
        for (pos, rid) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE rules SET position = ?2 WHERE id = ?1",
                params![rid, i64::try_from(pos).unwrap_or(i64::MAX)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// One rule.
    pub fn get_rule(&self, id: RuleId) -> Result<Option<Rule>, StoreError> {
        Ok(self.load_rules(Some(id), None, false)?.into_iter().next())
    }

    /// Every rule, in order (`account`: only the ones for it or for every
    /// account).
    pub fn list_rules(&self, account: Option<AccountId>) -> Result<Vec<Rule>, StoreError> {
        self.load_rules(None, account, account.is_some())
    }

    fn load_rules(
        &self,
        id: Option<RuleId>,
        account: Option<AccountId>,
        filter_account: bool,
    ) -> Result<Vec<Rule>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, name, position, enabled, match_all, stop_processing FROM rules \
             WHERE (?1 IS NULL OR id = ?1) AND (?3 = 0 OR account_id IS NULL OR account_id = ?2) \
             ORDER BY position, id",
        )?;
        let heads = stmt
            .query_map(
                params![id.map(|r| r.0), account.map(|a| a.0), filter_account],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, bool>(4)?,
                        r.get::<_, bool>(5)?,
                        r.get::<_, bool>(6)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let mut conds = self.conn.prepare_cached(
            "SELECT kind, value FROM rule_conditions WHERE rule_id = ?1 ORDER BY position",
        )?;
        let mut acts = self.conn.prepare_cached(
            "SELECT kind, folder_id FROM rule_actions WHERE rule_id = ?1 ORDER BY position",
        )?;
        let mut out = Vec::with_capacity(heads.len());
        for (rid, acc, name, position, enabled, match_all, stop) in heads {
            let conditions = conds
                .query_map(params![rid], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|(k, v)| condition_from(&k, v))
                .collect::<Result<Vec<_>, _>>()?;
            let actions = acts
                .query_map(params![rid], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|(k, f)| action_from(&k, f))
                .collect::<Result<Vec<_>, _>>()?;
            out.push(Rule {
                id: RuleId(rid),
                position: narrow(position, "position")?,
                rule: NewRule {
                    account_id: acc.map(AccountId),
                    name,
                    enabled,
                    match_all,
                    stop_processing: stop,
                    conditions,
                    actions,
                },
            });
        }
        Ok(out)
    }

    /// Stores List-Ids for messages of `folder` by UID.
    pub fn set_list_ids(
        &self,
        folder: FolderId,
        ids: &[(u32, Option<String>)],
    ) -> Result<(), StoreError> {
        let tx = self.write_tx()?;
        for (uid, list_id) in ids {
            tx.execute(
                "UPDATE messages SET list_id = ?3 WHERE folder_id = ?1 AND uid = ?2",
                params![folder.0, uid, list_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The List-Id of a message.
    pub fn message_list_id(&self, id: MessageId) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT list_id FROM messages WHERE id = ?1",
                params![id.0],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Marks messages outside inbox folders as looked at (rules only act on
    /// new mail in an inbox), and returns the inbox messages still to look
    /// at, oldest row first, with their List-Id.
    pub fn take_rules_pending(
        &self,
        account: AccountId,
    ) -> Result<Vec<(MessageHeader, Option<String>)>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET rules_done = 1 WHERE rules_done = 0 AND folder_id IN \
             (SELECT id FROM folders WHERE account_id = ?1 AND COALESCE(role, '') <> 'inbox')",
            params![account.0],
        )?;
        let cols = prefixed_cols();
        let sql = format!(
            "SELECT {cols}, m.list_id FROM messages m JOIN folders f ON f.id = m.folder_id \
             WHERE f.account_id = ?1 AND f.role = 'inbox' AND m.rules_done = 0 AND m.hidden = 0 \
             ORDER BY m.id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![account.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let header = from_row(r)?;
            let list_id: Option<String> = r.get(24)?;
            out.push((header, list_id));
        }
        Ok(out)
    }

    /// Whether rules already handled the message with this Message-ID.
    pub fn rules_seen(
        &self,
        account: AccountId,
        message_id_header: &str,
    ) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM rules_seen WHERE account_id = ?1 AND message_id_header = ?2",
                params![account.0, message_id_header],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Records that rules looked at a message.
    pub fn mark_rules_done(
        &self,
        account: AccountId,
        message: MessageId,
        message_id_header: Option<&str>,
    ) -> Result<(), StoreError> {
        let tx = self.write_tx()?;
        tx.execute(
            "UPDATE messages SET rules_done = 1 WHERE id = ?1",
            params![message.0],
        )?;
        if let Some(mid) = message_id_header {
            tx.execute(
                "INSERT OR IGNORE INTO rules_seen (account_id, message_id_header) VALUES (?1, ?2)",
                params![account.0, mid],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The messages of a folder (not hidden), with their List-Id, for
    /// running a rule on existing mail.
    pub fn folder_messages_with_list_id(
        &self,
        folder: FolderId,
    ) -> Result<Vec<(MessageHeader, Option<String>)>, StoreError> {
        let cols = prefixed_cols();
        let sql = format!(
            "SELECT {cols}, m.list_id FROM messages m WHERE m.folder_id = ?1 AND m.hidden = 0 \
             ORDER BY m.id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![folder.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let header = from_row(r)?;
            let list_id: Option<String> = r.get(24)?;
            out.push((header, list_id));
        }
        Ok(out)
    }
}

fn prefixed_cols() -> String {
    COLS.split(',')
        .map(|c| format!("m.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}
