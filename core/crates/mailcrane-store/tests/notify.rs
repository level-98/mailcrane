//! New-mail notifications (M4-05): each message is notified at most once,
//! the first look at an account notifies nothing, and resyncs don't repeat.

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

const NOW: i64 = 1_800_000_000;
const SINCE: i64 = NOW - 7 * 24 * 3600;

fn account() -> NewAccount {
    NewAccount {
        display_name: "Alice".into(),
        email: "alice@example.org".into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 587,
        smtp_security: Security::Starttls,
        username: "alice".into(),
    }
}

fn inbox(account_id: AccountId, uidvalidity: Option<i64>) -> NewFolder {
    NewFolder {
        account_id,
        remote_name: "INBOX".into(),
        display_name: "Inbox".into(),
        delimiter: Some("/".into()),
        role: Some(FolderRole::Inbox),
        uidvalidity,
        uidnext: Some(100),
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
        virtual_role: None,
    }
}

fn mail(
    folder_id: FolderId,
    uid: u32,
    message_id: Option<&str>,
    from: &str,
    date: i64,
) -> NewMessageHeader {
    NewMessageHeader {
        folder_id,
        uid,
        message_id_header: message_id.map(Into::into),
        in_reply_to: None,
        references_json: "[]".into(),
        thread_id: None,
        subject: format!("Message {uid}"),
        from_name: Some("Bob".into()),
        from_addr: Some(from.into()),
        to_json: "[]".into(),
        cc_json: "[]".into(),
        date,
        internal_date: date,
        size: 100,
        flags: Flags::default(),
        keywords_json: "[]".into(),
        has_attachments: false,
        snippet: String::new(),
    }
}

struct Fixture {
    store: Store,
    account: AccountId,
    inbox: FolderId,
}

fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    let store = Store::open_in_memory()?;
    let account = store.insert_account(&account())?;
    let inbox = store.upsert_folder(&inbox(account, Some(1)))?;
    Ok(Fixture {
        store,
        account,
        inbox,
    })
}

impl Fixture {
    fn take(&self) -> Result<Vec<NewMail>, StoreError> {
        self.store.take_new_mail(
            self.account,
            self.inbox,
            "alice@example.org",
            SINCE,
            NOW,
            50,
        )
    }
}

#[test]
fn the_first_look_notifies_nothing_already_there() -> R {
    let f = fixture()?;
    f.store.upsert_message_header(&mail(
        f.inbox,
        1,
        Some("<a@example.net>"),
        "bob@example.net",
        NOW - 60,
    ))?;
    assert!(f.take()?.is_empty());
    // And it stays quiet afterwards about that message.
    assert!(f.take()?.is_empty());
    Ok(())
}

#[test]
fn nothing_happens_before_the_inbox_has_synced() -> R {
    let store = Store::open_in_memory()?;
    let account = store.insert_account(&account())?;
    let inbox = store.upsert_folder(&inbox(account, None))?;
    assert!(store
        .take_new_mail(account, inbox, "alice@example.org", SINCE, NOW, 50)?
        .is_empty());
    // The first sync brings 3 unread messages: still only the first look, no flood.
    store.upsert_folder(&self::inbox(account, Some(7)))?;
    for uid in 1..=3 {
        store.upsert_message_header(&mail(
            inbox,
            uid,
            Some(&format!("<{uid}@example.net>")),
            "bob@example.net",
            NOW - 60,
        ))?;
    }
    assert!(store
        .take_new_mail(account, inbox, "alice@example.org", SINCE, NOW, 50)?
        .is_empty());
    Ok(())
}

#[test]
fn new_mail_is_notified_once() -> R {
    let f = fixture()?;
    assert!(f.take()?.is_empty()); // first look
    f.store.upsert_message_header(&mail(
        f.inbox,
        2,
        Some("<b@example.net>"),
        "bob@example.net",
        NOW - 30,
    ))?;
    let got = f.take()?;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].subject, "Message 2");
    assert_eq!(got[0].from_addr.as_deref(), Some("bob@example.net"));
    assert!(f.take()?.is_empty(), "never twice");
    Ok(())
}

