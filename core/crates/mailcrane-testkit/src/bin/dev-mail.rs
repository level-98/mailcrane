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

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use mailcrane_testkit::{DevCertificates, FakeImap, FakeSmtp, TestTls};

/// The account the emulator signs in with.
const USER: &str = "test@example.org";
const PASSWORD: &str = "password";
/// The names the certificate is valid for: the host as the emulator sees
/// it, and the host itself.
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

/// Loads the certificates from `state`, or makes and saves new ones.
/// Returns whether they were new (the CA must then be installed again).
fn certificates(state: &Path) -> Result<(DevCertificates, bool), String> {
    let read = |name: &str| std::fs::read(state.join(name));
    if let (Ok(ca_der), Ok(leaf_der), Ok(leaf_key_der)) =
        (read(CA_FILE), read(LEAF_FILE), read(LEAF_KEY_FILE))
    {
        return Ok((
            DevCertificates {
                ca_der,
                leaf_der,
                leaf_key_der,
            },
            false,
        ));
    }
    let dev = DevCertificates::generate(&NAMES)?;
    std::fs::create_dir_all(state).map_err(|e| format!("{}: {e}", state.display()))?;
    for (name, bytes) in [
        (CA_FILE, &dev.ca_der),
        (LEAF_FILE, &dev.leaf_der),
        (LEAF_KEY_FILE, &dev.leaf_key_der),
    ] {
        let path = state.join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok((dev, true))
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

/// A small synthetic message for `--new-mail-every`.
fn new_message(n: u64) -> Vec<u8> {
    format!(
        "From: Dev Mail <sender@example.com>\r\n\
         To: {USER}\r\n\
         Subject: Test message {n}\r\n\
         Message-ID: <dev-mail-{n}@example.com>\r\n\
         Date: Thu, 01 Jan 2026 00:00:00 +0000\r\n\
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

    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", inbox)
        .folder("Sent", Vec::new())
        .folder("Drafts", Vec::new())
        .folder("Archive", Vec::new())
        .folder("Junk", Vec::new())
        .folder("Trash", Vec::new())
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
    println!("  On the emulator, add a manual account:");
    println!("    email / user   {USER}");
    println!("    password       {PASSWORD}");
    println!(
        "    IMAP           10.0.2.2  port {} (SSL/TLS) or {} (STARTTLS)",
        IMAP_PORTS.0, IMAP_PORTS.1
    );
    println!(
        "    SMTP           10.0.2.2  port {} (SSL/TLS) or {} (STARTTLS)",
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
                imap.mailbox().deliver("INBOX", new_message(dripped)).await;
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
