//! Sending a composed message against the testkit's fake SMTP server.
//! Never a real mailbox. Nested in `compose` so that
//! `cargo test -p mailcrane-core compose` runs these too.

#![cfg(test)]

mod compose {
    use std::time::Duration;

    use mailcrane_core::compose::*;
    use mailcrane_core::secrets::{Secret, SecretKey, SecretStore};
    use mailcrane_mime::{parse_message, AddressProblem};
    use mailcrane_store::{AccountId, NewAccount, Security, Store};
    use mailcrane_testkit::{FakeSmtp, Fault, MemorySecretStore};

    struct Setup {
        smtp: FakeSmtp,
        store: Store,
        secrets: MemorySecretStore,
        account: AccountId,
    }

    async fn setup(faults: &[Fault], security: Security) -> Setup {
        let mut builder = FakeSmtp::builder();
        for fault in faults {
            builder = builder.fault(fault.clone());
        }
        let smtp = builder.start().await.expect("fake smtp");
        let port = match security {
            Security::Tls => smtp.addr().port(),
            Security::Starttls => smtp.starttls_addr().port(),
        };
        let store = Store::open_in_memory().expect("store");
        let account = store
            .insert_account(&NewAccount {
                display_name: "Ann Example".into(),
                email: "ann@example.org".into(),
                imap_host: "localhost".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "localhost".into(),
                smtp_port: port,
                smtp_security: security,
                username: "ann@example.org".into(),
            })
            .expect("account");
        let secrets = MemorySecretStore::new();
        secrets
            .put(
                &SecretKey::account_password(account.0),
                &Secret::from_string("s3cret-value".into()),
            )
            .expect("password");
        Setup {
            smtp,
            store,
            secrets,
            account,
        }
    }

    fn options(s: &Setup) -> SendOptions {
        SendOptions {
            timeout: Duration::from_secs(5),
            extra_root_certs: vec![s.smtp.root_cert_der().to_vec()],
        }
    }

    fn draft(account: AccountId) -> ComposeDraft {
        ComposeDraft {
            account_id: account,
            to: vec!["Bob Example <bob@example.net>".into()],
            cc: vec!["\"Example, Carol\" <carol@example.com>".into()],
            bcc: vec!["dan@example.org".into()],
            subject: "Grüße".into(),
            body_text: "Hello Bob,\nsee you at noon.\n".into(),
            in_reply_to: None,
            references: vec![],
            attachments: Vec::new(),
            forward_attachments_of: None,
        }
    }

    async fn run(s: &Setup, d: &ComposeDraft) -> Result<SendReport, SendError> {
        tokio::time::timeout(
            Duration::from_secs(20),
            send(&s.store, &s.secrets, d, &options(s)),
        )
        .await
        .expect("send must not hang")
    }

    #[tokio::test]
    async fn sends_end_to_end_over_tls_and_starttls() {
        for security in [Security::Tls, Security::Starttls] {
            let s = setup(&[], security).await;
            let report = run(&s, &draft(s.account)).await.expect("sent");
            assert!(report.rejected.is_empty());

            let sent = s.smtp.sent().await;
            assert_eq!(sent.len(), 1);
            let m = &sent[0];
            assert_eq!(m.envelope_from.as_deref(), Some("ann@example.org"));
            assert_eq!(
                m.envelope_to,
                vec!["bob@example.net", "carol@example.com", "dan@example.org"]
            );
            let raw = String::from_utf8_lossy(&m.data).to_ascii_lowercase();
            assert!(!raw.contains("bcc:"), "no Bcc header on the wire");
            assert!(!raw.contains("dan@example.org"));

            let parsed = parse_message(&m.data).expect("parses");
            assert_eq!(parsed.subject.as_deref(), Some("Grüße"));
            assert_eq!(parsed.from[0].name.as_deref(), Some("Ann Example"));
            assert_eq!(parsed.from[0].email, "ann@example.org");
            assert_eq!(parsed.to[0].email, "bob@example.net");
            assert_eq!(parsed.cc[0].name.as_deref(), Some("Example, Carol"));
            assert!(parsed
                .message_id
                .unwrap_or_default()
                .ends_with("@example.org"));
            assert_eq!(
                parsed.text_plain.unwrap_or_default().replace("\r\n", "\n"),
                "Hello Bob,\nsee you at noon.\n"
            );
        }
    }

