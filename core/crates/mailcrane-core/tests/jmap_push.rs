//! JMAP push subscriptions (M4-27, RFC 8620 §7.2), against the fake server:
//! create, verify, refresh, a new endpoint, a lost verification, and
//! teardown. The fake records the pushes it would send; the tests hand them
//! back as the Web Push service would. Never a real server.

#![allow(clippy::expect_used, clippy::panic)] // test helpers may panic

use mailcrane_core::sync::push::{
    register, teardown, verify, PushTarget, Registered, LIFETIME, REFRESH_BEFORE, VERIFY_WITHIN,
};
use mailcrane_core::sync::SyncError;
use mailcrane_jmap::push::{parse_push, utc_date};
use mailcrane_jmap::{Auth, ClientConfig, PushKeys, PushObject, Session};
use mailcrane_store::{AccountId, NewAccount, Security, Store};
use mailcrane_testkit::{FakeEmail, FakeJmap};
use std::time::Duration;

/// Only a hang guard: a busy machine (a Gradle build, disk or memory
/// pressure) can stall the fake servers for seconds, and a short limit
/// would fail the test for that alone (#409, as in #362).
const HANG_GUARD: Duration = Duration::from_secs(60);

const NOW: i64 = 1_764_144_000; // 2025-11-26T08:00:00Z
const URL: &str = "https://push.example.org/up/one";

struct Setup {
    server: FakeJmap,
    store: Store,
    account: AccountId,
    config: ClientConfig,
    session: Session,
    auth: Auth,
}

async fn setup() -> Setup {
    let server = FakeJmap::builder().start().await.expect("server");
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_jmap_account(
            &NewAccount {
                display_name: "Alice".into(),
                email: "alice@example.org".into(),
                imap_host: String::new(),
                imap_port: 0,
                imap_security: Security::Tls,
                smtp_host: String::new(),
                smtp_port: 0,
                smtp_security: Security::Tls,
                username: "alice@example.org".into(),
            },
            &server.base_url(),
        )
        .expect("account");
    let config = ClientConfig {
        timeout: HANG_GUARD,
        extra_roots: vec![server.root_cert_der().to_vec()],
    };
    let auth = Auth::Bearer("token".into());
    let session = mailcrane_jmap::discover(&server.base_url(), &auth, &config)
        .await
        .expect("session");
    Setup {
        server,
        store,
        account,
        config,
        session,
        auth,
    }
}

fn target(url: &str) -> PushTarget {
    PushTarget {
        url: url.into(),
        keys: Some(PushKeys {
            p256dh: "BPublicKeyExample".into(),
            auth: "authSecretExample".into(),
        }),
    }
}

impl Setup {
    async fn register(&self, t: &PushTarget, now: i64) -> Result<Registered, SyncError> {
        register(
            &self.store,
            self.account,
            &self.session,
            &self.auth,
            &self.config,
            t,
            now,
        )
        .await
    }

    /// Answers the one verification push the fake sent, as the app would.
    async fn answer_verification(&self) {
        let pushes = self.server.take_pushes();
        assert_eq!(pushes.len(), 1, "{pushes:?}");
        let Ok(PushObject::Verification {
            subscription_id,
            code,
        }) = parse_push(pushes[0].body.as_bytes())
        else {
            panic!("a verification: {pushes:?}");
        };
        verify(
            &self.store,
            self.account,
            &self.session,
            &self.auth,
            &self.config,
            &subscription_id,
            &code,
        )
        .await
        .expect("verified");
    }
}

#[tokio::test]
async fn a_subscription_is_created_with_keys_and_verified() {
    let s = setup().await;
    assert_eq!(
        s.register(&target(URL), NOW).await.expect("register"),
        Registered::Created
    );
    let subs = s.server.push_subscriptions();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].url, URL);
    assert_eq!(subs[0].types, ["Email", "Mailbox"]);
    assert_eq!(
        subs[0].keys,
        Some(("BPublicKeyExample".into(), "authSecretExample".into()))
    );
    assert_eq!(
        subs[0].expires.as_deref(),
        Some(utc_date(NOW + LIFETIME).as_str())
    );
    assert!(!subs[0].verified);
    // Waiting for the verification: nothing sent again.
    assert_eq!(
        s.register(&target(URL), NOW + 60).await.expect("again"),
        Registered::Waiting
    );
    assert_eq!(s.server.push_subscriptions().len(), 1);

    s.answer_verification().await;
    assert!(s.server.push_subscriptions()[0].verified);
    let row = s
        .store
        .push_subscription(s.account)
        .expect("read")
        .expect("row");
    assert!(row.verified);
    // The auth secret is never stored; the public key is.
    assert_eq!(row.p256dh.as_deref(), Some("BPublicKeyExample"));

    // Verified: new mail is pushed as a StateChange for the account.
    s.server.deliver(FakeEmail {
        id: "m9".into(),
        mailbox_id: "mb-inbox".into(),
        received_at: "2026-11-26T08:00:00Z".into(),
        subject: "Hi".into(),
        from: "bob@example.org".into(),
        seen: false,
        raw: "From: bob@example.org\r\nSubject: Hi\r\n\r\nHi\r\n".into(),
    });
    let pushes = s.server.take_pushes();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0].url, URL);
    assert_eq!(
        parse_push(pushes[0].body.as_bytes()),
        Ok(PushObject::StateChange {
            account_ids: vec!["A1".into()]
        })
    );
    // And with nothing to do, registering again sends nothing.
    let before = s.server.requests().await.len();
    assert_eq!(
        s.register(&target(URL), NOW + 3600).await.expect("again"),
        Registered::Unchanged
    );
    assert_eq!(s.server.requests().await.len(), before);
}

