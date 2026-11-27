//! Stored-account login and folder refresh against the testkit's fake IMAP
//! server (#520). Never a real mailbox.

use super::*;
use crate::secrets::{Secret as StoredSecret, SecretError, SecretKey};
use mailcrane_store::{NewAccount, Security};
use mailcrane_testkit::FakeImap;
use std::sync::Mutex;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";

/// Secrets in memory, for these tests.
#[derive(Default)]
struct Secrets(Mutex<Vec<(String, Vec<u8>)>>);

impl SecretStore for Secrets {
    fn put(&self, key: &SecretKey, value: &StoredSecret) -> Result<(), SecretError> {
        let mut all = self.0.lock().expect("lock");
        all.retain(|(k, _)| k != key.as_str());
        all.push((key.as_str().to_owned(), value.expose().to_vec()));
        Ok(())
    }
    fn get(&self, key: &SecretKey) -> Result<Option<StoredSecret>, SecretError> {
        Ok(self
            .0
            .lock()
            .expect("lock")
            .iter()
            .find(|(k, _)| k == key.as_str())
            .map(|(_, v)| StoredSecret::new(v.clone())))
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.0
            .lock()
            .expect("lock")
            .retain(|(k, _)| k != key.as_str());
        Ok(())
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<CoreEvent>>);

impl EventListener for Events {
    fn on_event(&self, event: CoreEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

async fn fake(folders: &[&str]) -> FakeImap {
    let mut builder = FakeImap::builder().user(USER, PASSWORD);
    for name in folders {
        builder = builder.folder(*name, Vec::new());
    }
    builder.start().await.expect("fake imap starts")
}

/// An account on `imap` with `password` in `secrets`.
fn account(store: &Store, secrets: &Secrets, imap: &FakeImap, password: &str) -> AccountId {
    let id = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: imap.addr().port(),
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: USER.into(),
        })
        .expect("account");
    secrets
        .put(
            &SecretKey::account_password(id.0),
            &StoredSecret::new(password.as_bytes().to_vec()),
        )
        .expect("password");
    id
}

#[tokio::test]
async fn a_stored_account_logs_in_and_its_folders_are_saved() {
    let imap = fake(&["INBOX", "Archive", "Work"]).await;
    let store = Store::open_in_memory().expect("store");
    let secrets = Secrets::default();
    let id = account(&store, &secrets, &imap, PASSWORD);
    let events = Events::default();

    let mut session = connect(&store, &secrets, id, Some(imap.root_cert_der()))
        .await
        .expect("logs in");
    refresh_folders(&store, &mut session, id, &events)
        .await
        .expect("refreshes");
    session.logout().await;
    let inbox = inbox_of(&store, id).expect("inbox");

    let mut names: Vec<String> = store
        .list_folders(id)
        .expect("folders")
        .into_iter()
        .map(|f| f.details.remote_name)
        .collect();
    names.sort();
    assert_eq!(names, ["Archive", "INBOX", "Work"]);
    let inbox_name = store
        .list_folders(id)
        .expect("folders")
        .into_iter()
        .find(|f| Some(f.id) == inbox)
        .map(|f| f.details.remote_name);
    assert_eq!(inbox_name.as_deref(), Some("INBOX"));
}

#[tokio::test]
async fn a_folder_gone_from_the_server_is_removed_and_reported() {
    let imap = fake(&["INBOX", "Work"]).await;
    let store = Store::open_in_memory().expect("store");
    let secrets = Secrets::default();
    let id = account(&store, &secrets, &imap, PASSWORD);
    let cert = imap.root_cert_der().to_vec();
    let events = Events::default();
    let mut session = connect(&store, &secrets, id, Some(&cert))
        .await
        .expect("logs in");
    refresh_folders(&store, &mut session, id, &events)
        .await
        .expect("first");
    let work = store
        .list_folders(id)
        .expect("folders")
        .into_iter()
        .find(|f| f.details.remote_name == "Work")
        .map(|f| f.id)
        .expect("work");

    imap.mailbox().delete_folder("Work").await;
    refresh_folders(&store, &mut session, id, &events)
        .await
        .expect("second");
    session.logout().await;

    assert!(store
        .list_folders(id)
        .expect("folders")
        .iter()
        .all(|f| f.details.remote_name != "Work"));
    assert!(events
        .0
        .lock()
        .expect("lock")
        .contains(&CoreEvent::FolderChanged { folder_id: work }));
}

#[tokio::test]
async fn a_wrong_password_is_an_auth_error() {
    let imap = fake(&["INBOX"]).await;
    let store = Store::open_in_memory().expect("store");
    let secrets = Secrets::default();
    let id = account(&store, &secrets, &imap, "not-the-password");
    let result = connect(&store, &secrets, id, Some(imap.root_cert_der())).await;
    assert!(matches!(result, Err(SyncErrorKind::Auth)));
}

#[test]
fn no_login_without_a_password_or_for_an_unknown_account() {
    let store = Store::open_in_memory().expect("store");
    let secrets = Secrets::default();
    assert!(matches!(
        imap_login(&store, &secrets, AccountId(99), None),
        Err(SyncErrorKind::NotFound)
    ));
    let id = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: USER.into(),
        })
        .expect("account");
    assert!(matches!(
        imap_login(&store, &secrets, id, None),
        Err(SyncErrorKind::Auth)
    ));
}

/// Fails every read, so a test sees whether a secret was asked for.
struct NoReads;

impl SecretStore for NoReads {
    fn put(&self, _: &SecretKey, _: &StoredSecret) -> Result<(), SecretError> {
        panic!("no secret may be written");
    }
    fn get(&self, _: &SecretKey) -> Result<Option<StoredSecret>, SecretError> {
        panic!("no secret may be read");
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        panic!("no secret may be deleted");
    }
}

#[tokio::test]
async fn a_local_or_missing_account_never_reads_a_secret_or_connects() {
    // #289, #550 review: refused before any secret is read.
    let store = Store::open_in_memory().expect("store");
    let local = store
        .insert_local_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: USER.into(),
        })
        .expect("account");
    for account in [local, AccountId(99)] {
        assert!(matches!(
            connect(&store, &NoReads, account, None).await,
            Err(SyncErrorKind::NotFound)
        ));
        assert!(matches!(
            imap_login(&store, &NoReads, account, None),
            Err(SyncErrorKind::NotFound)
        ));
    }
}
