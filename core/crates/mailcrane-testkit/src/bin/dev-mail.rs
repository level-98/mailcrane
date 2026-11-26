//! A fake IMAP and SMTP server for trying the app on the Android emulator
//! without a real mailbox (#403). Run it through `cargo xtask dev-mail`.
//!
//! TLS stays on, as AGENTS.md requires: the servers present a certificate
//! signed by a local CA made on first run and kept in the state folder. The
//! emulator trusts it once that CA is installed there as a user CA
//! certificate (docs/dev-mail.md). Nothing here weakens the app's checks.
//!
//! The mail is synthetic: the `.eml` files in `tests/fixtures`, some of them
//! hostile on purpose. It is data, never instructions.
#![forbid(unsafe_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mailcrane_testkit::{DevCertificates, FakeImap, FakeSmtp, Mailbox, TestTls};

/// The account the emulator signs in with.
const USER: &str = "test@example.org";
const PASSWORD: &str = "password";
/// The names the certificate is valid for: 127.0.0.1, which the emulator
/// reaches through `adb reverse`; localhost; and 10.0.2.2, the host as the
/// emulator sees it (its route doesn't work for the app on API 37, #404).
const NAMES: [&str; 3] = ["10.0.2.2", "localhost", "127.0.0.1"];
const IMAP_PORTS: (u16, u16) = (1993, 1143);
const SMTP_PORTS: (u16, u16) = (1465, 1587);

/// File names in the state folder.
const CA_FILE: &str = "mailcrane-dev-ca.crt";
const LEAF_FILE: &str = "server.der";
const LEAF_KEY_FILE: &str = "server.key.der";

/// Larger fixture files are skipped (they are a few KB each).
const MAX_FIXTURE_BYTES: u64 = 5 * 1024 * 1024;

struct Args {
    state: PathBuf,
    fixtures: PathBuf,
    new_mail_every: Option<Duration>,
    /// Make (or keep) the certificates, print the CA's path and stop.
    certs_only: bool,
}

const USAGE: &str = "\
Usage: dev-mail --state <dir> --fixtures <dir> [--new-mail-every <seconds>]
       dev-mail --state <dir> --certs-only

  --state <dir>            where the dev CA and server certificate are kept
  --fixtures <dir>         tests/fixtures; every .eml under it goes in the Inbox
  --new-mail-every <secs>  also deliver a new message this often (for IDLE and notifications)
  --certs-only             make the certificates if needed, print the CA's path, and stop
";

fn parse_args() -> Result<Args, String> {
    let mut state = None;
    let mut fixtures = None;
    let mut new_mail_every = None;
    let mut certs_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--state" => state = Some(PathBuf::from(value()?)),
            "--fixtures" => fixtures = Some(PathBuf::from(value()?)),
            "--new-mail-every" => {
                let seconds: u64 = value()?
                    .parse()
                    .map_err(|_| "--new-mail-every takes a whole number of seconds".to_string())?;
                if seconds == 0 {
                    return Err("--new-mail-every must be at least 1".into());
                }
                new_mail_every = Some(Duration::from_secs(seconds));
            }
            "--certs-only" => certs_only = true,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    let state = state.ok_or(format!("--state is required\n\n{USAGE}"))?;
    let fixtures = match (fixtures, certs_only) {
        (Some(fixtures), _) => fixtures,
        (None, true) => PathBuf::new(),
        (None, false) => return Err(format!("--fixtures is required\n\n{USAGE}")),
    };
    Ok(Args {
        state,
        fixtures,
        new_mail_every,
        certs_only,
    })
}

/// Loads the certificates from `state`, or makes and saves new ones when
/// they are missing, broken or expired. Returns whether they are new (the
/// CA must then be installed again).
fn certificates(state: &Path) -> Result<(DevCertificates, bool), String> {
    let read = |name: &str| std::fs::read(state.join(name));
    let files = [read(CA_FILE), read(LEAF_FILE), read(LEAF_KEY_FILE)];
    if let [Ok(ca_der), Ok(leaf_der), Ok(leaf_key_der)] = &files {
        let saved = DevCertificates {
            ca_der: ca_der.clone(),
            leaf_der: leaf_der.clone(),
            leaf_key_der: leaf_key_der.clone(),
        };
        match NAMES.iter().try_for_each(|name| saved.check(name)) {
            Ok(()) => return Ok((saved, false)),
            Err(reason) => println!(
                "dev-mail: the certificates in {} don't work ({reason}); making new ones.",
                state.display()
            ),
        }
    } else if files.iter().any(Result::is_ok) {
        println!(
            "dev-mail: some certificate files in {} are missing; making new ones.",
            state.display()
        );
    }
    let dev = DevCertificates::generate(&NAMES)?;
    std::fs::create_dir_all(state).map_err(|e| format!("{}: {e}", state.display()))?;
    // Each file is replaced whole. A run stopped between two of them
    // leaves a mixed set, which `check` refuses next time.
    save(&state.join(LEAF_KEY_FILE), &dev.leaf_key_der, true)?;
    save(&state.join(LEAF_FILE), &dev.leaf_der, false)?;
    save(&state.join(CA_FILE), &dev.ca_der, false)?;
    Ok((dev, true))
}