#[tokio::test]
async fn a_wrong_or_foreign_verification_is_refused() {
    let s = setup().await;
    s.register(&target(URL), NOW).await.expect("register");
    let id = s.server.push_subscriptions()[0].id.clone();
    let wrong = verify(
        &s.store, s.account, &s.session, &s.auth, &s.config, &id, "nope",
    )
    .await;
    assert!(matches!(wrong, Err(SyncError::Jmap(_))), "{wrong:?}");
    let foreign = verify(
        &s.store, s.account, &s.session, &s.auth, &s.config, "P-999", "code",
    )
    .await;
    assert!(
        matches!(foreign, Err(SyncError::UnknownPushSubscription)),
        "{foreign:?}"
    );
    assert!(
        !s.store
            .push_subscription(s.account)
            .expect("read")
            .expect("row")
            .verified
    );
}

#[tokio::test]
async fn it_is_refreshed_before_it_expires_and_takes_the_servers_shorter_time() {
    let s = setup().await;
    s.register(&target(URL), NOW).await.expect("register");
    s.answer_verification().await;
    // Not yet close to expiring.
    let later = NOW + LIFETIME - REFRESH_BEFORE - 60;
    assert_eq!(
        s.register(&target(URL), later).await.expect("early"),
        Registered::Unchanged
    );
    // Within the refresh window; the server allows less than asked.
    let soon = NOW + LIFETIME - REFRESH_BEFORE + 60;
    let cap = soon + 24 * 3600;
    s.server.cap_push_expiry(&utc_date(cap));
    assert_eq!(
        s.register(&target(URL), soon).await.expect("refresh"),
        Registered::Refreshed
    );
    let row = s
        .store
        .push_subscription(s.account)
        .expect("read")
        .expect("row");
    assert_eq!(row.expires, Some(cap));
    assert_eq!(
        s.server.push_subscriptions().len(),
        1,
        "the same subscription"
    );
}

#[tokio::test]
async fn a_new_endpoint_replaces_the_subscription() {
    let s = setup().await;
    s.register(&target(URL), NOW).await.expect("register");
    s.answer_verification().await;
    let old = s.server.push_subscriptions()[0].clone();

    let next = "https://push.example.org/up/two";
    assert_eq!(
        s.register(&target(next), NOW + 60).await.expect("new"),
        Registered::Created
    );
    let subs = s.server.push_subscriptions();
    assert_eq!(subs.len(), 1, "the old one is destroyed: {subs:?}");
    assert_eq!(subs[0].url, next);
    assert_ne!(subs[0].id, old.id);
    // The same install keeps its deviceClientId.
    assert_eq!(subs[0].device_client_id, old.device_client_id);
    assert!(
        !s.store
            .push_subscription(s.account)
            .expect("read")
            .expect("row")
            .verified
    );

    // New keys with the same URL count as a new endpoint too.
    s.answer_verification().await;
    let mut rekeyed = target(next);
    rekeyed.keys = Some(PushKeys {
        p256dh: "BOtherKey".into(),
        auth: "otherAuth".into(),
    });
    assert_eq!(
        s.register(&rekeyed, NOW + 120).await.expect("rekey"),
        Registered::Created
    );
}

#[tokio::test]
async fn a_lost_verification_makes_it_again() {
    let s = setup().await;
    s.register(&target(URL), NOW).await.expect("register");
    let _lost = s.server.take_pushes();
    assert_eq!(
        s.register(&target(URL), NOW + VERIFY_WITHIN + 1)
            .await
            .expect("again"),
        Registered::Created
    );
    assert_eq!(s.server.push_subscriptions().len(), 1);
    s.answer_verification().await;
}

#[tokio::test]
async fn teardown_destroys_it_and_forgets_it_even_if_the_server_already_had() {
    let s = setup().await;
    s.register(&target(URL), NOW).await.expect("register");
    s.answer_verification().await;
    teardown(&s.store, s.account, &s.session, &s.auth, &s.config)
        .await
        .expect("teardown");
    assert!(s.server.push_subscriptions().is_empty());
    assert_eq!(s.store.push_subscription(s.account).expect("read"), None);
    // Nothing left: a second teardown is a no-op.
    teardown(&s.store, s.account, &s.session, &s.auth, &s.config)
        .await
        .expect("again");

    // The server dropped it on its own (it expired there): still fine.
    s.register(&target(URL), NOW + 60).await.expect("register");
    let id = s.server.push_subscriptions()[0].id.clone();
    mailcrane_jmap::push::destroy_push_subscription(&s.session, &s.auth, &s.config, &id)
        .await
        .expect("server-side drop");
    teardown(&s.store, s.account, &s.session, &s.auth, &s.config)
        .await
        .expect("notFound counts as gone");
    assert_eq!(s.store.push_subscription(s.account).expect("read"), None);
}

#[tokio::test]
async fn a_plain_http_endpoint_is_refused_before_anything_is_sent() {
    let s = setup().await;
    let before = s.server.requests().await.len();
    let plain = s.register(&target("http://push.example.org/up"), NOW).await;
    assert!(matches!(plain, Err(SyncError::Jmap(_))), "{plain:?}");
    assert_eq!(s.server.requests().await.len(), before);
    assert_eq!(s.store.push_subscription(s.account).expect("read"), None);
}
