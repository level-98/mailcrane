//! #462: the apps call the engine on threads with small stacks (Android's
//! `Dispatchers.Default` workers get about 1 MiB), and the core's async work
//! has large frames in debug builds. A snooze overflowed such a thread and
//! crashed the app on every launch. Every call here runs on a thread with a
//! stack far smaller than that work needs, so it passes only if the engine
//! runs its work on a thread of its own.

use super::tests::Platform;
use super::*;
use crate::actions::MessageAction;
use crate::secrets::{install_secret_store, PlatformSecretStore};
use mailcrane_testkit::FakeImap;

/// Well below what a pooled snooze needs today in a debug build.
const SMALL_STACK: usize = 256 * 1024;

/// Runs `f` on a new thread with a [`SMALL_STACK`] stack and returns what it
/// returns. A stack overflow there aborts the whole test binary (or, on
/// Windows, can surface as a failed TLS handshake).
pub(super) fn on_small_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("small-stack-caller".into())
            .stack_size(SMALL_STACK)
            .spawn_scoped(scope, f)
            .expect("thread starts")
            .join()
            .expect("no panic")
    })
}

fn raw(n: u32) -> Vec<u8> {
    format!(
        "From: Ana <ana@example.net>\r\nTo: alice@example.org\r\nSubject: Lunch {n}\r\n\
         Message-ID: <lunch-{n}@example.net>\r\n\r\nHello {n}\r\n"
    )
    .into_bytes()
}

#[test]
fn pooled_and_sync_calls_run_from_a_small_stack_thread() {
    // The fake server runs on a runtime of its own while the engine blocks.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", vec![raw(1), raw(2)])
            .folder("Archive", vec![])
            .special_use("Archive", "\\Archive")
            .start()
            .await
            .expect("fake imap")
    });
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("stack.db");
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_account(&mailcrane_store::NewAccount {
            display_name: "Alice".into(),
            email: "alice@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "alice@example.org".into(),
        })
        .expect("account");
    drop(store);
    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);
    let engine = SyncEngine::new(
        path.to_string_lossy().into_owned(),
        Arc::new(super::tests::Collect::default()),
    );
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());
    let inbox_count = || runtime.block_on(imap.mailbox().messages("INBOX")).len();
    let archive_count = || runtime.block_on(imap.mailbox().messages("Archive")).len();

    // Set up on this thread, so the small stack below carries only the
    // call under test.
    let inbox = engine
        .refresh_folders(account.0)
        .expect("folders")
        .expect("an inbox");
    assert_eq!(engine.sync_folder(inbox), Ok(2));
    let store = Store::open(&path).expect("store");
    let ids: Vec<i64> = store
        .list_messages(FolderId(inbox), 10, 0)
        .expect("messages")
        .into_iter()
        .map(|m| m.id.0)
        .collect();
    drop(store);
    let (first, second) = (ids[0], ids[1]);

    // Opening a message, twice: the first call opens the pooled connection.
    on_small_stack(|| assert_eq!(engine.fetch_body(second), Ok(())));
    on_small_stack(|| assert_eq!(engine.fetch_body(first), Ok(())));
    // The urgent mark covers only the body save, which the core makes on the
    // thread that writes (`save_opened_body`): the job thread isn't marked
    // when the job starts, after waiting for the connection and before
    // downloading (#473). One entry per pooled attempt (#483).
    let marks = engine.urgent_at_open.lock().expect("lock").clone();
    assert!(marks.len() >= 2, "both opens ran a pooled job: {marks:?}");
    assert!(
        marks.iter().all(|m| !m),
        "not marked while waiting or downloading: {marks:?}"
    );
    // Searching the server: pooled, under the account's writer lock. Both
    // matches are stored already, so it stores none.
    on_small_stack(|| {
        assert_eq!(engine.search_server(inbox, "Lunch".into()), Ok(0));
    });
    // A folder change on the server (its own connection).
    on_small_stack(|| {
        assert!(engine
            .create_folder(account.0, None, "Receipts".into())
            .is_ok());
    });
    assert!(runtime
        .block_on(imap.mailbox().folder_names())
        .iter()
        .any(|n| n == "Receipts"));

    // Snoozing: the replay moves the message to Snoozed on the server
    // (the path that crashed in #462).
    on_small_stack(|| {
        assert_eq!(
            engine
                .snooze_messages(vec![first], 4_000_000_000)
                .map(|a| a.len()),
            Ok(1)
        );
        assert_eq!(engine.replay_actions(account.0), Ok(0));
    });
    assert_eq!(inbox_count(), 1, "snoozed out of the inbox");

    // Same root cause, another pooled path: a plain move (archive).
    on_small_stack(|| {
        assert_eq!(
            engine.apply_action(vec![second], MessageAction::Archive),
            Ok(Some(account.0))
        );
        assert_eq!(engine.replay_actions(account.0), Ok(0));
    });
    assert_eq!((inbox_count(), archive_count()), (0, 1), "archived");

    // A folder sync (its own connection) and listing folders, likewise.
    on_small_stack(|| {
        assert_eq!(engine.sync_folder(inbox), Ok(0));
        assert_eq!(
            engine.refresh_folders(account.0).map(|i| i.is_some()),
            Ok(true)
        );
    });

    // Closing the pooled connection (LOGOUT) from a small stack, too. Every
    // connection ends with LOGOUT, so count them: exactly one more (#468).
    let logouts = || {
        runtime
            .block_on(imap.received_commands())
            .iter()
            .filter(|c| c.as_str() == "LOGOUT")
            .count()
    };
    let before = logouts();
    on_small_stack(|| engine.close_connections(CloseReason::Background));
    assert_eq!(logouts(), before + 1, "the pooled connection logged out");
    assert!(!engine.pool.is_open(account), "and is gone");
}

