//! Local rules (M4-09) against the testkit's fake IMAP server. (The fake's
//! ENVELOPE always names the same sender, so these use Subject and List-Id;
//! the engine's unit tests cover From and To.)

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::actions::replay;
use mailcrane_core::events::{CoreEvent, EventListener};
use mailcrane_core::rules::{apply_new, run_on_existing, NewRule, RuleAction, RuleCondition};
use mailcrane_core::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret, Session};
use mailcrane_store::{AccountId, FolderId, NewAccount, Security, Store};
use mailcrane_testkit::FakeImap;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const NOW: i64 = 1_795_550_400;

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _event: CoreEvent) {}
}

fn raw(from: &str, subject: &str, id: &str, list: Option<&str>) -> Vec<u8> {
    let list = list
        .map(|l| format!("List-Id: News <{l}>\r\n"))
        .unwrap_or_default();
    format!(
        "From: {from}\r\nTo: alice@example.org\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\n{list}\r\nHi\r\n"
    )
    .into_bytes()
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

struct Setup {
    store: Store,
    account: AccountId,
    inbox: FolderId,
    deals: FolderId,
}

async fn setup(imap: &FakeImap) -> Setup {
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: USER.into(),
        })
        .expect("account");
    let mut session = connect(imap).await;
    let listed = session.list_folders().await.expect("list");
    save_folders(&store, account, &listed).expect("save");
    session.logout().await;
    let id = |name: &str| {
        store
            .list_folders(account)
            .expect("folders")
            .into_iter()
            .find(|f| f.details.remote_name == name)
            .map(|f| f.id)
            .expect("folder")
    };
    let (inbox, deals) = (id("INBOX"), id("Deals"));
    Setup {
        store,
        account,
        inbox,
        deals,
    }
}

/// Syncs the inbox, runs the rules on new mail and sends their actions.
async fn sync_and_run(imap: &FakeImap, s: &Setup) -> mailcrane_core::rules::RulesReport {
    let mut session = connect(imap).await;
    {
        let mut syncer = FolderSyncer::new(
            &s.store,
            &mut session,
            &Quiet,
            s.account,
            SyncConfig::default(),
        );
        syncer.sync_folder(s.inbox).await.expect("sync");
    }
    let report = apply_new(&s.store, s.account, NOW, &Quiet).expect("rules");
    replay(&s.store, &mut session, &Quiet, s.account, NOW + 1)
        .await
        .expect("replay");
    session.logout().await;
    report
}

fn rule(name: &str, conditions: Vec<RuleCondition>, actions: Vec<RuleAction>) -> NewRule {
    NewRule {
        account_id: None,
        name: name.into(),
        enabled: true,
        match_all: true,
        stop_processing: false,
        conditions,
        actions,
    }
}

#[tokio::test]
async fn new_mail_matching_a_rule_is_moved_and_marked_on_the_server() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder(
            "INBOX",
            vec![raw("Old <old@shop.example>", "Before rules", "o1@x", None)],
        )
        .folder("Deals", vec![])
        .start()
        .await
        .expect("imap");
    let s = setup(&imap).await;
    // The first sync happens before any rule exists: that mail is left alone.
    assert_eq!(sync_and_run(&imap, &s).await.checked, 0);
    s.store
        .insert_rule(&rule(
            "Shop deals",
            vec![RuleCondition::ListId("deals.shop.example".into())],
            vec![RuleAction::MarkRead, RuleAction::Move(s.deals)],
        ))
        .expect("rule");
    imap.mailbox()
        .deliver(
            "INBOX",
            raw(
                "Shop <news@shop.example>",
                "Sale!",
                "n1@x",
                Some("deals.shop.example"),
            ),
        )
        .await;
    imap.mailbox()
        .deliver("INBOX", raw("Ana <ana@example.net>", "Lunch", "n2@x", None))
        .await;
    let report = sync_and_run(&imap, &s).await;
    assert_eq!((report.checked, report.matched, report.actions), (2, 1, 2));
    let deals = imap.mailbox().messages("Deals").await;
    assert_eq!(deals.len(), 1);
    assert!(String::from_utf8_lossy(&deals[0].raw).contains("Sale!"));
    assert!(deals[0].flags.contains(&"\\Seen".to_owned()));
    let inbox = imap.mailbox().messages("INBOX").await;
    assert_eq!(inbox.len(), 2, "the old mail and Ana's stay");
}

#[tokio::test]
async fn a_message_is_processed_once_even_after_a_resync() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![])
        .folder("Deals", vec![])
        .start()
        .await
        .expect("imap");
    let s = setup(&imap).await;
    s.store
        .insert_rule(&rule(
            "Read newsletters",
            vec![RuleCondition::Subject("weekly".into())],
            vec![RuleAction::MarkRead],
        ))
        .expect("rule");
    imap.mailbox()
        .deliver(
            "INBOX",
            raw("News <news@shop.example>", "Weekly", "w1@x", None),
        )
        .await;
    assert_eq!(sync_and_run(&imap, &s).await.matched, 1);
    assert!(imap
        .mailbox()
        .flags("INBOX", 1)
        .await
        .expect("flags")
        .contains(&"\\Seen".to_owned()));
    // The user marks it unread on another device, then this device resyncs
    // from scratch (UIDVALIDITY changed): the rule must not run again.
    imap.mailbox().remove_flag("INBOX", 1, "\\Seen").await;
    s.store.reset_folder(s.inbox, 99, 1).expect("reset");
    let again = sync_and_run(&imap, &s).await;
    assert_eq!(again.checked, 0);
    assert!(!imap
        .mailbox()
        .flags("INBOX", 1)
        .await
        .expect("flags")
        .contains(&"\\Seen".to_owned()));
}

