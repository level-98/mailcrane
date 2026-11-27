use crate::{check_login, send, Envelope, Secret, SmtpAuth, SmtpConfig, SmtpError, SmtpSecurity};
use mailcrane_testkit::{FakeSmtp, Fault};
use std::time::Duration;

const MESSAGE: &[u8] = b"From: alice@example.org\r\nTo: bob@example.net\r\nBcc: carol@example.com\r\nSubject: Hi\r\n\r\nHello\r\n.dot line\r\n";
const EXPECTED: &[u8] =
    b"From: alice@example.org\r\nTo: bob@example.net\r\nSubject: Hi\r\n\r\nHello\r\n.dot line\r\n";

fn config(smtp: &FakeSmtp, security: SmtpSecurity) -> SmtpConfig {
    let addr = match security {
        SmtpSecurity::ImplicitTls => smtp.addr(),
        SmtpSecurity::StartTls => smtp.starttls_addr(),
    };
    let mut config = SmtpConfig::new("localhost", addr.port(), security);
    config.extra_root_certs = vec![smtp.root_cert_der().to_vec()];
    config
}

fn auth() -> SmtpAuth {
    SmtpAuth::Password {
        username: "alice@example.org".into(),
        password: Secret::new("s3cret-value"),
    }
}

fn envelope() -> Envelope {
    Envelope {
        from: "alice@example.org".into(),
        to: vec!["bob@example.net".into(), "carol@example.com".into()],
    }
}

async fn fake(faults: Vec<Fault>) -> FakeSmtp {
    let mut builder = FakeSmtp::builder();
    for fault in faults {
        builder = builder.fault(fault);
    }
    builder.start().await.expect("fake smtp")
}

#[tokio::test]
async fn sends_over_implicit_tls_and_starttls() {
    for security in [SmtpSecurity::ImplicitTls, SmtpSecurity::StartTls] {
        let smtp = fake(vec![]).await;
        let report = send(&config(&smtp, security), &auth(), &envelope(), MESSAGE)
            .await
            .expect("send");
        assert_eq!(report.accepted, envelope().to);
        assert!(report.rejected.is_empty());
        let sent = smtp.sent().await;
        assert_eq!(sent.len(), 1);
        // Bcc is in the envelope but not in the transmitted bytes, and the
        // dot-stuffed line arrives exactly as given.
        assert_eq!(sent[0].data, EXPECTED);
        assert_eq!(sent[0].envelope_from.as_deref(), Some("alice@example.org"));
        assert_eq!(sent[0].envelope_to, envelope().to);
        assert!(!String::from_utf8_lossy(&sent[0].data).contains("carol"));
    }
}

#[tokio::test]
async fn no_starttls_is_an_error_and_nothing_else_is_sent() {
    let smtp = fake(vec![Fault::NoStartTls]).await;
    let result = send(
        &config(&smtp, SmtpSecurity::StartTls),
        &auth(),
        &envelope(),
        MESSAGE,
    )
    .await;
    assert_eq!(result, Err(SmtpError::StartTlsNotOffered));
    assert!(smtp.sent().await.is_empty());
    // The fake answers AUTH and MAIL with success, so a leak would have shown
    // up as a delivered message above; also check login does the same.
    let login = check_login(&config(&smtp, SmtpSecurity::StartTls), &auth()).await;
    assert_eq!(login, Err(SmtpError::StartTlsNotOffered));
}

#[tokio::test]
async fn untrusted_certificate_is_a_tls_failure() {
    for security in [SmtpSecurity::ImplicitTls, SmtpSecurity::StartTls] {
        let smtp = fake(vec![]).await;
        let mut config = config(&smtp, security);
        config.extra_root_certs.clear();
        let result = send(&config, &auth(), &envelope(), MESSAGE).await;
        assert_eq!(result, Err(SmtpError::TlsFailure));
        assert!(smtp.sent().await.is_empty());
    }
}

#[tokio::test]
async fn wrong_password_is_auth_failed() {
    for security in [SmtpSecurity::ImplicitTls, SmtpSecurity::StartTls] {
        let smtp = fake(vec![Fault::RejectLogin]).await;
        let config = config(&smtp, security);
        assert_eq!(
            send(&config, &auth(), &envelope(), MESSAGE).await,
            Err(SmtpError::AuthFailed)
        );
        assert_eq!(
            check_login(&config, &auth()).await,
            Err(SmtpError::AuthFailed)
        );
        assert!(smtp.sent().await.is_empty());
    }
}

#[tokio::test]
async fn check_login_succeeds() {
    for security in [SmtpSecurity::ImplicitTls, SmtpSecurity::StartTls] {
        let smtp = fake(vec![]).await;
        assert_eq!(check_login(&config(&smtp, security), &auth()).await, Ok(()));
    }
}

