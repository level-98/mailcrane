//! `cargo xtask`: the one place every check lives. CI runs `cargo xtask ci --strict`.
//!
//! Std only, no dependencies, and no panics: every failure is an `Err` or a
//! failed step, never an unwind.
#![forbid(unsafe_code)]

mod apk;
mod devmail;
mod ispdb;
mod licences;
mod prepush;
mod release;
mod strings;

use std::env;
use std::ffi::OsStr;
use std::fmt::Display;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Error = String;

/// How one step ended.
enum Outcome {
    Passed,
    Failed(String),
    Skipped(String),
}

struct Step {
    name: &'static str,
    run: fn(&Ctx) -> Result<Outcome, Error>,
}

/// Steps in the order `ci` runs them. `bindings` is not part of `ci`.
const CI_STEPS: [Step; 6] = [
    Step {
        name: "design",
        run: design,
    },
    Step {
        name: "fmt",
        run: fmt,
    },
    Step {
        name: "lint",
        run: lint,
    },
    Step {
        name: "test",
        run: test,
    },
    Step {
        name: "deny",
        run: deny,
    },
    Step {
        name: "android",
        run: android,
    },
];

const BINDINGS: Step = Step {
    name: "bindings",
    run: bindings,
};

const USAGE: &str = "\
Usage: cargo xtask <command> [--strict] [--only <step>]... [--skip <step>]...

Commands:
  ci        every step below except bindings, then a summary table
  design    generated theme check, its unit tests, and the release scripts' tests
  fmt       cargo fmt --all --check
  lint      release guard (#289), translated strings keep their placeholders (#69),
            cargo clippy --workspace --all-targets -- -D warnings,
            and clippy on mailcrane-ffi with --features debug-fixtures
  test      cargo test --workspace, then the fixture loader tests (#289)
  deny      cargo deny check, then a check that android/app/licences/rust-licences.txt
            is current (needs cargo-about 0.9.3)
  android   ktlintCheck, lintDebug, testDebugUnitTest, assembleDebug and checkFreeDependencies
            through Gradle; then assembleRelease and assembleDebug, and a check that
            each APK packages its own core (the debug one has the fixture loader)
  bindings  generate Kotlin and Swift bindings into core/target/bindings/
  fuzz <target> [--seconds N]
            run a fuzz target (default 60 s); needs nightly and cargo-fuzz.
            Not part of ci. Targets: mime_parse, imap_response, html_sanitize, address_list, sieve_response
  licences  regenerate android/app/licences/rust-licences.txt, the Rust crates' licences
            for the in-app list (#366), with cargo-about. Not part of ci.
  release-check <tag>
            check that a release tag (vMAJOR.MINOR.PATCH[-pre]) matches versionName and
            versionCode in android/app/build.gradle.kts (docs/releasing.md). Not part of ci.
  dev-mail [--new-mail-every <seconds>]
            run a fake IMAP and SMTP server for the Android emulator, seeded with tests/fixtures;
            copy its CA certificate to running emulators and forward its ports with adb reverse,
            so the app reaches it at 127.0.0.1 (docs/dev-mail.md). Not part of ci.
  ispdb <checkout>
            import the ISPDB snapshot from a thunderbird/autoconfig checkout into
            mailcrane-core/data/ispdb/ and regenerate its index (ADR 0003). Not part of ci.

Flags (for ci):
  --strict       a skipped step counts as a failure
  --only <step>  run only this step (repeatable, or a comma-separated list)
  --skip <step>  leave this step out (repeatable, or a comma-separated list)
";

struct Ctx {
    /// The repository root.
    root: PathBuf,
    /// The Cargo workspace, `<root>/core`.
    core: PathBuf,
}

#[derive(Default)]
struct Options {
    strict: bool,
    only: Vec<String>,
    skip: Vec<String>,
}

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(message) => {
            say(format_args!("error: {message}"));
            ExitCode::from(2)
        }
    }
}

/// Prints a line to stdout, ignoring a closed pipe instead of panicking.
fn say(line: impl Display) {
    let _ = writeln!(io::stdout(), "{line}");
}

fn real_main() -> Result<ExitCode, Error> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        say(USAGE);
        return Ok(ExitCode::from(2));
    };
    if command == "help" || command == "--help" || command == "-h" {
        say(USAGE);
        return Ok(ExitCode::SUCCESS);
    }
    if command == "fuzz" {
        return fuzz(&context()?, args);
    }
    if command == "licences" {
        licences::write(&context()?)?;
        return Ok(ExitCode::SUCCESS);
    }
    if command == "release-check" {
        release::run(&context()?, args)?;
        return Ok(ExitCode::SUCCESS);
    }
    if command == "dev-mail" {
        return devmail::run_server(&context()?, args);
    }
    if command == "ispdb" {
        ispdb::import(&context()?, args)?;
        return Ok(ExitCode::SUCCESS);
    }
    let options = parse_options(args)?;
    let ctx = context()?;

    let steps: Vec<&Step> = if command == "ci" {
        for name in options.only.iter().chain(&options.skip) {
            if !CI_STEPS.iter().any(|s| s.name == name) {
                return Err(format!("unknown step `{name}` (see `cargo xtask help`)"));
            }
        }
        CI_STEPS
            .iter()
            .filter(|s| options.only.is_empty() || options.only.iter().any(|o| o == s.name))
            .filter(|s| !options.skip.iter().any(|o| o == s.name))
            .collect()
    } else if command == "bindings" {
        vec![&BINDINGS]
    } else if let Some(step) = CI_STEPS.iter().find(|s| s.name == command) {
        vec![step]
    } else {
        return Err(format!(
            "unknown command `{command}` (see `cargo xtask help`)"
        ));
    };

    let mut results = Vec::new();
    for step in steps {
        say(format_args!("\n==> {}", step.name));
        let outcome = match (step.run)(&ctx) {
            Ok(outcome) => outcome,
            Err(message) => Outcome::Failed(message),
        };
        results.push((step.name, outcome));
    }
    Ok(summarise(&results, options.strict))
}