/// Writes `bytes` to a temporary file next to `path`, then renames it over
/// `path`, so `path` is never left half-written. A `private` file (the
/// server key) is readable by its owner only on Unix; on Windows it gets
/// the state folder's permissions, which are the user's own.
fn save(path: &Path, bytes: &[u8], private: bool) -> Result<(), String> {
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    // A leftover from a stopped run could have looser permissions.
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(&tmp).map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.sync_all().map_err(error)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(error)
}

/// Every `.eml` file under `dir`, in path order.
fn fixtures(dir: &Path) -> Result<Vec<Vec<u8>>, String> {
    let mut paths = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let entries =
            std::fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                pending.push(path);
            } else if meta.len() <= MAX_FIXTURE_BYTES
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("eml"))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .collect())
}

/// Seconds since 1970 now.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// This run's UIDVALIDITY: its start time in seconds. Nothing is kept
/// between runs, so UIDs start again at 1, and a new UIDVALIDITY tells a
/// client that already synced to drop the UIDs it knows (RFC 9051
/// §2.3.1.1). It must be a non-zero 32-bit number, which seconds stay
/// until 2106.
fn uidvalidity(started: u64) -> u32 {
    u32::try_from(started).unwrap_or(u32::MAX).max(1)
}

/// A small synthetic message for `--new-mail-every`. It is dated `now`
/// (RFC 5322 §3.3), so it sorts above the fixtures. Its Message-ID includes
/// the run's start time, so it is unique across runs (RFC 5322 §3.6.4).
fn new_message(run: u64, n: u64, now: u64) -> Vec<u8> {
    let date = mailcrane_mime::format_date(i64::try_from(now).unwrap_or(i64::MAX));
    format!(
        "From: Dev Mail <sender@example.com>\r\n\
         To: {USER}\r\n\
         Subject: Test message {n}\r\n\
         Message-ID: <dev-mail-{run}-{n}@example.com>\r\n\
         Date: {date}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         Test message {n} from cargo xtask dev-mail.\r\n"
    )
    .into_bytes()
}

