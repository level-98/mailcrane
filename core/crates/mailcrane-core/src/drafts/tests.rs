//! Drafts against the testkit's fake IMAP server. Never a real mailbox.

use super::*;
use crate::actions::replay;
use crate::events::{CoreEvent, EventListener};
use crate::sync::{FolderSyncer, SyncConfig};
use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret};
use mailcrane_store::{NewAccount, Security};
use mailcrane_testkit::{FakeImap, Fault, Quirk};
use std::sync::Mutex;
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const NOW: i64 = 1_800_000_000;

#[derive(Default)]
struct Events(Mutex<Vec<CoreEvent>>);

impl EventListener for Events {
    fn on_event(&self, event: CoreEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

async fn fake(quirks: &[Quirk], drafts: bool) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![]);
    if drafts {
        builder = builder
            .folder("Drafts", vec![])
            .special_use("Drafts", "\\Drafts");
    }
    for q in quirks {
        builder = builder.quirk(*q);
    }
    builder.start().await.expect("starts")
}

async fn flaky(imap: &FakeImap, faults: &[Fault]) -> FakeImap {
    let mut builder = FakeImap::builder()
        .mailbox(imap.mailbox().clone())
        .user(USER, PASSWORD)
        .special_use("Drafts", "\\Drafts");
    for f in faults {
        builder = builder.fault(f.clone());
    }
    builder.start().await.expect("starts")
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

async fn setup(imap: &FakeImap, store: &Store) -> AccountId {
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: USER.into(),
        })
        .expect("account");
    let mut session = connect(imap).await;
    let listed = session.list_folders().await.expect("list");
    save_folders(store, account, &listed).expect("save");
    session.logout().await;
    account
}

fn fields(body: &str) -> DraftFields {
    DraftFields {
        to: vec!["Bob <bob@example.net>".into(), "half-typ".into()],
        cc: vec![],
        bcc: vec!["carol@example.com".into()],
        subject: "Plans".into(),
        body: body.into(),
        in_reply_to: None,
        references: vec![],
    }
}

async fn upload(imap: &FakeImap, store: &Store, account: AccountId, now: i64) -> UploadReport {
    let mut session = connect(imap).await;
    let report = upload_drafts(store, &mut session, account, now)
        .await
        .expect("upload");
    session.logout().await;
    report
}

/// Drafts on the server that aren't flagged `\Deleted`, as (uid, body).
async fn live_drafts(imap: &FakeImap) -> Vec<(u32, String)> {
    imap.mailbox()
        .messages("Drafts")
        .await
        .into_iter()
        .filter(|m| !m.flags.iter().any(|f| f == "\\Deleted"))
        .map(|m| {
            let text = String::from_utf8_lossy(&m.raw).into_owned();
            (m.uid, text)
        })
        .collect()
}

#[tokio::test]
async fn a_saved_draft_survives_closing_the_app() {
    let imap = fake(&[], true).await;
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("d.db");
    let store = Store::open(&path).expect("store");
    let account = setup(&imap, &store).await;
    let id = save_draft(&store, account, None, &fields("Hello"), NOW).expect("saved");
    let id2 =
        save_draft(&store, account, Some(id), &fields("Hello again"), NOW + 1).expect("saved");
    assert_eq!(id, id2);
    drop(store);

    let store = Store::open(&path).expect("reopen");
    let draft = store.get_draft(id).expect("read").expect("kept");
    assert_eq!(draft.fields, fields("Hello again"));
    assert!(draft.dirty);
    assert!(draft.message_id_header.ends_with("@example.org"));
}