fn parse_options(args: impl Iterator<Item = String>) -> Result<Options, Error> {
    let mut options = Options::default();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--strict" => options.strict = true,
            "--only" | "--skip" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} needs a step name"))?;
                // `--only fmt,lint,test` is the same as three `--only`s.
                let names = value
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(String::from);
                if arg == "--only" {
                    options.only.extend(names);
                } else {
                    options.skip.extend(names);
                }
            }
            other => {
                return Err(format!(
                    "unknown argument `{other}` (see `cargo xtask help`)"
                ))
            }
        }
    }
    Ok(options)
}

/// Finds the repo root from this crate's location, so it works from any directory.
fn context() -> Result<Ctx, Error> {
    let xtask_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let core = xtask_dir
        .parent()
        .ok_or("cannot find the core/ directory")?
        .to_path_buf();
    let root = core
        .parent()
        .ok_or("cannot find the repository root")?
        .to_path_buf();
    Ok(Ctx { root, core })
}

/// Prints the table and returns the exit code: 0 only if nothing failed
/// (and, with `--strict`, nothing was skipped).
fn summarise(results: &[(&'static str, Outcome)], strict: bool) -> ExitCode {
    say("\nSummary");
    let mut bad = false;
    for (name, outcome) in results {
        let (label, detail) = match outcome {
            Outcome::Passed => ("passed", String::new()),
            Outcome::Failed(why) => {
                bad = true;
                ("FAILED", format!(": {why}"))
            }
            Outcome::Skipped(why) => {
                if strict {
                    bad = true;
                    ("FAILED", format!(": skipped in --strict mode ({why})"))
                } else {
                    ("skipped", format!(": {why}"))
                }
            }
        };
        say(format_args!("  {name:<10} {label}{detail}"));
    }
    if bad {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ---------------------------------------------------------------- helpers

/// Runs a command in `dir`, inheriting stdio. Failure to start is an `Err`.
fn run<S: AsRef<OsStr>>(dir: &Path, program: &str, args: &[S]) -> Result<bool, Error> {
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("could not start `{program}`: {e}"))?;
    Ok(status.success())
}

fn outcome(ok: bool, what: &str) -> Outcome {
    if ok {
        Outcome::Passed
    } else {
        Outcome::Failed(format!("`{what}` failed"))
    }
}

/// Whether `program --version` runs.
fn has_tool(program: &str, version_arg: &str) -> bool {
    Command::new(program)
        .arg(version_arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

// ------------------------------------------------------------------ steps

fn design(ctx: &Ctx) -> Result<Outcome, Error> {
    // On Windows `python3` is often missing; the `py -3` launcher is the
    // reliable way to start Python 3 there.
    let (python, prefix): (&str, &[&str]) = if cfg!(windows) {
        ("py", &["-3"])
    } else {
        ("python3", &[])
    };
    if !has_tool(python, "--version") {
        return Ok(Outcome::Skipped(format!("{python} is not installed")));
    }
    let check: Vec<&str> = [prefix, &["design/gen_theme.py", "--check"]].concat();
    if !run(&ctx.root, python, &check)? {
        return Ok(Outcome::Failed(
            "`design/gen_theme.py --check` failed".into(),
        ));
    }
    let tests: Vec<&str> = [prefix, &["-m", "unittest", "design/test_gen_theme.py"]].concat();
    if !run(&ctx.root, python, &tests)? {
        return Ok(Outcome::Failed(
            "`python -m unittest design/test_gen_theme.py` failed".into(),
        ));
    }
    // The release tooling's tests run here too: this is CI's Python step.
    let scripts: Vec<&str> = [
        prefix,
        &[
            "-m",
            "unittest",
            "scripts/test_release_notes.py",
            "scripts/test_prune_test_builds.py",
        ],
    ]
    .concat();
    let ok = run(&ctx.root, python, &scripts)?;
    Ok(outcome(
        ok,
        "python -m unittest scripts/test_release_notes.py scripts/test_prune_test_builds.py",
    ))
}

fn fmt(ctx: &Ctx) -> Result<Outcome, Error> {
    let ok = run(&ctx.core, &cargo(), &["fmt", "--all", "--check"])?;
    Ok(outcome(ok, "cargo fmt --all --check"))
}

fn lint(ctx: &Ctx) -> Result<Outcome, Error> {
    if let Err(problem) = release_guard(ctx) {
        return Ok(Outcome::Failed(format!("release guard: {problem}")));
    }
    // The pre-push ktlint is the Gradle plugin's version (#564).
    let read = |path: &str| std::fs::read_to_string(ctx.root.join(path)).unwrap_or_default();
    match prepush::check_pins(
        &read("android/app/build.gradle.kts"),
        &read("scripts/fleet/prepush.sh"),
        &read("scripts/fleet/prepush.ps1"),
    ) {
        Ok(version) => say(format_args!(
            "ktlint: Gradle and the pre-push scripts pin {version}"
        )),
        Err(problem) => return Ok(Outcome::Failed(format!("ktlint pins: {problem}"))),
    }
    // Translations from Weblate keep the English placeholders (#69).
    match strings::check_dir(&ctx.root.join("android/app/src/main/res")) {
        Ok(n) => say(format_args!(
            "strings: {n} translation(s) keep their placeholders"
        )),
        Err(problem) => return Ok(Outcome::Failed(format!("strings: {problem}"))),
    }
    let args = [
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ];
    if !run(&ctx.core, &cargo(), &args)? {
        return Ok(Outcome::Failed("`cargo clippy` failed".into()));
    }
    // The debug-only fixture loader (#289) is behind a feature the workspace
    // run leaves off; lint it too.
    let fixtures = [
        "clippy",
        "-p",
        "mailcrane-ffi",
        "--all-targets",
        "--features",
        DEBUG_FIXTURES,
        "--",
        "-D",
        "warnings",
    ];
    Ok(outcome(
        run(&ctx.core, &cargo(), &fixtures)?,
        "cargo clippy -p mailcrane-ffi --features debug-fixtures",
    ))
}

fn test(ctx: &Ctx) -> Result<Outcome, Error> {
    if !run(&ctx.core, &cargo(), &["test", "--workspace"])? {
        return Ok(Outcome::Failed("`cargo test --workspace` failed".into()));
    }
    // The fixture loader's tests (#289), in the core and through the FFI.
    let core = [
        "test",
        "-p",
        "mailcrane-core",
        "--features",
        "fixtures",
        "--lib",
        "fixtures",
    ];
    if !run(&ctx.core, &cargo(), &core)? {
        return Ok(Outcome::Failed(
            "`cargo test -p mailcrane-core --features fixtures` failed".into(),
        ));
    }
    let ffi = [
        "test",
        "-p",
        "mailcrane-ffi",
        "--features",
        DEBUG_FIXTURES,
        "--lib",
        "fixtures",
    ];
    Ok(outcome(
        run(&ctx.core, &cargo(), &ffi)?,
        "cargo test -p mailcrane-ffi --features debug-fixtures",
    ))
}

// ---------------------------------------------------------- release guard

/// The feature behind "Load test mail" (#289). Debug Android builds only.
const DEBUG_FIXTURES: &str = "debug-fixtures";

/// The one line in `android/app/build.gradle.kts` that may turn on
/// [`DEBUG_FIXTURES`]: only when the build is not a release build.
const GRADLE_FEATURE_LINE: &str = r#"val rustFeatures = if (rustRelease) emptyList() else listOf("--features", "debug-fixtures")"#;

/// The Gradle check that refuses to package a release APK with a core built
/// for debug (and so with the loader in it).
const GRADLE_GUARD: &str = "checkNoDebugCoreInRelease";

/// Checks that no release build can contain the fixture loader (#289):
/// it is never a default feature, no crate turns it on, the Android build
/// enables it only for debug and refuses to mix a debug core into a release
/// APK, and only the debug source set calls it.
fn release_guard(ctx: &Ctx) -> Result<(), String> {
    let read = |p: &Path| {
        std::fs::read_to_string(p).map_err(|e| format!("can't read {}: {e}", p.display()))
    };
    let crates = ctx.core.join("crates");
    let mut manifests = Vec::new();
    let entries = std::fs::read_dir(&crates).map_err(|e| format!("can't list crates: {e}"))?;
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        if manifest.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            manifests.push((name, read(&manifest)?));
        }
    }
    let app = ctx.root.join("android/app");
    let gradle = read(&app.join("build.gradle.kts"))?;
    let mut release_sources = Vec::new();
    for set in ["main", "release"] {
        collect_kotlin(&app.join("src").join(set), &mut release_sources)?;
    }
    check_release_guard(&manifests, &gradle, &release_sources)
}

/// Every `.kt` file under `dir`, as (path, contents). A missing `dir` is
/// fine (there may be no `release` source set).
fn collect_kotlin(dir: &Path, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kotlin(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "kt") {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("can't read {}: {e}", path.display()))?;
            out.push((path.display().to_string(), text));
        }
    }
    Ok(())
}

/// The checks of [`release_guard`] on file contents, so they can be tested.
fn check_release_guard(
    manifests: &[(String, String)],
    gradle: &str,
    release_sources: &[(String, String)],
) -> Result<(), String> {
    for (name, text) in manifests {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if line.starts_with("default") && line.contains('=') && line.contains("fixtures") {
                return Err(format!("{name}: a `fixtures` feature is in `default`"));
            }
            let defines_it = name == "mailcrane-ffi"
                && line == r#"debug-fixtures = ["mailcrane-core/fixtures"]"#;
            let enables = line.contains(DEBUG_FIXTURES) || line.contains("mailcrane-core/fixtures");
            if enables && !defines_it {
                return Err(format!("{name}: `{line}` turns on the fixture loader"));
            }
            if line.starts_with("mailcrane-core") && line.contains("fixtures") {
                return Err(format!("{name}: depends on mailcrane-core with `fixtures`"));
            }
        }
    }
    let mentions = gradle.matches(DEBUG_FIXTURES).count();
    if !gradle.lines().any(|l| l.trim() == GRADLE_FEATURE_LINE) || mentions != 1 {
        return Err(format!(
            "android/app/build.gradle.kts must turn on `{DEBUG_FIXTURES}` only through              `{GRADLE_FEATURE_LINE}`"
        ));
    }
    if !gradle.contains(GRADLE_GUARD) {
        return Err(format!(
            "android/app/build.gradle.kts lost its `{GRADLE_GUARD}` check"
        ));
    }
    for (path, text) in release_sources {
        if text.contains("loadTestMail") || text.contains("FixtureFile") {
            return Err(format!(
                "{path} uses the fixture loader; only the debug source set may"
            ));
        }
    }
    Ok(())
}

