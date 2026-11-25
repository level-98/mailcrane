//! A rule that moves mail to a folder is turned off with a note when the
//! folder is removed, rather than silently losing its move (#323).

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn setup(store: &Store) -> Result<(AccountId, FolderId, RuleId), StoreError> {
    let account = store.insert_account(&NewAccount {
        display_name: "A".into(),
        email: "a@example.org".into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 465,
        smtp_security: Security::Tls,
        username: "a".into(),
    })?;
    let folder = store.upsert_folder(&NewFolder {
        account_id: account,
        remote_name: "Receipts".into(),
        display_name: "Receipts".into(),
        delimiter: Some("/".into()),
        role: None,
        uidvalidity: Some(1),
        uidnext: Some(1),
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
    })?;
    let rule = store.insert_rule(&rule(account, folder))?;
    Ok((account, folder, rule))
}

fn rule(account: AccountId, folder: FolderId) -> NewRule {
    NewRule {
        account_id: Some(account),
        name: "Receipts".into(),
        enabled: true,
        match_all: true,
        stop_processing: false,
        conditions: vec![RuleCondition::Subject("receipt".into())],
        actions: vec![RuleAction::Move(folder)],
    }
}

#[test]
fn deleting_a_folder_turns_its_rules_off_with_a_note() -> R {
    let store = Store::open_in_memory()?;
    let (account, folder, id) = setup(&store)?;
    // An unrelated rule stays as it is.
    let other = store.insert_rule(&NewRule {
        actions: vec![RuleAction::Flag],
        ..rule(account, folder)
    })?;
    assert_eq!(store.get_rule(id)?.ok_or("rule")?.missing_folder, None);

    assert!(store.delete_folder(folder)?);
    let r = store.get_rule(id)?.ok_or("rule")?;
    assert!(!r.rule.enabled);
    assert_eq!(r.missing_folder.as_deref(), Some("Receipts"));
    assert!(r.rule.actions.is_empty());
    let o = store.get_rule(other)?.ok_or("rule")?;
    assert!(o.rule.enabled && o.missing_folder.is_none());
    Ok(())
}

#[test]
fn editing_or_enabling_the_rule_clears_the_note() -> R {
    let store = Store::open_in_memory()?;
    let (account, folder, id) = setup(&store)?;
    store.delete_folder(folder)?;
    // Turning it off again keeps the note.
    store.set_rule_enabled(id, false)?;
    assert!(store.get_rule(id)?.ok_or("rule")?.missing_folder.is_some());
    // Saving it from the editor clears it.
    let mut edited = rule(account, folder);
    edited.actions = vec![RuleAction::Flag];
    edited.enabled = false;
    assert!(store.update_rule(id, &edited)?);
    assert_eq!(store.get_rule(id)?.ok_or("rule")?.missing_folder, None);
    Ok(())
}

#[test]
fn a_rule_left_without_actions_cant_be_turned_on() -> R {
    let store = Store::open_in_memory()?;
    let (_, folder, id) = setup(&store)?;
    store.delete_folder(folder)?;
    // Its only action went with the folder: turning it on is refused.
    assert!(matches!(
        store.set_rule_enabled(id, true),
        Err(StoreError::Invalid { field: "actions" })
    ));
    let r = store.get_rule(id)?.ok_or("rule")?;
    assert!(!r.rule.enabled);
    assert_eq!(r.missing_folder.as_deref(), Some("Receipts"));
    // Turning it off, or anything for a rule that doesn't exist, is fine.
    store.set_rule_enabled(id, false)?;
    store.set_rule_enabled(RuleId(999), true)?;
    Ok(())
}