#[tokio::test]
async fn edits_leave_exactly_one_server_copy_with_the_latest_content() {
    for quirks in [vec![], vec![Quirk::NoUidplus]] {
        let imap = fake(&quirks, true).await;
        let store = Store::open_in_memory().expect("store");
        let account = setup(&imap, &store).await;
        let id = save_draft(&store, account, None, &fields("Version one"), NOW).expect("saved");
        assert_eq!(upload(&imap, &store, account, NOW).await.uploaded, 1);
        save_draft(&store, account, Some(id), &fields("Version two"), NOW + 5).expect("saved");
        save_draft(&store, account, Some(id), &fields("Version three"), NOW + 6).expect("saved");
        assert_eq!(upload(&imap, &store, account, NOW + 6).await.uploaded, 1);

        let live = live_drafts(&imap).await;
        assert_eq!(live.len(), 1, "{quirks:?}");
        assert!(live[0].1.contains("Version three"));
        assert!(
            !live[0].1.contains("half-typ"),
            "only valid addresses go out"
        );
        assert!(!live[0].1.to_ascii_lowercase().contains("bcc"));
        let message = imap
            .mailbox()
            .messages("Drafts")
            .await
            .into_iter()
            .find(|m| m.uid == live[0].0)
            .expect("there");
        assert!(message.flags.iter().any(|f| f == "\\Draft"));

        let draft = store.get_draft(id).expect("read").expect("kept");
        assert!(!draft.dirty);
        assert_eq!(draft.server_uid, Some(live[0].0));
        let received = imap.received_commands().await;
        assert!(!received.iter().any(|c| c == "EXPUNGE"), "{received:?}");
        if quirks.is_empty() {
            assert!(received.iter().any(|c| c == "UID EXPUNGE"));
            assert_eq!(imap.mailbox().messages("Drafts").await.len(), 1);
        }
    }
}

#[tokio::test]
async fn offline_edits_upload_when_the_connection_returns() {
    let imap = fake(&[], true).await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    save_draft(&store, account, None, &fields("Written offline"), NOW).expect("saved");

    // The connection drops at SELECT.
    let down = flaky(&imap, &[Fault::DropAfterCommands(4)]).await;
    let report = upload(&down, &store, account, NOW).await;
    assert_eq!((report.uploaded, report.remaining), (0, 1));
    assert!(live_drafts(&imap).await.is_empty());

    // Not due yet, then due.
    assert_eq!(upload(&imap, &store, account, NOW + 1).await.uploaded, 0);
    assert_eq!(upload(&imap, &store, account, NOW + 5).await.uploaded, 1);
    assert_eq!(live_drafts(&imap).await.len(), 1);
}

#[tokio::test]
async fn deleting_a_sent_draft_removes_it_locally_and_on_the_server() {
    let imap = fake(&[], true).await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let id = save_draft(&store, account, None, &fields("Send me"), NOW).expect("saved");
    upload(&imap, &store, account, NOW).await;
    assert_eq!(live_drafts(&imap).await.len(), 1);

    delete_draft(&store, id, NOW).expect("deleted");
    assert!(store.get_draft(id).expect("read").is_none());
    let mut session = connect(&imap).await;
    replay(&store, &mut session, &Events::default(), account, NOW)
        .await
        .expect("replay");
    session.logout().await;
    assert!(imap.mailbox().messages("Drafts").await.is_empty());
    assert!(matches!(
        delete_draft(&store, id, NOW),
        Err(DraftError::NotFound)
    ));
}

#[tokio::test]
async fn a_version_changed_elsewhere_is_kept_as_a_second_draft() {
    let imap = fake(&[], true).await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let id = save_draft(&store, account, None, &fields("Mine"), NOW).expect("saved");
    upload(&imap, &store, account, NOW).await;
    let draft = store.get_draft(id).expect("read").expect("kept");

    // Another device replaces our copy with its own version, same id.
    let theirs = format!(
        "From: alice@example.org\r\nTo: dan@example.org\r\nSubject: Plans\r\nMessage-ID: <{}>\r\n\r\nTheirs\r\n",
        draft.message_id_header
    );
    imap.mailbox()
        .append("Drafts", theirs.into_bytes(), vec!["\\Draft".into()])
        .await
        .expect("appended");
    imap.mailbox()
        .store_flag(
            "Drafts",
            |uid| Some(uid) == draft.server_uid,
            "\\Deleted",
            true,
        )
        .await;
    imap.mailbox()
        .expunge_deleted("Drafts", |uid| Some(uid) == draft.server_uid)
        .await;

    save_draft(&store, account, Some(id), &fields("Mine, edited"), NOW + 10).expect("saved");
    let report = upload(&imap, &store, account, NOW + 10).await;
    assert_eq!((report.uploaded, report.conflicts), (1, 1));

    let live = live_drafts(&imap).await;
    assert_eq!(live.len(), 2, "theirs is kept beside ours");
    assert!(live.iter().any(|(_, t)| t.contains("Theirs")));
    assert!(live.iter().any(|(_, t)| t.contains("Mine, edited")));
    let drafts = store.list_drafts(account).expect("list");
    assert_eq!(drafts.len(), 2);
    let conflict = drafts
        .iter()
        .find(|d| d.is_conflict)
        .expect("conflict draft");
    assert_eq!(conflict.fields.body.replace("\r\n", "\n").trim(), "Theirs");
    assert_eq!(conflict.fields.to, ["dan@example.org"]);

    // Uploading ours again never touches the conflict's copy.
    save_draft(&store, account, Some(id), &fields("Mine, again"), NOW + 20).expect("saved");
    upload(&imap, &store, account, NOW + 20).await;
    let live = live_drafts(&imap).await;
    assert_eq!(live.len(), 2);
    assert!(live.iter().any(|(_, t)| t.contains("Theirs")));
}

