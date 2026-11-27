use super::*;
use crate::account::autoconfig::dns::build;
use crate::account::autoconfig::NoDns;
use mailcrane_store::{FolderId, NewAccount, NewFolder, Security};
use mailcrane_testkit::{FakeSieve, SieveFault};
use std::future::Future;

const NOW: i64 = 1_795_550_400;

struct Fixture {
    store: Store,
    account: AccountId,
    bills: FolderId,
}

fn folder(account: AccountId, remote: &str, role: Option<FolderRole>) -> NewFolder {
    NewFolder {
        account_id: account,
        remote_name: remote.into(),
        display_name: remote.into(),
        delimiter: Some("/".into()),
        role,
        uidvalidity: None,
        uidnext: None,
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
        virtual_role: None,
    }
}

fn fixture() -> Fixture {
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: "alice@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: "alice@example.org".into(),
        })
        .expect("account");
    store
        .upsert_folder(&folder(account, "INBOX", Some(FolderRole::Inbox)))
        .expect("inbox");
    store
        .upsert_folder(&folder(account, "Trash", Some(FolderRole::Trash)))
        .expect("trash");
    let bills = store
        .upsert_folder(&folder(account, "Bills", None))
        .expect("bills");
    Fixture {
        store,
        account,
        bills,
    }
}

fn rule(conditions: Vec<RuleCondition>, actions: Vec<RuleAction>) -> NewRule {
    NewRule {
        account_id: None,
        name: "r".into(),
        enabled: true,
        match_all: true,
        stop_processing: false,
        conditions,
        actions,
    }
}

const ALL: Extensions = Extensions {
    fileinto: true,
    imap4flags: true,
    unicode_casemap: false,
};

fn login(server: &FakeSieve) -> SieveLogin {
    SieveLogin {
        username: "alice@example.org".into(),
        password: Secret::new("password"),
        imap_host: "localhost".into(),
        email_domain: "example.org".into(),
        extra_root_cert: Some(server.root_cert_der().to_vec()),
        port_override: Some(server.addr().port()),
    }
}

#[test]
fn plan_keeps_what_the_server_cannot_do_on_the_device() {
    let f = fixture();
    let s = &f.store;
    let to_bills = s
        .insert_rule(&rule(
            vec![RuleCondition::Subject("Invoice".into())],
            vec![RuleAction::MarkRead, RuleAction::Move(f.bills)],
        ))
        .expect("rule");
    let attachment = s
        .insert_rule(&rule(
            vec![RuleCondition::HasAttachment],
            vec![RuleAction::Flag],
        ))
        .expect("rule");
    let mut disabled = rule(
        vec![RuleCondition::Subject("x".into())],
        vec![RuleAction::Flag],
    );
    disabled.enabled = false;
    s.insert_rule(&disabled).expect("rule");
    let delete = s
        .insert_rule(&rule(
            vec![RuleCondition::From("spam@example.net".into())],
            vec![RuleAction::Delete],
        ))
        .expect("rule");
    let p = plan(s, f.account, &ALL).expect("plan");
    assert_eq!(
        p.server.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        [to_bills, delete]
    );
    assert_eq!(p.device, [attachment]);
    assert_eq!(p.server[0].1.moving, Some(Moving::FileInto("Bills".into())));
    // Delete goes to Trash, as it does locally.
    assert_eq!(p.server[1].1.moving, Some(Moving::FileInto("Trash".into())));

    // Without imap4flags the mark-read rule stays local, and since it moves
    // mail, so does every rule after it.
    let no_flags = Extensions {
        imap4flags: false,
        ..ALL
    };
    let p = plan(s, f.account, &no_flags).expect("plan");
    assert!(p.server.is_empty());
    assert_eq!(p.device, [to_bills, attachment, delete]);
}

#[test]
fn a_move_to_an_unusable_folder_stays_local() {
    let f = fixture();
    let mut parent = folder(f.account, "[Parent]", None);
    parent.selectable = false;
    let parent = f.store.upsert_folder(&parent).expect("folder");
    let id = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("x".into())],
            vec![RuleAction::Move(parent)],
        ))
        .expect("rule");
    let archive = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("y".into())],
            vec![RuleAction::Archive],
        ))
        .expect("rule");
    let p = plan(&f.store, f.account, &ALL).expect("plan");
    assert_eq!(
        p.device,
        [id, archive],
        "a \\Noselect folder, and no Archive folder"
    );
}

