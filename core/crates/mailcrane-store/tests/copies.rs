//! One message stored in several folders, the way Gmail labels look over
//! IMAP (Inbox, a label and All Mail each hold a copy). The conversation and
//! search must show it once, and a different message that only reuses the
//! Message-ID must not be hidden by it.

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

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

fn folder(account_id: AccountId, name: &str, role: Option<FolderRole>) -> NewFolder {
    NewFolder {
        account_id,
        remote_name: name.into(),
        display_name: name.into(),
        delimiter: Some("/".into()),
        role,
        uidvalidity: Some(1),
        uidnext: Some(100),
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
    }
}

fn header(
    folder_id: FolderId,
    uid: u32,
    message_id: Option<&str>,
    date: i64,
    size: i64,
) -> NewMessageHeader {
    NewMessageHeader {
        folder_id,
        uid,
        message_id_header: message_id.map(Into::into),
        in_reply_to: None,
        references_json: "[]".into(),
        thread_id: None,
        subject: "Plans".into(),
        from_name: Some("Bob".into()),
        from_addr: Some("bob@example.org".into()),
        to_json: "[]".into(),
        cc_json: "[]".into(),
        date,
        internal_date: date,
        size,
        flags: Flags::default(),
        keywords_json: "[]".into(),
        has_attachments: false,
        snippet: String::new(),
    }
}

struct Mailbox {
    store: Store,
    account: AccountId,
    thread: ThreadId,
    inbox: FolderId,
    work: FolderId,
    all_mail: FolderId,
    /// The message's copies in Inbox, the "Work" label and All Mail.
    copy_inbox: MessageId,
    copy_work: MessageId,
    copy_all: MessageId,
    /// A reply that is only in All Mail.
    reply: MessageId,
    /// A different message reusing the first one's Message-ID.
    impostor: MessageId,
    /// Two messages without a Message-ID, otherwise alike.
    no_id: [MessageId; 2],
}

fn mailbox() -> Result<Mailbox, Box<dyn std::error::Error>> {
    let store = Store::open_in_memory()?;
    let account = store.insert_account(&account())?;
    let inbox = store.upsert_folder(&folder(account, "INBOX", Some(FolderRole::Inbox)))?;
    let work = store.upsert_folder(&folder(account, "Work", None))?;
    let all_mail =
        store.upsert_folder(&folder(account, "[Gmail]/All Mail", Some(FolderRole::All)))?;
    let id = Some("plans@example.org");
    let copy_inbox = store.upsert_message_header(&header(inbox, 1, id, 100, 500))?;
    let copy_work = store.upsert_message_header(&header(work, 7, id, 100, 500))?;
    let copy_all = store.upsert_message_header(&header(all_mail, 40, id, 100, 500))?;
    let reply =
        store.upsert_message_header(&header(all_mail, 41, Some("reply@example.org"), 200, 300))?;
    let impostor = store.upsert_message_header(&header(inbox, 2, id, 150, 900))?;
    let no_id = [
        store.upsert_message_header(&header(inbox, 3, None, 50, 100))?,
        store.upsert_message_header(&header(inbox, 4, None, 50, 100))?,
    ];
    let thread = store.create_thread(account, "plans", 100)?;
    for m in [copy_inbox, copy_work, copy_all, reply, impostor] {
        store.set_message_thread(m, thread, "plans", 0, 100)?;
    }
    Ok(Mailbox {
        store,
        account,
        thread,
        inbox,
        work,
        all_mail,
        copy_inbox,
        copy_work,
        copy_all,
        reply,
        impostor,
        no_id,
    })
}

fn ids(list: &[MessageHeader]) -> Vec<MessageId> {
    list.iter().map(|h| h.id).collect()
}

fn search(m: &Mailbox, folder: Option<FolderId>, limit: u32) -> Result<Vec<MessageId>, StoreError> {
    Ok(ids(&m.store.search_messages(&SearchFilter {
        account: Some(m.account),
        account_name: None,
        folder,
        folder_name: None,
        fts: None,
        unread: false,
        flagged: false,
        has_attachment: false,
        after: None,
        before: None,
        limit,
    })?))
}

#[test]
fn conversation_shows_each_message_once() -> R {
    let m = mailbox()?;
    let got = ids(&m.store.thread_messages(m.thread)?);
    // Oldest first; one copy of the message, never the All Mail one.
    assert_eq!(got, [m.copy_inbox, m.impostor, m.reply]);
    Ok(())
}

#[test]
fn conversation_prefers_a_copy_whose_body_is_downloaded() -> R {
    let m = mailbox()?;
    m.store
        .store_body(m.copy_all, b"Subject: Plans\r\n\r\nhi", 1)?;
    m.store
        .store_body(m.copy_work, b"Subject: Plans\r\n\r\nhi", 1)?;
    let got = ids(&m.store.thread_messages(m.thread)?);
    // All Mail still loses: archiving or moving from there does nothing useful.
    assert_eq!(got, [m.copy_work, m.impostor, m.reply]);
    Ok(())
}

#[test]
fn a_hidden_copy_gives_way_to_another() -> R {
    let m = mailbox()?;
    m.store
        .store_body(m.copy_work, b"Subject: Plans\r\n\r\nhi", 1)?;
    // A queued move hides the Work copy; the Inbox one is shown instead.
    m.store.queue_op(
        m.account,
        OpKind::Move,
        m.work,
        Some(m.inbox),
        &[OpMessage {
            message_id: m.copy_work,
            uid: 7,
        }],
        1,
    )?;
    let got = ids(&m.store.thread_messages(m.thread)?);
    assert_eq!(got, [m.copy_inbox, m.impostor, m.reply]);
    Ok(())
}

#[test]
fn search_shows_each_message_once() -> R {
    let m = mailbox()?;
    // Newest first.
    assert_eq!(
        search(&m, None, 50)?,
        [m.reply, m.impostor, m.copy_inbox, m.no_id[1], m.no_id[0]]
    );
    Ok(())
}

#[test]
fn search_limit_counts_messages_not_copies() -> R {
    let m = mailbox()?;
    assert_eq!(search(&m, None, 3)?, [m.reply, m.impostor, m.copy_inbox]);
    Ok(())
}

#[test]
fn search_in_one_folder_returns_that_folders_copy() -> R {
    let m = mailbox()?;
    assert_eq!(search(&m, Some(m.all_mail), 50)?, [m.reply, m.copy_all]);
    Ok(())
}
