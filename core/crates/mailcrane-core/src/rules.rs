//! Local rules (M4-09).
//!
//! Rules run, in list order, on mail newly arriving in an account's inbox
//! (after each sync), and on a folder's existing mail when asked
//! ([`run_on_existing`]). Text conditions are "contains", ignoring case and
//! accents; there are no regular expressions. Actions go through the normal
//! message actions (M1-14), so they show at once and reach the server with
//! the action queue. Every message is looked at once: a resync that stores
//! it again (same Message-ID) doesn't run the rules again.
//!
//! When the account's server runs some rules itself ([`server`], M4-23),
//! the device doesn't apply those again to new mail.

mod engine;
pub mod server;

pub use engine::{evaluate, MessageFacts};
pub use mailcrane_store::{NewRule, Rule, RuleAction, RuleCondition, RuleId};

use crate::actions::{apply_action, Action};
use crate::events::EventListener;
use mailcrane_store::{AccountId, FolderId, MessageHeader, Store, StoreError};
use std::collections::HashSet;

/// What running rules did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RulesReport {
    /// Messages looked at.
    pub checked: usize,
    /// Messages at least one rule matched.
    pub matched: usize,
    /// Actions applied.
    pub actions: usize,
}

/// Runs the enabled rules on new inbox mail of `account`, once per message.
pub fn apply_new(
    store: &Store,
    account: AccountId,
    now: i64,
    listener: &dyn EventListener,
) -> Result<RulesReport, StoreError> {
    let pending = store.take_rules_pending(account)?;
    let mut report = RulesReport::default();
    if pending.is_empty() {
        return Ok(report);
    }
    let rules: Vec<Rule> = store
        .list_rules(Some(account))?
        .into_iter()
        .filter(|r| r.rule.enabled)
        .collect();
    // Rules the server already ran on this mail (no double moves).
    let on_server = server::on_server(store, account)?;
    for (header, list_id) in pending {
        let mid = header.details.message_id_header.clone();
        let already = match mid.as_deref() {
            Some(m) => store.rules_seen(account, m)?,
            None => false,
        };
        if !already && !rules.is_empty() {
            report.checked += 1;
            let n = run_rules(
                store,
                &rules,
                &on_server,
                &header,
                list_id.as_deref(),
                now,
                true,
                listener,
            )?;
            if n > 0 {
                report.matched += 1;
                report.actions += n;
            }
        }
        store.mark_rules_done(account, header.id, mid.as_deref())?;
    }
    Ok(report)
}

/// Runs one rule (enabled or not) on the messages now in `folder`.
pub fn run_on_existing(
    store: &Store,
    rule: RuleId,
    folder: FolderId,
    now: i64,
    listener: &dyn EventListener,
) -> Result<RulesReport, StoreError> {
    let mut report = RulesReport::default();
    let Some(rule) = store.get_rule(rule)? else {
        return Ok(report);
    };
    let Some(f) = store.get_folder(folder)? else {
        return Ok(report);
    };
    if rule
        .rule
        .account_id
        .is_some_and(|a| a != f.details.account_id)
    {
        return Ok(report);
    }
    let rules = [rule];
    for (header, list_id) in store.folder_messages_with_list_id(folder)? {
        report.checked += 1;
        let n = run_rules(
            store,
            &rules,
            &HashSet::new(),
            &header,
            list_id.as_deref(),
            now,
            false,
            listener,
        )?;
        if n > 0 {
            report.matched += 1;
            report.actions += n;
        }
    }
    Ok(report)
}

/// Runs `rules` in order on one message. Read and flag actions apply at
/// once; of the moving actions (move, archive, delete) the first one wins
/// and runs last. Returns how many actions were applied.
///
/// Rules in `on_server` are matched but not applied: the server did that.
/// They still count for "stop processing" and "first move wins", as they
/// did in the server's script.
#[allow(clippy::too_many_arguments)]
fn run_rules(
    store: &Store,
    rules: &[Rule],
    on_server: &HashSet<RuleId>,
    header: &MessageHeader,
    list_id: Option<&str>,
    now: i64,
    new_mail: bool,
    listener: &dyn EventListener,
) -> Result<usize, StoreError> {
    let facts = MessageFacts::new(header, list_id, now, new_mail);
    let mut flags: Vec<Action> = Vec::new();
    let mut moving: Option<Action> = None;
    // A moving action already taken, by the server or a rule here.
    let mut moved = false;
    for rule in rules {
        if !evaluate(&rule.rule, &facts) {
            continue;
        }
        if on_server.contains(&rule.id) {
            moved |= rule.rule.actions.iter().any(|a| {
                matches!(
                    a,
                    RuleAction::Move(_) | RuleAction::Archive | RuleAction::Delete
                )
            });
            if rule.rule.stop_processing {
                break;
            }
            continue;
        }
        for action in &rule.rule.actions {
            match action {
                RuleAction::MarkRead => flags.push(Action::MarkRead),
                RuleAction::Flag => flags.push(Action::Flag),
                _ if moved => {}
                RuleAction::Move(to) => {
                    moving.get_or_insert(Action::Move { to: *to });
                }
                RuleAction::Archive => {
                    moving.get_or_insert(Action::Archive);
                }
                RuleAction::Delete => {
                    moving.get_or_insert(Action::Delete);
                }
            }
        }
        if rule.rule.stop_processing {
            break;
        }
    }
    let mut applied = 0;
    for action in flags.into_iter().chain(moving) {
        match apply_action(store, &[header.id], action, now, listener) {
            Ok(Some(a)) if a.queued => applied += 1,
            Ok(_) => {}
            Err(crate::actions::ActionError::Store(e)) => return Err(e),
            // No Archive folder, a folder that's gone: skip that action.
            Err(_) => tracing::warn!(?action, "a rule's action could not be applied"),
        }
    }
    Ok(applied)
}
