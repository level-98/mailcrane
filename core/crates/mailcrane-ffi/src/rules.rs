//! Local rules for the apps (M4-09): list, save, delete, enable, reorder,
//! and run on a folder's existing mail. Rules run by themselves on new
//! inbox mail after each sync.
//!
//! Server-side rules (M4-23): where each rule runs ([`rule_locations`]),
//! accounts whose server runs another filter script
//! ([`server_rules_notices`]), and the user's explicit switch to
//! Mailcrane's script ([`use_server_rules`]).

use crate::actions::now;
use crate::sync::{ListenerAdapter, SyncEngine};
use mailcrane_core::rules;
use mailcrane_store::{
    AccountId, FolderId, NewRule, RuleAction, RuleCondition, RuleId, ServerRulesState, Store,
};

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
    /// The app turned this rule off because a folder it moved mail to was
    /// removed (#323): that folder's name, to explain why. Cleared when the
    /// rule is saved or turned back on.
    #[uniffi(default = None)]
    pub missing_folder: Option<String>,
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
            missing_folder: r.missing_folder.map(|n| crate::messages::one_line(&n)),
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

/// Where a rule runs (M4-23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RuleRunsOn {
    /// No account it applies to has server rules: it runs on the device, as
    /// all rules do; nothing to show.
    Unmarked,
    /// On the mail server, also while the phone is off.
    Server,
    /// On this device only: the server can't run it (or not yet).
    Device,
}

/// Where one rule runs.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RuleLocation {
    pub rule_id: i64,
    pub runs_on: RuleRunsOn,
}

/// An account whose server already runs another filter script, so its
/// rules stay on the device until the user switches.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerRulesNotice {
    pub account_id: i64,
    pub account_email: String,
    /// The other script's name, as one line of plain text.
    pub script_name: String,
    /// The user already asked to switch; it happens at the next sync.
    pub switch_requested: bool,
}

/// Where each rule runs, in list order.
#[uniffi::export]
pub fn rule_locations(db_path: String) -> Result<Vec<RuleLocation>, RulesFailure> {
    let store = open(&db_path)?;
    let storage = |_| RulesFailure::Storage;
    let active: Vec<AccountId> = store
        .list_server_rules()
        .map_err(storage)?
        .into_iter()
        .filter(|s| s.state == ServerRulesState::Active)
        .map(|s| s.account_id)
        .collect();
    // Every account, with the rules its server runs (empty when it has no
    // server rules).
    let mut on_server = Vec::new();
    for account in store.list_accounts().map_err(storage)? {
        let ids = if active.contains(&account.id) {
            rules::server::on_server(&store, account.id).map_err(storage)?
        } else {
            Default::default()
        };
        on_server.push((account.id, active.contains(&account.id), ids));
    }
    let rules = store.list_rules(None).map_err(storage)?;
    Ok(rules
        .into_iter()
        .map(|r| {
            let applies: Vec<_> = on_server
                .iter()
                .filter(|(a, _, _)| r.rule.account_id.is_none_or(|x| x == *a))
                .collect();
            let any_server = applies.iter().any(|(_, active, _)| *active);
            // "On the server" only if it runs there for every account it
            // applies to; for any other account it runs on the device.
            let runs_on = if !r.rule.enabled || !any_server {
                RuleRunsOn::Unmarked
            } else if applies.iter().all(|(_, _, ids)| ids.contains(&r.id)) {
                RuleRunsOn::Server
            } else {
                RuleRunsOn::Device
            };
            RuleLocation {
                rule_id: r.id.0,
                runs_on,
            }
        })
        .collect())
}

/// Accounts whose server runs another filter script.
#[uniffi::export]
pub fn server_rules_notices(db_path: String) -> Result<Vec<ServerRulesNotice>, RulesFailure> {
    let store = open(&db_path)?;
    let storage = |_| RulesFailure::Storage;
    let mut out = Vec::new();
    for s in store.list_server_rules().map_err(storage)? {
        if s.state != ServerRulesState::OtherActive {
            continue;
        }
        let Some(account) = store.get_account(s.account_id).map_err(storage)? else {
            continue;
        };
        out.push(ServerRulesNotice {
            account_id: s.account_id.0,
            account_email: account.details.email,
            script_name: crate::messages::one_line(s.other_script.as_deref().unwrap_or_default()),
            switch_requested: s.switch_requested,
        });
    }
    Ok(out)
}