#[test]
fn a_move_into_a_virtual_folder_is_dropped_as_on_the_device() {
    // #390 review: the device skips a stored rule's move into a virtual
    // folder such as Starred (the move doesn't take the "first move wins"
    // slot), so the server script does the same, and such a rule doesn't
    // keep every later rule off the server.
    let f = fixture();
    let mut starred = folder(f.account, "Starred", None);
    starred.virtual_role = Some(mailcrane_store::VirtualRole::Starred);
    let starred = f.store.upsert_folder(&starred).expect("folder");
    let legacy = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("sale".into())],
            vec![
                RuleAction::MarkRead,
                RuleAction::Move(starred),
                RuleAction::Move(f.bills),
            ],
        ))
        .expect("rule");
    // Nothing left to do anywhere: a server rule that the script leaves out.
    let only_virtual = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("x".into())],
            vec![RuleAction::Move(starred)],
        ))
        .expect("rule");
    let later = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("Invoice".into())],
            vec![RuleAction::Move(f.bills)],
        ))
        .expect("rule");
    let p = plan(&f.store, f.account, &ALL).expect("plan");
    assert_eq!(
        p.server.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        [legacy, only_virtual, later]
    );
    assert!(p.device.is_empty());
    assert_eq!(p.server[0].1.flags, [Flag::Seen]);
    assert_eq!(p.server[0].1.moving, Some(Moving::FileInto("Bills".into())));
    let script = p.script(&ALL).expect("script");
    assert!(
        !script.contains("Starred"),
        "no fileinto for a virtual folder"
    );
}

#[test]
fn folder_names_are_decoded_and_escaped() {
    let f = fixture();
    let ete = mailcrane_imap::mutf7::encode("Été/\"Quoted\" \\ box");
    let folder_id = f
        .store
        .upsert_folder(&folder(f.account, &ete, None))
        .expect("folder");
    f.store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("Ünïcode \"quoted\"".into())],
            vec![RuleAction::Move(folder_id)],
        ))
        .expect("rule");
    let script = plan(&f.store, f.account, &ALL)
        .expect("plan")
        .script(&ALL)
        .expect("script");
    assert!(
        script.contains(r#"fileinto "Été/\"Quoted\" \\ box";"#),
        "{script}"
    );
    assert!(script.contains(r#""Ünïcode \"quoted\"""#), "{script}");
}

#[test]
fn srv_targets_must_belong_to_the_provider() {
    let l = SieveLogin {
        username: "u".into(),
        password: Secret::new("p"),
        imap_host: "imap.mail.example.org".into(),
        email_domain: "example.com".into(),
        extra_root_cert: None,
        port_override: None,
    };
    assert!(trusted_target("imap.mail.example.org.", &l));
    assert!(trusted_target("sieve.example.com", &l));
    assert!(trusted_target("sieve.mail.example.org", &l));
    assert!(!trusted_target("evil.example.net", &l));
    assert!(!trusted_target("example.org.evil.net", &l));
    assert!(!trusted_target("notexample.com", &l));
}

struct OneSrv(Vec<u8>);

impl DnsResolver for OneSrv {
    fn query(&self, name: &str, rtype: RecordType) -> impl Future<Output = Option<Vec<u8>>> + Send {
        let answer =
            (name == "_sieve._tcp.example.com" && rtype == RecordType::Srv).then(|| self.0.clone());
        std::future::ready(answer)
    }
}

#[tokio::test]
async fn endpoints_try_srv_then_the_imap_host() {
    let l = SieveLogin {
        username: "u".into(),
        password: Secret::new("p"),
        imap_host: "imap.example.com".into(),
        email_domain: "example.com".into(),
        extra_root_cert: None,
        port_override: None,
    };
    assert_eq!(
        endpoints(&NoDns, &l).await,
        [("imap.example.com".to_string(), 4190)]
    );
    let good = OneSrv(build::response(
        "_sieve._tcp.example.com",
        RecordType::Srv,
        0,
        &[build::srv(0, 0, 2000, "sieve.example.com")],
    ));
    assert_eq!(
        endpoints(&good, &l).await,
        [
            ("sieve.example.com".to_string(), 2000),
            ("imap.example.com".to_string(), 4190)
        ]
    );
    let evil = OneSrv(build::response(
        "_sieve._tcp.example.com",
        RecordType::Srv,
        0,
        &[build::srv(0, 0, 4190, "collector.example.net")],
    ));
    assert_eq!(
        endpoints(&evil, &l).await,
        [("imap.example.com".to_string(), 4190)],
        "an untrusted target never gets the password"
    );
}

#[tokio::test]
async fn uploads_activates_and_then_waits() {
    let f = fixture();
    let id = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("Invoice".into())],
            vec![RuleAction::Move(f.bills)],
        ))
        .expect("rule");
    let server = FakeSieve::builder().start().await.expect("server");
    let out = sync(&f.store, f.account, &login(&server), &NoDns, NOW)
        .await
        .expect("sync");
    assert_eq!(out, SyncOutcome::Uploaded);
    assert_eq!(server.active().as_deref(), Some(SCRIPT_NAME));
    let script = server
        .scripts()
        .get(SCRIPT_NAME)
        .cloned()
        .unwrap_or_default();
    assert!(script.contains("fileinto \"Bills\";"), "{script}");
    let commands = server.commands();
    let check = commands.iter().position(|c| c == "CHECKSCRIPT+tls");
    let put = commands.iter().position(|c| c == "PUTSCRIPT+tls");
    assert!(
        check.is_some() && check < put,
        "CHECKSCRIPT before PUTSCRIPT: {commands:?}"
    );
    assert_eq!(
        on_server(&f.store, f.account).expect("ids"),
        HashSet::from([id])
    );

    // Soon after: no connection at all.
    let logins = server.logins();
    let out = sync(&f.store, f.account, &login(&server), &NoDns, NOW + 60)
        .await
        .expect("sync");
    assert_eq!(out, SyncOutcome::Skipped);
    assert_eq!(server.logins(), logins);

    // A day later: checked, already up to date, nothing written.
    let puts = |s: &FakeSieve| {
        s.commands()
            .iter()
            .filter(|c| c.starts_with("PUTSCRIPT"))
            .count()
    };
    let before = puts(&server);
    let out = sync(
        &f.store,
        f.account,
        &login(&server),
        &NoDns,
        NOW + RECHECK_SECS,
    )
    .await
    .expect("sync");
    assert_eq!(out, SyncOutcome::UpToDate);
    assert_eq!(puts(&server), before);

    // An edited rule runs on the device until the new script is up.
    let mut edited = rule(
        vec![RuleCondition::Subject("Receipt".into())],
        vec![RuleAction::Move(f.bills)],
    );
    edited.name = "edited".into();
    f.store.update_rule(id, &edited).expect("edit");
    assert!(on_server(&f.store, f.account).expect("ids").is_empty());
    let out = sync(
        &f.store,
        f.account,
        &login(&server),
        &NoDns,
        NOW + RECHECK_SECS + RETRY_SECS,
    )
    .await
    .expect("sync");
    assert_eq!(out, SyncOutcome::Uploaded);
    assert!(server.scripts()[SCRIPT_NAME].contains("Receipt"));
    assert_eq!(
        on_server(&f.store, f.account).expect("ids"),
        HashSet::from([id])
    );

    // A rename only changes the rule, not the script: still on the server.
    edited.name = "renamed".into();
    f.store.update_rule(id, &edited).expect("rename");
    assert_eq!(
        on_server(&f.store, f.account).expect("ids"),
        HashSet::from([id])
    );
}