fn deny(ctx: &Ctx) -> Result<Outcome, Error> {
    if !ctx.core.join("deny.toml").is_file() {
        return Ok(Outcome::Skipped("core/deny.toml does not exist yet".into()));
    }
    if !has_tool("cargo-deny", "--version") {
        return Ok(Outcome::Skipped("cargo-deny is not installed".into()));
    }
    if !run(&ctx.core, &cargo(), &["deny", "check"])? {
        return Ok(Outcome::Failed("`cargo deny check` failed".into()));
    }
    // The open-source licences list (#366) uses the same allowed licences.
    licences::check(ctx)
}

fn android(ctx: &Ctx) -> Result<Outcome, Error> {
    let dir = ctx.root.join("android");
    let gradlew = dir.join("gradlew");
    if !gradlew.is_file() {
        return Ok(Outcome::Skipped(
            "android/gradlew does not exist yet".into(),
        ));
    }
    let has_sdk = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .any(|v| env::var_os(v).is_some_and(|s| !s.is_empty()));
    if !has_sdk {
        return Ok(Outcome::Skipped(
            "neither ANDROID_HOME nor ANDROID_SDK_ROOT is set".into(),
        ));
    }
    // Windows can't run the `gradlew` shell script; it has `gradlew.bat`.
    let wrapper = if cfg!(windows) {
        dir.join("gradlew.bat")
    } else {
        gradlew
    };
    // assembleDebug cross-compiles the core for both ABIs (the buildRust task)
    // and proves the app really builds, not only that it lints and tests.
    // checkFreeDependencies (M6-04): the release build is free software
    // only, as F-Droid requires.
    let args = [
        "ktlintCheck",
        "lintDebug",
        "testDebugUnitTest",
        "assembleDebug",
        "checkFreeDependencies",
    ];
    let wrapper = wrapper.display().to_string();
    if !run(&dir, &wrapper, &args)? {
        return Ok(Outcome::Failed(
            "`./gradlew ktlintCheck lintDebug testDebugUnitTest assembleDebug checkFreeDependencies` failed"
                .into(),
        ));
    }
    // Each APK must package its own core: a debug APK built after a release
    // build once shipped the release core and crashed at launch (#71 review).
    // Release, then debug again, in that order, so the debug APK CI uploads
    // is the one checked.
    for task in ["assembleRelease", "assembleDebug"] {
        if !run(&dir, &wrapper, &[task])? {
            return Ok(Outcome::Failed(format!("`./gradlew {task}` failed")));
        }
    }
    let outputs = dir.join("app/build/outputs/apk");
    for (apk, debug) in [
        ("release/app-release-unsigned.apk", false),
        ("debug/app-debug.apk", true),
    ] {
        if let Err(e) = check_apk_core(&outputs.join(apk), debug) {
            return Ok(Outcome::Failed(e));
        }
    }
    Ok(Outcome::Passed)
}

