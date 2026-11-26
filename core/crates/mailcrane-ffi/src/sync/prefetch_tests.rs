//! Background body downloads (M4-25) through the FFI, against the fake IMAP
//! and JMAP servers.

use super::tests::{Collect, Platform};
use super::*;
use crate::secrets::{install_secret_store, PlatformSecretStore};
use mailcrane_core::sync::prefetch::{PrefetchLimits, MIN_FREE_BYTES};
use mailcrane_store::{BodyState, NewAccount, NewFolder};
use mailcrane_testkit::{FakeImap, Fault};
use std::time::Instant;

fn on_wifi() -> DeviceConditions {
    DeviceConditions {
        setting: BackgroundDownload::WifiOnly,
        unmetered: true,
        battery_saver: false,
        free_bytes: 10 * MIN_FREE_BYTES,
    }
}

pub(super) fn runtime_for_fakes() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime")
}

pub(super) fn raws(count: usize) -> Vec<Vec<u8>> {
    (1..=count)
        .map(|n| format!("Subject: Message {n}\r\n\r\nBody {n}\r\n").into_bytes())
        .collect()
}

/// An engine with one IMAP account whose INBOX is on `imap`.
pub(super) fn imap_engine(
    dir: &tempfile::TempDir,
    imap: &FakeImap,
) -> (Arc<SyncEngine>, String, FolderId) {
    imap_engine_with(dir, imap, Arc::new(Collect::default()))
}

