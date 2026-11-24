//! Manual account setup against the testkit's fake IMAP and SMTP servers.
//! Never a real mailbox.
//!
//! These live here rather than next to the code because the testkit depends
//! on `mailcrane-core`; only an integration test sees one copy of
//! `SecretStore`. Nested in `account::setup` so that
//! `cargo test -p mailcrane-core account::setup` runs them too.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

mod account {
    mod setup {

        use std::time::Duration;

        use mailcrane_core::account::setup::*;
        use mailcrane_core::secrets::{Secret, SecretKey, SecretStore};
        use mailcrane_store::{AccountId, Store};
        use mailcrane_testkit::{FakeImap, FakeSmtp, Fault, MemorySecretStore};

        const USER: &str = "alice@example.org";
        const PASSWORD: &str = "s3cret-value";

        fn good_config() -> ManualAccountConfig {
            ManualAccountConfig {
                email: "alice@example.org".into(),
                display_name: "Alice Example".into(),
                imap: ServerConfig {
                    host: "imap.example.org".into(),
                    port: 993,
                    security: Security::Tls,
                },
                smtp: ServerConfig {
                    host: "smtp.example.org".into(),
                    port: 465,
                    security: Security::Tls,
                },
                username: USER.into(),
            }
        }

        fn err(field: Field, problem: FieldProblem) -> FieldError {
            FieldError { field, problem }
        }

        fn stored(security: Security) -> mailcrane_store::Security {
            match security {
                Security::Tls => mailcrane_store::Security::Tls,
                Security::StartTls => mailcrane_store::Security::Starttls,
            }
        }

        struct Servers {
            imap: FakeImap,
            smtp: FakeSmtp,
        }

        async fn servers(imap_faults: &[Fault], smtp_faults: &[Fault]) -> Servers {
            let mut imap = FakeImap::builder().user(USER, PASSWORD);
            for fault in imap_faults {
                imap = imap.fault(fault.clone());
            }
            let mut smtp = FakeSmtp::builder();
            for fault in smtp_faults {
                smtp = smtp.fault(fault.clone());
            }
            Servers {
                imap: imap.start().await.expect("fake imap"),
                smtp: smtp.start().await.expect("fake smtp"),
            }
        }

        fn config_for(servers: &Servers, imap: Security, smtp: Security) -> ManualAccountConfig {
            let mut config = good_config();
            config.imap = ServerConfig {
                host: "localhost".into(),
                port: u32::from(servers.imap.addr().port()),
                security: imap,
            };
            config.smtp = ServerConfig {
                host: "localhost".into(),
                port: u32::from(servers.smtp.addr().port()),
                security: smtp,
            };
            config
        }

        fn options_for(servers: &Servers) -> SetupOptions {
            SetupOptions {
                timeout: Duration::from_secs(5),
                extra_root_certs: vec![
                    servers.imap.root_cert_der().to_vec(),
                    servers.smtp.root_cert_der().to_vec(),
                ],
            }
        }

        fn pw(value: &str) -> Secret {
            Secret::from_string(value.to_owned())
        }

        async fn add(
            store: &Store,
            secrets: &MemorySecretStore,
            config: &ManualAccountConfig,
            password: &str,
            options: &SetupOptions,
        ) -> Result<AccountId, AddAccountError> {
            // Any hang is a failure: setup's own timeout must fire first.
            tokio::time::timeout(
                Duration::from_secs(20),
                add_account_manual_with(store, secrets, config, pw(password), options),
            )
            .await
            .expect("setup must not hang")
        }

        fn assert_nothing_saved(store: &Store, secrets: &MemorySecretStore) {
            assert!(store.list_accounts().expect("list").is_empty());
            assert!(secrets.keys().is_empty());
        }