/// Checks that the APK at `path` packages the debug core (with the fixture
/// loader, #289) if `debug`, and the release core (without it) otherwise.
fn check_apk_core(path: &Path, debug: bool) -> Result<(), Error> {
    let bytes = std::fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let core = apk::stored_entry(&bytes, apk::CORE_ENTRY)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let has_loader = apk::contains(core, apk::DEBUG_ONLY_SYMBOL);
    match (debug, has_loader) {
        (true, false) => Err(format!(
            "{} packages a release core (no fixture loader): it would crash at launch",
            path.display()
        )),
        (false, true) => Err(format!(
            "{} packages a debug core with the fixture loader (#289)",
            path.display()
        )),
        _ => Ok(()),
    }
}

/// Cargo's output directory. Steps run Cargo from `core/`, so a relative
/// `CARGO_TARGET_DIR` is relative to `core/`, as Cargo itself treats it.
fn target_dir(ctx: &Ctx) -> PathBuf {
    match env::var_os("CARGO_TARGET_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => ctx.core.join(dir),
        None => ctx.core.join("target"),
    }
}

// ------------------------------------------------------------------- fuzz

/// Fuzz targets in `core/fuzz/fuzz_targets/`, each with the fixture folder
/// (under `tests/fixtures/`) and file extension its seed corpus comes from.
const FUZZ_TARGETS: [(&str, &str, &str); 5] = [
    ("mime_parse", "mime", "eml"),
    ("imap_response", "imap", "imap"),
    ("html_sanitize", "html", "html"),
    ("address_list", "address", "txt"),
    ("sieve_response", "sieve", "sieve"),
];