/// The user's explicit choice to run Mailcrane's rules on the account's
/// server instead of its other filter script. At the next sync, Mailcrane's
/// script is uploaded and made the active one; the other script is kept on
/// the server, only no longer active.
#[uniffi::export]
pub fn use_server_rules(db_path: String, account_id: i64) -> Result<(), RulesFailure> {
    match open(&db_path)?.request_server_rules_switch(AccountId(account_id)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(RulesFailure::Invalid),
        Err(_) => Err(RulesFailure::Storage),
    }
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

    #[test]
    fn locations_notices_and_the_switch() {
        use mailcrane_core::rules::server;
        use mailcrane_core::rules::server::Extensions;
        use mailcrane_store::ServerRules;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let account = |email: &str| NewAccount {
            display_name: "A".into(),
            email: email.into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: email.into(),
        };
        let a = store.insert_account(&account("a@example.org")).expect("a");
        let b = store.insert_account(&account("b@example.org")).expect("b");
        drop(store);
        // No server rules anywhere: nothing to show.
        let flag = save_rule(path.clone(), None, input("Flag invoices")).expect("save");
        let mut attach = input("Attachments");
        attach.account_id = Some(a.0);
        attach.conditions = vec![RuleConditionInfo::HasAttachment];
        let attach = save_rule(path.clone(), None, attach).expect("save");
        assert!(rule_locations(path.clone())
            .expect("locations")
            .iter()
            .all(|l| l.runs_on == RuleRunsOn::Unmarked));
        assert!(server_rules_notices(path.clone())
            .expect("notices")
            .is_empty());
        assert_eq!(
            use_server_rules(path.clone(), a.0),
            Err(RulesFailure::Invalid)
        );

        // Account A runs the flag rule on its server; B has another script.
        let store = Store::open(&path).expect("store");
        let ext = vec!["fileinto".to_string(), "imap4flags".to_string()];
        let plan = server::plan(&store, a, &Extensions::from_list(&ext)).expect("plan");
        let row = |account, state| ServerRules {
            account_id: account,
            state,
            host: None,
            port: None,
            extensions: ext.clone(),
            other_script: Some("Vacation\nreply".into()),
            script_hash: None,
            checked_at: 0,
            switch_requested: false,
            deployed: Vec::new(),
        };
        let mut ra = row(a, ServerRulesState::Active);
        ra.deployed = plan
            .server
            .iter()
            .map(|(id, _, f)| (*id, f.clone()))
            .collect();
        store.save_server_rules(&ra).expect("a");
        store
            .save_server_rules(&row(b, ServerRulesState::OtherActive))
            .expect("b");
        drop(store);

        let locations = rule_locations(path.clone()).expect("locations");
        let of = |id| {
            locations
                .iter()
                .find(|l| l.rule_id == id)
                .map(|l| l.runs_on)
        };
        // The flag rule applies to every account, and B doesn't run it on a
        // server, so it's marked as running on the device.
        assert_eq!(of(flag), Some(RuleRunsOn::Device));
        assert_eq!(of(attach), Some(RuleRunsOn::Device));
        let mut only_a = input("Flag invoices");
        only_a.account_id = Some(a.0);
        save_rule(path.clone(), Some(flag), only_a).expect("save");
        let store = Store::open(&path).expect("store");
        let plan = server::plan(&store, a, &Extensions::from_list(&ext)).expect("plan");
        ra.deployed = plan
            .server
            .iter()
            .map(|(id, _, f)| (*id, f.clone()))
            .collect();
        store.save_server_rules(&ra).expect("a");
        drop(store);
        let locations = rule_locations(path.clone()).expect("locations");
        let of = |id| {
            locations
                .iter()
                .find(|l| l.rule_id == id)
                .map(|l| l.runs_on)
        };
        assert_eq!(of(flag), Some(RuleRunsOn::Server));

        let notices = server_rules_notices(path.clone()).expect("notices");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].account_email, "b@example.org");
        assert!(!notices[0].script_name.contains('\n'));
        assert!(!notices[0].switch_requested);
        use_server_rules(path.clone(), b.0).expect("switch");
        assert!(server_rules_notices(path).expect("notices")[0].switch_requested);
    }
}