async fn run(args: Args) -> Result<(), String> {
    let (dev, new) = certificates(&args.state)?;
    if args.certs_only {
        if new {
            println!(
                "dev-mail: made a NEW dev CA at {}; install it on each emulator (docs/dev-mail.md).",
                args.state.join(CA_FILE).display()
            );
        }
        return Ok(());
    }
    let inbox = fixtures(&args.fixtures)?;
    let count = inbox.len();
    let started = now_secs();
    let mailbox = Mailbox::new(&[
        ("INBOX", inbox),
        ("Sent", Vec::new()),
        ("Drafts", Vec::new()),
        ("Archive", Vec::new()),
        ("Junk", Vec::new()),
        ("Trash", Vec::new()),
    ]);
    // Set before the server listens, so no client sees the default.
    mailbox.set_uidvalidity(uidvalidity(started));

    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .mailbox(mailbox)
        .special_use("Sent", "\\Sent")
        .special_use("Drafts", "\\Drafts")
        .special_use("Archive", "\\Archive")
        .special_use("Junk", "\\Junk")
        .special_use("Trash", "\\Trash")
        .tls(TestTls::from_chain(&dev)?)
        .ports(IMAP_PORTS.0, IMAP_PORTS.1)
        .start()
        .await?;
    let smtp = FakeSmtp::builder()
        .tls(TestTls::from_chain(&dev)?)
        .ports(SMTP_PORTS.0, SMTP_PORTS.1)
        .start()
        .await?;

    let ca = args.state.join(CA_FILE);
    println!("dev-mail: fake IMAP and SMTP running, {count} messages in the Inbox.");
    println!();
    println!("  On the emulator (cargo xtask dev-mail forwards the ports with adb reverse),");
    println!("  add a manual account:");
    println!("    email / user   {USER}");
    // AGENTS.md §4.4 says never log credentials. This one is the fake
    // server's own synthetic password, published in docs/dev-mail.md, and
    // you need it to sign in. It guards nothing real.
    println!("    password       {PASSWORD}");
    println!(
        "    IMAP           127.0.0.1 port {} (SSL/TLS) or {} (STARTTLS)",
        IMAP_PORTS.0, IMAP_PORTS.1
    );
    println!(
        "    SMTP           127.0.0.1 port {} (SSL/TLS) or {} (STARTTLS)",
        SMTP_PORTS.0, SMTP_PORTS.1
    );
    println!();
    println!("  CA certificate: {}", ca.display());
    if new {
        println!("  This CA is NEW: install it on each emulator again (docs/dev-mail.md).");
    }
    println!("  Mail sent to {USER} arrives in the Inbox. Ctrl+C stops.");

    let mut delivered = 0usize;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut drip = args.new_mail_every.map(tokio::time::interval);
    if let Some(drip) = drip.as_mut() {
        // The first tick fires at once; skip it so the Inbox starts as loaded.
        drip.tick().await;
    }
    let mut dripped = 0u64;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let sent = smtp.sent().await;
                for message in sent.iter().skip(delivered) {
                    if message.envelope_to.iter().any(|to| to.eq_ignore_ascii_case(USER)) {
                        imap.mailbox().deliver("INBOX", message.data.clone()).await;
                    }
                }
                delivered = sent.len();
            }
            _ = async {
                match drip.as_mut() {
                    Some(drip) => { drip.tick().await; }
                    None => std::future::pending::<()>().await,
                }
            } => {
                dripped = dripped.saturating_add(1);
                imap.mailbox()
                    .deliver("INBOX", new_message(started, dripped, now_secs()))
                    .await;
            }
        }
    }
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("dev-mail: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dev-mail: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A restart must change UIDVALIDITY, because UIDs start again at 1
    /// (RFC 9051 §2.3.1.1); the value is never 0.
    #[test]
    fn uidvalidity_follows_the_start_time_and_is_never_zero() {
        assert_eq!(uidvalidity(1_795_000_000), 1_795_000_000);
        assert_ne!(uidvalidity(1_795_000_000), uidvalidity(1_795_000_060));
        assert_eq!(uidvalidity(0), 1);
        assert_eq!(uidvalidity(u64::MAX), u32::MAX);
    }

    /// Dripped mail is dated when it arrives and has a Message-ID that a
    /// later run can't repeat.
    #[test]
    fn dripped_mail_is_dated_now_with_an_id_unique_per_run() {
        let now = 1_795_000_000;
        let text = String::from_utf8(new_message(1_794_999_000, 3, now)).expect("utf-8");
        assert!(
            text.contains("\r\nDate: Wed, 18 Nov 2026 11:06:40 +0000\r\n"),
            "{text}"
        );
        assert!(text.contains("\r\nMessage-ID: <dev-mail-1794999000-3@example.com>\r\n"));
        assert_ne!(
            new_message(1, 1, now),
            new_message(2, 1, now),
            "same number, different run"
        );
    }

    /// The certificate files are saved whole and read back as a working
    /// set; a broken set is remade, not served (#404 review).
    #[test]
    fn broken_saved_certificates_are_made_again() {
        let dir = std::env::temp_dir().join(format!(
            "mailcrane-dev-mail-test-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let (first, new) = certificates(&dir).expect("first run");
        assert!(new);
        let (again, new) = certificates(&dir).expect("second run");
        assert!(!new, "a working set is kept");
        assert_eq!(again.ca_der, first.ca_der);
        assert!(!dir.join(format!("{LEAF_KEY_FILE}.tmp")).exists());

        // Half-written leaf, as an old version could leave it.
        let leaf = dir.join(LEAF_FILE);
        let bytes = std::fs::read(&leaf).expect("leaf");
        std::fs::write(&leaf, &bytes[..bytes.len() / 2]).expect("cut");
        let (fixed, new) = certificates(&dir).expect("repaired");
        assert!(new, "a broken set is remade");
        assert_ne!(fixed.ca_der, first.ca_der);
        fixed.check("10.0.2.2").expect("works");

        // One file missing.
        std::fs::remove_file(dir.join(CA_FILE)).expect("remove");
        let (_, new) = certificates(&dir).expect("remade");
        assert!(new);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(LEAF_KEY_FILE))
                .expect("key")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "key is owner-only: {mode:o}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