/// `cargo xtask fuzz <target> [--seconds N]`. Seeds `core/fuzz/corpus/<target>/`
/// from the fixtures (the corpus is never committed), then runs
/// `cargo +nightly fuzz run` for N seconds. Exit code 0 only if no crash.
fn fuzz(ctx: &Ctx, args: impl Iterator<Item = String>) -> Result<ExitCode, Error> {
    let mut target = None;
    let mut seconds: u64 = 60;
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "--seconds" {
            let value = args.next().ok_or("--seconds needs a number")?;
            seconds = value
                .parse()
                .map_err(|_| format!("--seconds needs a whole number, not `{value}`"))?;
        } else if target.is_none() && !arg.starts_with('-') {
            target = Some(arg);
        } else {
            return Err(format!("unknown argument `{arg}` (see `cargo xtask help`)"));
        }
    }
    let names: Vec<&str> = FUZZ_TARGETS.iter().map(|t| t.0).collect();
    let target = target.ok_or_else(|| format!("name a target: {}", names.join(", ")))?;
    let (_, fixtures, ext) = FUZZ_TARGETS
        .iter()
        .find(|t| t.0 == target)
        .ok_or_else(|| format!("unknown target `{target}` (targets: {})", names.join(", ")))?;

    // Through the rustup proxy, not $CARGO: that points at the stable cargo
    // running xtask, which ignores `+nightly`.
    if !has_tool_args("cargo", &["+nightly", "--version"]) {
        return Err(
            "fuzzing needs the nightly toolchain: `rustup toolchain install nightly`".into(),
        );
    }
    if !has_tool_args("cargo", &["+nightly", "fuzz", "--version"]) {
        return Err("fuzzing needs cargo-fuzz: `cargo install cargo-fuzz`".into());
    }

    let fuzz_dir = ctx.core.join("fuzz");
    let corpus = fuzz_dir.join("corpus").join(target.as_str());
    let seeded = seed_corpus(
        &ctx.root.join("tests").join("fixtures").join(fixtures),
        ext,
        &corpus,
    )?;
    say(format_args!(
        "seeded {seeded} file(s) into core/fuzz/corpus/{target}/"
    ));

    // cargo-fuzz defaults `--target` to the platform it was itself built for.
    // Prebuilt Linux copies (as `taiki-e/install-action` installs) are musl
    // builds, and AddressSanitizer cannot link against static musl, so always
    // build for the machine's own platform. Linux only: on Windows the nightly
    // host can be `-gnu` while libFuzzer needs the MSVC target cargo-fuzz uses.
    let mut run_args = vec!["+nightly".to_string(), "fuzz".into(), "run".into()];
    if cfg!(target_os = "linux") {
        run_args.extend(["--target".into(), nightly_host()?]);
    }
    run_args.extend([
        target.clone(),
        "--".into(),
        format!("-max_total_time={seconds}"),
        // A single input taking this long is a hang worth reporting.
        "-timeout=10".into(),
    ]);
    // `cargo xtask` runs under stable, and the stable cargo and rustup set
    // variables that would make the nightly build use the stable compiler.
    let ok = Command::new("cargo")
        .args(&run_args)
        .current_dir(&fuzz_dir)
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC")
        .env_remove("RUSTDOC")
        .env_remove("CARGO")
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .map_err(|e| format!("could not start `cargo +nightly fuzz`: {e}"))?
        .success();
    if ok {
        say(format_args!("\n{target}: no crash in {seconds} s"));
        Ok(ExitCode::SUCCESS)
    } else {
        if cfg!(windows) {
            // libFuzzer on Windows only links with AddressSanitizer, whose
            // runtime DLL ships with MSVC but is not on PATH by default.
            say(
                "\nOn Windows, exit code 0xc0000135 means clang_rt.asan_dynamic-x86_64.dll \
                 is not on PATH. Add the MSVC folder that holds it, for example \
                 <Visual Studio>\\VC\\Tools\\MSVC\\<version>\\bin\\Hostx64\\x64.",
            );
        }
        let crashes = std::fs::read_dir(fuzz_dir.join("artifacts").join(target.as_str()))
            .map(|entries| entries.count())
            .unwrap_or(0);
        if crashes > 0 {
            say(format_args!(
                "\n{target}: crash. {crashes} input(s) in core/fuzz/artifacts/{target}/; \
                 see core/fuzz/README.md for turning one into a regression test."
            ));
        } else {
            say(format_args!(
                "\n{target}: `cargo fuzz run` failed without saving a crashing input; \
                 see the build output above."
            ));
        }
        Ok(ExitCode::FAILURE)
    }
}

