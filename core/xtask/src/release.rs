//! `cargo xtask release-check <tag>`: the release tag and the Android version
//! in `android/app/build.gradle.kts` must agree (M6-03, #70).
//!
//! Tags are `vMAJOR.MINOR.PATCH`, optionally with a pre-release suffix such as
//! `v0.1.0-rc.1`. `versionName` is the tag without the `v`, and
//! `versionCode = MAJOR * 10000 + MINOR * 100 + PATCH`, so MINOR and PATCH
//! stay below 100. docs/releasing.md explains the scheme.

use std::fs;

use crate::{say, Ctx, Error};

/// A parsed release tag.
#[derive(Debug, PartialEq, Eq)]
pub struct Version {
    /// The tag without its leading `v`: what `versionName` must be.
    pub name: String,
    /// What `versionCode` must be.
    pub code: u32,
}

/// Parses `vMAJOR.MINOR.PATCH[-pre]`.
pub fn parse_tag(tag: &str) -> Result<Version, Error> {
    let bad = |why: &str| {
        format!("bad release tag `{tag}`: {why} (expected vMAJOR.MINOR.PATCH, e.g. v0.1.0 or v0.1.0-rc.1)")
    };
    let name = tag.strip_prefix('v').ok_or_else(|| bad("no leading `v`"))?;
    let (core, pre) = match name.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (name, None),
    };
    if let Some(pre) = pre {
        let ok = !pre.is_empty()
            && pre
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if !ok {
            return Err(bad(
                "the pre-release part may hold only letters, digits, `.` and `-`",
            ));
        }
    }
    let parts: Vec<&str> = core.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        return Err(bad("needs three numbers"));
    };
    let number = |s: &str| -> Result<u32, Error> {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
            return Err(bad("each part must be a number"));
        }
        // Leading zeros would make two tags map to one version.
        if s.len() > 1 && s.starts_with('0') {
            return Err(bad("no leading zeros"));
        }
        s.parse::<u32>().map_err(|_| bad("number too large"))
    };
    let (major, minor, patch) = (number(major)?, number(minor)?, number(patch)?);
    if minor > 99 || patch > 99 {
        return Err(bad("MINOR and PATCH must be below 100 for versionCode"));
    }
    let code = major
        .checked_mul(10_000)
        .and_then(|c| c.checked_add(minor.checked_mul(100)?))
        .and_then(|c| c.checked_add(patch))
        // Google Play's limit for versionCode.
        .filter(|c| *c <= 2_100_000_000)
        .ok_or_else(|| bad("MAJOR is too large for versionCode"))?;
    if code == 0 {
        return Err(bad("v0.0.0 gives versionCode 0, which Android refuses"));
    }
    Ok(Version {
        name: name.to_string(),
        code,
    })
}

/// The `versionCode` and `versionName` set in the Gradle file. Each must
/// appear exactly once, so a second, overriding line can't slip past.
pub fn read_gradle(text: &str) -> Result<(u32, String), Error> {
    let lines = |key: &str| -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter_map(|l| l.strip_prefix(key))
            .map(str::trim_start)
            .filter_map(|l| l.strip_prefix('='))
            .map(|v| v.trim().to_string())
            .collect()
    };
    let one = |key: &str| -> Result<String, Error> {
        match lines(key).as_slice() {
            [v] => Ok(v.clone()),
            [] => Err(format!(
                "`{key} = …` not found in android/app/build.gradle.kts"
            )),
            _ => Err(format!(
                "`{key}` is set more than once in android/app/build.gradle.kts"
            )),
        }
    };
    let code = one("versionCode")?;
    let code = code
        .parse::<u32>()
        .map_err(|_| format!("versionCode `{code}` is not a plain number"))?;
    let name = one("versionName")?;
    let name = name
        .strip_prefix('"')
        .and_then(|n| n.strip_suffix('"'))
        .filter(|n| !n.contains('"') && !n.contains('$'))
        .ok_or_else(|| format!("versionName `{name}` is not a plain string"))?
        .to_string();
    Ok((code, name))
}

