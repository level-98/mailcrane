//! Server-side rules state per account (M4-23). See
//! `migrations/0018_server_rules.sql`.

use super::narrow;
use crate::{AccountId, RuleId, Store, StoreError};
use rusqlite::{params, OptionalExtension};

/// Where an account's server rules stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerRulesState {
    /// No usable ManageSieve server (checked again later).
    Unavailable,
    /// Another script is active; rules stay on the device.
    OtherActive,
    /// Mailcrane's script is the active one.
    Active,
}

impl ServerRulesState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::OtherActive => "other_active",
            Self::Active => "active",
        }
    }

    fn parse(s: &str) -> Result<Self, StoreError> {
        Ok(match s {
            "unavailable" => Self::Unavailable,
            "other_active" => Self::OtherActive,
            "active" => Self::Active,
            _ => return Err(StoreError::Corrupt { column: "state" }),
        })
    }
}

/// An account's server rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRules {
    pub account_id: AccountId,
    pub state: ServerRulesState,
    /// The ManageSieve server last used.
    pub host: Option<String>,
    pub port: Option<u16>,
    /// The server's Sieve extensions.
    pub extensions: Vec<String>,
    /// The other active script's name (state `OtherActive`).
    pub other_script: Option<String>,
    /// Hash of the script on the server (state `Active`).
    pub script_hash: Option<String>,
    /// When the server was last checked (Unix seconds).
    pub checked_at: i64,
    /// The user asked to switch to Mailcrane's script.
    pub switch_requested: bool,
    /// Rules in the server script, with their fingerprints as uploaded.
    pub deployed: Vec<(RuleId, String)>,
}

impl Store {
    /// The account's server rules, if the server has been checked.
    pub fn server_rules(&self, account: AccountId) -> Result<Option<ServerRules>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT state, host, port, extensions, other_script, script_hash, checked_at, \
                 switch_requested FROM server_rules WHERE account_id = ?1",
                params![account.0],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, bool>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((state, host, port, ext, other, hash, checked_at, switch)) = row else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT rule_id, fingerprint FROM server_rules_deployed WHERE account_id = ?1 \
             ORDER BY rule_id",
        )?;
        let deployed = stmt
            .query_map(params![account.0], |r| {
                Ok((RuleId(r.get::<_, i64>(0)?), r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(ServerRules {
            account_id: account,
            state: ServerRulesState::parse(&state)?,
            host,
            port: port.map(|p| narrow(p, "port")).transpose()?,
            extensions: ext.split_ascii_whitespace().map(str::to_owned).collect(),
            other_script: other,
            script_hash: hash,
            checked_at,
            switch_requested: switch,
            deployed,
        }))
    }

    /// Every account's server rules.
    pub fn list_server_rules(&self) -> Result<Vec<ServerRules>, StoreError> {
        let ids: Vec<i64> = {
            let mut stmt = self
                .conn
                .prepare("SELECT account_id FROM server_rules ORDER BY account_id")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(s) = self.server_rules(AccountId(id))? {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// Saves the account's server rules, replacing the deployed set.
    pub fn save_server_rules(&self, rules: &ServerRules) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO server_rules (account_id, state, host, port, extensions, \
             other_script, script_hash, checked_at, switch_requested) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rules.account_id.0,
                rules.state.as_str(),
                rules.host,
                rules.port,
                rules.extensions.join(" "),
                rules.other_script,
                rules.script_hash,
                rules.checked_at,
                rules.switch_requested
            ],
        )?;
        tx.execute(
            "DELETE FROM server_rules_deployed WHERE account_id = ?1",
            params![rules.account_id.0],
        )?;
        for (rule, fingerprint) in &rules.deployed {
            tx.execute(
                "INSERT OR REPLACE INTO server_rules_deployed (account_id, rule_id, fingerprint) \
                 VALUES (?1, ?2, ?3)",
                params![rules.account_id.0, rule.0, fingerprint],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records that the user asked to switch the account to Mailcrane's
    /// script. Returns false if the server hasn't been checked.
    pub fn request_server_rules_switch(&self, account: AccountId) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE server_rules SET switch_requested = 1 WHERE account_id = ?1",
            params![account.0],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewAccount, NewRule, RuleAction, RuleCondition, Security};

    #[test]
    fn save_load_switch_and_cascade() -> Result<(), StoreError> {
        let store = Store::open_in_memory()?;
        let account = store.insert_account(&NewAccount {
            display_name: "A".into(),
            email: "a@example.org".into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: "a@example.org".into(),
        })?;
        assert_eq!(store.server_rules(account)?, None);
        assert!(!store.request_server_rules_switch(account)?);
        let rule = store.insert_rule(&NewRule {
            account_id: None,
            name: "r".into(),
            enabled: true,
            match_all: true,
            stop_processing: false,
            conditions: vec![RuleCondition::Subject("x".into())],
            actions: vec![RuleAction::Flag],
        })?;
        let saved = ServerRules {
            account_id: account,
            state: ServerRulesState::Active,
            host: Some("imap.example.org".into()),
            port: Some(4190),
            extensions: vec!["fileinto".into(), "imap4flags".into()],
            other_script: None,
            script_hash: Some("abc".into()),
            checked_at: 100,
            switch_requested: false,
            deployed: vec![(rule, "f1".into())],
        };
        store.save_server_rules(&saved)?;
        assert_eq!(store.server_rules(account)?, Some(saved.clone()));
        assert!(store.request_server_rules_switch(account)?);
        assert!(store
            .server_rules(account)?
            .is_some_and(|s| s.switch_requested));
        assert_eq!(store.list_server_rules()?.len(), 1);
        // Deleting the rule drops it from the deployed set.
        store.delete_rule(rule)?;
        assert!(store
            .server_rules(account)?
            .is_some_and(|s| s.deployed.is_empty()));
        Ok(())
    }
}