/// Copies `<fixtures>/*.<ext>` into `corpus`, creating it. Returns how many.
fn seed_corpus(fixtures: &Path, ext: &str, corpus: &Path) -> Result<usize, Error> {
    std::fs::create_dir_all(corpus)
        .map_err(|e| format!("cannot create {}: {e}", corpus.display()))?;
    let entries = std::fs::read_dir(fixtures)
        .map_err(|e| format!("cannot read {}: {e}", fixtures.display()))?;
    let mut count = 0;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", fixtures.display()))?
            .path();
        if path.extension() != Some(OsStr::new(ext)) {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        std::fs::copy(&path, corpus.join(name))
            .map_err(|e| format!("cannot copy {}: {e}", path.display()))?;
        count += 1;
    }
    Ok(count)
}

/// The host platform of the nightly compiler, e.g. `x86_64-unknown-linux-gnu`,
/// from the `host:` line of `rustc +nightly -vV`.
fn nightly_host() -> Result<String, Error> {
    let out = Command::new("rustc")
        .args(["+nightly", "-vV"])
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC")
        .output()
        .map_err(|e| format!("could not start `rustc +nightly -vV`: {e}"))?;
    if !out.status.success() {
        return Err("`rustc +nightly -vV` failed".into());
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_string())
        .ok_or_else(|| "`rustc +nightly -vV` printed no `host:` line".into())
}

