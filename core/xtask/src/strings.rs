//! Translated Android strings keep their format placeholders (M6-02, #69).
//!
//! Translations arrive from Weblate. A translation that drops `%1$s`, adds
//! `%2$d` or turns `%d` into `%s` compiles fine, then shows wrong text or
//! throws when the app formats it. This check compares every
//! `values-*/strings.xml` with the English `values/strings.xml`:
//!
//! - a `<string>` must use the same placeholders (argument number and
//!   conversion) as the English one, in any order;
//! - in `<plurals>`, the `other` item must match the English `other`, and the
//!   other quantities may use only placeholders the English items use (a
//!   language's `one` may say "One message" with no number);
//! - every placeholder must be well formed, and names must exist in English.
//!
//! Strings marked `formatted="false"` or `translatable="false"` are skipped.
//! A deliberately small reader: Android's resource compiler and lint check
//! the XML itself. Std only, like the rest of xtask.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// One placeholder: the argument it reads (1-based) and its conversion,
/// lower-cased (`%S` formats like `%s`).
type Placeholder = (u32, char);
type Placeholders = BTreeSet<Placeholder>;

/// What a strings file holds, by resource name.
#[derive(Debug, Default)]
pub struct Resources {
    strings: BTreeMap<String, Placeholders>,
    /// Quantity → placeholders.
    plurals: BTreeMap<String, BTreeMap<String, Placeholders>>,
    /// Marked `formatted="false"` or `translatable="false"`. Translations of
    /// these aren't checked either: Weblate may not copy the attribute.
    skipped: BTreeSet<String>,
}

/// The placeholders in one string value, or why one is malformed.
pub fn placeholders(text: &str) -> Result<Placeholders, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Placeholders::new();
    let mut next_plain: u32 = 1;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        if c != '%' {
            continue;
        }
        let start = i - 1;
        let snippet = |end: usize| -> String {
            chars
                .get(start..end.min(chars.len()))
                .map(|s| s.iter().collect())
                .unwrap_or_default()
        };
        // `%%` is a literal percent sign and `%n` a line break.
        if matches!(chars.get(i), Some('%' | 'n')) {
            i += 1;
            continue;
        }
        // %[argument$][flags][width][.precision]conversion (java.util.Formatter).
        let digits_from = |mut j: usize| {
            while chars.get(j).is_some_and(char::is_ascii_digit) {
                j += 1;
            }
            j
        };
        let mut index = None;
        let after = digits_from(i);
        if after > i && chars.get(after) == Some(&'$') {
            let n: String = chars
                .get(i..after)
                .map(|s| s.iter().collect())
                .unwrap_or_default();
            let n = n
                .parse::<u32>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or_else(|| format!("bad argument number in `{}`", snippet(after + 1)))?;
            index = Some(n);
            i = after + 1;
        }
        while let Some(&flag) = chars
            .get(i)
            .filter(|c| matches!(c, '-' | '#' | '+' | ' ' | '0' | ',' | '('))
        {
            // "50 % de réduction" would read as `% d`, a number. No English
            // string uses the space flag, so it's always a stray percent sign.
            if flag == ' ' {
                return Err(format!(
                    "stray % in `{}` (write a literal % as %%)",
                    snippet(i + 2)
                ));
            }
            i += 1;
        }
        i = digits_from(i);
        if chars.get(i) == Some(&'.') {
            let after = digits_from(i + 1);
            if after == i + 1 {
                return Err(format!("bad precision in `{}`", snippet(i + 1)));
            }
            i = after;
        }
        let Some(&conv) = chars.get(i) else {
            return Err(format!("unfinished placeholder `{}`", snippet(i)));
        };
        let conv = conv.to_ascii_lowercase();
        if !matches!(
            conv,
            'b' | 'h' | 's' | 'c' | 'd' | 'o' | 'x' | 'e' | 'f' | 'g' | 'a' | 't'
        ) {
            return Err(format!(
                "unknown placeholder `{}` (write a literal % as %%)",
                snippet(i + 1)
            ));
        }
        i += 1;
        // Dates and times: %tY and friends take one more letter.
        if conv == 't' {
            if !chars.get(i).is_some_and(char::is_ascii_alphabetic) {
                return Err(format!("unfinished date placeholder `{}`", snippet(i)));
            }
            i += 1;
        }
        let n = match index {
            Some(n) => n,
            None => {
                let n = next_plain;
                next_plain = next_plain
                    .checked_add(1)
                    .ok_or_else(|| "too many placeholders".to_string())?;
                n
            }
        };
        out.insert((n, conv));
    }
    Ok(out)
}

