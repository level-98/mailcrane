//! `cargo xtask`: the one place every check lives. CI runs `cargo xtask ci --strict`.
//!
//! Std only, no dependencies, and no panics: every failure is an `Err` or a
//! failed step, never an unwind.
#![forbid(unsafe_code)]

mod ispdb;

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
const CI_STEPS: [Step; 7] = [
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
    Step {
        name: "ios",
        run: ios,
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
  design    generated theme check and unit tests
  fmt       cargo fmt --all --check
  lint      cargo clippy --workspace --all-targets -- -D warnings
  test      cargo test --workspace
  deny      cargo deny check
  android   ktlintCheck, lintDebug, testDebugUnitTest and assembleDebug through Gradle
  ios       XCFramework, xcodegen, swiftlint and xcodebuild test (macOS only)
  bindings  generate Kotlin and Swift bindings into core/target/bindings/
  fuzz <target> [--seconds N]
            run a fuzz target (default 60 s); needs nightly and cargo-fuzz.
            Not part of ci. Targets: mime_parse, imap_response, html_sanitize, address_list
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
    let ok = run(&ctx.root, python, &tests)?;
    Ok(outcome(ok, "python -m unittest design/test_gen_theme.py"))
}

fn fmt(ctx: &Ctx) -> Result<Outcome, Error> {
    let ok = run(&ctx.core, &cargo(), &["fmt", "--all", "--check"])?;
    Ok(outcome(ok, "cargo fmt --all --check"))
}

fn lint(ctx: &Ctx) -> Result<Outcome, Error> {
    let args = [
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ];
    Ok(outcome(run(&ctx.core, &cargo(), &args)?, "cargo clippy"))
}

fn test(ctx: &Ctx) -> Result<Outcome, Error> {
    let ok = run(&ctx.core, &cargo(), &["test", "--workspace"])?;
    Ok(outcome(ok, "cargo test --workspace"))
}

fn deny(ctx: &Ctx) -> Result<Outcome, Error> {
    if !ctx.core.join("deny.toml").is_file() {
        return Ok(Outcome::Skipped("core/deny.toml does not exist yet".into()));
    }
    if !has_tool("cargo-deny", "--version") {
        return Ok(Outcome::Skipped("cargo-deny is not installed".into()));
    }
    let ok = run(&ctx.core, &cargo(), &["deny", "check"])?;
    Ok(outcome(ok, "cargo deny check"))
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
    let args = [
        "ktlintCheck",
        "lintDebug",
        "testDebugUnitTest",
        "assembleDebug",
    ];
    let ok = run(&dir, &wrapper.display().to_string(), &args)?;
    Ok(outcome(
        ok,
        "./gradlew ktlintCheck lintDebug testDebugUnitTest assembleDebug",
    ))
}

fn ios(ctx: &Ctx) -> Result<Outcome, Error> {
    let dir = ctx.root.join("ios");
    if !cfg!(target_os = "macos") {
        return Ok(Outcome::Skipped("iOS checks need macOS".into()));
    }
    if !has_tool("xcodebuild", "-version") {
        return Ok(Outcome::Skipped("xcodebuild is not installed".into()));
    }
    if !dir.join("project.yml").is_file() {
        return Ok(Outcome::Skipped(
            "ios/project.yml does not exist yet".into(),
        ));
    }
    for tool in ["xcodegen", "swiftlint"] {
        if !has_tool(tool, "--version") {
            return Ok(Outcome::Skipped(format!("{tool} is not installed")));
        }
    }

    if !run(&dir, "sh", &["scripts/build-xcframework.sh"])? {
        return Ok(Outcome::Failed(
            "`scripts/build-xcframework.sh` failed".into(),
        ));
    }

    if !run(&dir, "xcodegen", &["generate"])? {
        return Ok(Outcome::Failed("`xcodegen generate` failed".into()));
    }
    if !run(&dir, "swiftlint", &["--strict"])? {
        return Ok(Outcome::Failed("`swiftlint --strict` failed".into()));
    }

    let result_bundle = dir.join("build").join("Mailcrane.xcresult");
    let test_args = vec![
        "test".to_string(),
        "-scheme".to_string(),
        "Mailcrane".to_string(),
        "-destination".to_string(),
        "platform=iOS Simulator,name=iPhone 17".to_string(),
        "-resultBundlePath".to_string(),
        result_bundle.display().to_string(),
    ];
    let ok = run(&dir, "xcodebuild", &test_args)?;
    Ok(outcome(ok, "xcodebuild test"))
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
const FUZZ_TARGETS: [(&str, &str, &str); 4] = [
    ("mime_parse", "mime", "eml"),
    ("imap_response", "imap", "imap"),
    ("html_sanitize", "html", "html"),
    ("address_list", "address", "txt"),
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
        let o = parse(&["--skip", "ios", "--skip", "deny,android,"]).unwrap();
        assert_eq!(o.skip, ["ios", "deny", "android"]);
        assert!(o.only.is_empty());
    }

    #[test]
    fn a_flag_without_a_value_is_an_error() {
        assert!(parse(&["--only"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
    }
}
