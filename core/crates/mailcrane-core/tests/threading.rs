//! Conversation threading (M4-02) on header-only messages.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::threading::{assign_new, rebuild};
use mailcrane_store::{
    AccountId, Flags, FolderId, FolderRole, NewAccount, NewFolder, NewMessageHeader, Security,
    Store,
};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

const NOW: i64 = 1_795_550_400;
const DAY: i64 = 86_400;

#[derive(Debug, Clone)]
struct Fixture {
    uid: u32,
    id: Option<String>,
    in_reply_to: Option<String>,
    refs: Vec<String>,
    subject: String,
    date: i64,
}

fn fx(uid: u32, id: &str, irt: Option<&str>, refs: &[&str], subject: &str, date: i64) -> Fixture {
    Fixture {
        uid,
        id: (!id.is_empty()).then(|| id.to_owned()),
        in_reply_to: irt.map(Into::into),
        refs: refs.iter().map(|r| (*r).to_owned()).collect(),
        subject: subject.into(),
        date,
    }
}

fn setup() -> (Store, AccountId, FolderId, FolderId) {
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: "alice@example.org".into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: "alice@example.org".into(),
        })
        .expect("account");
    let folder = |name: &str, role| {
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
                selectable: true,
                virtual_role: None,
            })
            .expect("folder")
    };
    let inbox = folder("INBOX", Some(FolderRole::Inbox));
    let sent = folder("Sent", Some(FolderRole::Sent));
    (store, account, inbox, sent)
}

fn json(list: &[String]) -> String {
    let items: Vec<String> = list.iter().map(|s| format!("\"{s}\"")).collect();
    format!("[{}]", items.join(","))
}

fn insert(store: &Store, folder: FolderId, f: &Fixture) {
    store
        .upsert_message_header(&NewMessageHeader {
            folder_id: folder,
            uid: f.uid,
            message_id_header: f.id.clone(),
            in_reply_to: f.in_reply_to.clone(),
            references_json: json(&f.refs),
            thread_id: None,
            subject: f.subject.clone(),
            from_name: Some(format!("Person {}", f.uid % 3)),
            from_addr: Some(format!("p{}@example.net", f.uid % 3)),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date: f.date,
            internal_date: f.date,
            size: 10,
            flags: Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
        })
        .expect("message");
}

/// The threads as sets of uids, independent of thread ids.
fn partition(store: &Store, folder: FolderId) -> BTreeSet<BTreeSet<u32>> {
    let mut by_thread: BTreeMap<i64, BTreeSet<u32>> = BTreeMap::new();
    for m in store.list_messages(folder, 100_000, 0).expect("list") {
        let t = m.details.thread_id.expect("threaded").0;
        by_thread.entry(t).or_default().insert(m.details.uid);
    }
    by_thread.into_values().collect()
}

fn groups(list: &[&[u32]]) -> BTreeSet<BTreeSet<u32>> {
    list.iter().map(|g| g.iter().copied().collect()).collect()
}

fn thread_all(fixtures: &[Fixture]) -> BTreeSet<BTreeSet<u32>> {
    let (store, account, inbox, _) = setup();
    for f in fixtures {
        insert(&store, inbox, f);
    }
    assign_new(&store, Some(account)).expect("thread");
    partition(&store, inbox)
}

#[test]
fn a_simple_chain() {
    let got = thread_all(&[
        fx(1, "a@x", None, &[], "Plans", NOW),
        fx(2, "b@x", Some("a@x"), &["a@x"], "Re: Plans", NOW + 60),
        fx(
            3,
            "c@x",
            Some("b@x"),
            &["a@x", "b@x"],
            "Re: Plans",
            NOW + 120,
        ),
        fx(4, "z@x", None, &[], "Other", NOW),
    ]);
    assert_eq!(got, groups(&[&[1, 2, 3], &[4]]));
}

#[test]
fn a_branching_tree() {
    let got = thread_all(&[
        fx(1, "a@x", None, &[], "Trip", NOW),
        fx(2, "b@x", Some("a@x"), &["a@x"], "Re: Trip", NOW + 1),
        fx(3, "c@x", Some("a@x"), &["a@x"], "Re: Trip", NOW + 2),
        fx(4, "d@x", Some("c@x"), &["a@x", "c@x"], "Re: Trip", NOW + 3),
        fx(5, "e@x", Some("b@x"), &["a@x", "b@x"], "Re: Trip", NOW + 4),
    ]);
    assert_eq!(got, groups(&[&[1, 2, 3, 4, 5]]));
}