#[tokio::test]
async fn another_active_script_is_never_touched_until_the_user_switches() {
    let f = fixture();
    let id = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("Invoice".into())],
            vec![RuleAction::Flag],
        ))
        .expect("rule");
    let server = FakeSieve::builder()
        .script("Vacation \u{202e}reply", "keep;", true)
        .start()
        .await
        .expect("server");
    let out = sync(&f.store, f.account, &login(&server), &NoDns, NOW)
        .await
        .expect("sync");
    assert_eq!(out, SyncOutcome::OtherActive);
    assert!(server
        .commands()
        .iter()
        .all(|c| !c.starts_with("PUTSCRIPT") && !c.starts_with("SETACTIVE")));
    let state = f
        .store
        .server_rules(f.account)
        .expect("state")
        .expect("row");
    assert_eq!(state.state, ServerRulesState::OtherActive);
    // Shown as one line of plain text: no controls or direction overrides.
    assert_eq!(state.other_script.as_deref(), Some("Vacation reply"));
    assert!(on_server(&f.store, f.account).expect("ids").is_empty());

    // Not asked: a later check still changes nothing.
    let out = sync(
        &f.store,
        f.account,
        &login(&server),
        &NoDns,
        NOW + RECHECK_SECS,
    )
    .await
    .expect("sync");
    assert_eq!(out, SyncOutcome::OtherActive);
    assert_eq!(server.scripts().len(), 1);

    // The user taps "switch": checked at once, uploaded and activated; the
    // other script is kept, only no longer active.
    assert!(f
        .store
        .request_server_rules_switch(f.account)
        .expect("switch"));
    let out = sync(
        &f.store,
        f.account,
        &login(&server),
        &NoDns,
        NOW + RECHECK_SECS + 1,
    )
    .await
    .expect("sync");
    assert_eq!(out, SyncOutcome::Uploaded);
    assert_eq!(server.active().as_deref(), Some(SCRIPT_NAME));
    assert_eq!(server.scripts()["Vacation \u{202e}reply"], "keep;");
    let state = f
        .store
        .server_rules(f.account)
        .expect("state")
        .expect("row");
    assert!(!state.switch_requested);
    assert_eq!(
        on_server(&f.store, f.account).expect("ids"),
        HashSet::from([id])
    );

    // Someone activates the other script again elsewhere: back to local.
    server.set_active(Some("Vacation \u{202e}reply"));
    let out = sync(
        &f.store,
        f.account,
        &login(&server),
        &NoDns,
        NOW + 3 * RECHECK_SECS,
    )
    .await
    .expect("sync");
    assert_eq!(out, SyncOutcome::OtherActive);
    assert!(on_server(&f.store, f.account).expect("ids").is_empty());
}