#[test]
fn a_resync_with_new_rows_and_uids_does_not_notify_again() -> R {
    let f = fixture()?;
    assert!(f.take()?.is_empty());
    f.store.upsert_message_header(&mail(
        f.inbox,
        2,
        Some("<b@example.net>"),
        "bob@example.net",
        NOW - 30,
    ))?;
    assert_eq!(f.take()?.len(), 1);
    // UIDVALIDITY reset: the folder's messages are dropped and come back with new UIDs.
    assert!(f.store.reset_folder(f.inbox, 2, 1000)?);
    f.store.upsert_message_header(&mail(
        f.inbox,
        900,
        Some("<b@example.net>"),
        "bob@example.net",
        NOW - 30,
    ))?;
    assert!(f.take()?.is_empty());
    Ok(())
}

#[test]
fn read_old_own_and_draft_messages_are_skipped() -> R {
    let f = fixture()?;
    assert!(f.take()?.is_empty());
    let mut read = mail(
        f.inbox,
        3,
        Some("<read@example.net>"),
        "bob@example.net",
        NOW - 10,
    );
    read.flags.seen = true;
    f.store.upsert_message_header(&read)?;
    f.store.upsert_message_header(&mail(
        f.inbox,
        4,
        Some("<old@example.net>"),
        "bob@example.net",
        SINCE - 1,
    ))?;
    f.store.upsert_message_header(&mail(
        f.inbox,
        5,
        Some("<own@example.net>"),
        "Alice@Example.org",
        NOW - 10,
    ))?;
    let mut draft = mail(
        f.inbox,
        6,
        Some("<draft@example.net>"),
        "bob@example.net",
        NOW - 10,
    );
    draft.flags.draft = true;
    f.store.upsert_message_header(&draft)?;
    assert!(f.take()?.is_empty());
    Ok(())
}

#[test]
fn messages_without_a_message_id_are_notified_once() -> R {
    let f = fixture()?;
    assert!(f.take()?.is_empty());
    f.store
        .upsert_message_header(&mail(f.inbox, 7, None, "bob@example.net", NOW - 10))?;
    f.store
        .upsert_message_header(&mail(f.inbox, 8, None, "bob@example.net", NOW - 10))?;
    assert_eq!(f.take()?.len(), 2);
    assert!(f.take()?.is_empty());
    Ok(())
}

#[test]
fn the_limit_leaves_the_rest_for_next_time() -> R {
    let f = fixture()?;
    assert!(f.take()?.is_empty());
    for uid in 10..15 {
        f.store.upsert_message_header(&mail(
            f.inbox,
            uid,
            Some(&format!("<{uid}@example.net>")),
            "bob@example.net",
            NOW - 100 + i64::from(uid),
        ))?;
    }
    let first = f
        .store
        .take_new_mail(f.account, f.inbox, "alice@example.org", SINCE, NOW, 3)?;
    assert_eq!(
        first.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(),
        ["Message 10", "Message 11", "Message 12"]
    );
    assert_eq!(f.take()?.len(), 2);
    Ok(())
}

#[test]
fn reopening_the_same_store_does_not_notify_again() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("n.db");
    let (account, inbox) = {
        let store = Store::open(&path)?;
        let account = store.insert_account(&account())?;
        let inbox = store.upsert_folder(&self::inbox(account, Some(1)))?;
        assert!(store
            .take_new_mail(account, inbox, "alice@example.org", SINCE, NOW, 50)?
            .is_empty());
        store.upsert_message_header(&mail(
            inbox,
            2,
            Some("<b@example.net>"),
            "bob@example.net",
            NOW - 30,
        ))?;
        assert_eq!(
            store
                .take_new_mail(account, inbox, "alice@example.org", SINCE, NOW, 50)?
                .len(),
            1
        );
        (account, inbox)
    };
    let store = Store::open(&path)?;
    assert!(store
        .take_new_mail(account, inbox, "alice@example.org", SINCE, NOW, 50)?
        .is_empty());
    Ok(())
}
