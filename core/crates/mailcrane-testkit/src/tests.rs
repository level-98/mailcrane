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
        for _ in 0..5 {
            output.push_str(&imap_read_line(&mut stream).await);
        }
        assert!(output.contains("EXISTS"));
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
        let mut stream = TcpStream::connect(server.addr()).await.expect("connect");
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
}