        #[tokio::test]
        async fn correct_settings_save_account_and_password_goes_to_secret_store() {
            for (imap_sec, smtp_sec) in [
                (Security::Tls, Security::Tls),
                (Security::StartTls, Security::StartTls),
                (Security::Tls, Security::StartTls),
            ] {
                let servers = servers(&[], &[]).await;
                let store = Store::open_in_memory().expect("store");
                let secrets = MemorySecretStore::new();
                let config = config_for(&servers, imap_sec, smtp_sec);
                let id = add(&store, &secrets, &config, PASSWORD, &options_for(&servers))
                    .await
                    .expect("account added");

                let account = store.get_account(id).expect("get").expect("saved");
                assert_eq!(account.details.email, "alice@example.org");
                assert_eq!(account.details.username, USER);
                assert_eq!(account.details.imap_port, servers.imap.addr().port());
                assert_eq!(account.details.imap_security, stored(imap_sec));
                assert_eq!(account.details.smtp_security, stored(smtp_sec));

                // The password is in the secret store under the account's key...
                let key = SecretKey::account_password(id.0);
                assert_eq!(secrets.keys(), vec![key.as_str().to_owned()]);
                let stored = secrets.get(&key).expect("get").expect("present");
                assert_eq!(stored.expose(), PASSWORD.as_bytes());
                // ...and nowhere in the account row.
                assert!(!format!("{account:?}").contains(PASSWORD));
            }
        }

        #[tokio::test]
        async fn wrong_password_is_auth_failed_and_nothing_saved() {
            let servers = servers(&[], &[]).await;
            let store = Store::open_in_memory().expect("store");
            let secrets = MemorySecretStore::new();
            let config = config_for(&servers, Security::Tls, Security::Tls);
            let got = add(&store, &secrets, &config, "wrong", &options_for(&servers)).await;
            assert_eq!(
                got,
                Err(AddAccountError::AuthFailed {
                    server: Protocol::Imap
                })
            );
            assert_nothing_saved(&store, &secrets);
        }

        #[tokio::test]
        async fn smtp_login_refused_is_auth_failed_and_nothing_saved() {
            let servers = servers(&[], &[Fault::RejectLogin]).await;
            let store = Store::open_in_memory().expect("store");
            let secrets = MemorySecretStore::new();
            let config = config_for(&servers, Security::Tls, Security::StartTls);
            let got = add(&store, &secrets, &config, PASSWORD, &options_for(&servers)).await;
            assert_eq!(
                got,
                Err(AddAccountError::AuthFailed {
                    server: Protocol::Smtp
                })
            );
            assert_nothing_saved(&store, &secrets);
        }

        #[tokio::test]
        async fn wrong_port_is_cannot_connect_and_nothing_saved() {
            // IMAP pointed at the SMTP server's port: something answers, but not IMAP.
            let servers = servers(&[], &[]).await;
            let store = Store::open_in_memory().expect("store");
            let secrets = MemorySecretStore::new();
            let mut config = config_for(&servers, Security::Tls, Security::Tls);
            config.imap.port = u32::from(servers.smtp.addr().port());
            let got = add(&store, &secrets, &config, PASSWORD, &options_for(&servers)).await;
            assert_eq!(
                got,
                Err(AddAccountError::CannotConnect {
                    server: Protocol::Imap
                })
            );
            assert_nothing_saved(&store, &secrets);
        }

        #[tokio::test]
        async fn unreachable_host_is_cannot_connect_and_nothing_saved() {
            let servers = servers(&[], &[]).await;
            // A port nothing listens on: bind one, then let it go.
            let closed = std::net::TcpListener::bind("127.0.0.1:0")
                .and_then(|l| l.local_addr())
                .expect("free port")
                .port();
            let store = Store::open_in_memory().expect("store");
            let secrets = MemorySecretStore::new();
            let mut config = config_for(&servers, Security::Tls, Security::Tls);
            config.smtp.host = "127.0.0.1".into();
            config.smtp.port = u32::from(closed);
            let got = add(&store, &secrets, &config, PASSWORD, &options_for(&servers)).await;
            assert_eq!(
                got,
                Err(AddAccountError::CannotConnect {
                    server: Protocol::Smtp
                })
            );
            assert_nothing_saved(&store, &secrets);
        }

        #[tokio::test]
        async fn untrusted_certificate_is_tls_failure_and_nothing_saved() {
            let servers = servers(&[], &[]).await;
            for security in [Security::Tls, Security::StartTls] {
                let store = Store::open_in_memory().expect("store");
                let secrets = MemorySecretStore::new();
                let config = config_for(&servers, security, security);
                let mut options = options_for(&servers);
                options.extra_root_certs.clear();
                let got = add(&store, &secrets, &config, PASSWORD, &options).await;
                assert_eq!(
                    got,
                    Err(AddAccountError::TlsFailure {
                        server: Protocol::Imap
                    }),
                    "{security:?}"
                );
                assert_nothing_saved(&store, &secrets);
            }
        }

