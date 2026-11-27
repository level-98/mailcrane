//! JMAP push through the FFI (M4-27), against the fake JMAP server: the
//! app's whole path from registering the endpoint to turning push off.

use super::prefetch_tests::runtime_for_fakes;
use super::tests::{Collect, Platform};
use super::*;
use crate::secrets::{install_secret_store, PlatformSecretStore};
use mailcrane_store::NewAccount;
use mailcrane_testkit::{FakeJmap, FakePush};

const ENDPOINT: &str = "https://push.example.org/up/abc";

fn jmap_engine(dir: &tempfile::TempDir, server: &FakeJmap) -> (Arc<SyncEngine>, String, AccountId) {
    let path = dir.path().join("p.db").to_string_lossy().into_owned();
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
    (engine, path, account)
}

fn only(pushes: Vec<FakePush>) -> Vec<u8> {
    assert_eq!(pushes.len(), 1, "{pushes:?}");
    pushes[0].body.clone().into_bytes()
}

#[test]
fn register_answer_and_unregister_through_the_engine() {
    let runtime = runtime_for_fakes();
    let server = runtime.block_on(async {
        FakeJmap::builder()
            .user("ana@example.org", "pw")
            .start()
            .await
            .expect("fake jmap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, account) = jmap_engine(&dir, &server);

    // Keys come both or not at all.
    assert!(engine
        .register_jmap_push(ENDPOINT.into(), Some("k".into()), None)
        .is_err());
    assert_eq!(
        engine.register_jmap_push(ENDPOINT.into(), Some("BKey".into()), Some("auth".into())),
        Ok(1)
    );
    let subs = server.push_subscriptions();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].url, ENDPOINT);

    // The verification is sorted cheaply, then answered.
    let verification = only(server.take_pushes());
    assert_eq!(classify_push(verification.clone()), PushBody::Verification);
    assert!(engine.answer_jmap_push(verification.clone()));
    assert!(server.push_subscriptions()[0].verified);
    let row = Store::open(&path)
        .expect("store")
        .push_subscription(account)
        .expect("read")
        .expect("row");
    assert!(row.verified);

    // Anything else is a wake-up, never answered.
    assert_eq!(
        classify_push(br#"{"@type":"StateChange","changed":{"A1":{}}}"#.to_vec()),
        PushBody::Wake
    );
    assert_eq!(classify_push(Vec::new()), PushBody::Wake);
    assert_eq!(classify_push(b"\xff not json".to_vec()), PushBody::Wake);
    assert!(!engine.answer_jmap_push(b"{}".to_vec()));
    // A verification for a subscription this app doesn't have.
    assert!(!engine.answer_jmap_push(
        br#"{"@type":"PushVerification","pushSubscriptionId":"P-404","verificationCode":"x"}"#
            .to_vec()
    ));

    engine.unregister_jmap_push();
    assert!(server.push_subscriptions().is_empty());
    assert!(Store::open(&path)
        .expect("store")
        .push_subscriptions()
        .expect("list")
        .is_empty());
}

#[test]
fn unregistering_forgets_subscriptions_even_when_the_server_is_gone() {
    let runtime = runtime_for_fakes();
    let server = runtime.block_on(async {
        FakeJmap::builder()
            .user("ana@example.org", "pw")
            .start()
            .await
            .expect("fake jmap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, _account) = jmap_engine(&dir, &server);
    assert_eq!(
        engine.register_jmap_push(ENDPOINT.into(), None, None),
        Ok(1)
    );
    runtime.block_on(server.shutdown());

    engine.unregister_jmap_push();
    assert!(Store::open(&path)
        .expect("store")
        .push_subscriptions()
        .expect("list")
        .is_empty());
    // And registering with the server gone fails per account, not overall.
    assert_eq!(
        engine.register_jmap_push(ENDPOINT.into(), None, None),
        Ok(0)
    );
}

#[test]
fn removing_the_account_destroys_its_subscription_on_the_server() {
    let runtime = runtime_for_fakes();
    let server = runtime.block_on(async {
        FakeJmap::builder()
            .user("ana@example.org", "pw")
            .start()
            .await
            .expect("fake jmap")
    });
    let dir = tempfile::tempdir().expect("tempdir");
    let (engine, path, account) = jmap_engine(&dir, &server);
    assert_eq!(
        engine.register_jmap_push(ENDPOINT.into(), None, None),
        Ok(1)
    );
    assert_eq!(server.push_subscriptions().len(), 1);

    // What remove_account runs first, with the fake's certificate.
    let store = Store::open(&path).expect("store");
    // A hang guard, not the production 5 s limit: a busy machine can stall
    // the fake for seconds (#487).
    crate::account::destroy_push_subscription(
        &store,
        account,
        Some(server.root_cert_der().to_vec()),
        std::time::Duration::from_secs(60),
    );
    assert!(server.push_subscriptions().is_empty());
    assert_eq!(store.push_subscription(account).expect("read"), None);
}
