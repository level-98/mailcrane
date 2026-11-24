//! Connecting, logging in and listing folders against the testkit's fake
//! IMAP server. Never a real mailbox.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

use mailcrane_imap::{save_folders, ImapConfig, ImapError, ImapSecurity, Secret, Session};
use mailcrane_store::{FolderRole, NewAccount, NewFolder, Security, Store};
use mailcrane_testkit::{FakeImap, Fault, Quirk};
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const BOTH: [ImapSecurity; 2] = [ImapSecurity::ImplicitTls, ImapSecurity::StartTls];

fn config(imap: &FakeImap, security: ImapSecurity) -> ImapConfig {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), security)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_millis(500);
    config.connect_timeout = Duration::from_secs(5);
    config
}

async fn fake(faults: &[Fault], quirks: &[Quirk]) -> FakeImap {
    let mut builder = FakeImap::builder().user(USER, PASSWORD);
    for fault in faults {
        builder = builder.fault(fault.clone());
    }
    for quirk in quirks {
        builder = builder.quirk(*quirk);
    }
    builder.start().await.expect("fake imap starts")
}

async fn connect(config: &ImapConfig, password: &str) -> Result<Session, ImapError> {
    // Any hang is a failure: the client's own timeouts must fire first.
    tokio::time::timeout(
        Duration::from_secs(10),
        Session::connect(config, USER, &Secret::new(password)),
    )
    .await
    .expect("connect must not hang")
}

fn credentials_sent(commands: &[String]) -> bool {
    commands.iter().any(|c| c == "LOGIN" || c == "AUTHENTICATE")
}

#[tokio::test]
async fn logs_in_over_implicit_tls_and_starttls() {
    for security in BOTH {
        let imap = fake(&[], &[]).await;
        let mut session = connect(&config(&imap, security), PASSWORD)
            .await
            .expect("logged in");
        assert!(session.capabilities().iter().any(|c| c == "AUTH=PLAIN"));
        // After STARTTLS the pre-TLS capabilities are gone.
        assert!(!session
            .capabilities()
            .iter()
            .any(|c| c == "STARTTLS" || c == "LOGINDISABLED"));
        let folders = session.list_folders().await.expect("list");
        assert!(folders.iter().any(|f| f.role == Some(FolderRole::Inbox)));
        session.logout().await;

        let commands = imap.received_commands().await;
        if security == ImapSecurity::StartTls {
            // Only CAPABILITY and STARTTLS before TLS; AUTHENTICATE after.
            let starttls = commands.iter().position(|c| c == "STARTTLS");
            let auth = commands.iter().position(|c| c == "AUTHENTICATE");
            assert!(
                matches!((starttls, auth), (Some(s), Some(a)) if s < a),
                "{commands:?}"
            );
            assert_eq!(commands.first().map(String::as_str), Some("CAPABILITY"));
        }
    }
}

#[tokio::test]
async fn no_starttls_is_an_error_and_no_credentials_are_sent() {
    let imap = fake(&[Fault::NoStartTls], &[]).await;
    let result = connect(&config(&imap, ImapSecurity::StartTls), PASSWORD).await;
    assert!(
        matches!(result, Err(ImapError::StartTlsNotOffered)),
        "{result:?}"
    );
    let commands = imap.received_commands().await;
    assert!(!credentials_sent(&commands), "{commands:?}");
    assert_eq!(commands, ["CAPABILITY"]);
}

#[tokio::test]
async fn untrusted_certificate_is_a_tls_failure() {
    for security in BOTH {
        let imap = fake(&[], &[]).await;
        // The same server, without trusting the testkit's root.
        let mut config = ImapConfig::new("localhost", imap.addr().port(), security);
        config.command_timeout = Duration::from_millis(500);
        let result = connect(&config, PASSWORD).await;
        assert!(matches!(result, Err(ImapError::TlsFailure)), "{result:?}");
        assert!(!credentials_sent(&imap.received_commands().await));
    }
}

