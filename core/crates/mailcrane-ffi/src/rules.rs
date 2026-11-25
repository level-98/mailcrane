//! Local rules for the apps (M4-09): list, save, delete, enable, reorder,
//! and run on a folder's existing mail. Rules run by themselves on new
//! inbox mail after each sync.

use crate::actions::now;
use crate::sync::{ListenerAdapter, SyncEngine};
use mailcrane_core::rules;
use mailcrane_store::{AccountId, FolderId, NewRule, RuleAction, RuleCondition, RuleId, Store};

/// Why a rule call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RulesFailure {
    /// A rule needs a name, at least one condition and at least one action
    /// (and a folder for "move"); or the rule doesn't exist.
    #[error("invalid rule")]
    Invalid,
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

/// What a rule looks at. Text is matched as "contains", ignoring case and
/// accents.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RuleConditionInfo {
    From {
        text: String,
    },
    ToOrCc {
        text: String,
    },
    Subject {
        text: String,
    },
    HasAttachment,
    /// Exactly this List-Id, for example `news.example.org`.
    ListId {
        id: String,
    },
    /// Only when running on existing mail.
    OlderThanDays {
        days: u32,
    },
}

/// What a rule does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RuleActionInfo {
    Move { folder_id: i64 },
    MarkRead,
    Flag,
    Archive,
    Delete,
}

/// A rule as the editor shows and saves it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RuleInput {
    /// `None`: every account.
    pub account_id: Option<i64>,
    pub name: String,
    pub enabled: bool,
    /// All conditions must match (`false`: any one).
    pub match_all: bool,
    /// Later rules don't run on a message this rule matched.
    pub stop_processing: bool,
    pub conditions: Vec<RuleConditionInfo>,
    pub actions: Vec<RuleActionInfo>,
}

/// A stored rule, in list order.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RuleInfo {
    pub id: i64,
    pub rule: RuleInput,
}

impl From<&RuleConditionInfo> for RuleCondition {
    fn from(c: &RuleConditionInfo) -> Self {
        match c {
            RuleConditionInfo::From { text } => Self::From(text.trim().to_owned()),
            RuleConditionInfo::ToOrCc { text } => Self::ToCc(text.trim().to_owned()),
            RuleConditionInfo::Subject { text } => Self::Subject(text.trim().to_owned()),
            RuleConditionInfo::HasAttachment => Self::HasAttachment,
            RuleConditionInfo::ListId { id } => Self::ListId(id.trim().to_owned()),
            RuleConditionInfo::OlderThanDays { days } => Self::OlderThanDays(*days),
        }
    }
}

impl From<&RuleCondition> for RuleConditionInfo {
    fn from(c: &RuleCondition) -> Self {
        match c {
            RuleCondition::From(t) => Self::From { text: t.clone() },
            RuleCondition::ToCc(t) => Self::ToOrCc { text: t.clone() },
            RuleCondition::Subject(t) => Self::Subject { text: t.clone() },
            RuleCondition::HasAttachment => Self::HasAttachment,
            RuleCondition::ListId(t) => Self::ListId { id: t.clone() },
            RuleCondition::OlderThanDays(d) => Self::OlderThanDays { days: *d },
        }
    }
}

impl From<RuleActionInfo> for RuleAction {
    fn from(a: RuleActionInfo) -> Self {
        match a {
            RuleActionInfo::Move { folder_id } => Self::Move(FolderId(folder_id)),
            RuleActionInfo::MarkRead => Self::MarkRead,
            RuleActionInfo::Flag => Self::Flag,
            RuleActionInfo::Archive => Self::Archive,
            RuleActionInfo::Delete => Self::Delete,
        }
    }
}

impl From<RuleAction> for RuleActionInfo {
    fn from(a: RuleAction) -> Self {
        match a {
            RuleAction::Move(f) => Self::Move { folder_id: f.0 },
            RuleAction::MarkRead => Self::MarkRead,
            RuleAction::Flag => Self::Flag,
            RuleAction::Archive => Self::Archive,
            RuleAction::Delete => Self::Delete,
        }
    }
}

fn to_store(input: &RuleInput) -> Result<NewRule, RulesFailure> {
    let rule = NewRule {
        account_id: input.account_id.map(AccountId),
        name: crate::messages::one_line(input.name.trim()),
        enabled: input.enabled,
        match_all: input.match_all,
        stop_processing: input.stop_processing,
        conditions: input.conditions.iter().map(RuleCondition::from).collect(),
        actions: input
            .actions
            .iter()
            .copied()
            .map(RuleAction::from)
            .collect(),
    };
    let empty_text = rule.conditions.iter().any(|c| match c {
        RuleCondition::From(t)
        | RuleCondition::ToCc(t)
        | RuleCondition::Subject(t)
        | RuleCondition::ListId(t) => t.is_empty(),
        RuleCondition::HasAttachment | RuleCondition::OlderThanDays(_) => false,
    });
    if rule.name.is_empty() || rule.conditions.is_empty() || rule.actions.is_empty() || empty_text {
        return Err(RulesFailure::Invalid);
    }
    Ok(rule)
}

fn open(db_path: &str) -> Result<Store, RulesFailure> {
    Store::open(db_path).map_err(|_| RulesFailure::Storage)
}

