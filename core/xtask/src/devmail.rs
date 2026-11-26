//! `cargo xtask dev-mail`: the fake IMAP and SMTP server for the Android
//! emulator (#403, docs/dev-mail.md). The server itself is the testkit's
//! `dev-mail` binary; this finds the paths, copies the CA certificate to any
//! running emulator, and starts it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::{cargo, run, say, Ctx, Error};

/// Where the copied CA lands on the emulator, for Settings to install.
const DEVICE_CA: &str = "/sdcard/Download/mailcrane-dev-ca.crt";

/// The fixed ports of the `dev-mail` binary (IMAP TLS and STARTTLS, SMTP
/// TLS and submission). They must match `IMAP_PORTS` and `SMTP_PORTS` in
/// `mailcrane-testkit/src/bin/dev-mail.rs`; a test checks they do.
const PORTS: [u16; 4] = [1993, 1143, 1465, 1587];

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
    let ca = state.join("mailcrane-dev-ca.crt");
    // Paths go through as `OsStr`, so a folder name that isn't valid
    // Unicode still works.
    let bin: Vec<&OsStr> = [
        "run",
        "--quiet",
        "-p",
        "mailcrane-testkit",
        "--bin",
        "dev-mail",
        "--",
        "--state",
    ]
    .into_iter()
    .map(OsStr::new)
    .chain([state.as_os_str()])
    .collect();

    // Make the certificates first, so the CA can be copied before serving.
    let before = std::fs::read(&ca).ok();
    let mut certs = bin.clone();
    certs.push(OsStr::new("--certs-only"));
    if !run(&ctx.core, &cargo(), &certs)? {
        return Err("could not make the dev mail certificates".into());
    }
    // The server's own banner can't tell that this step just made the CA,
    // so say it here, before the emulator steps.
    if std::fs::read(&ca).ok() != before {
        say("dev-mail: this is a NEW CA. Install it on each emulator again (docs/dev-mail.md).");
    }
    prepare_emulators(&ca);

    let mut serve = bin;
    serve.push(OsStr::new("--fixtures"));
    serve.push(fixtures.as_os_str());
    serve.extend(extra.iter().map(OsStr::new));
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

/// Gets every running emulator ready: copies the CA to it and forwards the
/// server's ports with `adb reverse`, so the app reaches the server at
/// `127.0.0.1`. On API 37 the app's connections to `10.0.2.2`, the
/// emulator's alias for the host, never arrive (#404 QA), while `adb
/// reverse` works. Never a physical phone: a CA on a real device would let
/// whoever holds the server key read its traffic, so only emulators
/// (`emulator-NNNN`) are touched.
fn prepare_emulators(ca: &Path) {
    let adb = adb();
    let Ok(out) = Command::new(&adb).arg("devices").output() else {
        say("dev-mail: adb not found. Copy the CA to the emulator yourself, and run:");
        say_reverse_commands(UNKNOWN_SERIAL);
        return;
    };
    if !out.status.success() {
        say(format_args!(
            "dev-mail: `adb devices` failed ({}). Copy the CA to the emulator yourself, \
             and run:",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
        say_reverse_commands(UNKNOWN_SERIAL);
        return;
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let (emulators, not_ready) = emulators(&listing);
    for (serial, state) in &not_ready {
        say(format_args!(
            "dev-mail: {serial} is {state}, so it wasn't set up. Wait for it to boot (or \
             accept the USB debugging prompt), then run this again."
        ));
    }
    if emulators.is_empty() {
        if not_ready.is_empty() {
            say("dev-mail: no running emulator; start one and run this again to set it up.");
        }
        return;
    }
    for serial in emulators {
        reverse_ports(&adb, serial);
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
                 Install it once: {SETTINGS_PATH} (docs/dev-mail.md)."
            ));
        } else {
            say(format_args!("dev-mail: could not copy the CA to {serial}."));
        }
    }
}

