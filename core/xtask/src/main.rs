//! `cargo xtask`: the one place every check lives. CI runs `cargo xtask ci --strict`.
//!
//! Std only, no dependencies, and no panics: every failure is an `Err` or a
//! failed step, never an unwind.
#![forbid(unsafe_code)]

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
  fmt       cargo fmt --all --check
  lint      cargo clippy --workspace --all-targets -- -D warnings
  test      cargo test --workspace
  deny      cargo deny check
  android   ktlintCheck, lintDebug and testDebugUnitTest through Gradle
  ios       XCFramework, xcodegen, swiftlint and xcodebuild test (macOS only)
  bindings  generate Kotlin and Swift bindings into core/target/bindings/

Flags (for ci):
  --strict       a skipped step counts as a failure
  --only <step>  run only this step (repeatable)
  --skip <step>  leave this step out (repeatable)
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
                if arg == "--only" {
                    options.only.push(value);
                } else {
                    options.skip.push(value);
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
    let args = ["ktlintCheck", "lintDebug", "testDebugUnitTest"];
    let ok = run(&dir, &wrapper.display().to_string(), &args)?;
    Ok(outcome(
        ok,
        "./gradlew ktlintCheck lintDebug testDebugUnitTest",
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
        "platform=iOS Simulator,OS=latest".to_string(),
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