#[tokio::test]
async fn stop_processing_and_run_on_existing() {
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder(
            "INBOX",
            vec![
                raw(
                    "Shop <news@shop.example>",
                    "Sale",
                    "e1@x",
                    Some("deals.shop.example"),
                ),
                raw("Ana <ana@example.net>", "Lunch", "e2@x", None),
            ],
        )
        .folder("Deals", vec![])
        .start()
        .await
        .expect("imap");
    let s = setup(&imap).await;
    sync_and_run(&imap, &s).await;
    let mut first = rule(
        "Flag shop",
        vec![RuleCondition::ListId("deals.shop.example".into())],
        vec![RuleAction::Flag],
    );
    first.stop_processing = true;
    let first = s.store.insert_rule(&first).expect("rule");
    let second = s
        .store
        .insert_rule(&rule(
            "Move shop",
            vec![RuleCondition::ListId("deals.shop.example".into())],
            vec![RuleAction::Move(s.deals)],
        ))
        .expect("rule");
    // Running a rule on existing mail only runs that rule.
    let report = run_on_existing(&s.store, first, s.inbox, NOW, &Quiet).expect("run");
    assert_eq!((report.checked, report.matched), (2, 1));
    // New mail runs the list: the first rule stops the second.
    imap.mailbox()
        .deliver(
            "INBOX",
            raw(
                "Shop <news@shop.example>",
                "More",
                "e3@x",
                Some("deals.shop.example"),
            ),
        )
        .await;
    sync_and_run(&imap, &s).await;
    assert!(imap.mailbox().messages("Deals").await.is_empty(), "stopped");
    // Turned off, and reordered: the move runs.
    s.store.set_rule_enabled(first, false).expect("disable");
    s.store.move_rule(second, 0).expect("move");
    let report = run_on_existing(&s.store, second, s.inbox, NOW, &Quiet).expect("run");
    assert_eq!(report.matched, 2);
    let mut session = connect(&imap).await;
    replay(&s.store, &mut session, &Quiet, s.account, NOW + 1)
        .await
        .expect("replay");
    session.logout().await;
    assert_eq!(imap.mailbox().messages("Deals").await.len(), 2);
    let names: Vec<String> = s
        .store
        .list_rules(None)
        .expect("rules")
        .into_iter()
        .map(|r| r.rule.name)
        .collect();
    assert_eq!(names, ["Move shop", "Flag shop"]);
}

/// M4-23: a rule the server runs isn't applied again on the device, while
/// a device-only rule after it still is.
#[tokio::test]
async fn rules_running_on_the_server_are_not_applied_again() {
    use mailcrane_core::rules::server;
    use mailcrane_sieve::script::Extensions;
    use mailcrane_store::{ServerRules, ServerRulesState};

    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![])
        .folder("Deals", vec![])
        .start()
        .await
        .expect("imap");
    let s = setup(&imap).await;
    assert_eq!(sync_and_run(&imap, &s).await.checked, 0);
    let on_server = s
        .store
        .insert_rule(&rule(
            "Shop deals",
            vec![RuleCondition::ListId("deals.shop.example".into())],
            vec![RuleAction::MarkRead, RuleAction::Move(s.deals)],
        ))
        .expect("rule");
    // "Has an attachment" needs the device.
    let mut local = rule(
        "Flag sales",
        vec![
            RuleCondition::HasAttachment,
            RuleCondition::Subject("Sale".into()),
        ],
        vec![RuleAction::Flag],
    );
    local.match_all = false;
    s.store.insert_rule(&local).expect("rule");
    let names = vec!["fileinto".to_string(), "imap4flags".to_string()];
    let plan = server::plan(&s.store, s.account, &Extensions::from_list(&names)).expect("plan");
    assert_eq!(plan.server.len(), 1);
    s.store
        .save_server_rules(&ServerRules {
            account_id: s.account,
            state: ServerRulesState::Active,
            host: Some("localhost".into()),
            port: Some(4190),
            extensions: names,
            other_script: None,
            script_hash: Some("h".into()),
            checked_at: NOW,
            switch_requested: false,
            deployed: plan
                .server
                .iter()
                .map(|(id, _, f)| (*id, f.clone()))
                .collect(),
        })
        .expect("state");
    assert!(server::on_server(&s.store, s.account)
        .expect("ids")
        .contains(&on_server));

    imap.mailbox()
        .deliver(
            "INBOX",
            raw(
                "Shop <news@shop.example>",
                "Sale!",
                "n1@x",
                Some("deals.shop.example"),
            ),
        )
        .await;
    let report = sync_and_run(&imap, &s).await;
    assert_eq!((report.matched, report.actions), (1, 1), "only the flag");
    assert!(
        imap.mailbox().messages("Deals").await.is_empty(),
        "no second move"
    );
    let inbox = imap.mailbox().messages("INBOX").await;
    assert_eq!(inbox.len(), 1);
    assert!(inbox[0].flags.contains(&"\\Flagged".to_owned()));
    assert!(!inbox[0].flags.contains(&"\\Seen".to_owned()));
}