#[test]
fn a_job_that_could_not_run_is_reported_for_its_account() {
    // #468: a thread that didn't start (or a job that panicked, in the app)
    // sends a `SyncError` event for the account, as a runtime that couldn't
    // be built does, and the call fails with `Network`.
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("report.db").to_string_lossy().into_owned();
    let collect = Arc::new(super::tests::Collect::default());
    let engine = SyncEngine::new(path, collect.clone());
    let got: Result<u8, SyncFailure> = engine.job_outcome(AccountId(7), None);
    assert_eq!(got, Err(failed(events::SyncErrorKind::Network)));
    let events = collect.0.lock().expect("lock").clone();
    assert_eq!(events.len(), 1, "reported once");
    assert!(matches!(
        events.first(),
        Some(CoreEvent::SyncError {
            account_id: 7,
            kind: SyncErrorKind::Network
        })
    ));
    // A job that ran keeps its own result and sends nothing more.
    let ran: Result<u8, SyncFailure> = engine.job_outcome(AccountId(7), Some(Ok(3)));
    assert_eq!(ran, Ok(3));
    assert_eq!(collect.0.lock().expect("lock").len(), events.len());
}

#[test]
fn the_entry_points_report_a_job_thread_that_could_not_run() {
    // #490: the test above checks only the mapping. This one drives the real
    // entry points with every account job's thread "failing to start", so if
    // one of them went back to plain `on_job_thread` it would send no event
    // and this would fail: `refresh_folders`, `run_replaying` (an IMAP
    // sync), `run_jmap` (a JMAP sync) and `pooled` (an IMAP open).
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("wiring.db");
    let store = Store::open(&path).expect("store");
    let new_account = |email: &str| mailcrane_store::NewAccount {
        display_name: email.into(),
        email: email.into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        imap_security: mailcrane_store::Security::Tls,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 465,
        smtp_security: mailcrane_store::Security::Tls,
        username: email.into(),
    };
    let imap = store
        .insert_account(&new_account("ana@example.org"))
        .expect("imap account");
    let jmap = store
        .insert_jmap_account(&new_account("bea@example.net"), "https://127.0.0.1:9/jmap")
        .expect("jmap account");
    let inbox = |account| {
        store
            .upsert_folder(&mailcrane_store::NewFolder {
                account_id: account,
                remote_name: "INBOX".into(),
                display_name: "INBOX".into(),
                delimiter: Some("/".into()),
                role: None,
                uidvalidity: Some(1),
                uidnext: None,
                highestmodseq: None,
                is_subscribed: true,
                selectable: true,
                virtual_role: None,
            })
            .expect("inbox")
    };
    let imap_inbox = inbox(imap);
    let jmap_inbox = inbox(jmap);
    let message = store
        .upsert_message_header(&mailcrane_store::NewMessageHeader {
            folder_id: imap_inbox,
            uid: 1,
            message_id_header: None,
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: "Not opened yet".into(),
            from_name: None,
            from_addr: Some("bob@example.net".into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date: 1_790_000_000,
            internal_date: 1_790_000_000,
            size: 10,
            flags: Default::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: String::new(),
        })
        .expect("message");
    drop(store);

    let collect = Arc::new(super::tests::Collect::default());
    let engine = SyncEngine::new(path.to_string_lossy().into_owned(), collect.clone());
    engine
        .job_threads_fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let network = failed(events::SyncErrorKind::Network);
    assert_eq!(engine.refresh_folders(imap.0), Err(network.clone()));
    assert_eq!(engine.sync_folder(imap_inbox.0), Err(network.clone()));
    assert_eq!(engine.sync_folder(jmap_inbox.0), Err(network.clone()));
    assert_eq!(engine.fetch_body(message.0), Err(network));

    let reported: Vec<i64> = collect
        .0
        .lock()
        .expect("lock")
        .iter()
        .filter_map(|event| match event {
            CoreEvent::SyncError {
                account_id,
                kind: SyncErrorKind::Network,
            } => Some(*account_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        reported,
        [imap.0, imap.0, jmap.0, imap.0],
        "each entry point reported its account once"
    );
}
