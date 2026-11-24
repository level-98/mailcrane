//! Validation tests for manual account setup. The tests against the
//! testkit's fake servers are in `tests/account_setup.rs`.

use super::*;

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

// ---- validation ---------------------------------------------------------

#[test]
fn good_config_validates() {
    assert_eq!(validate(&good_config()), Ok(()));
}

#[test]
fn host_validation_table() {
    use FieldProblem::*;
    let cases: &[(&str, Option<FieldProblem>)] = &[
        ("imap.example.org", None),
        ("192.0.2.1", None),
        ("localhost", None),
        ("", Some(Empty)),
        ("imap example.org", Some(ContainsSpace)),
        (" imap.example.org", Some(ContainsSpace)),
        ("imap.example.org\t", Some(ContainsSpace)),
        ("imaps://imap.example.org", Some(HasScheme)),
        ("https://imap.example.org", Some(HasScheme)),
        ("imap.example.org/inbox", Some(InvalidCharacters)),
        ("user@imap.example.org", Some(InvalidCharacters)),
        ("imap.example.org\0", Some(InvalidCharacters)),
    ];
    for (host, expected) in cases {
        for (field, set) in [(Field::ImapHost, true), (Field::SmtpHost, false)] {
            let mut config = good_config();
            if set {
                config.imap.host = (*host).into();
            } else {
                config.smtp.host = (*host).into();
            }
            let got = validate(&config);
            match expected {
                None => assert_eq!(got, Ok(()), "host {host:?}"),
                Some(problem) => {
                    assert_eq!(got, Err(vec![err(field, *problem)]), "host {host:?}")
                }
            }
        }
    }
}

#[test]
fn port_validation_table() {
    let cases: &[(u32, bool)] = &[
        (0, false),
        (1, true),
        (143, true),
        (65535, true),
        (65536, false),
        (100_000, false),
        (u32::MAX, false),
    ];
    for (port, ok) in cases {
        let mut config = good_config();
        config.imap.port = *port;
        config.smtp.port = *port;
        let got = validate(&config);
        if *ok {
            assert_eq!(got, Ok(()), "port {port}");
        } else {
            assert_eq!(
                got,
                Err(vec![
                    err(Field::ImapPort, FieldProblem::PortOutOfRange),
                    err(Field::SmtpPort, FieldProblem::PortOutOfRange),
                ]),
                "port {port}"
            );
        }
    }
}

#[test]
fn email_validation_table() {
    use FieldProblem::*;
    let cases: &[(&str, Option<FieldProblem>)] = &[
        ("alice@example.org", None),
        ("a.b+tag@mail.example.net", None),
        ("alice@localhost", None),
        ("", Some(Empty)),
        ("alice", Some(MalformedEmail)),
        ("alice@", Some(MalformedEmail)),
        ("@example.org", Some(MalformedEmail)),
        ("alice@@example.org", Some(MalformedEmail)),
        ("alice@bob@example.org", Some(MalformedEmail)),
        ("alice@.example.org", Some(MalformedEmail)),
        ("alice@example.org.", Some(MalformedEmail)),
        ("alice@example..org", Some(MalformedEmail)),
        ("al ice@example.org", Some(MalformedEmail)),
        (
            "alice@example.org\r\nBcc: x@example.com",
            Some(InvalidCharacters),
        ),
    ];
    for (email, expected) in cases {
        let mut config = good_config();
        config.email = (*email).into();
        let got = validate(&config);
        match expected {
            None => assert_eq!(got, Ok(()), "email {email:?}"),
            Some(problem) => assert_eq!(
                got,
                Err(vec![err(Field::Email, *problem)]),
                "email {email:?}"
            ),
        }
    }
}

#[test]
fn username_and_display_name_validation() {
    let mut config = good_config();
    config.username = String::new();
    assert_eq!(
        validate(&config),
        Err(vec![err(Field::Username, FieldProblem::Empty)])
    );
    config.username = "alice\r\nA1 LOGOUT".into();
    assert_eq!(
        validate(&config),
        Err(vec![err(Field::Username, FieldProblem::InvalidCharacters)])
    );

    let mut config = good_config();
    config.display_name = String::new();
    assert_eq!(validate(&config), Ok(()), "display name is optional");
    config.display_name = "Alice\r\nBcc: x@example.com".into();
    assert_eq!(
        validate(&config),
        Err(vec![err(
            Field::DisplayName,
            FieldProblem::InvalidCharacters
        )])
    );
}

#[test]
fn every_problem_is_reported_at_once() {
    let config = ManualAccountConfig {
        email: "nope".into(),
        display_name: String::new(),
        imap: ServerConfig {
            host: String::new(),
            port: 0,
            security: Security::Tls,
        },
        smtp: ServerConfig {
            host: "smtp://x".into(),
            port: 70000,
            security: Security::StartTls,
        },
        username: String::new(),
    };
    assert_eq!(
        validate(&config),
        Err(vec![
            err(Field::Email, FieldProblem::MalformedEmail),
            err(Field::ImapHost, FieldProblem::Empty),
            err(Field::ImapPort, FieldProblem::PortOutOfRange),
            err(Field::SmtpHost, FieldProblem::HasScheme),
            err(Field::SmtpPort, FieldProblem::PortOutOfRange),
            err(Field::Username, FieldProblem::Empty),
        ])
    );
}

#[test]
fn default_ports() {
    assert_eq!(default_port(Protocol::Imap, Security::Tls), 993);
    assert_eq!(default_port(Protocol::Imap, Security::StartTls), 143);
    assert_eq!(default_port(Protocol::Smtp, Security::Tls), 465);
    assert_eq!(default_port(Protocol::Smtp, Security::StartTls), 587);
}

#[test]
fn default_timeout_is_thirty_seconds_per_server() {
    assert_eq!(CHECK_TIMEOUT, Duration::from_secs(30));
    assert_eq!(SetupOptions::default().timeout, CHECK_TIMEOUT);
}

#[test]
fn errors_never_show_what_the_user_typed() {
    let printed = format!(
        "{} {:?}",
        AddAccountError::AuthFailed {
            server: Protocol::Imap
        },
        AddAccountError::Invalid(vec![err(Field::Password, FieldProblem::Empty)])
    );
    assert!(!printed.contains(PASSWORD));
    assert!(!printed.contains(USER));
}
