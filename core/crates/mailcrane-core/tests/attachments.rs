//! Attachments (M2-04): saving received ones safely, and sending,
//! forwarding and undoing a send with them.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_core::attachments::{save_attachment, AttachmentError};
use mailcrane_core::compose::{build_for_sending, prepare_reply, ComposeDraft, ReplyKind};
use mailcrane_mime::{build_message, parse_message, Address, OutgoingAttachment, OutgoingMessage};
use mailcrane_store::{
    AccountId, Flags, FolderId, MessageId, NewAccount, NewFolder, NewMessageHeader, Security, Store,
};

fn setup() -> (Store, AccountId, FolderId) {
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
    let folder = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "Inbox".into(),
            delimiter: None,
            role: None,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("folder");
    (store, account, folder)
}

/// Stores a downloaded message with these attachments; returns its id.
fn received(store: &Store, folder: FolderId, files: &[(&str, &[u8])]) -> MessageId {
    let raw = build_message(&OutgoingMessage {
        from: Address {
            name: Some("Bob".into()),
            email: "bob@example.net".into(),
        },
        to: vec![Address {
            name: None,
            email: "alice@example.org".into(),
        }],
        subject: "Files".into(),
        body_text: "Here you go".into(),
        attachments: files
            .iter()
            .map(|(name, bytes)| OutgoingAttachment {
                filename: (*name).into(),
                mime_type: "application/octet-stream".into(),
                bytes: bytes.to_vec(),
            })
            .collect(),
        ..OutgoingMessage::default()
    })
    .expect("build");
    let id = store
        .upsert_message_header(&NewMessageHeader {
            folder_id: folder,
            uid: 1,
            message_id_header: Some("files@example.net".into()),
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: "Files".into(),
            from_name: Some("Bob".into()),
            from_addr: Some("bob@example.net".into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date: 1_000,
            internal_date: 1_000,
            size: i64::try_from(raw.len()).unwrap_or(0),
            flags: Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: true,
            snippet: String::new(),
        })
        .expect("header");
    store.store_body(id, &raw, 1_000).expect("body");
    id
}

fn part_ids(store: &Store, id: MessageId) -> Vec<String> {
    let raw = store.get_body(id).expect("body").expect("body");
    parse_message(&raw)
        .expect("parse")
        .attachments
        .into_iter()
        .map(|a| a.part_id)
        .collect()
}

#[test]
fn hostile_names_are_saved_only_inside_the_folder() {
    let (store, _, folder) = setup();
    let id = received(
        &store,
        folder,
        &[
            ("../../escape.txt", b"one"),
            ("..\\..\\escape2.txt", b"two"),
            ("CON", b"three"),
            ("a\u{202e}gpj.exe", b"four"),
            ("same.txt", b"five"),
            ("same.txt", b"six"),
        ],
    );
    let outer = tempfile::tempdir().expect("dir");
    let dest = outer.path().join("attachments");
    std::fs::create_dir(&dest).expect("mkdir");
    let mut names = Vec::new();
    for part in part_ids(&store, id) {
        let path = save_attachment(&store, id, &part, &dest).expect("save");
        assert_eq!(
            path.parent().map(std::path::Path::to_path_buf),
            Some(dest.canonicalize().expect("canon")),
            "{path:?}"
        );
        names.push(
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned(),
        );
    }
    assert_eq!(
        names,
        [
            "escape.txt",
            "escape2.txt",
            "_CON",
            "agpj.exe",
            "same.txt",
            "same (2).txt"
        ]
    );
    // Nothing landed next to the folder.
    let outside: Vec<_> = std::fs::read_dir(outer.path()).expect("list").collect();
    assert_eq!(outside.len(), 1);
    assert_eq!(
        std::fs::read(dest.join("same (2).txt")).expect("read"),
        b"six"
    );
    let err = save_attachment(&store, id, "999", &dest).expect_err("missing part");
    assert!(matches!(err, AttachmentError::NotFound));
}

#[test]
fn forwarding_takes_the_attachments_along_and_sending_includes_new_ones() {
    let (store, account, folder) = setup();
    let id = received(&store, folder, &[("plan.pdf", b"%PDF-1.7 fake")]);
    let mut draft = prepare_reply(&store, id, ReplyKind::Forward, "Forwarded:").expect("forward");
    assert_eq!(draft.forward_attachments_of, Some(id));
    draft.to = vec!["carol@example.com".into()];
    draft.attachments.push(OutgoingAttachment {
        filename: "notes.txt".into(),
        mime_type: "text/plain".into(),
        bytes: b"see page 2".to_vec(),
    });
    let built = build_for_sending(&store, &draft).expect("build");
    let parsed = parse_message(&built.raw).expect("parse");
    let names: Vec<Option<String>> = parsed
        .attachments
        .iter()
        .map(|a| a.filename.clone())
        .collect();
    assert_eq!(names, [Some("notes.txt".into()), Some("plan.pdf".into())]);
    // A reply doesn't carry them.
    let reply = prepare_reply(&store, id, ReplyKind::Reply, "Wrote:").expect("reply");
    assert_eq!(reply.forward_attachments_of, None);
    let _ = account;
}

#[test]
fn a_plain_draft_has_no_attachments() {
    let (store, account, _) = setup();
    let draft = ComposeDraft {
        account_id: account,
        to: vec!["bob@example.net".into()],
        cc: vec![],
        bcc: vec![],
        subject: "Hi".into(),
        body_text: "Hello".into(),
        in_reply_to: None,
        references: vec![],
        attachments: Vec::new(),
        forward_attachments_of: None,
        rich: None,
    };
    let built = build_for_sending(&store, &draft).expect("build");
    assert!(parse_message(&built.raw)
        .expect("parse")
        .attachments
        .is_empty());
    assert!(!String::from_utf8_lossy(&built.raw).contains("multipart"));
}