#[tokio::test]
async fn temp_and_perm_failures_are_classified() {
    for security in [SmtpSecurity::ImplicitTls, SmtpSecurity::StartTls] {
        let smtp = fake(vec![Fault::TempFailure]).await;
        let result = send(&config(&smtp, security), &auth(), &envelope(), MESSAGE).await;
        assert!(matches!(
            result,
            Err(SmtpError::Temporary { code: 451, .. })
        ));

        let smtp = fake(vec![Fault::PermFailure]).await;
        let result = send(&config(&smtp, security), &auth(), &envelope(), MESSAGE).await;
        assert!(matches!(
            result,
            Err(SmtpError::Permanent { code: 550, .. })
        ));
        assert!(smtp.sent().await.is_empty());
    }
}

#[tokio::test]
async fn drop_mid_data_is_never_success() {
    for security in [SmtpSecurity::ImplicitTls, SmtpSecurity::StartTls] {
        let smtp = fake(vec![Fault::DropMidData]).await;
        let result = send(&config(&smtp, security), &auth(), &envelope(), MESSAGE).await;
        assert!(matches!(
            result,
            Err(SmtpError::Temporary { .. } | SmtpError::CannotConnect)
        ));
    }
}

#[tokio::test]
async fn partial_rejection_reports_rejected_addresses() {
    for security in [SmtpSecurity::ImplicitTls, SmtpSecurity::StartTls] {
        let smtp = fake(vec![Fault::RejectRecipient("bob@example.net".into())]).await;
        let report = send(&config(&smtp, security), &auth(), &envelope(), MESSAGE)
            .await
            .expect("send");
        assert_eq!(report.accepted, vec!["carol@example.com".to_string()]);
        assert_eq!(report.rejected, vec!["bob@example.net".to_string()]);
        let sent = smtp.sent().await;
        assert_eq!(sent[0].envelope_to, vec!["carol@example.com".to_string()]);
    }
}

#[tokio::test]
async fn all_recipients_rejected_sends_nothing() {
    let smtp = fake(vec![
        Fault::RejectRecipient("bob@example.net".into()),
        Fault::RejectRecipient("carol@example.com".into()),
    ])
    .await;
    let result = send(
        &config(&smtp, SmtpSecurity::ImplicitTls),
        &auth(),
        &envelope(),
        MESSAGE,
    )
    .await;
    assert_eq!(
        result,
        Err(SmtpError::RecipientsRejected {
            rejected: envelope().to
        })
    );
    assert!(smtp.sent().await.is_empty());
}

#[tokio::test]
async fn hostile_addresses_are_refused_before_connecting() {
    let smtp = fake(vec![]).await;
    let config = config(&smtp, SmtpSecurity::ImplicitTls);
    for bad in [
        "a@example.org>\r\nRCPT TO:<evil@example.com",
        "a b@example.org",
        "",
        "a@example.org\0",
    ] {
        let mut env = envelope();
        env.to = vec![bad.into()];
        assert_eq!(
            send(&config, &auth(), &env, MESSAGE).await,
            Err(SmtpError::InvalidInput)
        );
        let mut env = envelope();
        env.from = bad.into();
        assert_eq!(
            send(&config, &auth(), &env, MESSAGE).await,
            Err(SmtpError::InvalidInput)
        );
    }
    assert!(smtp.sent().await.is_empty());
}

#[tokio::test]
async fn slow_server_times_out() {
    let smtp = fake(vec![Fault::Delay(Duration::from_secs(2))]).await;
    let mut config = config(&smtp, SmtpSecurity::StartTls);
    config.command_timeout = Duration::from_millis(200);
    assert_eq!(
        send(&config, &auth(), &envelope(), MESSAGE).await,
        Err(SmtpError::Timeout)
    );
}

#[tokio::test]
async fn nothing_listening_is_cannot_connect() {
    let config = SmtpConfig::new("127.0.0.1", 1, SmtpSecurity::StartTls);
    assert_eq!(
        send(&config, &auth(), &envelope(), MESSAGE).await,
        Err(SmtpError::CannotConnect)
    );
}

#[tokio::test]
async fn xoauth2_signs_in_with_the_token() {
    // M3-02: Gmail's XOAUTH2.
    let auth = SmtpAuth::XOAuth2 {
        username: "alice@example.org".into(),
        token: Secret::new("ya29.token"),
    };
    let smtp = fake(vec![]).await;
    let settings = config(&smtp, SmtpSecurity::ImplicitTls);
    assert_eq!(check_login(&settings, &auth).await, Ok(()));
    assert!(send(&settings, &auth, &envelope(), MESSAGE).await.is_ok());

    let refusing = fake(vec![Fault::RejectLogin]).await;
    assert_eq!(
        check_login(&config(&refusing, SmtpSecurity::ImplicitTls), &auth).await,
        Err(SmtpError::AuthFailed)
    );
    // A token that could break the SASL string is refused before sending.
    let broken = SmtpAuth::XOAuth2 {
        username: "alice@example.org".into(),
        token: Secret::new("ya29\u{1}x"),
    };
    assert_eq!(
        check_login(&settings, &broken).await,
        Err(SmtpError::InvalidInput)
    );
}

#[test]
fn secret_is_not_printed() {
    assert!(!format!("{:?}", auth()).contains("s3cret-value"));
}