        #[tokio::test]
        async fn missing_starttls_is_reported_and_nothing_saved() {
            for (imap_faults, smtp_faults, server) in [
                (&[Fault::NoStartTls][..], &[][..], Protocol::Imap),
                (&[][..], &[Fault::NoStartTls][..], Protocol::Smtp),
            ] {
                let servers = servers(imap_faults, smtp_faults).await;
                let store = Store::open_in_memory().expect("store");
                let secrets = MemorySecretStore::new();
                let config = config_for(&servers, Security::StartTls, Security::StartTls);
                let got = add(&store, &secrets, &config, PASSWORD, &options_for(&servers)).await;
                assert_eq!(got, Err(AddAccountError::StartTlsNotOffered { server }));
                assert_nothing_saved(&store, &secrets);
            }
        }

        #[tokio::test]
        async fn slow_server_times_out_and_nothing_saved() {
            for (imap_faults, smtp_faults, server) in [
                (
                    &[Fault::Delay(Duration::from_secs(20))][..],
                    &[][..],
                    Protocol::Imap,
                ),
                (
                    &[][..],
                    &[Fault::Delay(Duration::from_secs(20))][..],
                    Protocol::Smtp,
                ),
            ] {
                let servers = servers(imap_faults, smtp_faults).await;
                let store = Store::open_in_memory().expect("store");
                let secrets = MemorySecretStore::new();
                let config = config_for(&servers, Security::Tls, Security::Tls);
                let mut options = options_for(&servers);
                // Long enough for the healthy server to answer (Windows spends ~2 s
                // trying ::1 for "localhost" first), well short of the delay.
                options.timeout = Duration::from_secs(4);
                let started = std::time::Instant::now();
                let got = add(&store, &secrets, &config, PASSWORD, &options).await;
                assert_eq!(got, Err(AddAccountError::Timeout { server }));
                // Both servers are checked at once, each within its own limit.
                assert!(started.elapsed() < Duration::from_secs(8));
                assert_nothing_saved(&store, &secrets);
            }
        }

        #[tokio::test]
        async fn invalid_input_never_contacts_a_server() {
            let servers = servers(&[], &[]).await;
            let store = Store::open_in_memory().expect("store");
            let secrets = MemorySecretStore::new();
            let mut config = config_for(&servers, Security::Tls, Security::Tls);
            config.email = "not-an-email".into();
            let got = add(&store, &secrets, &config, "", &options_for(&servers)).await;
            assert_eq!(
                got,
                Err(AddAccountError::Invalid(vec![
                    err(Field::Email, FieldProblem::MalformedEmail),
                    err(Field::Password, FieldProblem::Empty),
                ]))
            );
            assert!(servers.imap.received_commands().await.is_empty());
            assert!(servers.smtp.sent().await.is_empty());
            assert_nothing_saved(&store, &secrets);
        }

        #[tokio::test]
        async fn password_with_line_break_is_rejected_before_sending() {
            let servers = servers(&[], &[]).await;
            let store = Store::open_in_memory().expect("store");
            let secrets = MemorySecretStore::new();
            let config = config_for(&servers, Security::Tls, Security::Tls);
            let got = add(
                &store,
                &secrets,
                &config,
                "pw\r\nA1 LOGOUT",
                &options_for(&servers),
            )
            .await;
            assert_eq!(
                got,
                Err(AddAccountError::Invalid(vec![err(
                    Field::Password,
                    FieldProblem::InvalidCharacters
                )]))
            );
            assert!(servers.imap.received_commands().await.is_empty());
            assert_nothing_saved(&store, &secrets);
        }

        #[tokio::test]
        async fn secret_store_failure_removes_the_account_row() {
            let servers = servers(&[], &[]).await;
            let store = Store::open_in_memory().expect("store");
            let secrets = MemorySecretStore::new();
            secrets.fail_with(Some(mailcrane_core::secrets::SecretError::Unavailable));
            let config = config_for(&servers, Security::Tls, Security::Tls);
            let got = add(&store, &secrets, &config, PASSWORD, &options_for(&servers)).await;
            assert_eq!(got, Err(AddAccountError::SaveFailed));
            secrets.fail_with(None);
            assert_nothing_saved(&store, &secrets);
        }
    }
}