/// Compares the tag with the Gradle file's text.
pub fn check(tag: &str, gradle: &str) -> Result<Version, Error> {
    let want = parse_tag(tag)?;
    let (code, name) = read_gradle(gradle)?;
    let mut problems = Vec::new();
    if name != want.name {
        problems.push(format!(
            "versionName is \"{name}\" but tag {tag} needs \"{}\"",
            want.name
        ));
    }
    if code != want.code {
        problems.push(format!(
            "versionCode is {code} but tag {tag} needs {}",
            want.code
        ));
    }
    if problems.is_empty() {
        Ok(want)
    } else {
        Err(format!(
            "{}\nBump both in android/app/build.gradle.kts in a PR, merge it, then tag that commit (docs/releasing.md).",
            problems.join("\n")
        ))
    }
}

/// The command: `cargo xtask release-check <tag>`.
pub fn run(ctx: &Ctx, mut args: impl Iterator<Item = String>) -> Result<(), Error> {
    let tag = args.next().ok_or_else(|| {
        "release-check needs a tag, e.g. `cargo xtask release-check v0.1.0`".to_string()
    })?;
    if let Some(extra) = args.next() {
        return Err(format!("release-check takes one tag; unexpected `{extra}`"));
    }
    let path = ctx.root.join("android/app/build.gradle.kts");
    let gradle =
        fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let v = check(&tag, &gradle)?;
    say(format_args!(
        "release-check: {tag} matches versionName \"{}\" and versionCode {}",
        v.name, v.code
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradle(code: &str, name: &str) -> String {
        format!("    defaultConfig {{\n        versionCode = {code}\n        versionName = {name}\n    }}\n")
    }

    #[test]
    fn parses_plain_and_pre_release_tags() {
        assert_eq!(
            parse_tag("v0.1.0"),
            Ok(Version {
                name: "0.1.0".into(),
                code: 100
            })
        );
        assert_eq!(
            parse_tag("v1.2.3-rc.1"),
            Ok(Version {
                name: "1.2.3-rc.1".into(),
                code: 10_203
            })
        );
        assert_eq!(parse_tag("v0.0.1").map(|v| v.code), Ok(1));
        assert_eq!(parse_tag("v12.99.99").map(|v| v.code), Ok(129_999));
    }

    #[test]
    fn refuses_malformed_tags() {
        for tag in [
            "",
            "v",
            "0.1.0",
            "v0.1",
            "v0.1.0.0",
            "v0.1.x",
            "v0.100.0",
            "v0.1.100",
            "v01.0.0",
            "v0.0.0",
            "v0.1.0-",
            "v0.1.0-rc 1",
            "v0.1.0-rc/1",
            "v-1.0.0",
            "v+1.0.0",
            "v99999999.0.0",
            "v0..0",
        ] {
            assert!(parse_tag(tag).is_err(), "{tag} should be refused");
        }
    }

    #[test]
    fn matching_versions_pass() {
        assert!(check("v0.1.0", &gradle("100", "\"0.1.0\"")).is_ok());
        assert!(check("v0.1.0-rc.1", &gradle("100", "\"0.1.0-rc.1\"")).is_ok());
    }

    #[test]
    fn mismatches_are_reported_together() {
        let err = check("v0.2.0", &gradle("1", "\"0.0.0\"")).unwrap_err();
        assert!(err.contains("versionName"), "{err}");
        assert!(err.contains("versionCode"), "{err}");
        assert!(check("v0.1.0", &gradle("100", "\"0.1.0-rc.1\"")).is_err());
        assert!(check("v0.1.0", &gradle("101", "\"0.1.0\"")).is_err());
    }

    #[test]
    fn odd_gradle_files_are_refused() {
        // Missing, computed or duplicated values.
        assert!(read_gradle("versionName = \"0.1.0\"").is_err());
        assert!(read_gradle(&gradle("1 + 99", "\"0.1.0\"")).is_err());
        assert!(read_gradle(&gradle("100", "computeName()")).is_err());
        assert!(read_gradle(&gradle("100", "\"0.$minor.0\"")).is_err());
        let twice = gradle("100", "\"0.1.0\"") + "versionCode = 100\n";
        assert!(read_gradle(&twice).is_err());
    }

    #[test]
    fn reads_the_real_gradle_file() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../android/app/build.gradle.kts"
        );
        let text = std::fs::read_to_string(path).unwrap();
        assert!(read_gradle(&text).is_ok());
    }
}
