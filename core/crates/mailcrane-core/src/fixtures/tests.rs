use super::*;
use mailcrane_store::{NewAccount, Security};
use std::path::Path;

const NOW: i64 = 1_790_000_000;

fn file(path: &str, bytes: &[u8]) -> FixtureFile {
    FixtureFile {
        path: path.into(),
        bytes: bytes.to_vec(),
    }
}

fn eml(subject: &str) -> Vec<u8> {
    format!(
        "From: Ann <ann@example.org>\r\nTo: bob@example.com\r\nSubject: {subject}\r\n\
         Message-ID: <{subject}@example.org>\r\n\r\nHello {subject}\r\n"
    )
    .into_bytes()
}

/// Every `.eml` and `.html` under `dir`, with paths relative to it.
fn read_dir_files(root: &Path, dir: &Path, out: &mut Vec<FixtureFile>) {
    let entries = std::fs::read_dir(dir).expect("fixtures dir");
    for entry in entries {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            read_dir_files(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .expect("relative")
                .to_string_lossy()
                .replace('\\', "/");
            out.push(FixtureFile {
                path: rel,
                bytes: std::fs::read(&path).expect("read"),
            });
        }
    }
}

#[test]
fn imports_into_a_local_only_inbox() {
    let store = Store::open_in_memory().expect("store");
    let files = [
        file("mime/a.eml", &eml("first")),
        file(
            "html/page.html",
            b"<p><a href=\"https://example.net/\">example.com</a></p>",
        ),
        file("address/README.md", b"not a message"),
    ];
    let report = import_fixtures(&store, &files, NOW).expect("import");
    assert_eq!(report.imported, 2);
    assert_eq!(report.skipped, 1);

    assert!(store.is_local_account(report.account_id).expect("local"));
    let account = store
        .get_account(report.account_id)
        .expect("get")
        .expect("exists");
    assert_eq!(account.details.email, FIXTURES_EMAIL);
    assert_eq!(account.details.display_name, FIXTURES_NAME);
    assert!(account.details.imap_host.is_empty());
    assert!(account.details.smtp_host.is_empty());

    let folder = store.get_folder(report.inbox).expect("get").expect("inbox");
    assert_eq!(folder.details.role, Some(FolderRole::Inbox));
    let messages = store.list_messages(report.inbox, 100, 0).expect("list");
    assert_eq!(messages.len(), 2);
    // The HTML file became a message with its path as the subject, and its
    // body is stored and parses back to the same HTML.
    let page = messages
        .iter()
        .find(|m| m.details.subject == "html/page.html")
        .expect("html message");
    let raw = store.get_body(page.id).expect("body").expect("stored");
    let parsed = mailcrane_mime::parse_message(&raw).expect("parses");
    let html = parsed.text_html.expect("html part");
    // Test-only read of the raw HTML, to compare it with the fixture.
    assert!(html
        .as_unsanitised_str()
        .contains("<a href=\"https://example.net/\">example.com</a>"));
    assert_eq!(
        parsed.from.first().map(|a| a.email.as_str()),
        Some(FIXTURES_EMAIL)
    );
}

#[test]
fn running_twice_replaces_instead_of_adding() {
    let store = Store::open_in_memory().expect("store");
    let files = [file("mime/a.eml", &eml("a")), file("mime/b.eml", &eml("b"))];
    let first = import_fixtures(&store, &files, NOW).expect("first");
    let second = import_fixtures(&store, &files, NOW + 60).expect("second");
    assert_eq!(first.account_id, second.account_id);
    assert_eq!(first.inbox, second.inbox);
    assert_eq!(
        store
            .list_messages(second.inbox, 100, 0)
            .expect("list")
            .len(),
        2
    );
    assert_eq!(store.list_accounts().expect("accounts").len(), 1);
    // Fewer files the second time: the old messages are gone.
    let third = import_fixtures(&store, &files[..1], NOW).expect("third");
    assert_eq!(
        store
            .list_messages(third.inbox, 100, 0)
            .expect("list")
            .len(),
        1
    );
}

#[test]
fn other_accounts_are_untouched() {
    let store = Store::open_in_memory().expect("store");
    let other = store
        .insert_account(&NewAccount {
            display_name: "Real".into(),
            email: FIXTURES_EMAIL.into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: "real".into(),
        })
        .expect("account");
    let report = import_fixtures(&store, &[file("a.eml", &eml("a"))], NOW).expect("import");
    // Even an ordinary account with the same address is never reused.
    assert_ne!(report.account_id, other);
    assert!(!store.is_local_account(other).expect("remote"));
    assert!(store.list_folders(other).expect("folders").is_empty());
}

#[test]
fn dates_are_spread_newest_first() {
    let store = Store::open_in_memory().expect("store");
    let files: Vec<FixtureFile> = (0..10)
        .map(|i| file(&format!("m{i}.eml"), &eml(&format!("s{i}"))))
        .collect();
    let report = import_fixtures(&store, &files, NOW).expect("import");
    let mut dates: Vec<i64> = store
        .list_messages(report.inbox, 100, 0)
        .expect("list")
        .iter()
        .map(|m| m.details.date)
        .collect();
    dates.sort_unstable();
    dates.dedup();
    assert_eq!(dates.len(), 10, "every message has its own date");
    assert_eq!(dates.last(), Some(&NOW));
    let oldest = dates.first().copied().unwrap_or_default();
    assert!(NOW - oldest > 60 * 24 * 60 * 60, "spread over months");
    assert!(NOW - oldest <= SPREAD_SECONDS);
}

