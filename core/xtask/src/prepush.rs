//! The ktlint version pins (#564): the Gradle plugin's ktlint and the
//! standalone CLI that `scripts/fleet/prepush.{sh,ps1}` run before a push
//! must be the same version, or the pre-push check and CI disagree.

/// Checks that the ktlint version pinned in `android/app/build.gradle.kts`
/// (`version.set("…")` in its `ktlint` block) is the one both pre-push
/// scripts pin. Returns that version.
pub fn check_pins(gradle: &str, sh: &str, ps1: &str) -> Result<String, String> {
    let gradle_pin = pinned(gradle, "version.set(\"", "\")")
        .ok_or("android/app/build.gradle.kts pins no ktlint version (version.set(\"…\"))")?;
    let sh_pin = pinned(sh, "KTLINT_VERSION=", "")
        .ok_or("scripts/fleet/prepush.sh pins no KTLINT_VERSION")?;
    let ps1_pin = pinned(ps1, "$KtlintVersion = '", "'")
        .ok_or("scripts/fleet/prepush.ps1 pins no $KtlintVersion")?;
    if sh_pin != gradle_pin || ps1_pin != gradle_pin {
        return Err(format!(
            "ktlint versions differ: Gradle {gradle_pin}, prepush.sh {sh_pin}, prepush.ps1 {ps1_pin}"
        ));
    }
    Ok(gradle_pin.to_owned())
}

/// The text between `start` and `end` on the first line that begins (after
/// indentation) with `start`; to the line's end when `end` is empty.
fn pinned<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(start)?;
        let value = if end.is_empty() {
            rest.trim()
        } else {
            &rest[..rest.find(end)?]
        };
        (!value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRADLE: &str = "ktlint {\n    version.set(\"1.5.0\")\n}\n";
    const SH: &str = "set -eu\nKTLINT_VERSION=1.5.0\n";
    const PS1: &str = "$KtlintVersion = '1.5.0'\n";

    #[test]
    fn matching_pins_pass() {
        assert_eq!(check_pins(GRADLE, SH, PS1), Ok("1.5.0".into()));
    }

    #[test]
    fn a_script_on_another_version_fails() {
        let err = check_pins(GRADLE, "KTLINT_VERSION=1.6.0\n", PS1).unwrap_err();
        assert!(err.contains("prepush.sh 1.6.0"), "{err}");
        assert!(check_pins(GRADLE, SH, "$KtlintVersion = '1.4.1'\n").is_err());
    }

    #[test]
    fn a_missing_pin_fails() {
        assert!(check_pins("ktlint {\n}\n", SH, PS1).is_err());
        assert!(check_pins(GRADLE, "set -eu\n", PS1).is_err());
        assert!(check_pins(GRADLE, SH, "").is_err());
    }

    /// The real files agree, so `cargo xtask ci` passes on main.
    #[test]
    fn the_real_pins_match() {
        let gradle = include_str!("../../../android/app/build.gradle.kts");
        let sh = include_str!("../../../scripts/fleet/prepush.sh");
        let ps1 = include_str!("../../../scripts/fleet/prepush.ps1");
        assert_eq!(check_pins(gradle, sh, ps1), Ok("1.5.0".into()));
    }
}
