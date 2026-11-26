//! `cargo xtask dev-mail`: the fake IMAP and SMTP server for the Android
//! emulator (#403, docs/dev-mail.md). The server itself is the testkit's
//! `dev-mail` binary; this finds the paths, copies the CA certificate to any
//! running emulator, and starts it.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::{cargo, run, say, Ctx, Error};

/// Where the copied CA lands on the emulator, for Settings to install.
const DEVICE_CA: &str = "/sdcard/Download/mailcrane-dev-ca.crt";

pub fn run_server(ctx: &Ctx, args: impl Iterator<Item = String>) -> Result<ExitCode, Error> {
    let mut extra = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--new-mail-every" => {
                let value = args
                    .next()
                    .ok_or("--new-mail-every needs a number of seconds")?;
                extra.push(arg);
                extra.push(value);
            }
            other => return Err(format!("unknown argument `{other}` for dev-mail")),
        }
    }

    let state = ctx.root.join(".dev-mail");
    let fixtures = ctx.root.join("tests").join("fixtures");
    let state_arg = state.display().to_string();
    let bin = [
        "run",
        "--quiet",
        "-p",
        "mailcrane-testkit",
        "--bin",
        "dev-mail",
        "--",
        "--state",
        state_arg.as_str(),
    ];

    // Make the certificates first, so the CA can be copied before serving.
    let mut certs = bin.to_vec();
    certs.push("--certs-only");
    if !run(&ctx.core, &cargo(), &certs)? {
        return Err("could not make the dev mail certificates".into());
    }
    push_ca_to_emulators(&state.join("mailcrane-dev-ca.crt"));

    let fixtures_arg = fixtures.display().to_string();
    let mut serve: Vec<&str> = bin.to_vec();
    serve.push("--fixtures");
    serve.push(fixtures_arg.as_str());
    serve.extend(extra.iter().map(String::as_str));
    Ok(if run(&ctx.core, &cargo(), &serve)? {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `adb` from the SDK when it's there, otherwise from PATH.
fn adb() -> PathBuf {
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(sdk) = std::env::var(var) {
            let tools = Path::new(&sdk).join("platform-tools");
            for name in ["adb.exe", "adb"] {
                let candidate = tools.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("adb")
}

/// Copies the CA to every running emulator. Never to a physical phone: a
/// CA on a real device would let whoever holds the server key read its
/// traffic, so only emulators (`emulator-NNNN`) get it.
fn push_ca_to_emulators(ca: &Path) {
    let adb = adb();
    let Ok(out) = Command::new(&adb).arg("devices").output() else {
        say("dev-mail: adb not found; copy the CA to the emulator yourself (docs/dev-mail.md).");
        return;
    };
    let listing = String::from_utf8_lossy(&out.stdout);
    let emulators: Vec<&str> = listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(serial), Some("device")) if serial.starts_with("emulator-") => Some(serial),
                _ => None,
            }
        })
        .collect();
    if emulators.is_empty() {
        say("dev-mail: no running emulator; start one and run this again to copy the CA.");
        return;
    }
    for serial in emulators {
        let pushed = Command::new(&adb)
            .args(["-s", serial, "push"])
            .arg(ca)
            .arg(DEVICE_CA)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if pushed {
            say(format_args!(
                "dev-mail: copied the CA to {serial} as Download/mailcrane-dev-ca.crt. \
                 Install it once: Settings > Security > More security settings > \
                 Encryption & credentials > Install a certificate > CA certificate."
            ));
        } else {
            say(format_args!("dev-mail: could not copy the CA to {serial}."));
        }
    }
}