/// `adb -s <serial> reverse tcp:P tcp:P` for each server port: on the
/// emulator, `127.0.0.1:P` then reaches `127.0.0.1:P` on this machine.
fn reverse_ports(adb: &Path, serial: &str) {
    let failed: Vec<u16> = PORTS
        .iter()
        .copied()
        .filter(|port| {
            let spec = format!("tcp:{port}");
            !Command::new(adb)
                .args(["-s", serial, "reverse", &spec, &spec])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
        .collect();
    if failed.is_empty() {
        say(format_args!(
            "dev-mail: forwarded ports {} on {serial} to this machine (adb reverse); \
             use 127.0.0.1 as the server in the app.",
            port_list()
        ));
    } else {
        say(format_args!(
            "dev-mail: adb reverse failed on {serial} for port(s) {failed:?}; run:"
        ));
        say_reverse_commands(serial);
    }
}

/// Stands in for the serial when adb couldn't list the emulators.
const UNKNOWN_SERIAL: &str = "emulator-NNNN";

/// Prints the `adb reverse` commands for setting up emulator `serial` by
/// hand.
fn say_reverse_commands(serial: &str) {
    for line in reverse_commands(serial) {
        say(format_args!("    {line}"));
    }
}

fn reverse_commands(serial: &str) -> Vec<String> {
    PORTS
        .iter()
        .map(|port| format!("adb -s {serial} reverse tcp:{port} tcp:{port}"))
        .collect()
}

fn port_list() -> String {
    PORTS.map(|p| p.to_string()).join(", ")
}

/// Where to install a CA certificate in the emulator's Settings, as
/// docs/dev-mail.md gives it.
const SETTINGS_PATH: &str = "Settings > Security & privacy > More security settings > \
     Encryption & credentials > Install a certificate > CA certificate \
     (or search Settings for \"CA certificate\")";

/// From `adb devices` output: the emulators that are ready, and the
/// emulators that aren't (`offline`, `unauthorized`), with their state.
/// Phones are left out of both, as they never get the CA.
fn emulators(listing: &str) -> (Vec<&str>, Vec<(&str, &str)>) {
    let mut ready = Vec::new();
    let mut not_ready = Vec::new();
    for line in listing.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(serial), Some(state)) = (parts.next(), parts.next()) {
            if !serial.starts_with("emulator-") {
                continue;
            }
            if state == "device" {
                ready.push(serial);
            } else {
                not_ready.push((serial, state));
            }
        }
    }
    (ready, not_ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ready_emulators_get_the_ca_and_others_are_named() {
        let listing = "List of devices attached\n\
                       emulator-5554\tdevice\n\
                       emulator-5556\toffline\n\
                       emulator-5558\tunauthorized\n\
                       R5CT0000000\tdevice\n\
                       \n";
        let (ready, not_ready) = emulators(listing);
        assert_eq!(ready, ["emulator-5554"]);
        assert_eq!(
            not_ready,
            [
                ("emulator-5556", "offline"),
                ("emulator-5558", "unauthorized")
            ]
        );
        assert_eq!(emulators("List of devices attached\n\n"), (vec![], vec![]));
    }

    #[test]
    fn the_fallback_commands_name_the_emulator_that_failed() {
        assert_eq!(
            reverse_commands("emulator-5556"),
            [
                "adb -s emulator-5556 reverse tcp:1993 tcp:1993",
                "adb -s emulator-5556 reverse tcp:1143 tcp:1143",
                "adb -s emulator-5556 reverse tcp:1465 tcp:1465",
                "adb -s emulator-5556 reverse tcp:1587 tcp:1587",
            ]
        );
    }

    #[test]
    fn the_forwarded_ports_are_the_servers_ports() {
        let server = include_str!("../../crates/mailcrane-testkit/src/bin/dev-mail.rs");
        let imap = format!("IMAP_PORTS: (u16, u16) = ({}, {});", PORTS[0], PORTS[1]);
        let smtp = format!("SMTP_PORTS: (u16, u16) = ({}, {});", PORTS[2], PORTS[3]);
        assert!(server.contains(&imap), "{imap}");
        assert!(server.contains(&smtp), "{smtp}");
    }

    #[test]
    fn the_settings_path_matches_the_docs() {
        let docs = include_str!("../../../docs/dev-mail.md");
        let path = SETTINGS_PATH
            .split(" (")
            .next()
            .unwrap_or_default()
            .replace(" > ", " → ");
        assert!(docs.contains(&path), "{path}");
    }
}