    #[tokio::test]
    async fn a_reply_carries_thread_headers_on_the_wire() {
        use mailcrane_store::{Flags, NewFolder, NewMessageHeader};
        let s = setup(&[], Security::Tls).await;
        let folder = s
            .store
            .upsert_folder(&NewFolder {
                account_id: s.account,
                remote_name: "INBOX".into(),
                display_name: "INBOX".into(),
                delimiter: Some("/".into()),
                role: None,
                uidvalidity: Some(1),
                uidnext: Some(2),
                highestmodseq: None,
                is_subscribed: true,
            })
            .expect("folder");
        let raw = b"From: Bob Example <bob@example.net>\r\n\
To: Ann Example <ann@example.org>, carol@example.com\r\n\
Subject: Lunch\r\n\
Message-ID: <m2@example.net>\r\n\
References: <m1@example.net>\r\n\
In-Reply-To: <m1@example.net>\r\n\
\r\n\
Noon?\r\n";
        let id = s
            .store
            .upsert_message_header(&NewMessageHeader {
                folder_id: folder,
                uid: 1,
                message_id_header: Some("m2@example.net".into()),
                in_reply_to: None,
                references_json: "[]".into(),
                thread_id: None,
                subject: "Lunch".into(),
                from_name: Some("Bob Example".into()),
                from_addr: Some("bob@example.net".into()),
                to_json: "[]".into(),
                cc_json: "[]".into(),
                date: 1_795_516_920,
                internal_date: 1_795_516_920,
                size: raw.len() as i64,
                flags: Flags::default(),
                keywords_json: "[]".into(),
                has_attachments: false,
                snippet: String::new(),
            })
            .expect("message");
        assert_eq!(
            prepare_reply(&s.store, id, ReplyKind::Reply, "Bob wrote:"),
            Err(ReplyError::BodyNotDownloaded)
        );
        s.store.store_body(id, raw, 0).expect("body");

        let mut reply =
            prepare_reply(&s.store, id, ReplyKind::ReplyAll, "Bob wrote:").expect("reply");
        assert_eq!(
            reply.to,
            ["Bob Example <bob@example.net>", "carol@example.com"]
        );
        reply.body_text.insert_str(0, "Noon works.");
        run(&s, &reply).await.expect("sent");

        let sent = s.smtp.sent().await;
        let parsed = parse_message(&sent[0].data).expect("parses");
        assert_eq!(parsed.subject.as_deref(), Some("Re: Lunch"));
        assert_eq!(parsed.in_reply_to, ["m2@example.net"]);
        assert_eq!(parsed.references, ["m1@example.net", "m2@example.net"]);
        let text = parsed.text_plain.unwrap_or_default().replace("\r\n", "\n");
        assert!(
            text.starts_with("Noon works.\n\nBob wrote:\n> Noon?"),
            "{text}"
        );
        assert_eq!(
            prepare_reply(
                &s.store,
                mailcrane_store::MessageId(999),
                ReplyKind::Reply,
                ""
            ),
            Err(ReplyError::NotFound)
        );
    }

    #[tokio::test]
    async fn invalid_recipients_send_nothing() {
        let s = setup(&[], Security::Tls).await;
        let mut d = draft(s.account);
        d.to.push("not an address".into());
        assert_eq!(
            run(&s, &d).await,
            Err(SendError::InvalidRecipients(vec![RecipientError {
                field: RecipientField::To,
                entry: 1,
                problem: AddressProblem::MissingAt
            }]))
        );
        d.to.clear();
        d.cc.clear();
        d.bcc.clear();
        assert_eq!(run(&s, &d).await, Err(SendError::NoRecipients));
        assert!(s.smtp.sent().await.is_empty());
    }

    #[tokio::test]
    async fn some_recipients_refused_are_reported() {
        let s = setup(
            &[Fault::RejectRecipient("carol@example.com".into())],
            Security::Tls,
        )
        .await;
        let report = run(&s, &draft(s.account)).await.expect("sent");
        assert_eq!(report.rejected, vec!["carol@example.com"]);
        assert_eq!(s.smtp.sent().await.len(), 1);
    }

    #[tokio::test]
    async fn failures_are_classified() {
        let cases = [
            (Fault::RejectLogin, SendError::AuthFailed),
            (Fault::TempFailure, SendError::Refused { temporary: true }),
            (Fault::PermFailure, SendError::Refused { temporary: false }),
        ];
        for (fault, want) in cases {
            let s = setup(std::slice::from_ref(&fault), Security::Tls).await;
            let got = run(&s, &draft(s.account)).await;
            assert_eq!(got, Err(want), "fault {fault:?}");
            assert!(s.smtp.sent().await.is_empty());
        }

        let s = setup(&[Fault::NoStartTls], Security::Starttls).await;
        assert_eq!(
            run(&s, &draft(s.account)).await,
            Err(SendError::StartTlsNotOffered)
        );

        let s = setup(
            &[Fault::RejectRecipient("x@example.org".into())],
            Security::Tls,
        )
        .await;
        let mut d = draft(s.account);
        d.to = vec!["x@example.org".into()];
        d.cc.clear();
        d.bcc.clear();
        assert_eq!(
            run(&s, &d).await,
            Err(SendError::RecipientsRejected {
                rejected: vec!["x@example.org".into()]
            })
        );
        assert!(s.smtp.sent().await.is_empty());
    }

    #[tokio::test]
    async fn untrusted_certificate_is_a_tls_failure() {
        let s = setup(&[], Security::Tls).await;
        let mut opts = options(&s);
        opts.extra_root_certs.clear();
        let got = send(&s.store, &s.secrets, &draft(s.account), &opts).await;
        assert_eq!(got, Err(SendError::TlsFailure));
        assert!(s.smtp.sent().await.is_empty());
    }

    #[tokio::test]
    async fn missing_account_or_password() {
        let s = setup(&[], Security::Tls).await;
        let mut d = draft(s.account);
        d.account_id = AccountId(999);
        assert_eq!(run(&s, &d).await, Err(SendError::AccountNotFound));

        s.secrets
            .delete(&SecretKey::account_password(s.account.0))
            .expect("delete");
        assert_eq!(
            run(&s, &draft(s.account)).await,
            Err(SendError::MissingCredentials)
        );
        assert!(s.smtp.sent().await.is_empty());
    }
}