/// Every rule, in the order they run.
#[uniffi::export]
pub fn list_rules(db_path: String) -> Result<Vec<RuleInfo>, RulesFailure> {
    let store = open(&db_path)?;
    let rules = store.list_rules(None).map_err(|_| RulesFailure::Storage)?;
    Ok(rules
        .into_iter()
        .map(|r| RuleInfo {
            id: r.id.0,
            rule: RuleInput {
                account_id: r.rule.account_id.map(|a| a.0),
                name: r.rule.name,
                enabled: r.rule.enabled,
                match_all: r.rule.match_all,
                stop_processing: r.rule.stop_processing,
                conditions: r
                    .rule
                    .conditions
                    .iter()
                    .map(RuleConditionInfo::from)
                    .collect(),
                actions: r
                    .rule
                    .actions
                    .into_iter()
                    .map(RuleActionInfo::from)
                    .collect(),
            },
        })
        .collect())
}

/// Saves a rule: a new one (at the end) when `id` is `None`. Returns its id.
#[uniffi::export]
pub fn save_rule(db_path: String, id: Option<i64>, rule: RuleInput) -> Result<i64, RulesFailure> {
    let store = open(&db_path)?;
    let new = to_store(&rule)?;
    match id {
        None => store
            .insert_rule(&new)
            .map(|r| r.0)
            .map_err(|_| RulesFailure::Storage),
        Some(id) => match store.update_rule(RuleId(id), &new) {
            Ok(true) => Ok(id),
            Ok(false) => Err(RulesFailure::Invalid),
            Err(_) => Err(RulesFailure::Storage),
        },
    }
}

#[uniffi::export]
pub fn delete_rule(db_path: String, id: i64) -> Result<(), RulesFailure> {
    open(&db_path)?
        .delete_rule(RuleId(id))
        .map_err(|_| RulesFailure::Storage)
}

#[uniffi::export]
pub fn set_rule_enabled(db_path: String, id: i64, enabled: bool) -> Result<(), RulesFailure> {
    open(&db_path)?
        .set_rule_enabled(RuleId(id), enabled)
        .map_err(|_| RulesFailure::Storage)
}

/// Moves a rule to `index` in the list (0 = runs first).
#[uniffi::export]
pub fn move_rule(db_path: String, id: i64, index: u32) -> Result<(), RulesFailure> {
    open(&db_path)?
        .move_rule(RuleId(id), index)
        .map_err(|_| RulesFailure::Storage)
}

#[uniffi::export]
impl SyncEngine {
    /// Runs one rule on the messages now in `folder_id` (changes show at
    /// once; then call `replay_actions`). Returns how many messages it
    /// changed.
    pub fn run_rule_on_existing(&self, rule_id: i64, folder_id: i64) -> Result<u32, RulesFailure> {
        let store = self.open_store().map_err(|_| RulesFailure::Storage)?;
        let listener = ListenerAdapter(self.listener.clone());
        let report = rules::run_on_existing(
            &store,
            RuleId(rule_id),
            FolderId(folder_id),
            now(),
            &listener,
        )
        .map_err(|_| RulesFailure::Storage)?;
        Ok(u32::try_from(report.matched).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{NewAccount, Security};

    fn input(name: &str) -> RuleInput {
        RuleInput {
            account_id: None,
            name: name.into(),
            enabled: true,
            match_all: true,
            stop_processing: false,
            conditions: vec![RuleConditionInfo::Subject {
                text: " Invoice ".into(),
            }],
            actions: vec![RuleActionInfo::Flag],
        }
    }

    #[test]
    fn save_list_reorder_and_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        store
            .insert_account(&NewAccount {
                display_name: "A".into(),
                email: "a@example.org".into(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 465,
                smtp_security: Security::Tls,
                username: "a@example.org".into(),
            })
            .expect("account");
        drop(store);
        let a = save_rule(path.clone(), None, input("Invoices")).expect("save");
        let b = save_rule(path.clone(), None, input("Second")).expect("save");
        let listed = list_rules(path.clone()).expect("list");
        assert_eq!(listed.iter().map(|r| r.id).collect::<Vec<_>>(), [a, b]);
        assert_eq!(
            listed[0].rule.conditions,
            [RuleConditionInfo::Subject {
                text: "Invoice".into()
            }],
            "trimmed"
        );
        move_rule(path.clone(), b, 0).expect("move");
        set_rule_enabled(path.clone(), a, false).expect("disable");
        let listed = list_rules(path.clone()).expect("list");
        assert_eq!(listed[0].id, b);
        assert!(!listed[1].rule.enabled);
        let mut changed = input("Renamed");
        changed.actions = vec![RuleActionInfo::Archive];
        assert_eq!(save_rule(path.clone(), Some(a), changed.clone()), Ok(a));
        assert_eq!(
            save_rule(path.clone(), Some(999), changed),
            Err(RulesFailure::Invalid)
        );
        delete_rule(path.clone(), b).expect("delete");
        let listed = list_rules(path).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].rule.name, "Renamed");
        assert_eq!(listed[0].rule.actions, [RuleActionInfo::Archive]);
    }

    #[test]
    fn incomplete_rules_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let mut r = input("  ");
        assert_eq!(
            save_rule(path.clone(), None, r.clone()),
            Err(RulesFailure::Invalid)
        );
        r.name = "x".into();
        r.conditions.clear();
        assert_eq!(
            save_rule(path.clone(), None, r.clone()),
            Err(RulesFailure::Invalid)
        );
        r.conditions = vec![RuleConditionInfo::From { text: " ".into() }];
        assert_eq!(
            save_rule(path.clone(), None, r.clone()),
            Err(RulesFailure::Invalid)
        );
        r.conditions = vec![RuleConditionInfo::HasAttachment];
        r.actions.clear();
        assert_eq!(save_rule(path, None, r), Err(RulesFailure::Invalid));
    }
}
