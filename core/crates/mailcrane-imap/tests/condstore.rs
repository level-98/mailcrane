//! CONDSTORE and QRESYNC (RFC 7162) against the testkit's fake IMAP
//! server. Never a real mailbox.

// Lets clippy's test allowances (clippy.toml) cover the helpers too.
#![cfg(test)]

mod condstore {
    use mailcrane_imap::{ImapConfig, ImapError, ImapSecurity, QresyncParams, Secret, Session};
    use mailcrane_testkit::{FakeImap, Quirk};
    use std::time::Duration;

    const USER: &str = "alice@example.org";
    const PASSWORD: &str = "s3cret-value";

    fn raws(count: usize) -> Vec<Vec<u8>> {
        (1..=count)
            .map(|n| {
                format!("From: alice@example.org\r\nSubject: M{n}\r\n\r\n{n}\r\n").into_bytes()
            })
            .collect()
    }

    async fn fake(count: usize, quirks: &[Quirk]) -> FakeImap {
        let mut builder = FakeImap::builder()
            .user(USER, PASSWORD)
            .folder("INBOX", raws(count));
        for quirk in quirks {
            builder = builder.quirk(*quirk);
        }
        builder.start().await.expect("fake imap starts")
    }

    async fn connect(imap: &FakeImap) -> Session {
        let mut config =
            ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
                .with_extra_root_cert(imap.root_cert_der().to_vec());
        config.command_timeout = Duration::from_millis(500);
        config.connect_timeout = Duration::from_secs(5);
        Session::connect(&config, USER, &Secret::new(PASSWORD))
            .await
            .expect("connects")
    }

    #[tokio::test]
    async fn select_reports_highestmodseq_or_none() {
        let imap = fake(3, &[]).await;
        let mut session = connect(&imap).await;
        let info = session.select("INBOX").await.expect("select");
        assert_eq!(info.highestmodseq, Some(3));

        imap.mailbox().set_nomodseq("INBOX", true).await;
        let info = session.select("INBOX").await.expect("select");
        assert_eq!(info.highestmodseq, None);

        let imap = fake(3, &[Quirk::NoCondstore]).await;
        let mut session = connect(&imap).await;
        assert!(!session.supports_condstore());
        let info = session.select("INBOX").await.expect("select");
        assert_eq!(info.highestmodseq, None);
    }

    #[tokio::test]
    async fn qresync_is_enabled_only_where_offered() {
        let imap = fake(1, &[]).await;
        let mut session = connect(&imap).await;
        assert!(session.supports_qresync() && session.supports_condstore());
        assert!(session.enable_qresync().await.expect("enable"));
        // Once per session.
        assert!(session.enable_qresync().await.expect("enable again"));
        let enables = imap
            .received_commands()
            .await
            .iter()
            .filter(|c| *c == "ENABLE")
            .count();
        assert_eq!(enables, 1);

        let imap = fake(1, &[Quirk::NoQresync]).await;
        let mut session = connect(&imap).await;
        assert!(!session.supports_qresync());
        assert!(session.supports_condstore());
        assert!(!session.enable_qresync().await.expect("not offered"));
        assert!(!imap.received_commands().await.iter().any(|c| c == "ENABLE"));
    }

    #[tokio::test]
    async fn select_qresync_needs_enable_and_sends_nothing_without_it() {
        let imap = fake(2, &[]).await;
        let mut session = connect(&imap).await;
        let params = QresyncParams {
            uidvalidity: 1,
            modseq: 1,
            known_uids: None,
        };
        let before = imap.received_commands().await.len();
        assert!(matches!(
            session.select_qresync("INBOX", params).await,
            Err(ImapError::InvalidInput)
        ));
        assert_eq!(imap.received_commands().await.len(), before);
    }

    #[tokio::test]
    async fn select_qresync_returns_vanished_and_changed_flags() {
        let imap = fake(10, &[]).await;
        let mailbox = imap.mailbox();
        let mut session = connect(&imap).await;
        let modseq = session
            .select("INBOX")
            .await
            .expect("select")
            .highestmodseq
            .expect("modseq");
        mailbox.add_flag("INBOX", 2, "\\Seen").await;
        mailbox.expunge("INBOX", 4).await;
        mailbox.expunge("INBOX", 5).await;
        // Outside the known UIDs below: not reported.
        mailbox.add_flag("INBOX", 9, "\\Flagged").await;

        assert!(session.enable_qresync().await.expect("enable"));
        let answer = session
            .select_qresync(
                "INBOX",
                QresyncParams {
                    uidvalidity: 1,
                    modseq,
                    known_uids: Some((1, 8)),
                },
            )
            .await
            .expect("select qresync");
        assert_eq!(answer.vanished, [(4, 4), (5, 5)]);
        assert_eq!(answer.changed.updates.len(), 1);
        let update = answer.changed.updates.first().expect("one update");
        assert_eq!(update.uid, 2);
        assert_eq!(update.flags, ["\\Seen"]);
        assert!(update.modseq.is_some_and(|m| m > modseq));
        assert_eq!(
            answer.info.highestmodseq,
            mailbox.highest_modseq("INBOX").await
        );

        // RFC 7162 §3.2.5.2: another UIDVALIDITY gets no changes.
        mailbox.set_uidvalidity(7);
        let answer = session
            .select_qresync(
                "INBOX",
                QresyncParams {
                    uidvalidity: 1,
                    modseq,
                    known_uids: None,
                },
            )
            .await
            .expect("select qresync");
        assert_eq!(answer.info.uidvalidity, 7);
        assert!(answer.vanished.is_empty() && answer.changed.updates.is_empty());
    }

    #[tokio::test]
    async fn uid_fetch_flags_with_and_without_changedsince() {
        let imap = fake(6, &[Quirk::NoQresync]).await;
        let mailbox = imap.mailbox();
        let mut session = connect(&imap).await;
        let modseq = session
            .select("INBOX")
            .await
            .expect("select")
            .highestmodseq
            .expect("modseq");
        mailbox.add_flag("INBOX", 3, "$Label").await;

        let changed = session
            .uid_fetch_flags(1, 6, Some(modseq))
            .await
            .expect("changed since");
        let uids: Vec<u32> = changed.updates.iter().map(|u| u.uid).collect();
        assert_eq!(uids, [3]);

        let all = session.uid_fetch_flags(2, 5, None).await.expect("all");
        let uids: Vec<u32> = all.updates.iter().map(|u| u.uid).collect();
        assert_eq!(uids, [2, 3, 4, 5]);
        assert!(all.skipped.is_empty());

        assert!(session
            .uid_fetch_flags(5, 2, None)
            .await
            .expect("empty range")
            .updates
            .is_empty());
    }
}
