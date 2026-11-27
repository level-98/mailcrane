//! The Rust half of the in-app open-source licences list (#366).
//!
//! `cargo xtask licences` runs cargo-about over `mailcrane-ffi` (the core as
//! shipped in the APK, see `core/about.toml`) with the template
//! `android/app/licences/rust-licences.hbs`, and writes
//! `android/app/licences/rust-licences.txt`. The Android build merges that file
//! with the Android libraries' licences into `assets/licences.json`.
//!
//! The file is committed, so building the app (including F-Droid's build) needs
//! no cargo-about. `cargo xtask ci` regenerates it and fails when the committed
//! copy is out of date, and cargo-about itself fails on a licence missing from
//! `accepted`, which must equal `allow` in `deny.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{cargo, has_tool, run, say, Ctx, Error, Outcome};

const NOTICES: &str = "android/app/licences/rust-licences.txt";
const TEMPLATE: &str = "android/app/licences/rust-licences.hbs";

/// The quoted strings in the `key = [ … ]` list of a TOML file.
pub fn toml_list(text: &str, key: &str) -> Result<Vec<String>, Error> {
    let start = format!("{key} = [");
    let mut lines = text
        .lines()
        .skip_while(|l| !l.trim_start().starts_with(&start));
    if lines.next().is_none() {
        return Err(format!("no `{key} = [` list"));
    }
    let mut out = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.starts_with(']') {
            return Ok(out);
        }
        if let Some(rest) = line.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                out.push(rest.get(..end).unwrap_or_default().to_string());
            }
        }
    }
    Err(format!("the `{key}` list isn't closed"))
}

/// cargo-about's `accepted` must be cargo deny's `allow`, in any order.
pub fn check_lists(deny: &str, about: &str) -> Result<(), Error> {
    let mut allow = toml_list(deny, "allow").map_err(|e| format!("deny.toml: {e}"))?;
    let mut accepted = toml_list(about, "accepted").map_err(|e| format!("about.toml: {e}"))?;
    allow.sort();
    accepted.sort();
    if allow == accepted {
        Ok(())
    } else {
        Err(format!(
            "core/about.toml `accepted` {accepted:?} differs from core/deny.toml `allow` {allow:?}"
        ))
    }
}

/// Line endings made LF, so Windows and Linux produce the same file.
pub fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn generate(ctx: &Ctx, out: &Path) -> Result<String, Error> {
    let template = ctx.root.join(TEMPLATE);
    let args = [
        "about".to_string(),
        "generate".to_string(),
        "--locked".to_string(),
        // Fail on any crate whose licence cargo-about could not determine.
        "--fail".to_string(),
        "--manifest-path".to_string(),
        "crates/mailcrane-ffi/Cargo.toml".to_string(),
        "--config".to_string(),
        "about.toml".to_string(),
        "--output-file".to_string(),
        out.display().to_string(),
        template.display().to_string(),
    ];
    if !run(&ctx.core, &cargo(), &args)? {
        return Err(
            "`cargo about generate` failed (a licence not in core/about.toml `accepted`?)".into(),
        );
    }
    let text = fs::read_to_string(out).map_err(|e| format!("reading {}: {e}", out.display()))?;
    Ok(normalise(&text))
}

fn read(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))
}

fn scratch(ctx: &Ctx) -> PathBuf {
    crate::target_dir(ctx).join("xtask-rust-licences.txt")
}

/// `cargo xtask licences`: rewrite the committed notices file.
pub fn write(ctx: &Ctx) -> Result<(), Error> {
    check_lists(
        &read(&ctx.core.join("deny.toml"))?,
        &read(&ctx.core.join("about.toml"))?,
    )?;
    let tmp = scratch(ctx);
    if let Some(dir) = tmp.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let text = generate(ctx, &tmp)?;
    let path = ctx.root.join(NOTICES);
    fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
    let _ = fs::remove_file(&tmp);
    say(format_args!("wrote {NOTICES}"));
    Ok(())
}

/// Part of the `deny` step: the lists agree and the committed file is current.
pub fn check(ctx: &Ctx) -> Result<Outcome, Error> {
    if let Err(e) = check_lists(
        &read(&ctx.core.join("deny.toml"))?,
        &read(&ctx.core.join("about.toml"))?,
    ) {
        return Ok(Outcome::Failed(e));
    }
    if !has_tool("cargo-about", "--version") {
        return Ok(Outcome::Skipped(
            "cargo-about is not installed, so rust-licences.txt wasn't checked (cargo install cargo-about --version 0.9.3 --locked --features cli)".into(),
        ));
    }
    let tmp = scratch(ctx);
    if let Some(dir) = tmp.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let fresh = generate(ctx, &tmp)?;
    let _ = fs::remove_file(&tmp);
    let committed = normalise(&read(&ctx.root.join(NOTICES))?);
    if fresh == committed {
        Ok(Outcome::Passed)
    } else {
        Ok(Outcome::Failed(format!(
            "{NOTICES} is out of date (a Rust dependency changed): run `cargo xtask licences` and commit the result"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_toml_lists() {
        let t = "x = 1\nallow = [\n    \"MIT\",\n    # a comment\n    \"Apache-2.0 WITH LLVM-exception\",\n]\n";
        assert_eq!(
            toml_list(t, "allow"),
            Ok(vec![
                "MIT".to_string(),
                "Apache-2.0 WITH LLVM-exception".to_string()
            ])
        );
        assert!(toml_list(t, "accepted").is_err());
        assert!(toml_list("allow = [\n\"MIT\",\n", "allow").is_err());
    }

    #[test]
    fn lists_must_agree() {
        let deny = "allow = [\n\"MIT\",\n\"ISC\",\n]\n";
        assert!(check_lists(deny, "accepted = [\n\"ISC\",\n\"MIT\",\n]\n").is_ok());
        assert!(check_lists(deny, "accepted = [\n\"MIT\",\n]\n").is_err());
        assert!(check_lists(deny, "accepted = [\n\"MIT\",\n\"ISC\",\n\"GPL-3.0\",\n]\n").is_err());
    }

    #[test]
    fn the_real_lists_agree() {
        let core = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let deny = std::fs::read_to_string(format!("{core}/deny.toml")).unwrap();
        let about = std::fs::read_to_string(format!("{core}/about.toml")).unwrap();
        assert_eq!(check_lists(&deny, &about), Ok(()));
    }

    #[test]
    fn normalises_line_endings() {
        assert_eq!(normalise("a\r\nb\n"), "a\nb\n");
    }
}
