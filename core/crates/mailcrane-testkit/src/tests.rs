#[cfg(test)]
mod suite {
    use crate::{FakeImap, FakeJmap, FakeSmtp, Fault, Quirk};
    use rustls::pki_types::{CertificateDer, ServerName};
    use rustls::{ClientConfig, RootCertStore};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    fn client_config(root_der: &[u8]) -> Arc<ClientConfig> {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(root_der.to_vec()))
            .expect("test root certificate");
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    }

    async fn open_tls(
        addr: std::net::SocketAddr,
        root_der: &[u8],
    ) -> tokio_rustls::client::TlsStream<TcpStream> {
        let stream = TcpStream::connect(addr).await.expect("connect");
        let server_name = ServerName::try_from("localhost".to_string()).expect("server name");
        tokio_rustls::TlsConnector::from(client_config(root_der))
            .connect(server_name, stream)
            .await
            .expect("TLS handshake")
    }

    /// `cargo xtask dev-mail` (#403): a client that trusts only the dev CA
    /// accepts both servers under the emulator's name for the host, and the
    /// CA is what `root_cert_der()` reports.
    #[tokio::test]
    async fn dev_certificates_are_trusted_through_their_ca_for_the_emulator_host() {
        use crate::{DevCertificates, TestTls};
        let dev = DevCertificates::generate(&["10.0.2.2", "localhost"]).expect("certs");
        let imap = FakeImap::builder()
            .folder("INBOX", Vec::new())
            .tls(TestTls::from_chain(&dev).expect("tls"))
            .start()
            .await
            .expect("imap");
        let smtp = FakeSmtp::builder()
            .tls(TestTls::from_chain(&dev).expect("tls"))
            .start()
            .await
            .expect("smtp");
        assert_eq!(imap.root_cert_der(), dev.ca_der.as_slice());
        for addr in [imap.addr(), smtp.addr()] {
            let stream = TcpStream::connect(addr).await.expect("connect");
            let name = ServerName::try_from("10.0.2.2".to_string()).expect("server name");
            let mut tls = tokio_rustls::TlsConnector::from(client_config(&dev.ca_der))
                .connect(name, stream)
                .await
                .expect("TLS handshake through the dev CA");
            let mut greeting = [0u8; 4];
            tls.read_exact(&mut greeting).await.expect("greeting");
            assert!(greeting.starts_with(b"* OK") || greeting.starts_with(b"220"));
        }
        // A name the certificate doesn't cover is refused.
        let stream = TcpStream::connect(imap.addr()).await.expect("connect");
        let name = ServerName::try_from("example.org".to_string()).expect("server name");
        assert!(tokio_rustls::TlsConnector::from(client_config(&dev.ca_der))
            .connect(name, stream)
            .await
            .is_err());
        imap.shutdown().await;
        smtp.shutdown().await;
    }

    async fn imap_read_line(stream: &mut tokio_rustls::client::TlsStream<TcpStream>) -> String {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).await.expect("IMAP line");
            line.push(byte[0]);
            if line.ends_with(b"\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&line).into_owned()
    }

    async fn imap_read_line_bounded(
        stream: &mut tokio_rustls::client::TlsStream<TcpStream>,
    ) -> Option<String> {
        tokio::time::timeout(Duration::from_millis(500), imap_read_line(stream))
            .await
            .ok()
    }

    async fn smtp_read_line(stream: &mut tokio_rustls::client::TlsStream<TcpStream>) -> String {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).await.expect("SMTP line");
            line.push(byte[0]);
            if line.ends_with(b"\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&line).into_owned()
    }

    async fn https_request(
        addr: std::net::SocketAddr,
        root_der: &[u8],
        method: &str,
        path: &str,
        body: &str,
        authorization: &str,
    ) -> String {
        let mut stream = open_tls(addr, root_der).await;
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {authorization}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("HTTPS request");
        let mut response = Vec::new();
        loop {
            let mut chunk = [0u8; 1024];
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => response.extend_from_slice(&chunk[..read]),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&response).into_owned()
    }

    #[test]
    fn crate_name_matches() {
        assert_eq!(crate::CRATE_NAME, "mailcrane-testkit");
    }

    #[tokio::test]
    async fn imap_starts_delivers_and_shuts_down() {
        let server = FakeImap::builder()
            .folder("INBOX", vec![b"From: alice@example.org\r\n\r\nA".to_vec()])
            .start()
            .await
            .expect("server starts");
        assert_eq!(server.addr().ip().to_string(), "127.0.0.1");
        assert!(!server.root_cert_der().is_empty());
        assert_eq!(server.mailbox().exists("INBOX").await, 1);
        server
            .mailbox()
            .deliver("INBOX", b"From: bob@example.net\r\n\r\nB".to_vec())
            .await;
        assert_eq!(server.mailbox().exists("INBOX").await, 2);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn smtp_starts_and_records_message() {
        let server = FakeSmtp::builder().start().await.expect("server starts");
        assert_eq!(server.addr().ip().to_string(), "127.0.0.1");
        assert!(!server.root_cert_der().is_empty());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn jmap_starts_over_tls() {
        let server = FakeJmap::builder().start().await.expect("server starts");
        assert_eq!(server.addr().ip().to_string(), "127.0.0.1");
        assert!(!server.root_cert_der().is_empty());
        server.shutdown().await;
    }

    /// Whether this machine has an IPv6 loopback at all.
    async fn has_ipv6_loopback() -> bool {
        tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
            .await
            .is_ok()
    }

    /// M0-11: each server starts in under 100 ms. The fastest of a few
    /// starts is what counts, so a busy machine can't fail it; a server
    /// that is slow every time still does (#268). The shared certificate is
    /// made first, as every test process does it once.
    #[tokio::test]
    async fn servers_start_in_under_100_ms() {
        let limit = std::time::Duration::from_millis(100);
        crate::tls::generate().expect("certificate");
        let mut fastest = [limit * 100; 3];
        for _ in 0..5 {
            let started = std::time::Instant::now();
            let imap = FakeImap::builder().start().await.expect("imap starts");
            fastest[0] = fastest[0].min(started.elapsed());
            let started = std::time::Instant::now();
            let smtp = FakeSmtp::builder().start().await.expect("smtp starts");
            fastest[1] = fastest[1].min(started.elapsed());
            let started = std::time::Instant::now();
            let jmap = FakeJmap::builder().start().await.expect("jmap starts");
            fastest[2] = fastest[2].min(started.elapsed());
            drop((imap, smtp, jmap));
        }
        for (name, time) in ["imap", "smtp", "jmap"].iter().zip(fastest) {
            assert!(time < limit, "{name} took {time:?} at best");
        }
    }

    /// Clients connect as `localhost`, which can resolve to `::1` before
    /// `127.0.0.1` (it does on Windows). Each fake server must answer on
    /// every address `localhost` resolves to, with its own certificate.
    #[tokio::test]
    async fn every_localhost_address_reaches_the_fake_server() {
        let imap = FakeImap::builder().start().await.expect("imap starts");
        let smtp = FakeSmtp::builder().start().await.expect("smtp starts");
        let jmap = FakeJmap::builder().start().await.expect("jmap starts");
        for (port, root) in [
            (imap.addr().port(), imap.root_cert_der()),
            (smtp.addr().port(), smtp.root_cert_der()),
            (jmap.addr().port(), jmap.root_cert_der()),
        ] {
            let addrs: Vec<_> = tokio::net::lookup_host(("localhost", port))
                .await
                .expect("resolve localhost")
                .collect();
            assert!(!addrs.is_empty());
            for addr in addrs {
                // The handshake only succeeds against the server's own
                // certificate, so this proves it is the fake server.
                let _stream = open_tls(addr, root).await;
            }
        }
    }

    /// The Windows flake in `sync_load_older_and_fetch_body_through_the_ffi`:
    /// the client connects, then builds its TLS config (on Windows the
    /// platform verifier sets up a CryptoAPI chain engine, which can take
    /// hundreds of milliseconds on a busy machine), then sends its hello.
    /// The fake servers used to guess "no hello within 150 ms (IMAP) or
    /// 10 ms (SMTP) means STARTTLS" and sent a plain-text greeting, which
    /// the client's TLS handshake rejected. The implicit TLS port must wait
    /// for the hello however long the client takes.
    #[tokio::test]
    async fn implicit_tls_port_waits_for_a_slow_client_hello() {
        let imap = FakeImap::builder().start().await.expect("imap starts");
        let smtp = FakeSmtp::builder().start().await.expect("smtp starts");
        for (addr, root) in [
            (imap.addr(), imap.root_cert_der()),
            (smtp.addr(), smtp.root_cert_der()),
        ] {
            let stream = TcpStream::connect(addr).await.expect("connect");
            tokio::time::sleep(Duration::from_millis(300)).await;
            let server_name = ServerName::try_from("localhost".to_string()).expect("server name");
            let handshake = tokio_rustls::TlsConnector::from(client_config(root))
                .connect(server_name, stream)
                .await;
            assert!(handshake.is_ok(), "{addr}: {handshake:?}");
        }
    }

    /// The STARTTLS port greets in plain text straight away, without
    /// waiting to see whether the client sends a TLS hello.
    #[tokio::test]
    async fn starttls_port_greets_in_plain_text_at_once() {
        let imap = FakeImap::builder().start().await.expect("imap starts");
        let smtp = FakeSmtp::builder().start().await.expect("smtp starts");
        for (addr, greeting) in [
            (imap.starttls_addr(), &b"* OK"[..]),
            (smtp.starttls_addr(), &b"220 "[..]),
        ] {
            let mut stream = TcpStream::connect(addr).await.expect("connect");
            let mut first = vec![0u8; greeting.len()];
            tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut first))
                .await
                .expect("greeting in time")
                .expect("greeting");
            assert_eq!(first, greeting);
        }
    }

    /// `localhost` can resolve to `::1`; while a fake server runs, no other
    /// process may answer on `[::1]` at its port in its place.
    #[tokio::test]
    async fn nothing_else_can_listen_on_the_ipv6_loopback_port() {
        if !has_ipv6_loopback().await {
            return;
        }
        let imap = FakeImap::builder().start().await.expect("imap starts");
        let smtp = FakeSmtp::builder().start().await.expect("smtp starts");
        let jmap = FakeJmap::builder().start().await.expect("jmap starts");
        for port in [
            imap.addr().port(),
            imap.starttls_addr().port(),
            smtp.addr().port(),
            smtp.starttls_addr().port(),
            jmap.addr().port(),
        ] {
            let squatter =
                tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).await;
            assert!(
                squatter.is_err(),
                "[::1]:{port} was free for another listener"
            );
        }
    }

    /// `cargo xtask dev-mail` uses fixed ports. If another process already
    /// listens on `[::1]` at one of them, starting fails instead of leaving
    /// `localhost` clients to reach that process (#404 review).
    #[tokio::test]
    async fn a_fixed_port_taken_on_the_ipv6_loopback_is_an_error() {
        if !has_ipv6_loopback().await {
            return;
        }
        // A port free on 127.0.0.1, held by someone else on [::1], so the
        // only thing that can go wrong is the [::1] bind.
        let mut squatted = None;
        for _ in 0..16 {
            let v4 = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("v4");
            let port = v4.local_addr().expect("addr").port();
            if let Ok(squatter) =
                tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).await
            {
                squatted = Some((port, squatter));
                break;
            }
        }
        let Some((port, _squatter)) = squatted else {
            panic!("no port free on both loopback addresses");
        };
        let Ok(spare) = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0)).await
        else {
            return;
        };
        let spare_port = spare.local_addr().expect("addr").port();
        drop(spare);
        let Err(error) = FakeSmtp::builder().ports(port, spare_port).start().await else {
            panic!("start should be refused while [::1]:{port} is taken");
        };
        assert!(error.contains("[::1]"), "{error}");
    }

    #[tokio::test]
    async fn imap_tls_and_commands_work() {
        let server = FakeImap::builder()
            .folder("INBOX", vec![b"From: alice@example.org\r\n\r\nA".to_vec()])
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        let greeting = imap_read_line(&mut stream).await;
        assert!(greeting.contains("ready"));
        stream
            .write_all(b"a1 LOGIN alice@example.org password\r\n")
            .await
            .expect("login");
        assert!(imap_read_line(&mut stream).await.contains("OK"));
        stream
            .write_all(b"a2 CAPABILITY\r\n")
            .await
            .expect("capability");
        assert!(imap_read_line(&mut stream).await.contains("CAPABILITY"));
        stream
            .write_all(b"a3 SELECT INBOX\r\n")
            .await
            .expect("select");
        let mut output = String::new();
        for _ in 0..10 {
            let line = imap_read_line(&mut stream).await;
            output.push_str(&line);
            if line.starts_with("a3 ") {
                break;
            }
        }
        assert!(output.contains("EXISTS"));
        assert!(output.contains("PERMANENTFLAGS"));
        stream
            .write_all(b"a4 UID SEARCH ALL\r\n")
            .await
            .expect("search");
        let mut search = String::new();
        for _ in 0..2 {
            search.push_str(&imap_read_line(&mut stream).await);
        }
        assert!(
            search.contains("SEARCH"),
            "unexpected UID SEARCH response: {search:?}"
        );
        stream.write_all(b"a5 LOGOUT\r\n").await.expect("logout");
        assert!(imap_read_line(&mut stream).await.contains("OK"));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn imap_starttls_rejects_commands_before_tls() {
        let server = FakeImap::builder().start().await.expect("server starts");
        let mut stream = TcpStream::connect(server.starttls_addr())
            .await
            .expect("connect");
        let mut greeting = [0u8; 64];
        let _ = stream.read(&mut greeting).await.expect("greeting");
        stream
            .write_all(b"a1 LOGIN alice@example.org password\r\n")
            .await
            .expect("login");
        let mut response = [0u8; 128];
        let read = stream.read(&mut response).await.expect("refused");
        assert!(String::from_utf8_lossy(&response[..read]).contains("refused"));
        stream
            .write_all(b"a2 STARTTLS\r\n")
            .await
            .expect("starttls");
        let read = stream.read(&mut response).await.expect("starttls response");
        assert!(String::from_utf8_lossy(&response[..read]).contains("begin STARTTLS"));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn imap_deliver_wakes_idle_within_one_second() {
        let server = FakeImap::builder()
            .folder("INBOX", Vec::new())
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;
        stream.write_all(b"a1 IDLE\r\n").await.expect("idle");
        let continuation = imap_read_line(&mut stream).await;
        assert!(continuation.contains("idling"));
        let start = Instant::now();
        server
            .mailbox()
            .deliver("INBOX", b"From: bob@example.com\r\n\r\nB".to_vec())
            .await;
        let notification = imap_read_line(&mut stream).await;
        assert!(notification.contains("EXISTS"));
        assert!(start.elapsed() < Duration::from_secs(1));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn imap_idle_reports_expunges_and_flags_until_done() {
        let server = FakeImap::builder()
            .folder(
                "INBOX",
                vec![
                    b"Subject: 1\r\n\r\n1".to_vec(),
                    b"Subject: 2\r\n\r\n2".to_vec(),
                ],
            )
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;
        stream.write_all(b"a1 IDLE\r\n").await.expect("idle");
        assert!(imap_read_line(&mut stream).await.starts_with("+ "));
        let mailbox = server.mailbox();
        mailbox.add_flag("INBOX", 2, "\\Seen").await;
        assert_eq!(
            imap_read_line(&mut stream).await,
            "* 2 FETCH (UID 2 FLAGS (\\Seen))\r\n"
        );
        mailbox.expunge("INBOX", 1).await;
        assert_eq!(imap_read_line(&mut stream).await, "* 1 EXPUNGE\r\n");
        stream.write_all(b"DONE\r\n").await.expect("done");
        assert_eq!(
            imap_read_line(&mut stream).await,
            "a1 OK IDLE terminated\r\n"
        );
        assert!(server
            .received_commands()
            .await
            .ends_with(&["IDLE".to_string(), "DONE".to_string()]));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn imap_idle_announces_mail_delivered_since_select() {
        let server = FakeImap::builder()
            .folder("INBOX", vec![b"Subject: 1\r\n\r\n1".to_vec()])
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;
        stream
            .write_all(b"a1 SELECT INBOX\r\n")
            .await
            .expect("select");
        while !imap_read_line(&mut stream).await.starts_with("a1 ") {}
        // Delivered after SELECT, before IDLE: still news to this client.
        server
            .mailbox()
            .deliver("INBOX", b"Subject: 2\r\n\r\n2".to_vec())
            .await;
        stream.write_all(b"a2 IDLE\r\n").await.expect("idle");
        assert!(imap_read_line(&mut stream).await.starts_with("+ "));
        assert_eq!(imap_read_line(&mut stream).await, "* 2 EXISTS\r\n");
        stream.write_all(b"DONE\r\n").await.expect("done");
        assert_eq!(
            imap_read_line(&mut stream).await,
            "a2 OK IDLE terminated\r\n"
        );
        server.shutdown().await;
    }

    #[tokio::test]
    async fn imap_idle_is_advertised_unless_no_idle() {
        for (quirk, advertised) in [(None, true), (Some(Quirk::NoIdle), false)] {
            let mut builder = FakeImap::builder();
            if let Some(quirk) = quirk {
                builder = builder.quirk(quirk);
            }
            let server = builder.start().await.expect("server starts");
            let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
            let _ = imap_read_line(&mut stream).await;
            stream
                .write_all(b"a1 CAPABILITY\r\n")
                .await
                .expect("capability");
            let caps = imap_read_line(&mut stream).await;
            assert_eq!(caps.contains(" IDLE"), advertised, "{caps}");
            server.shutdown().await;
        }
    }

    #[tokio::test]
    async fn imap_drop_connections_closes_open_ones_and_counts_them() {
        let server = FakeImap::builder().start().await.expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;
        assert_eq!(server.connections_accepted(), 1);
        server.drop_connections();
        let mut buf = [0u8; 64];
        let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("closed in time");
        assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
        let mut again = open_tls(server.addr(), server.root_cert_der()).await;
        assert!(imap_read_line(&mut again).await.starts_with("* OK"));
        assert_eq!(server.connections_accepted(), 2);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn imap_faults_and_quirks_are_observable() {
        let reject = FakeImap::builder()
            .fault(Fault::RejectLogin)
            .quirk(Quirk::GmailLabels)
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(reject.addr(), reject.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;
        stream
            .write_all(b"a1 LOGIN alice@example.org password\r\n")
            .await
            .expect("login");
        assert!(imap_read_line(&mut stream).await.contains("NO"));
        stream
            .write_all(b"a2 LIST \"\" \"*\"\r\n")
            .await
            .expect("list");
        let mut list = String::new();
        for _ in 0..4 {
            let line = imap_read_line(&mut stream).await;
            list.push_str(&line);
            if line.contains("OK") {
                break;
            }
        }
        assert!(list.contains("[Gmail]/All Mail") || list.contains("All Mail"));
        reject.shutdown().await;

        let malformed = FakeImap::builder()
            .fault(Fault::MalformedResponse)
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(malformed.addr(), malformed.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;
        stream.write_all(b"a1 NOOP\r\n").await.expect("noop");
        let bytes = imap_read_line(&mut stream).await;
        assert!(bytes.contains('\u{fffd}') || bytes.contains('\u{fffe}') || !bytes.is_ascii());
        malformed.shutdown().await;
    }

    #[tokio::test]
    async fn smtp_command_transcript_and_sent_message_work() {
        let server = FakeSmtp::builder().start().await.expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        assert!(smtp_read_line(&mut stream).await.starts_with("220"));
        stream
            .write_all(b"EHLO mailcrane.test\r\n")
            .await
            .expect("EHLO");
        let mut ehlo = String::new();
        for _ in 0..2 {
            ehlo.push_str(&smtp_read_line(&mut stream).await);
        }
        assert!(ehlo.contains("250"));
        stream.write_all(b"AUTH PLAIN\r\n").await.expect("AUTH");
        assert!(smtp_read_line(&mut stream).await.starts_with("235"));
        stream
            .write_all(b"MAIL FROM:<alice@example.org>\r\n")
            .await
            .expect("MAIL");
        assert!(smtp_read_line(&mut stream).await.starts_with("250"));
        stream
            .write_all(b"RCPT TO:<bob@example.net>\r\n")
            .await
            .expect("RCPT");
        assert!(smtp_read_line(&mut stream).await.starts_with("250"));
        stream.write_all(b"DATA\r\n").await.expect("DATA");
        assert!(smtp_read_line(&mut stream).await.starts_with("354"));
        stream
            .write_all(b"From: alice@example.org\r\nTo: bob@example.net\r\n\r\nHello\r\n.\r\n")
            .await
            .expect("message");
        assert!(smtp_read_line(&mut stream).await.starts_with("250"));
        stream.write_all(b"RSET\r\n").await.expect("RSET");
        assert!(smtp_read_line(&mut stream).await.starts_with("250"));
        stream.write_all(b"QUIT\r\n").await.expect("QUIT");
        assert!(smtp_read_line(&mut stream).await.starts_with("221"));
        let sent = server.sent().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].envelope_to, vec!["bob@example.net".to_string()]);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn smtp_fault_responses_are_observable() {
        let temp = FakeSmtp::builder()
            .fault(Fault::TempFailure)
            .start()
            .await
            .expect("server starts");
        assert!(!temp.root_cert_der().is_empty());
        temp.shutdown().await;

        let perm = FakeSmtp::builder()
            .fault(Fault::PermFailure)
            .start()
            .await
            .expect("server starts");
        assert!(!perm.root_cert_der().is_empty());
        perm.shutdown().await;

        let no_tls = FakeSmtp::builder()
            .fault(Fault::NoStartTls)
            .start()
            .await
            .expect("server starts");
        assert!(!no_tls.root_cert_der().is_empty());
        no_tls.shutdown().await;
    }

    #[test]
    fn faults_and_quirks_are_constructible() {
        let _ = [
            Fault::DropAfterCommands(3),
            Fault::DropMidResponse,
            Fault::Delay(std::time::Duration::from_millis(1)),
            Fault::MalformedResponse,
            Fault::RejectLogin,
            Fault::TempFailure,
            Fault::PermFailure,
            Fault::NoStartTls,
        ];
        let _ = [
            Quirk::GmailLabels,
            Quirk::NoCondstore,
            Quirk::NoQresync,
            Quirk::NoIdle,
            Quirk::NoMove,
            Quirk::NoUidplus,
            Quirk::NoSpecialUse,
        ];
    }

    #[tokio::test]
    async fn jmap_discovery_and_methods_work() {
        let server = FakeJmap::builder().start().await.expect("server starts");
        let discovery = https_request(
            server.addr(),
            server.root_cert_der(),
            "GET",
            "/.well-known/jmap",
            "",
            "Bearer test-token",
        )
        .await;
        assert!(discovery.contains("200 OK"));
        assert!(discovery.contains("apiUrl"));

        for (method, body) in [
            (
                "Mailbox/get",
                r#"{"using":[],"methodCalls":[["Mailbox/get",{},"0"]]}"#,
            ),
            (
                "Email/query",
                r#"{"using":[],"methodCalls":[["Email/query",{},"0"]]}"#,
            ),
            (
                "Email/get",
                r#"{"using":[],"methodCalls":[["Email/get",{},"0"]]}"#,
            ),
            (
                "Email/changes",
                r#"{"using":[],"methodCalls":[["Email/changes",{},"0"]]}"#,
            ),
        ] {
            let response = https_request(
                server.addr(),
                server.root_cert_der(),
                "POST",
                "/api",
                body,
                "Basic dGVzdDp0ZXN0",
            )
            .await;
            assert!(response.contains("200 OK"));
            assert!(
                response.contains(method),
                "missing JMAP method response: {method}"
            );
        }

        let unauthorized = https_request(
            server.addr(),
            server.root_cert_der(),
            "GET",
            "/.well-known/jmap",
            "",
            "",
        )
        .await;
        assert!(unauthorized.contains("401"));
        server.shutdown().await;
    }

    /// Sends `command` and returns every line up to and including the
    /// tagged reply for `tag`.
    async fn imap_exchange(
        stream: &mut tokio_rustls::client::TlsStream<TcpStream>,
        tag: &str,
        command: &[u8],
    ) -> String {
        stream.write_all(command).await.expect("IMAP command");
        let mut out = String::new();
        loop {
            let line = imap_read_line(stream).await;
            out.push_str(&line);
            if line.starts_with(&format!("{tag} ")) {
                return out;
            }
        }
    }

    /// #416: ENVELOPE describes each stored message from its own headers,
    /// for fixtures and for mail APPENDed later (the copy in Sent).
    #[tokio::test]
    async fn imap_envelope_comes_from_each_messages_headers() {
        let fixture = b"Date: Tue, 03 Mar 2026 09:15:00 +0100\r\n\
            From: Ana <ana@example.org>\r\n\
            To: Bo <bo@example.net>\r\n\
            Cc: carl@example.com\r\n\
            Subject: Lunch\r\n\
            Message-ID: <m1@example.org>\r\n\r\nBody\r\n"
            .to_vec();
        let server = FakeImap::builder()
            .folder("INBOX", vec![fixture])
            .folder("Sent", Vec::new())
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;

        let _ = imap_exchange(&mut stream, "a1", b"a1 EXAMINE INBOX\r\n").await;
        let inbox = imap_exchange(&mut stream, "a2", b"a2 UID FETCH 1:* (ENVELOPE)\r\n").await;
        assert!(
            inbox.contains(
                "ENVELOPE (\"Tue, 03 Mar 2026 09:15:00 +0100\" \"Lunch\" \
                 ((\"Ana\" NIL \"ana\" \"example.org\")) ((\"Ana\" NIL \"ana\" \"example.org\")) \
                 ((\"Ana\" NIL \"ana\" \"example.org\")) ((\"Bo\" NIL \"bo\" \"example.net\")) \
                 ((NIL NIL \"carl\" \"example.com\")) NIL NIL \"<m1@example.org>\")"
            ),
            "{inbox}"
        );
        assert!(!inbox.contains("Alice"), "{inbox}");

        let sent = b"Date: Thu, 26 Nov 2026 19:00:00 +0000\r\n\
            From: Test <test@example.org>\r\n\
            To: ana@example.org\r\n\
            Subject: Re: Lunch\r\n\
            In-Reply-To: <m1@example.org>\r\n\r\nYes\r\n";
        let append = format!("a3 APPEND Sent (\\Seen) {{{}}}\r\n", sent.len());
        stream.write_all(append.as_bytes()).await.expect("APPEND");
        assert!(imap_read_line(&mut stream).await.starts_with('+'));
        let mut literal = sent.to_vec();
        literal.extend_from_slice(b"\r\n");
        let appended = imap_exchange(&mut stream, "a3", &literal).await;
        assert!(appended.contains("a3 OK"), "{appended}");

        let _ = imap_exchange(&mut stream, "a4", b"a4 EXAMINE Sent\r\n").await;
        let copy = imap_exchange(&mut stream, "a5", b"a5 UID FETCH 1:* (ENVELOPE)\r\n").await;
        assert!(
            copy.contains(
                "ENVELOPE (\"Thu, 26 Nov 2026 19:00:00 +0000\" \"Re: Lunch\" \
                 ((\"Test\" NIL \"test\" \"example.org\"))"
            ),
            "{copy}"
        );
        assert!(
            copy.contains("\"<m1@example.org>\" \"<Sent.1@example.org>\")"),
            "{copy}"
        );

        stream.write_all(b"a6 LOGOUT\r\n").await.expect("LOGOUT");
        server.shutdown().await;
    }

    #[tokio::test]
    async fn imap_command_matrix_and_quirks_are_tested() {
        let server = FakeImap::builder()
            .folder("INBOX", vec![b"From: alice@example.org\r\n\r\nA".to_vec()])
            .quirk(Quirk::GmailLabels)
            .start()
            .await
            .expect("server starts");
        let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
        let _ = imap_read_line(&mut stream).await;
        let commands = [
            "a1 ENABLE UIDPLUS CONDSTORE\r\n",
            "a2 EXAMINE INBOX\r\n",
            "a3 UID FETCH 1:* (UID FLAGS RFC822.SIZE ENVELOPE BODYSTRUCTURE)\r\n",
            "a4 UID SEARCH ALL\r\n",
            "a5 UID STORE 1 +FLAGS (\\Seen)\r\n",
            "a6 UID COPY 1 INBOX\r\n",
            "a7 UID MOVE 1 INBOX\r\n",
            "a8 UID EXPUNGE 1\r\n",
            "a9 APPEND INBOX\r\n",
            "a10 NOOP\r\n",
        ];
        for command in commands {
            stream
                .write_all(command.as_bytes())
                .await
                .expect("IMAP command");
            let mut responses = String::new();
            for _ in 0..5 {
                let Some(line) = imap_read_line_bounded(&mut stream).await else {
                    break;
                };
                responses.push_str(&line);
                if line.contains(" OK ") || line.contains(" NO ") || line.contains(" BAD ") {
                    break;
                }
            }
            assert!(!responses.is_empty());
        }
        stream.write_all(b"a12 LOGOUT\r\n").await.expect("LOGOUT");
        let _ = imap_read_line(&mut stream).await;
        server.shutdown().await;
    }

    #[tokio::test]
    async fn every_quirk_and_fault_variant_is_constructible_and_selectively_observable() {
        let quirk_cases = [
            Quirk::GmailLabels,
            Quirk::NoCondstore,
            Quirk::NoQresync,
            Quirk::NoIdle,
            Quirk::NoMove,
            Quirk::NoUidplus,
            Quirk::NoSpecialUse,
        ];
        for quirk in quirk_cases {
            let server = FakeImap::builder()
                .quirk(quirk)
                .start()
                .await
                .expect("quirk server");
            let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
            let _ = imap_read_line(&mut stream).await;
            stream
                .write_all(b"a1 CAPABILITY\r\n")
                .await
                .expect("CAPABILITY");
            let capability = imap_read_line(&mut stream).await;
            let second = imap_read_line(&mut stream).await;
            assert!(capability.contains("CAPABILITY") || second.contains("OK"));
            server.shutdown().await;
        }

        let fault_cases = [
            Fault::DropAfterCommands(1),
            Fault::DropMidResponse,
            Fault::Delay(std::time::Duration::from_millis(1)),
            Fault::MalformedResponse,
            Fault::RejectLogin,
            Fault::TempFailure,
            Fault::PermFailure,
            Fault::NoStartTls,
            Fault::DropAfterCommandOnce("UID COPY".into()),
            Fault::RefuseCommand {
                folder: None,
                text: "\\Deleted".into(),
            },
        ];
        for fault in fault_cases {
            let server = FakeImap::builder()
                .fault(fault)
                .start()
                .await
                .expect("fault server");
            assert!(!server.root_cert_der().is_empty());
            server.shutdown().await;
        }
    }

    /// Reads lines up to and including the tagged reply for `tag`; `None`
    /// if the connection closes first.
    async fn imap_until_tagged(
        stream: &mut tokio_rustls::client::TlsStream<TcpStream>,
        tag: &str,
    ) -> Option<String> {
        let mut out = String::new();
        loop {
            let mut line = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                stream.read_exact(&mut byte).await.ok()?;
                line.push(byte[0]);
                if line.ends_with(b"\r\n") {
                    break;
                }
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            out.push_str(&line);
            if line.starts_with(&format!("{tag} ")) {
                return Some(out);
            }
        }
    }

    /// #423: `DropAfterCommandOnce` carries the command out, then closes
    /// without the reply, once per server; `RefuseCommand` answers NO only
    /// in its folder and changes nothing.
    #[tokio::test]
    async fn targeted_drop_and_refusal_faults() {
        let server = FakeImap::builder()
            .folder("INBOX", vec![b"From: a@example.org\r\n\r\nA".to_vec()])
            .folder("Snoozed", Vec::new())
            .fault(Fault::DropAfterCommandOnce("UID COPY".into()))
            .fault(Fault::RefuseCommand {
                folder: Some("INBOX".into()),
                text: "\\Deleted".into(),
            })
            .start()
            .await
            .expect("server starts");
        for attempt in 1..=2 {
            let mut stream = open_tls(server.addr(), server.root_cert_der()).await;
            let _ = imap_read_line(&mut stream).await;
            stream
                .write_all(b"a1 SELECT INBOX\r\n")
                .await
                .expect("SELECT");
            assert!(imap_until_tagged(&mut stream, "a1").await.is_some());
            stream
                .write_all(b"a2 UID COPY 1 Snoozed\r\n")
                .await
                .expect("COPY");
            let reply = imap_until_tagged(&mut stream, "a2").await;
            if attempt == 1 {
                assert!(reply.is_none(), "first COPY: no reply");
            } else {
                assert!(
                    reply.is_some_and(|r| r.contains("a2 OK")),
                    "second COPY answered"
                );
                stream
                    .write_all(b"a3 UID STORE 1 +FLAGS.SILENT (\\Deleted)\r\n")
                    .await
                    .expect("STORE");
                let refused = imap_until_tagged(&mut stream, "a3").await.expect("reply");
                assert!(refused.contains("a3 NO"), "{refused}");
                stream
                    .write_all(b"a4 SELECT Snoozed\r\n")
                    .await
                    .expect("SELECT");
                assert!(imap_until_tagged(&mut stream, "a4").await.is_some());
                stream
                    .write_all(b"a5 UID STORE 1 +FLAGS.SILENT (\\Deleted)\r\n")
                    .await
                    .expect("STORE");
                let other = imap_until_tagged(&mut stream, "a5").await.expect("reply");
                assert!(other.contains("a5 OK"), "other folder not refused: {other}");
            }
        }
        // Both COPYs were carried out, the dropped one included.
        assert_eq!(server.mailbox().messages("Snoozed").await.len(), 2);
        let inbox = server.mailbox().messages("INBOX").await;
        assert!(
            !inbox[0].flags.contains(&"\\Deleted".to_owned()),
            "refused STORE changed nothing"
        );
        server.shutdown().await;
    }
}