/// [`imap_engine`] sending events to `listener`.
pub(super) fn imap_engine_with(
    dir: &tempfile::TempDir,
    imap: &FakeImap,
    listener: Arc<Collect>,
) -> (Arc<SyncEngine>, String, FolderId) {
    let path = dir.path().join("p.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_account(&NewAccount {
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
    let folder = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "INBOX".into(),
            delimiter: Some("/".into()),
            role: Some(FolderRole::Inbox),
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("folder");
    drop(store);
    let platform = Arc::new(Platform::default());
    platform
        .put(
            SecretKey::account_password(account.0).as_str().to_owned(),
            b"pw".to_vec(),
        )
        .expect("password");
    install_secret_store(platform);
    let engine = SyncEngine::new(path.clone(), listener);
    *engine.extra_root_cert.lock().expect("lock") = Some(imap.root_cert_der().to_vec());
    (engine, path, folder)
}

fn cached(path: &str, folder: FolderId) -> usize {
    Store::open(path)
        .expect("store")
        .list_messages(folder, 1000, 0)
        .expect("list")
        .iter()
        .filter(|m| m.body_state == BodyState::Fetched)
        .count()
}

/// How long a wait may go without progress before the test calls it a hang.
const STALLED: Duration = Duration::from_secs(60);
/// A cap on any one wait however steady its progress: only a hang guard.
const HANG_GUARD: Duration = Duration::from_secs(300);

/// Polls `done` every `poll` until it holds. A loaded CI runner can be many
/// times slower than a quiet machine (#388), so there is no tight deadline:
/// the wait fails only when `progress` has not risen for [`STALLED`], or
/// after [`HANG_GUARD`] in all. A fast run returns as soon as `done` holds.
fn wait_while_progressing(
    what: &str,
    poll: Duration,
    mut progress: impl FnMut() -> usize,
    mut done: impl FnMut() -> bool,
) {
    let started = Instant::now();
    let mut last = progress();
    let mut last_rise = started;
    while !done() {
        let now = progress();
        if now > last {
            last = now;
            last_rise = Instant::now();
        }
        assert!(
            last_rise.elapsed() < STALLED,
            "{what}: no progress for {STALLED:?} (stuck at {last})"
        );
        assert!(
            started.elapsed() < HANG_GUARD,
            "{what}: not done after {HANG_GUARD:?} (at {last})"
        );
        std::thread::sleep(poll);
    }
}

/// Waits for every running download to finish, for as long as bodies keep
/// arriving in `folder`.
fn wait_for_downloads(engine: &SyncEngine, path: &str, folder: FolderId) {
    wait_while_progressing(
        "downloads",
        Duration::from_millis(50),
        || cached(path, folder),
        || engine.prefetch.wait(Duration::ZERO),
    );
}

#[test]
fn a_sync_on_wifi_downloads_recent_bodies_so_opening_needs_no_network() {
    let runtime = runtime_for_fakes();
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws(30))
            .start()
            .await
            .expect("fake imap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);

    // Until the app reports the conditions, nothing is downloaded.
    assert_eq!(engine.sync_folder(folder.0), Ok(30));
    wait_for_downloads(&engine, &path, folder);
    assert_eq!(cached(&path, folder), 0);
    // Nor on mobile data with the default setting, nor with "Never".
    for conditions in [
        DeviceConditions {
            unmetered: false,
            ..on_wifi()
        },
        DeviceConditions {
            setting: BackgroundDownload::Never,
            ..on_wifi()
        },
    ] {
        engine.set_device_conditions(conditions);
        assert!(engine.sync_folder(folder.0).is_ok());
        wait_for_downloads(&engine, &path, folder);
        assert_eq!(cached(&path, folder), 0);
    }

    engine.set_device_conditions(on_wifi());
    assert!(engine.sync_folder(folder.0).is_ok());
    wait_for_downloads(&engine, &path, folder);
    assert_eq!(cached(&path, folder), 30);
    // Nothing became \Seen on the server.
    for uid in 1..=30 {
        let flags = runtime
            .block_on(imap.mailbox().flags("INBOX", uid))
            .expect("flags");
        assert!(!flags.iter().any(|f| f == "\\Seen"), "uid {uid}");
    }

    // Opening a downloaded message connects to nothing.
    let message = Store::open(&path)
        .expect("store")
        .list_messages(folder, 1, 0)
        .expect("list")
        .pop()
        .expect("a message");
    let connections = imap.connections_accepted();
    assert_eq!(engine.fetch_body(message.id.0), Ok(()));
    assert_eq!(imap.connections_accepted(), connections);
}

#[test]
fn opening_a_message_during_a_slow_download_waits_for_at_most_one_batch() {
    let runtime = runtime_for_fakes();
    // Every command takes 60 ms; 200 messages are 10 batches of 20.
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws(300))
            .fault(Fault::Delay(Duration::from_millis(60)))
            .start()
            .await
            .expect("fake imap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);
    assert_eq!(engine.sync_folder(folder.0), Ok(200));
    // The message the download would reach last, and the account's gate:
    // looked up now, so no store read after the download starts lets more
    // batches run before the open.
    let store = Store::open(&path).expect("store");
    let last = mailcrane_core::sync::prefetch::candidates(
        &store,
        folder,
        unix_now(),
        &PrefetchLimits::default(),
        &std::collections::HashSet::new(),
    )
    .expect("candidates")
    .pop()
    .expect("a candidate");
    let account = store
        .get_folder(folder)
        .expect("read")
        .expect("folder")
        .details
        .account_id;
    drop(store);
    let gate = engine.account_lock(account);

    // Since the sync, every UID FETCH is a download batch, or the open's.
    let uid_fetches = |imap: &FakeImap, runtime: &tokio::runtime::Handle| {
        runtime
            .block_on(imap.received_commands())
            .iter()
            .filter(|c| c.as_str() == "UID FETCH")
            .count()
    };
    let synced_fetches = uid_fetches(&imap, runtime.handle());
    engine.set_device_conditions(on_wifi());
    engine.prefetch_after_sync(folder);
    // Polled often, so the open below comes soon after the first batch. The
    // first batch is the first progress there is, so only the stall guard
    // (connecting, logging in, one batch) applies here.
    wait_while_progressing(
        "the first batch",
        Duration::from_millis(5),
        || 0,
        || cached(&path, folder) >= 20,
    );

    // Opening announces itself on the account's gate, and from then on the
    // download starts no new batch. Count the batches at that very moment,
    // from inside the announcement: on a loaded runner the open's local reads
    // before it can take as long as several batches (#388), and an open that
    // announced late, or not at all, must fail here.
    let at_announce = Arc::new(std::sync::Mutex::new(None::<usize>));
    {
        let at_announce = at_announce.clone();
        let imap = imap.clone();
        let handle = runtime.handle().clone();
        gate.set_on_announce(Some(Box::new(move || {
            let count = uid_fetches(&imap, &handle);
            at_announce
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get_or_insert(count);
        })));
    }
    assert_eq!(engine.fetch_body(last.id.0), Ok(()));
    gate.set_on_announce(None);
    let before = at_announce
        .lock()
        .expect("lock")
        .expect("opening didn't announce itself")
        .saturating_sub(synced_fetches);
    assert!(before < 10, "the download is still running");
    // At most the batch that had just been sent, then the open's own fetch.
    let after = uid_fetches(&imap, runtime.handle()).saturating_sub(synced_fetches);
    assert!(
        after <= before + 2,
        "{before} fetches when opening announced itself, {after} after"
    );
    // The open's own fetch comes after its announcement.
    assert!(
        after > before,
        "the open fetched before it announced itself"
    );
    assert!(cached(&path, folder) < 200);
    // The download then carries on to the end.
    wait_for_downloads(&engine, &path, folder);
    assert_eq!(cached(&path, folder), 200);
}

#[test]
fn battery_saver_and_low_storage_stop_downloads() {
    let runtime = runtime_for_fakes();
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws(5))
            .start()
            .await
            .expect("fake imap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, folder) = imap_engine(&dir, &imap);
    assert_eq!(engine.sync_folder(folder.0), Ok(5));
    for conditions in [
        DeviceConditions {
            battery_saver: true,
            ..on_wifi()
        },
        DeviceConditions {
            free_bytes: MIN_FREE_BYTES,
            ..on_wifi()
        },
    ] {
        engine.set_device_conditions(conditions);
        engine.prefetch_after_sync(folder);
        wait_for_downloads(&engine, &path, folder);
        assert_eq!(cached(&path, folder), 0);
    }
    // Messages older than the age limit are left too.
    engine.prefetch.set_limits(PrefetchLimits {
        max_age_secs: -3600,
        ..PrefetchLimits::default()
    });
    engine.set_device_conditions(on_wifi());
    engine.prefetch_after_sync(folder);
    wait_for_downloads(&engine, &path, folder);
    assert_eq!(cached(&path, folder), 0);
}

#[test]
fn a_jmap_inbox_is_downloaded_too_without_setting_seen() {
    use mailcrane_testkit::{FakeEmail, FakeJmap};
    let runtime = runtime_for_fakes();
    let emails: Vec<FakeEmail> = (1..=3)
        .map(|n| FakeEmail {
            id: format!("m{n}"),
            mailbox_id: "mb-inbox".into(),
            received_at: format!("2026-11-2{n}T08:00:00Z"),
            subject: format!("Hello {n}"),
            from: "bob@example.net".into(),
            seen: false,
            raw: format!("From: bob@example.net\r\nSubject: Hello {n}\r\n\r\nJMAP body {n}\r\n"),
        })
        .collect();
    let server = runtime.block_on(async {
        let mut builder = FakeJmap::builder().user("ana@example.org", "pw");
        for email in emails {
            builder = builder.email(email);
        }
        builder.start().await.expect("fake jmap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("j.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Ana".into(),
                email: "ana@example.org".into(),
                imap_host: String::new(),
                imap_port: 0,
                imap_security: Security::Tls,
                smtp_host: String::new(),
                smtp_port: 0,
                smtp_security: Security::Tls,
                username: "ana@example.org".into(),
            },
            &server.base_url(),
        )
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
    let engine = SyncEngine::new(path.clone(), Arc::new(Collect::default()));
    *engine.extra_root_cert.lock().expect("lock") = Some(server.root_cert_der().to_vec());
    // The fake's dates are fixed; don't let the age limit depend on today.
    engine.prefetch.set_limits(PrefetchLimits {
        max_age_secs: i64::MAX,
        ..PrefetchLimits::default()
    });
    engine.set_device_conditions(on_wifi());

    let inbox = engine
        .refresh_folders(account.0)
        .expect("folders")
        .expect("an inbox");
    assert_eq!(engine.sync_folder(inbox), Ok(3));
    wait_for_downloads(&engine, &path, FolderId(inbox));
    assert_eq!(cached(&path, FolderId(inbox)), 3);
    for n in 1..=3 {
        let keywords = server.email_keywords(&format!("m{n}")).expect("email");
        assert!(!keywords.iter().any(|k| k == "$seen"), "m{n}: {keywords:?}");
    }
}

#[test]
fn live_updates_and_a_download_share_one_writer_and_never_hit_busy() {
    let runtime = runtime_for_fakes();
    // Slow enough that new mail arrives while the download runs.
    let imap = runtime.block_on(async {
        FakeImap::builder()
            .user("alice@example.org", "pw")
            .folder("INBOX", raws(200))
            .fault(Fault::Delay(Duration::from_millis(20)))
            .start()
            .await
            .expect("fake imap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let collect = Arc::new(Collect::default());
    let (engine, path, folder) = imap_engine_with(&dir, &imap, collect.clone());
    let account = Store::open(&path)
        .expect("store")
        .get_folder(folder)
        .expect("read")
        .expect("folder")
        .details
        .account_id;
    assert_eq!(engine.sync_folder(folder.0), Ok(200));
    assert_eq!(engine.start_live_updates(account.0), Ok(()));

    engine.set_device_conditions(on_wifi());
    engine.prefetch_after_sync(folder);
    // New mail while the download runs: live updates store it (and start
    // downloads of their own, which join the running one).
    for n in 201..=205 {
        runtime.block_on(imap.mailbox().deliver(
            "INBOX",
            format!("Subject: Message {n}\r\n\r\nBody {n}\r\n").into_bytes(),
        ));
        std::thread::sleep(Duration::from_millis(150));
    }
    let stored = || {
        Store::open(&path)
            .expect("store")
            .list_uids(folder)
            .expect("uids")
            .len()
    };
    wait_while_progressing(
        "live updates and downloads",
        Duration::from_millis(50),
        || stored() + cached(&path, folder),
        || stored() >= 205 && cached(&path, folder) >= 200 && engine.prefetch.wait(Duration::ZERO),
    );
    // Downloads started by the last live sync may still be running.
    wait_for_downloads(&engine, &path, folder);
    engine.stop_live_updates(account.0);

    // The new messages were downloaded too (the window moves with them).
    let fresh = Store::open(&path)
        .expect("store")
        .list_messages(folder, 1000, 0)
        .expect("list")
        .into_iter()
        .filter(|m| m.details.uid > 200)
        .filter(|m| m.body_state == BodyState::Fetched)
        .count();
    assert_eq!(fresh, 5);
    let errors: Vec<CoreEvent> = collect
        .0
        .lock()
        .expect("lock")
        .iter()
        .filter(|e| matches!(e, CoreEvent::SyncError { .. }))
        .cloned()
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}