/// `text` without `<!-- … -->` comments.
fn strip_comments(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("<!--") {
        out.push_str(rest.get(..at).unwrap_or_default());
        let after = rest.get(at + 4..).unwrap_or_default();
        let end = after.find("-->").ok_or("unclosed comment")?;
        rest = after.get(end + 3..).unwrap_or_default();
    }
    out.push_str(rest);
    Ok(out)
}

/// The value of `name="…"` in a start tag's attributes.
fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = format!(" {attrs}").find(&key)?;
    // `at` is in the padded string; the value starts after the key there,
    // which is one byte earlier in `attrs`.
    let start = at + key.len() - 1;
    let value = attrs.get(start..)?;
    value.get(..value.find('"')?)
}

/// Each `<tag …>content</tag>` in `text`, as (attributes, content). A
/// self-closing `<tag … />` has empty content.
fn elements<'a>(text: &'a str, tag: &str) -> Result<Vec<(&'a str, &'a str)>, String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(&open) {
        let after = rest.get(at + open.len()..).unwrap_or_default();
        // `<string-array` starts with `<string`; only a space or `>` ends the name.
        if !after.starts_with([' ', '\t', '\r', '\n', '>']) {
            rest = after;
            continue;
        }
        let end = after
            .find('>')
            .ok_or_else(|| format!("unclosed <{tag}> tag"))?;
        let attrs = after.get(..end).unwrap_or_default();
        if let Some(attrs) = attrs.strip_suffix('/') {
            out.push((attrs.trim(), ""));
            rest = after.get(end + 1..).unwrap_or_default();
            continue;
        }
        let body = after.get(end + 1..).unwrap_or_default();
        let stop = body
            .find(&close)
            .ok_or_else(|| format!("<{tag}> without {close}"))?;
        out.push((attrs.trim(), body.get(..stop).unwrap_or_default()));
        rest = body.get(stop + close.len()..).unwrap_or_default();
    }
    Ok(out)
}

fn skipped(attrs: &str) -> bool {
    attribute(attrs, "formatted") == Some("false")
        || attribute(attrs, "translatable") == Some("false")
}

/// Reads a strings file, leaving out the names in `skip` (the English
/// file's skipped ones). Problems name the resource they're in.
pub fn parse(text: &str, skip: &BTreeSet<String>) -> Result<Resources, Vec<String>> {
    let text = strip_comments(text).map_err(|e| vec![e])?;
    let mut res = Resources::default();
    let mut problems = Vec::new();
    let name_of = |attrs: &str| attribute(attrs, "name").map(str::to_string);
    for (attrs, body) in elements(&text, "string").map_err(|e| vec![e])? {
        let Some(name) = name_of(attrs) else {
            problems.push("a <string> has no name".into());
            continue;
        };
        if skipped(attrs) || skip.contains(&name) {
            res.skipped.insert(name);
            continue;
        }
        match placeholders(body) {
            Ok(p) => {
                res.strings.insert(name, p);
            }
            Err(e) => problems.push(format!("{name}: {e}")),
        }
    }
    for (attrs, body) in elements(&text, "plurals").map_err(|e| vec![e])? {
        let Some(name) = name_of(attrs) else {
            problems.push("a <plurals> has no name".into());
            continue;
        };
        if skipped(attrs) || skip.contains(&name) {
            res.skipped.insert(name);
            continue;
        }
        let mut items = BTreeMap::new();
        for (item_attrs, item) in elements(body, "item").map_err(|e| vec![e])? {
            let Some(quantity) = attribute(item_attrs, "quantity") else {
                problems.push(format!("{name}: an <item> has no quantity"));
                continue;
            };
            match placeholders(item) {
                Ok(p) => {
                    items.insert(quantity.to_string(), p);
                }
                Err(e) => problems.push(format!("{name} ({quantity}): {e}")),
            }
        }
        res.plurals.insert(name, items);
    }
    if problems.is_empty() {
        Ok(res)
    } else {
        Err(problems)
    }
}