#[tokio::test]
async fn a_draft_opened_from_the_drafts_folder_replaces_its_own_copy() {
    let imap = fake(&[], true).await;
    imap.mailbox()
        .append(
            "Drafts",
            b"From: alice@example.org\r\nTo: bob@example.net\r\nSubject: Old\r\nMessage-ID: <old1@example.org>\r\n\r\nFrom another device\r\n".to_vec(),
            vec!["\\Draft".into(), "\\Seen".into()],
        )
        .await
        .expect("appended");
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    let drafts_id = drafts_folder(&store, account)
        .expect("read")
        .expect("has drafts");
    let mut session = connect(&imap).await;
    {
        let listener = Events::default();
        let mut syncer = FolderSyncer::new(
            &store,
            &mut session,
            &listener,
            account,
            SyncConfig::default(),
        );
        syncer.sync_folder(drafts_id).await.expect("sync");
        let message = store.list_messages(drafts_id, 10, 0).expect("list")[0].id;
        assert!(matches!(
            open_draft(&store, message, NOW),
            Err(DraftError::BodyNotDownloaded)
        ));
        syncer.fetch_body(message).await.expect("body");
    }
    session.logout().await;
    let message = store.list_messages(drafts_id, 10, 0).expect("list")[0].id;
    let id = open_draft(&store, message, NOW).expect("opened");
    assert_eq!(
        open_draft(&store, message, NOW).expect("again"),
        id,
        "same draft"
    );
    let draft = store.get_draft(id).expect("read").expect("kept");
    assert_eq!(draft.message_id_header, "old1@example.org");
    assert!(draft.fields.body.contains("From another device"));
    assert!(!draft.dirty && !draft.is_conflict);

    let mut edited = draft.fields.clone();
    edited.body = "Finished here".into();
    save_draft(&store, account, Some(id), &edited, NOW + 1).expect("saved");
    let report = upload(&imap, &store, account, NOW + 1).await;
    assert_eq!((report.uploaded, report.conflicts), (1, 0));
    let live = live_drafts(&imap).await;
    assert_eq!(live.len(), 1);
    assert!(live[0].1.contains("Finished here"));
}

#[tokio::test]
async fn without_a_drafts_folder_drafts_stay_on_the_device() {
    let imap = fake(&[], false).await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    save_draft(&store, account, None, &fields("Local"), NOW).expect("saved");
    let report = upload(&imap, &store, account, NOW).await;
    assert!(report.no_drafts_folder);
    assert_eq!((report.uploaded, report.remaining), (0, 1));
    assert!(imap.received_commands().await.iter().all(|c| c != "APPEND"));
}

#[tokio::test]
async fn bad_requests_change_nothing() {
    let imap = fake(&[], true).await;
    let store = Store::open_in_memory().expect("store");
    let account = setup(&imap, &store).await;
    assert!(matches!(
        save_draft(&store, account, Some(DraftId(99)), &fields("x"), NOW),
        Err(DraftError::NotFound)
    ));
    assert!(matches!(
        save_draft(&store, AccountId(99), None, &fields("x"), NOW),
        Err(DraftError::NotFound)
    ));
    assert!(matches!(
        open_draft(&store, MessageId(99), NOW),
        Err(DraftError::NotFound)
    ));
    assert!(store.list_drafts(account).expect("list").is_empty());
}