#[tokio::test]
async fn wrong_password_is_auth_failed() {
    for security in BOTH {
        let imap = fake(&[], &[]).await;
        let result = connect(&config(&imap, security), "wrong-password").await;
        assert!(matches!(result, Err(ImapError::AuthFailed)), "{result:?}");

        let imap = fake(&[Fault::RejectLogin], &[]).await;
        let result = connect(&config(&imap, security), PASSWORD).await;
        assert!(matches!(result, Err(ImapError::AuthFailed)), "{result:?}");
    }
}

#[tokio::test]
async fn invalid_credentials_are_refused_before_connecting() {
    let imap = fake(&[], &[]).await;
    for password in ["", "a\r\nA1 LOGOUT", "nul\0byte"] {
        let result = connect(&config(&imap, ImapSecurity::ImplicitTls), password).await;
        assert!(matches!(result, Err(ImapError::InvalidInput)), "{result:?}");
    }
    assert!(imap.received_commands().await.is_empty());
}

#[tokio::test]
async fn hostile_servers_give_errors_not_panics_or_hangs() {
    for security in BOTH {
        let imap = fake(&[Fault::DropMidResponse], &[]).await;
        let result = connect(&config(&imap, security), PASSWORD).await;
        assert!(
            matches!(result, Err(ImapError::ConnectionClosed)),
            "{security:?}: {result:?}"
        );

        let imap = fake(&[Fault::MalformedResponse], &[]).await;
        let result = connect(&config(&imap, security), PASSWORD).await;
        assert!(
            matches!(result, Err(ImapError::Protocol)),
            "{security:?}: {result:?}"
        );

        let imap = fake(&[Fault::Delay(Duration::from_secs(3))], &[]).await;
        let started = std::time::Instant::now();
        let result = connect(&config(&imap, security), PASSWORD).await;
        assert!(
            matches!(result, Err(ImapError::Timeout)),
            "{security:?}: {result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

#[tokio::test]
async fn connection_refused_is_cannot_connect() {
    let imap = fake(&[], &[]).await;
    let port = imap.addr().port();
    imap.shutdown().await;
    drop(imap);
    // Give the listener a moment to close.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let config = ImapConfig::new("localhost", port, ImapSecurity::ImplicitTls);
    let result = connect(&config, PASSWORD).await;
    assert!(result.is_err(), "{result:?}");
}

fn account() -> NewAccount {
    NewAccount {
        display_name: "Alice".into(),
        email: USER.into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 465,
        smtp_security: Security::Tls,
        username: USER.into(),
    }
}

/// A server with a nested folder, a non-ASCII (modified UTF-7) name, and
/// folders whose SPECIAL-USE attributes disagree with their names.
async fn folder_server(quirks: &[Quirk]) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", Vec::new())
        .folder("Projects", Vec::new())
        .folder("Projects/Crane", Vec::new())
        // "Reçus"
        .folder("Re&AOc-us", Vec::new())
        // "Entwürfe" (German for drafts)
        .folder("Entw&APw-rfe", Vec::new())
        .folder("Sent Items", Vec::new())
        .folder("Trash", Vec::new())
        .folder("Junk", Vec::new())
        .folder("Old Spam", Vec::new())
        .special_use("Entw&APw-rfe", "\\Drafts")
        .special_use("Sent Items", "\\Sent")
        .special_use("Trash", "\\Trash")
        .special_use("Old Spam", "\\Junk");
    for quirk in quirks {
        builder = builder.quirk(*quirk);
    }
    builder.start().await.expect("fake imap starts")
}

async fn listed_and_saved(
    quirks: &[Quirk],
    security: ImapSecurity,
) -> Vec<(String, String, Option<FolderRole>)> {
    let imap = folder_server(quirks).await;
    let mut session = connect(&config(&imap, security), PASSWORD)
        .await
        .expect("logged in");
    let folders = session.list_folders().await.expect("list");
    session.logout().await;

    let store = Store::open_in_memory().expect("store");
    let account = store.insert_account(&account()).expect("account");
    let ids = save_folders(&store, account, &folders).expect("saved");
    assert_eq!(ids.len(), folders.len());
    let saved = store.list_folders(account).expect("list saved");
    assert!(saved
        .iter()
        .all(|f| f.details.delimiter.as_deref() == Some("/")));
    saved
        .into_iter()
        .map(|f| {
            (
                f.details.remote_name,
                f.details.display_name,
                f.details.role,
            )
        })
        .collect()
}

#[tokio::test]
async fn folders_with_special_use_reach_the_store() {
    for security in BOTH {
        let saved = listed_and_saved(&[], security).await;
        let expected = vec![
            ("Entw&APw-rfe", "Entw\u{fc}rfe", Some(FolderRole::Drafts)),
            ("INBOX", "INBOX", Some(FolderRole::Inbox)),
            ("Junk", "Junk", None),
            ("Old Spam", "Old Spam", Some(FolderRole::Spam)),
            ("Projects", "Projects", None),
            ("Projects/Crane", "Projects/Crane", None),
            ("Re&AOc-us", "Re\u{e7}us", None),
            ("Sent Items", "Sent Items", Some(FolderRole::Sent)),
            ("Trash", "Trash", Some(FolderRole::Trash)),
        ];
        let expected: Vec<(String, String, Option<FolderRole>)> = expected
            .into_iter()
            .map(|(r, d, role)| (r.to_string(), d.to_string(), role))
            .collect();
        assert_eq!(saved, expected, "{security:?}");
    }
}

#[tokio::test]
async fn folders_without_special_use_fall_back_to_names() {
    let saved = listed_and_saved(&[Quirk::NoSpecialUse], ImapSecurity::ImplicitTls).await;
    let expected = vec![
        // No attribute and not an English name: no role.
        ("Entw&APw-rfe", "Entw\u{fc}rfe", None),
        ("INBOX", "INBOX", Some(FolderRole::Inbox)),
        ("Junk", "Junk", Some(FolderRole::Spam)),
        ("Old Spam", "Old Spam", None),
        ("Projects", "Projects", None),
        ("Projects/Crane", "Projects/Crane", None),
        ("Re&AOc-us", "Re\u{e7}us", None),
        ("Sent Items", "Sent Items", Some(FolderRole::Sent)),
        ("Trash", "Trash", Some(FolderRole::Trash)),
    ];
    let expected: Vec<(String, String, Option<FolderRole>)> = expected
        .into_iter()
        .map(|(r, d, role)| (r.to_string(), d.to_string(), role))
        .collect();
    assert_eq!(saved, expected);
}

#[tokio::test]
async fn saving_again_keeps_sync_state() {
    let imap = folder_server(&[]).await;
    let mut session = connect(&config(&imap, ImapSecurity::ImplicitTls), PASSWORD)
        .await
        .expect("logged in");
    let folders = session.list_folders().await.expect("list");
    session.logout().await;

    let store = Store::open_in_memory().expect("store");
    let account = store.insert_account(&account()).expect("account");
    store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "INBOX".into(),
            delimiter: Some("/".into()),
            role: None,
            uidvalidity: Some(42),
            uidnext: Some(7),
            highestmodseq: Some(99),
            is_subscribed: false,
        })
        .expect("seed");
    save_folders(&store, account, &folders).expect("saved");
    save_folders(&store, account, &folders).expect("saved twice");
    let saved = store.list_folders(account).expect("list saved");
    assert_eq!(saved.len(), folders.len());
    let inbox = saved
        .iter()
        .find(|f| f.details.remote_name == "INBOX")
        .expect("inbox");
    assert_eq!(inbox.details.role, Some(FolderRole::Inbox));
    assert_eq!(inbox.details.uidvalidity, Some(42));
    assert_eq!(inbox.details.uidnext, Some(7));
    assert_eq!(inbox.details.highestmodseq, Some(99));
    assert!(!inbox.details.is_subscribed);
}
