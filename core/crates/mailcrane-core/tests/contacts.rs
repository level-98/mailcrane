//! Recipient suggestions (M2-08): counting and ranking.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::contacts::{fold, record_sent, suggest};
use mailcrane_store::{
    AccountId, DraftFields, Flags, FolderId, FolderRole, NewAccount, NewFolder, NewMessageHeader,
    NewOutboxMessage, Security, Store,
};

const NOW: i64 = 1_795_550_400;
const DAY: i64 = 86_400;

fn account(store: &Store, email: &str) -> AccountId {
    store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: email.into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: email.into(),
        })
        .expect("account")
}

fn folder(store: &Store, account: AccountId, name: &str, role: Option<FolderRole>) -> FolderId {
    store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: name.into(),
            display_name: name.into(),
            delimiter: Some("/".into()),
            role,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
        })
        .expect("folder")
}

fn receive(store: &Store, folder: FolderId, uid: u32, name: &str, addr: &str, date: i64) {
    store
        .upsert_message_header(&NewMessageHeader {
            folder_id: folder,
            uid,
            message_id_header: None,
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: "Hi".into(),
            from_name: Some(name.into()),
            from_addr: Some(addr.into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date,
            internal_date: date,
            size: 10,
            flags: Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
        })
        .expect("message");
}

fn emails(store: &Store, account: Option<AccountId>, prefix: &str) -> Vec<String> {
    suggest(store, account, prefix, 8, NOW)
        .expect("suggest")
        .into_iter()
        .map(|s| s.email)
        .collect()
}

#[test]
fn received_mail_is_counted_once_and_follows_the_stored_messages() {
    let store = Store::open_in_memory().expect("store");
    let a = account(&store, "alice@example.org");
    let inbox = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    let sent = folder(&store, a, "Sent", Some(FolderRole::Sent));
    let spam = folder(&store, a, "Spam", Some(FolderRole::Spam));
    receive(&store, inbox, 1, "Ana Silva", "Ana@Example.net", NOW - DAY);
    // A resync of the same message updates the row: not counted again.
    receive(&store, inbox, 1, "Ana Silva", "ana@example.net", NOW - DAY);
    receive(&store, inbox, 2, "Ana Silva", "ana@example.net", NOW);
    // Sent and spam don't count as hearing from someone.
    receive(&store, sent, 1, "Me", "alice@example.org", NOW);
    receive(&store, spam, 1, "Spammer", "spam@example.com", NOW);
    let list = store.list_correspondents(Some(a)).expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].email, "ana@example.net");
    assert_eq!(list[0].received_from_count, 2);
    assert_eq!(list[0].last_seen_at, NOW);

    // The folder is reset (UIDVALIDITY changed): its rows go, then come back.
    store.reset_folder(inbox, 7, 10).expect("reset");
    assert!(store.list_correspondents(Some(a)).expect("list").is_empty());
    receive(&store, inbox, 1, "Ana Silva", "ana@example.net", NOW);
    assert_eq!(
        store.list_correspondents(Some(a)).expect("list")[0].received_from_count,
        1
    );
}

#[test]
fn no_reply_addresses_are_not_suggested_from_received_mail() {
    let store = Store::open_in_memory().expect("store");
    let a = account(&store, "alice@example.org");
    let inbox = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    for (uid, addr) in [
        "noreply@shop.example",
        "no-reply@shop.example",
        "no_reply@shop.example",
        "DoNotReply@bank.example",
        "do-not-reply@bank.example",
        "MAILER-DAEMON@mx.example",
    ]
    .iter()
    .enumerate()
    {
        receive(&store, inbox, uid as u32 + 1, "Robot", addr, NOW);
    }
    receive(&store, inbox, 99, "Nora Reply", "nora@example.net", NOW);
    assert_eq!(
        emails(&store, Some(a), "no"),
        ["nora@example.net"],
        "only the person"
    );
    assert!(emails(&store, Some(a), "do").is_empty());
    assert!(emails(&store, Some(a), "mailer").is_empty());
    assert_eq!(emails(&store, Some(a), "nora"), ["nora@example.net"]);
}

#[test]
fn sending_counts_every_recipient() {
    let store = Store::open_in_memory().expect("store");
    let a = account(&store, "alice@example.org");
    let id = store
        .insert_outbox(
            &NewOutboxMessage {
                account_id: a,
                raw: b"Subject: x\r\n\r\nx".to_vec(),
                envelope_from: "alice@example.org".into(),
                envelope_to: vec![],
                message_id_header: "m@example.org".into(),
                fields: DraftFields {
                    to: vec!["Bob <Bob@Example.net>, carol@example.net".into()],
                    cc: vec!["Dee <dee@example.net>".into()],
                    bcc: vec!["bob@example.net".into()],
                    subject: "x".into(),
                    body: "x".into(),
                    in_reply_to: None,
                    references: vec![],
                },
                not_before: 0,
                scheduled: false,
            },
            NOW,
        )
        .expect("outbox");
    let message = store.get_outbox(id).expect("get").expect("message");
    record_sent(&store, &message, NOW).expect("record");
    record_sent(&store, &message, NOW).expect("record");
    let mut list = store.list_correspondents(Some(a)).expect("list");
    list.sort_by(|x, y| x.email.cmp(&y.email));
    let got: Vec<(&str, &str, u32)> = list
        .iter()
        .map(|c| (c.email.as_str(), c.name.as_str(), c.sent_to_count))
        .collect();
    assert_eq!(
        got,
        [
            ("bob@example.net", "Bob", 2),
            ("carol@example.net", "", 2),
            ("dee@example.net", "Dee", 2),
        ]
    );
}

