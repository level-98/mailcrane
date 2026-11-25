//! `cargo xtask ispdb <checkout>`: imports a snapshot of Mozilla's ISPDB
//! (ADR 0003) from a local checkout of `github.com/thunderbird/autoconfig`.
//!
//! Copies `ispdb/*.xml` into `mailcrane-core/data/ispdb/`, with the licence
//! and a note of the commit, and regenerates the domain index the core
//! compiles in. Nothing is downloaded here: clone or update the checkout
//! yourself, pinned to the commit you mean to ship.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::{say, Ctx, Error};

/// Largest single file accepted; the biggest upstream file is a few KB.
const MAX_FILE: u64 = 64 * 1024;

fn valid_domain(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| {
            !l.is_empty()
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// The `<domain>` values in one file, lowercase.
fn domains(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<domain>") {
        let after = &rest[start + "<domain>".len()..];
        let Some(end) = after.find("</domain>") else {
            break;
        };
        out.push(after[..end].trim().to_ascii_lowercase());
        rest = &after[end..];
    }
    out
}

/// A safe file name: lowercase letters, digits, `.` and `-`, not starting
/// with a dot. Names needn't be domains (upstream has `oxcloud-eu.xml`).
fn valid_file_stem(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// A git command's raw output.
fn git_bytes(checkout: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let out = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed in {}",
            args.join(" "),
            checkout.display()
        ));
    }
    Ok(out.stdout)
}

fn git(checkout: &Path, args: &[&str]) -> Result<String, Error> {
    Ok(String::from_utf8_lossy(&git_bytes(checkout, args)?)
        .trim()
        .to_owned())
}

pub(crate) fn import(ctx: &Ctx, mut args: impl Iterator<Item = String>) -> Result<(), Error> {
    let checkout = args
        .next()
        .ok_or("usage: cargo xtask ispdb <path to a thunderbird/autoconfig checkout>")?;
    let checkout = Path::new(&checkout);
    let commit = git(checkout, &["rev-parse", "HEAD"])?;
    let date = git(checkout, &["log", "-1", "--format=%cs"])?;

    let crate_dir = ctx.core.join("crates").join("mailcrane-core");
    let dest = crate_dir.join("data").join("ispdb");
    fs::create_dir_all(&dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    // Start clean, so files removed upstream go away here too.
    for entry in fs::read_dir(&dest).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|x| x == "xml") {
            fs::remove_file(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        }
    }

    // From the committed objects, not the working tree, so the copies are
    // byte-for-byte upstream's whatever line-ending settings the checkout has.
    let listing = git(checkout, &["ls-tree", "--name-only", "HEAD", "ispdb/"])?;
    let mut names: Vec<String> = listing
        .lines()
        .filter_map(|l| l.strip_prefix("ispdb/"))
        .filter(|n| n.ends_with(".xml"))
        .map(str::to_owned)
        .collect();
    names.sort();

    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut files = Vec::new();
    for name in &names {
        let stem = name.trim_end_matches(".xml");
        if !valid_file_stem(stem) {
            return Err(format!("unexpected file name `{name}`"));
        }
        let bytes = git_bytes(checkout, &["show", &format!("HEAD:ispdb/{name}")])?;
        let size = bytes.len() as u64;
        if size > MAX_FILE {
            return Err(format!(
                "`{name}` is {size} bytes, over the {MAX_FILE}-byte limit"
            ));
        }
        let text =
            String::from_utf8(bytes).map_err(|e| format!("`{name}` isn't UTF-8 text: {e}"))?;
        if !text.contains("<clientConfig") || text.contains("<!DOCTYPE") {
            return Err(format!("`{name}` isn't a plain clientConfig document"));
        }
        let number = files.len();
        for d in domains(&text) {
            if !valid_domain(&d) {
                return Err(format!("`{name}` lists an invalid domain `{d}`"));
            }
            if let Some(first) = index.get(&d) {
                if *first != number {
                    say(format_args!(
                        "note: `{d}` is in {} and {name}; keeping the first",
                        files[*first]
                    ));
                }
            } else {
                index.insert(d, number);
            }
        }
        fs::write(dest.join(name), &text).map_err(|e| format!("cannot write {name}: {e}"))?;
        files.push(name.clone());
    }

    let licence = git_bytes(checkout, &["show", "HEAD:LICENSE"])?;
    fs::write(dest.join("LICENSE"), licence).map_err(|e| format!("cannot write LICENSE: {e}"))?;
    let source_note = format!(
        "# ISPDB snapshot\n\n\
         Mozilla Thunderbird's database of mail server settings, bundled for automatic account \
         setup ([ADR 0003](../../../../../docs/decisions/0003-bundled-ispdb.md)).\n\n\
         - Source: <https://github.com/thunderbird/autoconfig>, directory `ispdb/`\n\
         - Commit: `{commit}` ({date})\n\
         - Files: {}; domains indexed: {}\n\
         - Licence: Mozilla Public License 2.0 (`LICENSE` in this folder)\n\n\
         Don't edit these files by hand. To update, check out a newer commit and run \
         `cargo xtask ispdb <checkout>`.\n",
        files.len(),
        index.len()
    );
    fs::write(dest.join("SOURCE.md"), source_note).map_err(|e| e.to_string())?;

    let mut rs = String::from(
        "// @generated by `cargo xtask ispdb`. Don't edit; see data/ispdb/SOURCE.md.\n\
         //! The bundled ISPDB snapshot (ADR 0003): every file, and each domain it\n\
         //! lists, sorted for binary search.\n\n\
         #[rustfmt::skip]\n\
         pub(super) static FILES: &[&[u8]] = &[\n",
    );
    for name in &files {
        rs.push_str(&format!(
            "    include_bytes!(\"../../../data/ispdb/{name}\"),\n"
        ));
    }
    rs.push_str("];\n\n#[rustfmt::skip]\npub(super) static DOMAINS: &[(&str, usize)] = &[\n");
    for (domain, number) in &index {
        rs.push_str(&format!("    (\"{domain}\", {number}),\n"));
    }
    rs.push_str("];\n");
    let index_path = crate_dir
        .join("src")
        .join("account")
        .join("autoconfig")
        .join("ispdb_index.rs");
    fs::write(&index_path, rs)
        .map_err(|e| format!("cannot write {}: {e}", index_path.display()))?;

    say(format_args!(
        "Imported {} files ({} domains) from {commit} ({date}).",
        files.len(),
        index.len()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_read_in_order() {
        let xml = "<clientConfig><emailProvider><domain>gmail.com</domain>\n<domain> GoogleMail.com </domain><!-- MX --><domain>google.com</domain></emailProvider></clientConfig>";
        assert_eq!(domains(xml), ["gmail.com", "googlemail.com", "google.com"]);
        assert!(domains("<domain>unclosed").is_empty());
    }

    #[test]
    fn domain_rules() {
        assert!(valid_domain("1und1.de"));
        assert!(!valid_domain("localhost"));
        assert!(!valid_domain("Bad.com"));
        assert!(!valid_domain("../x.com"));
        assert!(!valid_domain("a b.com"));
        assert!(valid_file_stem("oxcloud-eu"));
        assert!(!valid_file_stem("../evil"));
        assert!(!valid_file_stem(".hidden"));
    }
}
