use super::*;
use crate::search::parse_query;
use mailcrane_store::{Flags, NewAccount, NewFolder, NewMessageHeader, Security};

struct Fixture {
    store: Store,
    account: AccountId,
    inbox: FolderId,
}

fn fixture() -> Fixture {
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Me".into(),
            email: "me@example.org".into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: "me".into(),
        })
        .expect("account");
    let folder = |name: &str| NewFolder {
        account_id: account,
        remote_name: name.into(),
        display_name: name.into(),
        delimiter: Some("/".into()),
        role: None,
        uidvalidity: Some(1),
        uidnext: Some(1),
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
        virtual_role: None,
    };
    let inbox = store.upsert_folder(&folder("INBOX")).expect("inbox");
    store.upsert_folder(&folder("Work")).expect("work");
    Fixture {
        store,
        account,
        inbox,
    }
}

fn add(f: &Fixture, folder: FolderId, uid: u32, subject: &str, from: &str, date: i64) -> MessageId {
    f.store
        .upsert_message_header(&NewMessageHeader {
            folder_id: folder,
            uid,
            message_id_header: None,
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: subject.into(),
            from_name: Some(from.split('@').next().unwrap_or("").into()),
            from_addr: Some(from.into()),
            to_json: r#"[{"name":"Bob","email":"bob@example.com"}]"#.into(),
            cc_json: "[]".into(),
            date,
            internal_date: date,
            size: 10,
            flags: Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
        })
        .expect("message")
}

use mailcrane_store::MessageId;

fn subjects(f: &Fixture, query: &str) -> Vec<String> {
    local_search(&f.store, f.account, None, &parse_query(query), 100)
        .expect("search")
        .into_iter()
        .map(|m| m.details.subject)
        .collect()
}

#[test]
fn finds_by_words_prefix_people_and_body() {
    let f = fixture();
    let lunch = add(&f, f.inbox, 1, "Lunch plans", "ana@example.org", 300);
    add(&f, f.inbox, 2, "Quarterly report", "carol@example.net", 200);
    add(&f, f.inbox, 3, "Launch party", "dan@example.org", 100);
    f.store
        .index_body_text(lunch, "Shall we try the new Thai place?")
        .expect("indexed");
    assert_eq!(subjects(&f, "lunch"), ["Lunch plans"]);
    assert_eq!(
        subjects(&f, "lun"),
        ["Lunch plans"],
        "last word is a prefix"
    );
    assert_eq!(subjects(&f, "thai"), ["Lunch plans"], "body text");
    assert_eq!(subjects(&f, "from:carol"), ["Quarterly report"]);
    assert_eq!(subjects(&f, "to:bob").len(), 3);
    assert_eq!(subjects(&f, "subject:party"), ["Launch party"]);
    assert_eq!(subjects(&f, "\"report\""), ["Quarterly report"]);
    assert!(subjects(&f, "nothing-like-this").is_empty());
    assert!(subjects(&f, "").is_empty());
}

#[test]
fn diacritics_match_both_ways() {
    let f = fixture();
    add(&f, f.inbox, 1, "Note from Élodie", "x@example.org", 1);
    add(&f, f.inbox, 2, "Elodie again", "y@example.org", 2);
    assert_eq!(subjects(&f, "élodie").len(), 2);
    assert_eq!(subjects(&f, "elodie").len(), 2);
}

#[test]
fn fts_syntax_in_queries_is_only_text() {
    let f = fixture();
    add(&f, f.inbox, 1, "Plain subject", "x@example.org", 1);
    for query in [
        "\" OR body:*",
        "NEAR(",
        "NEAR(plain subject)",
        "-",
        "*",
        "plain OR",
        "\"unbalanced",
        "subject:*",
        "from_text:x",
        "a AND NOT b",
        "^plain",
        "plain\"",
        "{subject}: plain",
    ] {
        // Never an error; at most no results.
        let result = local_search(&f.store, f.account, None, &parse_query(query), 10);
        assert!(result.is_ok(), "{query}: {result:?}");
    }
    // Operators are words: as an operator "plain OR zz" would match "plain";
    // as words it needs "or" and a word starting "zz" too.
    assert!(subjects(&f, "plain OR zz").is_empty());
    assert_eq!(subjects(&f, "plain subject").len(), 1);
    assert_eq!(
        fts_expression(&parse_query("\" OR body:*")).as_deref(),
        // One quoted string (the unbalanced quote keeps its space): FTS5 sees no syntax.
        Some("\" OR body:*\" *")
    );
    assert_eq!(fts_expression(&parse_query("- * ^")), None);
}