#[test]
fn ranking_and_matching() {
    let store = Store::open_in_memory().expect("store");
    let a = account(&store, "alice@example.org");
    let inbox = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    // Frequent beats rare.
    for uid in 1..=5 {
        receive(
            &store,
            inbox,
            uid,
            "Ana Silva",
            "ana.silva@example.net",
            NOW,
        );
    }
    receive(&store, inbox, 10, "Ana Rare", "ana.rare@example.net", NOW);
    assert_eq!(
        emails(&store, Some(a), "ana"),
        ["ana.silva@example.net", "ana.rare@example.net"]
    );
    // Recent beats old: the same counts, a year apart.
    receive(
        &store,
        inbox,
        20,
        "Old Friend",
        "pat.old@example.net",
        NOW - 365 * DAY,
    );
    receive(&store, inbox, 21, "New Friend", "pat.new@example.net", NOW);
    assert_eq!(
        emails(&store, Some(a), "pat"),
        ["pat.new@example.net", "pat.old@example.net"]
    );
    // Writing to someone counts three times as much as hearing from them.
    store
        .record_sent_to(a, &[("sam@example.net".into(), "Sam".into())], NOW)
        .expect("sent");
    for uid in 30..32 {
        receive(&store, inbox, uid, "Sammy", "sammy@example.net", NOW);
    }
    assert_eq!(
        emails(&store, Some(a), "sam"),
        ["sam@example.net", "sammy@example.net"]
    );

    // Start of the name, any word of it, or the start of the address.
    receive(
        &store,
        inbox,
        40,
        "Silva, Ana",
        "asilva@example.net",
        NOW - 400 * DAY,
    );
    let ana = emails(&store, Some(a), "ana");
    assert!(ana.contains(&"asilva@example.net".to_owned()), "{ana:?}");
    assert_eq!(emails(&store, Some(a), "asil"), ["asilva@example.net"]);
    assert!(
        emails(&store, Some(a), "ilva").is_empty(),
        "not the middle of a word"
    );
    // Accents and case.
    receive(
        &store,
        inbox,
        50,
        "Élodie Durand",
        "e.durand@example.fr",
        NOW,
    );
    assert_eq!(emails(&store, Some(a), "elodie"), ["e.durand@example.fr"]);
    assert_eq!(emails(&store, Some(a), "ÉLO"), ["e.durand@example.fr"]);
    assert_eq!(emails(&store, Some(a), "durand"), ["e.durand@example.fr"]);
    assert!(emails(&store, Some(a), "  ").is_empty());
    assert_eq!(fold("Łukasz Straße"), "lukasz strasse");
}

#[test]
fn accounts_are_merged_when_asking_for_all() {
    let store = Store::open_in_memory().expect("store");
    let a = account(&store, "alice@example.org");
    let b = account(&store, "alice@work.example");
    let ia = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    let ib = folder(&store, b, "INBOX", Some(FolderRole::Inbox));
    receive(&store, ia, 1, "Bob", "bob@example.net", NOW);
    receive(&store, ib, 1, "Bob B.", "bob@example.net", NOW - DAY);
    let all = suggest(&store, None, "bob", 8, NOW).expect("suggest");
    assert_eq!(all.len(), 1, "one entry per address");
    assert_eq!(all[0].name, "Bob", "the newest name");
    assert_eq!(emails(&store, Some(b), "bob"), ["bob@example.net"]);
}

/// `cargo test -p mailcrane-core --release -- --ignored ten_thousand`.
/// Ignored by default: a timing test.
#[test]
#[ignore = "timing test; run in release mode"]
fn ten_thousand_correspondents_under_20_ms() {
    let store = Store::open_in_memory().expect("store");
    let a = account(&store, "alice@example.org");
    let inbox = folder(&store, a, "INBOX", Some(FolderRole::Inbox));
    for i in 0..10_000u32 {
        receive(
            &store,
            inbox,
            i + 1,
            &format!("Person {i} Example"),
            &format!("person{i}@example.net"),
            NOW - i64::from(i) * 60,
        );
    }
    let _ = suggest(&store, Some(a), "person", 8, NOW).expect("warm up");
    let started = std::time::Instant::now();
    let found = suggest(&store, Some(a), "person 99", 8, NOW).expect("suggest");
    let took = started.elapsed();
    let all = suggest(&store, Some(a), "pers", 8, NOW).expect("suggest");
    let took_all = started.elapsed() - took;
    assert!(found.is_empty() || found.len() <= 8);
    assert_eq!(all.len(), 8);
    assert!(took.as_millis() < 20, "{took:?}");
    assert!(took_all.as_millis() < 20, "{took_all:?}");
}
