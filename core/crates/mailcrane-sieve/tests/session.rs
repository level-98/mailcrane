//! The ManageSieve client against the testkit's fake server.

use mailcrane_sieve::{Secret, Session, SieveConfig, SieveError};
use mailcrane_testkit::{FakeSieve, SieveFault};

fn config(server: &FakeSieve) -> SieveConfig {
    SieveConfig::new("localhost", server.addr().port())
        .with_extra_root_cert(server.root_cert_der().to_vec())
}

async fn login(server: &FakeSieve) -> Result<Session, SieveError> {
    Session::connect(
        &config(server),
        "alice@example.org",
        &Secret::new("password"),
    )
    .await
}

#[tokio::test]
async fn logs_in_over_starttls_and_manages_scripts() {
    let server = FakeSieve::builder()
        .extensions("fileinto imap4flags vacation")
        .script("vacation", "keep;", true)
        .start()
        .await
        .expect("server");
    let mut s = login(&server).await.expect("login");
    assert!(s.capabilities().has_extension("imap4flags"));
    let scripts = s.list_scripts().await.expect("list");
    assert_eq!(scripts.len(), 1);
    assert!(scripts[0].active && scripts[0].name == "vacation");
    assert_eq!(s.get_script("vacation").await.expect("get"), "keep;");

    let body = "# test\r\nif header :contains \"subject\" \"x\" {\r\n  discard;\r\n}\r\n";
    s.have_space("mailcrane", body.len()).await.expect("space");
    s.check_script(body).await.expect("check");
    s.put_script("mailcrane", body).await.expect("put");
    assert_eq!(s.get_script("mailcrane").await.expect("get"), body);
    // Storing doesn't change which script is active.
    assert_eq!(server.active().as_deref(), Some("vacation"));
    s.set_active("mailcrane").await.expect("activate");
    assert_eq!(server.active().as_deref(), Some("mailcrane"));
    s.logout().await;

    // Nothing but the greeting exchange and STARTTLS went in clear text.
    let commands = server.commands();
    assert_eq!(commands.first().map(String::as_str), Some("STARTTLS"));
    assert!(
        commands.iter().skip(1).all(|c| c.ends_with("+tls")),
        "{commands:?}"
    );
}

#[tokio::test]
async fn no_starttls_means_no_login() {
    let server = FakeSieve::builder()
        .fault(SieveFault::NoStartTls)
        .start()
        .await
        .expect("server");
    assert_eq!(
        login(&server).await.err(),
        Some(SieveError::StartTlsNotOffered)
    );
    // Not even AUTHENTICATE was sent.
    assert!(server.commands().is_empty(), "{:?}", server.commands());
}

#[tokio::test]
async fn bytes_injected_before_the_handshake_are_refused() {
    let server = FakeSieve::builder()
        .fault(SieveFault::InjectAfterStartTls(
            b"\"SASL\" \"PLAIN\"\r\nOK\r\n".to_vec(),
        ))
        .start()
        .await
        .expect("server");
    let err = login(&server).await.err();
    assert!(
        matches!(
            err,
            Some(SieveError::Protocol) | Some(SieveError::TlsFailure)
        ),
        "{err:?}"
    );
    assert_eq!(server.logins(), 0);
}

#[tokio::test]
async fn wrong_password_and_missing_plain() {
    let server = FakeSieve::builder().start().await.expect("server");
    let err = Session::connect(&config(&server), "alice@example.org", &Secret::new("nope"))
        .await
        .err();
    assert_eq!(err, Some(SieveError::AuthFailed));

    let server = FakeSieve::builder()
        .fault(SieveFault::NoPlain)
        .start()
        .await
        .expect("server");
    assert_eq!(
        login(&server).await.err(),
        Some(SieveError::NoSupportedMechanism)
    );
}

#[tokio::test]
async fn an_untrusted_certificate_fails() {
    let server = FakeSieve::builder().start().await.expect("server");
    // No extra root: the testkit's certificate isn't trusted.
    let config = SieveConfig::new("localhost", server.addr().port());
    let err = Session::connect(&config, "alice@example.org", &Secret::new("password"))
        .await
        .err();
    assert_eq!(err, Some(SieveError::TlsFailure));
    assert_eq!(server.logins(), 0);
}

#[tokio::test]
async fn refused_scripts_and_quota() {
    let server = FakeSieve::builder()
        .fault(SieveFault::RejectScripts)
        .start()
        .await
        .expect("server");
    let mut s = login(&server).await.expect("login");
    assert_eq!(s.check_script("bad").await, Err(SieveError::ScriptRejected));
    assert_eq!(
        s.put_script("mailcrane", "bad").await,
        Err(SieveError::ScriptRejected)
    );
    assert!(server.scripts().is_empty());

    let server = FakeSieve::builder()
        .fault(SieveFault::NoSpace)
        .start()
        .await
        .expect("server");
    let mut s = login(&server).await.expect("login");
    assert_eq!(
        s.have_space("mailcrane", 10).await,
        Err(SieveError::QuotaExceeded)
    );
    assert_eq!(
        s.put_script("mailcrane", "keep;").await,
        Err(SieveError::QuotaExceeded)
    );
}

#[tokio::test]
async fn garbage_greetings_are_errors_not_panics() {
    for greeting in [
        b"\xff\xfe\r\n".to_vec(),
        b"\"SASL\" \"PLAIN\"\r\n".to_vec(),
        b"BYE \"go away\"\r\n".to_vec(),
        format!("\"{}\"\r\n", "a".repeat(70_000)).into_bytes(),
        b"{999999999}\r\n".to_vec(),
    ] {
        let server = FakeSieve::builder()
            .fault(SieveFault::Greeting(greeting.clone()))
            .start()
            .await
            .expect("server");
        let mut config = config(&server);
        config.command_timeout = std::time::Duration::from_secs(2);
        let err = Session::connect(&config, "alice@example.org", &Secret::new("password"))
            .await
            .err();
        assert!(err.is_some(), "{greeting:?}");
    }
}

#[tokio::test]
async fn script_names_with_line_breaks_are_never_sent() {
    let server = FakeSieve::builder().start().await.expect("server");
    let mut s = login(&server).await.expect("login");
    let before = server.commands().len();
    assert_eq!(
        s.put_script("x\r\nSETACTIVE \"y\"", "keep;").await,
        Err(SieveError::InvalidInput)
    );
    assert_eq!(server.commands().len(), before);
}