#[tokio::test]
async fn no_server_is_remembered() {
    let f = fixture();
    f.store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("x".into())],
            vec![RuleAction::Flag],
        ))
        .expect("rule");
    // A port nothing listens on any more.
    let server = FakeSieve::builder().start().await.expect("server");
    let mut l = login(&server);
    server.shutdown();
    drop(server);
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    l.port_override = Some(closed.local_addr().expect("addr").port());
    drop(closed);
    let out = sync(&f.store, f.account, &l, &NoDns, NOW)
        .await
        .expect("sync");
    assert!(matches!(out, SyncOutcome::Unavailable(_)), "{out:?}");
    let out = sync(&f.store, f.account, &l, &NoDns, NOW + RECHECK_SECS)
        .await
        .expect("sync");
    assert_eq!(out, SyncOutcome::Skipped);
    let out = sync(
        &f.store,
        f.account,
        &l,
        &NoDns,
        NOW + RECHECK_UNAVAILABLE_SECS,
    )
    .await
    .expect("sync");
    assert!(matches!(out, SyncOutcome::Unavailable(_)));
}

#[tokio::test]
async fn a_wrong_password_is_not_retried_every_sync() {
    let f = fixture();
    let server = FakeSieve::builder()
        .user("alice@example.org", "other")
        .start()
        .await
        .expect("server");
    let out = sync(&f.store, f.account, &login(&server), &NoDns, NOW)
        .await
        .expect("sync");
    assert_eq!(out, SyncOutcome::Unavailable(SieveError::AuthFailed));
    let attempts = server
        .commands()
        .iter()
        .filter(|c| c.starts_with("AUTHENTICATE"))
        .count();
    sync(&f.store, f.account, &login(&server), &NoDns, NOW + 3600)
        .await
        .expect("sync");
    let again = server
        .commands()
        .iter()
        .filter(|c| c.starts_with("AUTHENTICATE"))
        .count();
    assert_eq!((attempts, again), (1, 1));
}

#[tokio::test]
async fn a_refused_upload_keeps_the_old_script_and_local_rules() {
    let f = fixture();
    let id = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("Invoice".into())],
            vec![RuleAction::Move(f.bills)],
        ))
        .expect("rule");
    let good = FakeSieve::builder().start().await.expect("server");
    sync(&f.store, f.account, &login(&good), &NoDns, NOW)
        .await
        .expect("sync");
    let before = f.store.server_rules(f.account).expect("state");

    // Same account, but now the server refuses scripts (say the new rule
    // trips a server limit).
    let refusing = FakeSieve::builder()
        .script(SCRIPT_NAME, "old", true)
        .fault(SieveFault::RejectScripts)
        .start()
        .await
        .expect("server");
    let new_rule = f
        .store
        .insert_rule(&rule(
            vec![RuleCondition::Subject("New".into())],
            vec![RuleAction::Flag],
        ))
        .expect("rule");
    let out = sync(
        &f.store,
        f.account,
        &login(&refusing),
        &NoDns,
        NOW + RETRY_SECS,
    )
    .await
    .expect("sync");
    assert_eq!(out, SyncOutcome::Failed(SieveError::ScriptRejected));
    assert_eq!(refusing.scripts()[SCRIPT_NAME], "old");
    let after = f.store.server_rules(f.account).expect("state");
    assert_eq!(
        after.as_ref().map(|s| (&s.deployed, &s.script_hash)),
        before.as_ref().map(|s| (&s.deployed, &s.script_hash)),
    );
    // The old rule still runs on the server; the new one on the device.
    let ids = on_server(&f.store, f.account).expect("ids");
    assert!(ids.contains(&id) && !ids.contains(&new_rule));
}

#[tokio::test]
async fn no_rules_means_no_script() {
    let f = fixture();
    let server = FakeSieve::builder().start().await.expect("server");
    let out = sync(&f.store, f.account, &login(&server), &NoDns, NOW)
        .await
        .expect("sync");
    assert_eq!(out, SyncOutcome::UpToDate);
    assert!(server.scripts().is_empty() && server.active().is_none());
}