#[test]
fn a_missing_parent_still_links_its_replies() {
    // Neither has the original (never downloaded): the shared parent id
    // links them, like a placeholder container.
    let got = thread_all(&[
        fx(1, "b@x", Some("gone@x"), &["gone@x"], "Re: Lost", NOW),
        fx(2, "c@x", Some("gone@x"), &["gone@x"], "Re: Lost", NOW + 1),
        fx(3, "d@x", None, &[], "Unrelated", NOW),
    ]);
    assert_eq!(got, groups(&[&[1, 2], &[3]]));
}

#[test]
fn a_references_cycle_is_harmless() {
    let got = thread_all(&[
        fx(1, "a@x", Some("b@x"), &["b@x", "a@x"], "Loop", NOW),
        fx(2, "b@x", Some("a@x"), &["a@x", "b@x"], "Re: Loop", NOW + 1),
        fx(3, "c@x", Some("c@x"), &["c@x"], "Self", NOW),
    ]);
    assert_eq!(got, groups(&[&[1, 2], &[3]]));
}

#[test]
fn five_hundred_references_in_one_message() {
    let refs: Vec<String> = (0..500).map(|i| format!("r{i}@x")).collect();
    let refs_str: Vec<&str> = refs.iter().map(String::as_str).collect();
    let got = thread_all(&[
        fx(1, "m@x", Some("r499@x"), &refs_str, "Re: Long", NOW),
        // Its nearest ancestor is kept…
        fx(2, "r499@x", None, &[], "Long", NOW - 1),
        // …but the 500th-from-last is past the limit of 100.
        fx(3, "r0@x", None, &[], "Ancient", NOW - 2),
    ]);
    assert_eq!(got, groups(&[&[1, 2], &[3]]));
}

#[test]
fn unrelated_hellos_stay_apart_but_a_reply_without_references_joins() {
    let got = thread_all(&[
        fx(1, "h1@x", None, &[], "Hello", NOW),
        fx(2, "h2@x", None, &[], "hello", NOW + DAY),
    ]);
    assert_eq!(got, groups(&[&[1], &[2]]));
    let got = thread_all(&[
        fx(1, "h1@x", None, &[], "Hello", NOW),
        fx(2, "r1@x", None, &[], "Re: Hello", NOW + DAY),
    ]);
    assert_eq!(got, groups(&[&[1, 2]]));
    // Too far apart, or the reply came first: not linked by subject.
    let got = thread_all(&[
        fx(1, "h1@x", None, &[], "Hello", NOW),
        fx(2, "r1@x", None, &[], "Re: Hello", NOW + 60 * DAY),
        fx(3, "r2@x", None, &[], "RE: hello", NOW - DAY),
    ]);
    assert_eq!(got, groups(&[&[1], &[2], &[3]]));
}

#[test]
fn threads_span_folders_and_follow_new_mail() {
    let (store, account, inbox, sent) = setup();
    insert(&store, inbox, &fx(1, "a@x", None, &[], "Lunch?", NOW));
    assign_new(&store, Some(account)).expect("thread");
    // My reply, in Sent.
    insert(
        &store,
        sent,
        &fx(1, "b@x", Some("a@x"), &["a@x"], "Re: Lunch?", NOW + 60),
    );
    // Their answer, in the inbox.
    insert(
        &store,
        inbox,
        &fx(
            2,
            "c@x",
            Some("b@x"),
            &["a@x", "b@x"],
            "Re: Lunch?",
            NOW + 120,
        ),
    );
    assert_eq!(assign_new(&store, Some(account)).expect("thread"), 2);
    assert_eq!(
        assign_new(&store, Some(account)).expect("thread"),
        0,
        "once"
    );
    let threads = store.list_threads(inbox, 0, 50).expect("threads");
    assert_eq!(threads.len(), 1);
    let t = &threads[0];
    assert_eq!(
        (t.message_count, t.unread_count, t.last_date),
        (2, 2, NOW + 120)
    );
    let all = store.thread_messages(t.thread_id).expect("messages");
    let subjects: Vec<&str> = all.iter().map(|m| m.details.subject.as_str()).collect();
    assert_eq!(
        subjects,
        ["Lunch?", "Re: Lunch?", "Re: Lunch?"],
        "Sent included, by date"
    );
}