/// Whether `program args...` runs and succeeds.
fn has_tool_args(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn bindings(ctx: &Ctx) -> Result<Outcome, Error> {
    let cargo = cargo();
    let build = ["build", "-p", "mailcrane-ffi", "--features", "bindgen"];
    if !run(&ctx.core, &cargo, &build)? {
        return Ok(Outcome::Failed("building mailcrane-ffi failed".into()));
    }
    let target_dir = target_dir(ctx);
    let lib_name = if cfg!(target_os = "macos") {
        "libmailcrane_ffi.dylib"
    } else if cfg!(target_os = "windows") {
        "mailcrane_ffi.dll"
    } else {
        "libmailcrane_ffi.so"
    };
    let lib = target_dir.join("debug").join(lib_name);
    for lang in ["kotlin", "swift"] {
        let out = target_dir.join("bindings").join(lang);
        let args: Vec<String> = vec![
            "run".into(),
            "-q".into(),
            "-p".into(),
            "mailcrane-ffi".into(),
            "--features".into(),
            "bindgen".into(),
            "--bin".into(),
            "uniffi-bindgen".into(),
            "--".into(),
            "generate".into(),
            "--library".into(),
            lib.display().to_string(),
            "--language".into(),
            lang.into(),
            "--out-dir".into(),
            out.display().to_string(),
        ];
        if !run(&ctx.core, &cargo, &args)? {
            return Ok(Outcome::Failed(format!(
                "generating {lang} bindings failed"
            )));
        }
    }
    Ok(Outcome::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffi_manifest() -> (String, String) {
        (
            "mailcrane-ffi".to_string(),
            "[features]\nbindgen = [\"uniffi/cli\"]\n\
             # Debug only; never in default.\n\
             debug-fixtures = [\"mailcrane-core/fixtures\"]\n"
                .to_string(),
        )
    }

    fn gradle() -> String {
        format!("val x = 1\n    {GRADLE_FEATURE_LINE}\ntasks.register(\"{GRADLE_GUARD}\")\n")
    }

    #[test]
    fn release_guard_accepts_the_intended_setup() {
        // Main code reaches the loader only through the DebugTools
        // interface, whose release implementation does nothing.
        let main = vec![(
            "Settings.kt".to_string(),
            "debugTools.loadFixtures()".to_string(),
        )];
        assert_eq!(
            check_release_guard(&[ffi_manifest()], &gradle(), &main),
            Ok(())
        );
    }

    #[test]
    fn release_guard_rejects_each_way_in() {
        let m = |name: &str, text: &str| vec![(name.to_string(), text.to_string())];
        let g = gradle();
        // A default feature.
        assert!(check_release_guard(
            &m("mailcrane-ffi", "default = [\"debug-fixtures\"]"),
            &g,
            &[]
        )
        .is_err());
        assert!(
            check_release_guard(&m("mailcrane-core", "default = [\"fixtures\"]"), &g, &[]).is_err()
        );
        // Another crate turning it on.
        assert!(check_release_guard(
            &m(
                "mailcrane-x",
                "mailcrane-ffi = { path = \"../mailcrane-ffi\", features = [\"debug-fixtures\"] }"
            ),
            &g,
            &[]
        )
        .is_err());
        assert!(check_release_guard(
            &m(
                "mailcrane-x",
                "mailcrane-core = { path = \"../mailcrane-core\", features = [\"fixtures\"] }"
            ),
            &g,
            &[]
        )
        .is_err());
        // Gradle enabling it unconditionally, twice, or without the guard.
        let always = "listOf(\"--features\", \"debug-fixtures\")\ncheckNoDebugCoreInRelease";
        assert!(check_release_guard(&[ffi_manifest()], always, &[]).is_err());
        let twice = format!("{g}\n// debug-fixtures\n");
        assert!(check_release_guard(&[ffi_manifest()], &twice, &[]).is_err());
        let unguarded = format!("{GRADLE_FEATURE_LINE}\n");
        assert!(check_release_guard(&[ffi_manifest()], &unguarded, &[]).is_err());
        // Main or release Kotlin calling the FFI function.
        let src = m("MainScreen.kt", "loadTestMail(path, listOf<FixtureFile>())");
        assert!(check_release_guard(&[ffi_manifest()], &g, &src).is_err());
    }

    fn parse(args: &[&str]) -> Result<Options, Error> {
        parse_options(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn only_takes_a_comma_separated_list() {
        let o = parse(&["--strict", "--only", "fmt,lint, test"]).unwrap();
        assert!(o.strict);
        assert_eq!(o.only, ["fmt", "lint", "test"]);
    }

    #[test]
    fn lists_and_repeats_combine() {
        let o = parse(&["--skip", "design", "--skip", "deny,android,"]).unwrap();
        assert_eq!(o.skip, ["design", "deny", "android"]);
        assert!(o.only.is_empty());
    }

    #[test]
    fn a_flag_without_a_value_is_an_error() {
        assert!(parse(&["--only"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
    }
}