#[test]
fn filters_narrow_results() {
    let f = fixture();
    let a = add(&f, f.inbox, 1, "Alpha", "x@example.org", 1_793_491_200);
    add(&f, f.inbox, 2, "Beta", "x@example.org", 1_790_000_000);
    let work = f
        .store
        .list_folders(f.account)
        .expect("folders")
        .into_iter()
        .find(|x| x.details.remote_name == "Work")
        .expect("work")
        .id;
    add(&f, work, 1, "Gamma", "x@example.org", 1_793_491_300);
    f.store
        .set_flags(
            a,
            Flags {
                flagged: true,
                ..Flags::default()
            },
        )
        .expect("flag");
    assert_eq!(subjects(&f, "is:flagged"), ["Alpha"]);
    assert_eq!(subjects(&f, "is:unread").len(), 3);
    assert_eq!(subjects(&f, "after:2026-11-01"), ["Gamma", "Alpha"]);
    assert_eq!(subjects(&f, "before:2026-11-01"), ["Beta"]);
    assert_eq!(subjects(&f, "in:work"), ["Gamma"]);
    assert!(subjects(&f, "has:attachment").is_empty());
    let only_inbox = local_search(
        &f.store,
        f.account,
        Some(f.inbox),
        &parse_query("example"),
        10,
    )
    .expect("search");
    assert_eq!(only_inbox.len(), 2);
}

/// `cargo test -p mailcrane-core --release -- --ignored fifty_thousand`
#[test]
#[ignore = "timing test; run in release mode"]
fn fifty_thousand_messages_search_fast() {
    let f = fixture();
    for uid in 1..=50_000u32 {
        let subject = format!("Message {uid} about topic{}", uid % 500);
        add(
            &f,
            f.inbox,
            uid,
            &subject,
            "sender@example.org",
            i64::from(uid),
        );
    }
    let start = std::time::Instant::now();
    let found =
        local_search(&f.store, f.account, None, &parse_query("topic42"), 100).expect("search");
    let took = start.elapsed();
    assert_eq!(found.len(), 100);
    assert!(took.as_millis() < 200, "took {took:?}");
}

#[test]
fn every_account_is_searched_and_account_narrows_it() {
    let f = fixture();
    let other = f
        .store
        .insert_account(&NewAccount {
            display_name: "Ann Work".into(),
            email: "ann@example.net".into(),
            imap_host: "imap.example.net".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.net".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: "ann".into(),
        })
        .expect("account");
    let other_inbox = f
        .store
        .upsert_folder(&NewFolder {
            account_id: other,
            remote_name: "INBOX".into(),
            display_name: "INBOX".into(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: Some(1),
            uidnext: Some(1),
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
            virtual_role: None,
        })
        .expect("inbox");
    add(&f, f.inbox, 1, "lunch at home", "a@example.com", 100);
    add(&f, other_inbox, 1, "lunch at work", "b@example.com", 200);
    let all = |query: &str| -> Vec<String> {
        local_search_all(&f.store, &parse_query(query), 100)
            .expect("search")
            .into_iter()
            .map(|m| m.details.subject)
            .collect()
    };
    assert_eq!(all("lunch"), ["lunch at work", "lunch at home"]);
    assert_eq!(all("lunch account:EXAMPLE.NET"), ["lunch at work"]);
    assert_eq!(all("lunch account:\"ann work\""), ["lunch at work"]);
    assert_eq!(all("account:me@example.org"), ["lunch at home"]);
    assert!(all("lunch account:nobody").is_empty());
    // One account's search stays in that account, and `account:` narrows it further.
    assert_eq!(subjects(&f, "lunch"), ["lunch at home"]);
    assert!(subjects(&f, "lunch account:example.net").is_empty());
}