#[test]
fn oversized_and_unparseable_files() {
    let store = Store::open_in_memory().expect("store");
    let files = [
        file("big.eml", &vec![b'a'; MAX_FILE_BYTES + 1]),
        file("junk.eml", &[0xFF, 0x00, 0xFE, b'\n', b'\r']),
        file("empty.html", b""),
    ];
    let report = import_fixtures(&store, &files, NOW).expect("import");
    assert_eq!(report.imported, 2);
    assert_eq!(report.skipped, 1);
    // A message with no subject is listed under its path.
    let subjects: Vec<String> = store
        .list_messages(report.inbox, 100, 0)
        .expect("list")
        .into_iter()
        .map(|m| m.details.subject)
        .collect();
    assert!(subjects.iter().any(|s| s == "junk.eml"), "{subjects:?}");
    assert!(subjects.iter().any(|s| s == "empty.html"), "{subjects:?}");
}

/// Every file under the repository's `tests/fixtures`.
fn repository_fixtures() -> Vec<FixtureFile> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures");
    let mut files = Vec::new();
    read_dir_files(&root, &root, &mut files);
    files
}

#[test]
fn every_repository_fixture_imports() {
    let files = repository_fixtures();
    let messages = files
        .iter()
        .filter(|f| f.path.ends_with(".eml") || f.path.ends_with(".html"))
        .count();
    assert!(messages > 50, "found {messages}");

    let store = Store::open_in_memory().expect("store");
    let report = import_fixtures(&store, &files, NOW).expect("import");
    assert_eq!(report.imported, messages);
    let listed = store.list_messages(report.inbox, 10_000, 0).expect("list");
    let sent = store.list_messages(report.sent, 10_000, 0).expect("sent");
    assert_eq!(listed.len() + sent.len(), messages);
    // The link fixture #267 is checked with is there (when this branch has it).
    if files.iter().any(|f| f.path == "html/link-mismatch.html") {
        assert!(listed
            .iter()
            .any(|m| m.details.subject == "html/link-mismatch.html"));
    }
}

#[test]
fn sent_fixtures_go_in_sent() {
    let store = Store::open_in_memory().expect("store");
    let files = [
        file("t/1-reply.eml", &eml("in")),
        file("t/2-mine.sent.eml", &eml("out")),
    ];
    let report = import_fixtures(&store, &files, NOW).expect("import");
    assert_eq!(report.imported, 2);
    let subjects = |folder| -> Vec<String> {
        store
            .list_messages(folder, 100, 0)
            .expect("list")
            .into_iter()
            .map(|m| m.details.subject)
            .collect()
    };
    assert_eq!(subjects(report.inbox), ["in"]);
    assert_eq!(subjects(report.sent), ["out"]);
    let sent = store.get_folder(report.sent).expect("get").expect("sent");
    assert_eq!(sent.details.role, Some(FolderRole::Sent));
}

/// The conversations in `tests/fixtures/conversations` (#573): QA needs them to
/// check conversation features with Load test mail.
#[test]
fn the_conversation_fixtures_group_into_conversations() {
    let store = Store::open_in_memory().expect("store");
    let report = import_fixtures(&store, &repository_fixtures(), NOW).expect("import");
    let threads = store
        .list_threads(report.inbox, 0, 10_000)
        .expect("threads");
    let count_of = |subject: &str| -> Vec<u32> {
        threads
            .iter()
            .filter(|t| t.subject.trim_start_matches("Re: ") == subject)
            .map(|t| t.message_count)
            .collect()
    };
    assert_eq!(count_of("Lunch on Friday?"), [2]);
    assert_eq!(count_of("Release plans"), [4]);
    assert_eq!(count_of("Train times for the trip"), [2]);
    // The reply in Sent is part of the same conversation.
    let trip = threads
        .iter()
        .find(|t| t.subject.ends_with("Train times for the trip"))
        .expect("trip");
    let sent = store.list_threads(report.sent, 0, 100).expect("sent");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent.first().map(|t| t.thread_id), Some(trip.thread_id));
    // The newest message is the conversation's latest reply.
    let newest = store
        .get_message_header(trip.newest_message_id)
        .expect("get")
        .expect("newest");
    assert_eq!(
        newest.details.message_id_header.as_deref(),
        Some("thread-trip-3@example.com")
    );
}

#[test]
fn base64_matches_rfc_4648_vectors() {
    let enc = |b: &[u8]| {
        let mut s = String::new();
        base64_lines(b, &mut s);
        s
    };
    assert_eq!(enc(b""), "");
    assert_eq!(enc(b"f"), "Zg==\r\n");
    assert_eq!(enc(b"fo"), "Zm8=\r\n");
    assert_eq!(enc(b"foo"), "Zm9v\r\n");
    assert_eq!(enc(b"foobar"), "Zm9vYmFy\r\n");
    // 57 bytes per 76-character line.
    let long = enc(&[0u8; 58]);
    let lines: Vec<&str> = long.split("\r\n").collect();
    assert_eq!(lines.first().map(|l| l.len()), Some(76));
    assert_eq!(lines.get(1).copied(), Some("AA=="));
}