#[test]
fn a_late_message_merges_two_threads() {
    let (store, account, inbox, _) = setup();
    insert(&store, inbox, &fx(1, "a@x", None, &[], "Plan", NOW));
    insert(
        &store,
        inbox,
        &fx(3, "c@x", Some("b@x"), &["b@x"], "Re: Plan", NOW + 2),
    );
    assign_new(&store, Some(account)).expect("thread");
    assert_eq!(partition(&store, inbox), groups(&[&[1], &[3]]));
    // The missing middle arrives and links both.
    insert(
        &store,
        inbox,
        &fx(2, "b@x", Some("a@x"), &["a@x"], "Re: Plan", NOW + 1),
    );
    assign_new(&store, Some(account)).expect("thread");
    assert_eq!(partition(&store, inbox), groups(&[&[1, 2, 3]]));
    // A rebuild agrees.
    rebuild(&store, account).expect("rebuild");
    assert_eq!(partition(&store, inbox), groups(&[&[1, 2, 3]]));
}

fn arb_fixtures() -> impl Strategy<Value = Vec<Fixture>> {
    let ids = 0u8..12;
    prop::collection::vec(
        (
            prop::option::of(ids.clone()),
            prop::option::of(ids.clone()),
            prop::collection::vec(ids, 0..4),
            0usize..4,
            any::<bool>(),
            0i64..80,
        ),
        1..18,
    )
    .prop_map(|rows| {
        let subjects = ["Hello", "Plans", "Trip", ""];
        rows.into_iter()
            .enumerate()
            .map(|(i, (id, irt, refs, subject, reply, day))| {
                let base = subjects[subject];
                let subject = if reply {
                    format!("Re: {base}")
                } else {
                    base.to_owned()
                };
                Fixture {
                    uid: u32::try_from(i).unwrap_or(0) + 1,
                    id: id.map(|n| format!("m{n}@x")),
                    in_reply_to: irt.map(|n| format!("m{n}@x")),
                    refs: refs.into_iter().map(|n| format!("m{n}@x")).collect(),
                    subject,
                    date: NOW + day * DAY,
                }
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Messages arriving one at a time, in any order, end up in the same
    /// threads as a full rebuild.
    #[test]
    fn incremental_equals_rebuild(fixtures in arb_fixtures(), seed in any::<u64>()) {
        let mut order: Vec<usize> = (0..fixtures.len()).collect();
        // A cheap deterministic shuffle from the seed.
        let mut s = seed | 1;
        for i in (1..order.len()).rev() {
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let j = usize::try_from(s % (i as u64 + 1)).unwrap_or(0);
            order.swap(i, j);
        }
        let (store, account, inbox, _) = setup();
        for &i in &order {
            insert(&store, inbox, &fixtures[i]);
            assign_new(&store, Some(account)).expect("thread");
        }
        let incremental = partition(&store, inbox);
        rebuild(&store, account).expect("rebuild");
        let rebuilt = partition(&store, inbox);
        prop_assert_eq!(&incremental, &rebuilt);
        prop_assert_eq!(&rebuilt, &thread_all(&fixtures));
    }
}

/// `cargo test -p mailcrane-core --release --test threading -- --ignored ten_thousand`.
#[test]
#[ignore = "timing test; run in release mode"]
fn ten_thousand_messages_under_a_second() {
    let (store, account, inbox, _) = setup();
    for i in 0..10_000u32 {
        // Conversations of 10 messages each, chained by References.
        let conv = i / 10;
        let pos = i % 10;
        let refs: Vec<String> = (0..pos).map(|p| format!("c{conv}-{p}@x")).collect();
        let f = Fixture {
            uid: i + 1,
            id: Some(format!("c{conv}-{pos}@x")),
            in_reply_to: refs.last().cloned(),
            refs,
            subject: if pos == 0 {
                format!("Topic {conv}")
            } else {
                format!("Re: Topic {conv}")
            },
            date: NOW + i64::from(i),
        };
        insert(&store, inbox, &f);
    }
    let started = std::time::Instant::now();
    assert_eq!(assign_new(&store, Some(account)).expect("thread"), 10_000);
    let took = started.elapsed();
    assert_eq!(
        store.list_threads(inbox, 0, 5_000).expect("threads").len(),
        1_000
    );
    assert!(took.as_millis() < 1_000, "{took:?}");
}