fn show(p: &Placeholders) -> String {
    if p.is_empty() {
        return "none".into();
    }
    p.iter()
        .map(|(n, c)| format!("%{n}${c}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compares a translation with the English resources.
pub fn compare(base: &Resources, tr: &Resources) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, got) in &tr.strings {
        match base.strings.get(name) {
            None => problems.push(format!(
                "{name}: not in values/strings.xml (renamed or removed?)"
            )),
            Some(want) if want != got => problems.push(format!(
                "{name}: placeholders {} but English has {}",
                show(got),
                show(want)
            )),
            Some(_) => {}
        }
    }
    for (name, items) in &tr.plurals {
        let Some(want) = base.plurals.get(name) else {
            problems.push(format!(
                "{name}: plurals not in values/strings.xml (renamed or removed?)"
            ));
            continue;
        };
        let allowed: Placeholders = want.values().flatten().copied().collect();
        for (quantity, got) in items {
            if quantity == "other" {
                let want_other = want.get("other").cloned().unwrap_or_default();
                if *got != want_other {
                    problems.push(format!(
                        "{name} (other): placeholders {} but English has {}",
                        show(got),
                        show(&want_other)
                    ));
                }
            } else if !got.is_subset(&allowed) {
                problems.push(format!(
                    "{name} ({quantity}): placeholders {} but English allows only {}",
                    show(got),
                    show(&allowed)
                ));
            }
        }
    }
    problems
}

/// Checks every `values-*/strings.xml` under `res` against `values/strings.xml`.
/// Returns how many translations were checked.
pub fn check_dir(res: &Path) -> Result<usize, String> {
    let read =
        |p: &Path| fs::read_to_string(p).map_err(|e| format!("can't read {}: {e}", p.display()));
    let base_path = res.join("values/strings.xml");
    let base = parse(&read(&base_path)?, &BTreeSet::new())
        .map_err(|p| format!("values/strings.xml:\n  {}", p.join("\n  ")))?;
    let mut dirs: Vec<_> = fs::read_dir(res)
        .map_err(|e| format!("can't list {}: {e}", res.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("values-"))
        })
        .filter(|p| p.join("strings.xml").is_file())
        .collect();
    dirs.sort();
    let mut problems = Vec::new();
    for dir in &dirs {
        let label = format!(
            "{}/strings.xml",
            dir.file_name().and_then(|n| n.to_str()).unwrap_or("?")
        );
        let found = match parse(&read(&dir.join("strings.xml"))?, &base.skipped) {
            Ok(tr) => compare(&base, &tr),
            Err(p) => p,
        };
        problems.extend(found.into_iter().map(|p| format!("{label}: {p}")));
    }
    if problems.is_empty() {
        Ok(dirs.len())
    } else {
        Err(format!(
            "translated strings don't match the English placeholders:\n  {}",
            problems.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[(u32, char)]) -> Placeholders {
        items.iter().copied().collect()
    }

    #[test]
    fn reads_placeholders() {
        assert_eq!(placeholders("Hello"), Ok(set(&[])));
        assert_eq!(placeholders("%d selected"), Ok(set(&[(1, 'd')])));
        assert_eq!(placeholders("%s and %s"), Ok(set(&[(1, 's'), (2, 's')])));
        assert_eq!(
            placeholders("%2$s before %1$d"),
            Ok(set(&[(1, 'd'), (2, 's')]))
        );
        assert_eq!(placeholders("100%% sure%n"), Ok(set(&[])));
        assert_eq!(
            placeholders("%1$.2f GB, %-5S"),
            Ok(set(&[(1, 'f'), (1, 's')]))
        );
        assert_eq!(placeholders("on %1$tY"), Ok(set(&[(1, 't')])));
    }

    #[test]
    fn refuses_malformed_placeholders() {
        for bad in [
            "50% off",
            "%",
            "ends with %1$",
            "%0$s",
            "%1$.f",
            "%1$t",
            "%q",
        ] {
            assert!(placeholders(bad).is_err(), "{bad:?} should be refused");
        }
    }

    const BASE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<resources>
    <!-- <string name="commented">%9$s</string> -->
    <string name="hello">Hello %1$s</string>
    <string name="two">%1$s sent %2$d</string>
    <string name="plain">Inbox</string>
    <string name="pct" formatted="false">Can't contain %, * or controls.</string>
    <string-array name="arr"><item>x</item></string-array>
    <plurals name="count">
        <item quantity="one">%d message</item>
        <item quantity="other">%d messages</item>
    </plurals>
</resources>
"#;

    fn base() -> Resources {
        parse(BASE, &BTreeSet::new()).unwrap()
    }

    fn check(tr: &str) -> Vec<String> {
        let b = base();
        compare(&b, &parse(tr, &b.skipped).unwrap())
    }

    #[test]
    fn reads_the_base_file() {
        let b = base();
        assert_eq!(b.strings.len(), 3, "{b:?}");
        assert!(!b.strings.contains_key("pct"));
        assert!(!b.strings.contains_key("commented"));
        assert_eq!(b.plurals["count"]["other"], set(&[(1, 'd')]));
    }

    #[test]
    fn good_translations_pass() {
        let tr = r#"<resources>
            <string name="hello">Hallo %1$s</string>
            <string name="two">%2$d von %1$s</string>
            <string name="pct">Darf kein %, * enthalten.</string>
            <plurals name="count">
                <item quantity="one">Eine Nachricht</item>
                <item quantity="few">%d zprávy</item>
                <item quantity="other">%d Nachrichten</item>
            </plurals>
        </resources>"#;
        assert_eq!(check(tr), Vec::<String>::new());
    }

    #[test]
    fn bad_translations_fail() {
        let cases = [
            r#"<string name="hello">Hallo</string>"#,
            r#"<string name="hello">Hallo %1$d</string>"#,
            r#"<string name="hello">Hallo %1$s %2$s</string>"#,
            r#"<string name="plain">Posteingang %s</string>"#,
            r#"<string name="gone">Weg</string>"#,
            r#"<plurals name="count"><item quantity="other">Nachrichten</item></plurals>"#,
            r#"<plurals name="count"><item quantity="one">%s</item><item quantity="other">%d</item></plurals>"#,
            r#"<plurals name="missing"><item quantity="other">%d</item></plurals>"#,
        ];
        for case in cases {
            let tr = format!("<resources>{case}</resources>");
            assert_eq!(check(&tr).len(), 1, "{case}");
        }
        let broken = r#"<resources><string name="hello">50% %1$s</string></resources>"#;
        assert!(parse(broken, &BTreeSet::new()).is_err());
    }

    #[test]
    fn checks_a_resource_folder() {
        let dir = std::env::temp_dir().join(format!("xtask-strings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("values")).unwrap();
        fs::create_dir_all(dir.join("values-de")).unwrap();
        fs::create_dir_all(dir.join("values-night")).unwrap();
        fs::write(dir.join("values/strings.xml"), BASE).unwrap();
        fs::write(
            dir.join("values-de/strings.xml"),
            r#"<resources><string name="hello">Hallo %1$s</string></resources>"#,
        )
        .unwrap();
        assert_eq!(check_dir(&dir), Ok(1));
        fs::write(
            dir.join("values-de/strings.xml"),
            r#"<resources><string name="hello">Hallo %1$d</string></resources>"#,
        )
        .unwrap();
        let err = check_dir(&dir).unwrap_err();
        assert!(err.contains("values-de/strings.xml: hello"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_real_strings_file_parses() {
        let res = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../android/app/src/main/res"
        );
        let base = parse(
            &fs::read_to_string(format!("{res}/values/strings.xml")).unwrap(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(base.strings.len() > 100);
        assert!(check_dir(Path::new(res)).is_ok());
    }
}
