//! Writing Mailcrane's rules as one Sieve script (RFC 5228).
//!
//! The rules keep their local meaning (M4-09):
//!
//! - they run in order; a rule's text tests are "contains", ignoring case;
//! - flag actions (mark read, flag) add up;
//! - of the moving actions (move, archive, delete) the first matching rule's
//!   wins;
//! - after a matching rule with "stop processing", later rules don't run.
//!
//! So the script has two parts. First every rule's flags, each guarded by
//! "and no earlier stop-rule matched". Then one `if … elsif …` chain of the
//! moving actions under the same guards, so at most one runs. Flags set
//! with `addflag` (RFC 5232 §5) apply to the message wherever it ends up,
//! including the implicit keep.
//!
//! Every string goes through [`quote`]: `"` and `\` are escaped and CR, LF
//! and NUL are dropped, so nothing in a rule can end a string or add a
//! command.

use std::fmt::Write as _;

/// The extensions a script may use, from the server's `SIEVE` capability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Extensions {
    /// `fileinto` (RFC 5228 §4.1).
    pub fileinto: bool,
    /// `imap4flags` (RFC 5232).
    pub imap4flags: bool,
    /// `comparator-i;unicode-casemap` (RFC 5051): case-insensitive beyond
    /// ASCII. Without it, matching ignores ASCII case only.
    pub unicode_casemap: bool,
}

impl Extensions {
    /// From the server's list of extensions (any case).
    pub fn from_list<S: AsRef<str>>(list: &[S]) -> Self {
        let has = |name: &str| list.iter().any(|e| e.as_ref().eq_ignore_ascii_case(name));
        Self {
            fileinto: has("fileinto"),
            imap4flags: has("imap4flags"),
            unicode_casemap: has("comparator-i;unicode-casemap"),
        }
    }
}

/// Which header a test looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The From header (name and address).
    From,
    /// The To and Cc headers.
    ToCc,
    Subject,
    /// The List-Id header (RFC 2919); the text is the id without `<>`.
    ListId,
}

/// One condition: the header contains the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Test {
    pub field: Field,
    pub text: String,
}

/// A flag to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Seen,
    Flagged,
}

/// Where the message goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moving {
    /// Into this mailbox (its full name, UTF-8).
    FileInto(String),
    /// Deleted for good (local "delete" when there is no Trash).
    Discard,
}

/// One rule, ready for the script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRule {
    /// All tests must match (`false`: any one).
    pub match_all: bool,
    pub tests: Vec<Test>,
    pub flags: Vec<Flag>,
    pub moving: Option<Moving>,
    pub stop: bool,
}

impl ScriptRule {
    /// Whether the server's extensions can express this rule.
    pub fn supported(&self, ext: &Extensions) -> bool {
        let flags_ok = self.flags.is_empty() || ext.imap4flags;
        let moving_ok = match &self.moving {
            None | Some(Moving::Discard) => true,
            Some(Moving::FileInto(_)) => ext.fileinto,
        };
        flags_ok && moving_ok
    }

    /// Whether the rule can ever do anything (local rules without tests or
    /// actions never match).
    fn does_something(&self) -> bool {
        !self.tests.is_empty() && (!self.flags.is_empty() || self.moving.is_some() || self.stop)
    }
}

/// Why no script was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    /// A rule needs an extension the server doesn't have.
    #[error("unsupported rule")]
    Unsupported,
    /// The script would be larger than [`MAX_SCRIPT`].
    #[error("script too large")]
    TooLarge,
}

/// Largest script we write. Servers commonly allow 1 MiB or less.
pub const MAX_SCRIPT: usize = 256 * 1024;

/// The first line of every script Mailcrane writes.
pub const HEADER: &str =
    "# Written by Mailcrane from its rules. Changes made here are replaced.\r\n";

/// A Sieve quoted string (RFC 5228 §2.4.2): `"` and `\` escaped; CR, LF and
/// NUL dropped (header values are unfolded, and a folder name never has
/// them).
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\r' | '\n' | '\0' => {}
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn test_text(t: &Test, ext: &Extensions) -> String {
    let text = t.text.trim();
    if text.is_empty() {
        // Locally an empty text never matches.
        return "false".into();
    }
    let headers = match t.field {
        Field::From => "\"from\"",
        Field::ToCc => "[\"to\", \"cc\"]",
        Field::Subject => "\"subject\"",
        Field::ListId => "\"list-id\"",
    };
    let value = match t.field {
        Field::ListId => {
            let id = text.trim_start_matches('<').trim_end_matches('>');
            quote(&format!("<{id}>"))
        }
        _ => quote(text),
    };
    let comparator = if ext.unicode_casemap {
        " :comparator \"i;unicode-casemap\""
    } else {
        ""
    };
    // RFC 5228 §5.7; `:contains` has no wildcards, so the text is literal.
    format!("header :contains{comparator} {headers} {value}")
}

fn condition(rule: &ScriptRule, ext: &Extensions) -> String {
    let tests: Vec<String> = rule.tests.iter().map(|t| test_text(t, ext)).collect();
    match tests.as_slice() {
        [one] => one.clone(),
        _ if rule.match_all => format!("allof({})", tests.join(", ")),
        _ => format!("anyof({})", tests.join(", ")),
    }
}

/// The script for `rules`, in order. Fails if any rule is unsupported (the
/// caller keeps those on the device) or the script is too large.
pub fn compile(rules: &[ScriptRule], ext: &Extensions) -> Result<String, ScriptError> {
    if rules.iter().any(|r| !r.supported(ext)) {
        return Err(ScriptError::Unsupported);
    }
    let rules: Vec<&ScriptRule> = rules.iter().filter(|r| r.does_something()).collect();
    let conds: Vec<String> = rules.iter().map(|r| condition(r, ext)).collect();
    // guard[i]: rule i matches and no earlier stop-rule did.
    let mut guards = Vec::with_capacity(rules.len());
    let mut stops: Vec<&str> = Vec::new();
    for (rule, cond) in rules.iter().zip(&conds) {
        let guard = match stops.as_slice() {
            [] => cond.clone(),
            [one] => format!("allof({cond}, not {one})"),
            many => format!("allof({cond}, not anyof({}))", many.join(", ")),
        };
        guards.push(guard);
        if rule.stop {
            stops.push(cond);
        }
        if stops.iter().map(|s| s.len()).sum::<usize>() > MAX_SCRIPT {
            return Err(ScriptError::TooLarge);
        }
    }

    let uses_flags = rules.iter().any(|r| !r.flags.is_empty());
    let uses_fileinto = rules
        .iter()
        .any(|r| matches!(r.moving, Some(Moving::FileInto(_))));
    let mut require = Vec::new();
    if uses_fileinto {
        require.push("\"fileinto\"");
    }
    if uses_flags {
        require.push("\"imap4flags\"");
    }
    if ext.unicode_casemap
        && rules
            .iter()
            .any(|r| r.tests.iter().any(|t| !t.text.trim().is_empty()))
    {
        require.push("\"comparator-i;unicode-casemap\"");
    }

    let mut out = String::from(HEADER);
    if !require.is_empty() {
        let _ = write!(out, "require [{}];\r\n", require.join(", "));
    }
    for (rule, guard) in rules.iter().zip(&guards) {
        if rule.flags.is_empty() {
            continue;
        }
        let mut flags: Vec<&str> = Vec::new();
        for f in &rule.flags {
            let name = match f {
                Flag::Seen => "\"\\\\Seen\"",
                Flag::Flagged => "\"\\\\Flagged\"",
            };
            if !flags.contains(&name) {
                flags.push(name);
            }
        }
        let _ = write!(
            out,
            "if {guard} {{\r\n  addflag [{}];\r\n}}\r\n",
            flags.join(", ")
        );
        if out.len() > MAX_SCRIPT {
            return Err(ScriptError::TooLarge);
        }
    }
    let mut first = true;
    for (rule, guard) in rules.iter().zip(&guards) {
        let Some(moving) = &rule.moving else {
            continue;
        };
        let action = match moving {
            Moving::FileInto(mailbox) => format!("fileinto {};", quote(mailbox)),
            Moving::Discard => "discard;".into(),
        };
        let keyword = if first { "if" } else { "elsif" };
        let _ = write!(out, "{keyword} {guard} {{\r\n  {action}\r\n}}\r\n");
        first = false;
        if out.len() > MAX_SCRIPT {
            return Err(ScriptError::TooLarge);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ALL: Extensions = Extensions {
        fileinto: true,
        imap4flags: true,
        unicode_casemap: false,
    };

    fn rule(tests: Vec<Test>, flags: Vec<Flag>, moving: Option<Moving>, stop: bool) -> ScriptRule {
        ScriptRule {
            match_all: true,
            tests,
            flags,
            moving,
            stop,
        }
    }

    fn subject(t: &str) -> Test {
        Test {
            field: Field::Subject,
            text: t.into(),
        }
    }

    #[test]
    fn extensions_come_from_the_capability() {
        let ext = Extensions::from_list(&["FileInto", "vacation", "comparator-i;unicode-casemap"]);
        assert!(ext.fileinto && !ext.imap4flags && ext.unicode_casemap);
    }

    #[test]
    fn one_rule_of_each_kind() {
        let rules = [
            rule(
                vec![Test {
                    field: Field::From,
                    text: "boss@example.org".into(),
                }],
                vec![Flag::Flagged],
                None,
                false,
            ),
            ScriptRule {
                match_all: false,
                tests: vec![
                    subject("Invoice"),
                    Test {
                        field: Field::ToCc,
                        text: "billing".into(),
                    },
                ],
                flags: vec![Flag::Seen, Flag::Seen],
                moving: Some(Moving::FileInto("Bills".into())),
                stop: false,
            },
            rule(
                vec![Test {
                    field: Field::ListId,
                    text: "<news.example.net>".into(),
                }],
                vec![],
                Some(Moving::Discard),
                false,
            ),
        ];
        let script = compile(&rules, &ALL).expect("compiles");
        let want = format!(
            "{HEADER}require [\"fileinto\", \"imap4flags\"];\r\n\
             if header :contains \"from\" \"boss@example.org\" {{\r\n  addflag [\"\\\\Flagged\"];\r\n}}\r\n\
             if anyof(header :contains \"subject\" \"Invoice\", header :contains [\"to\", \"cc\"] \"billing\") {{\r\n  addflag [\"\\\\Seen\"];\r\n}}\r\n\
             if anyof(header :contains \"subject\" \"Invoice\", header :contains [\"to\", \"cc\"] \"billing\") {{\r\n  fileinto \"Bills\";\r\n}}\r\n\
             elsif header :contains \"list-id\" \"<news.example.net>\" {{\r\n  discard;\r\n}}\r\n"
        );
        assert_eq!(script, want);
    }

    #[test]
    fn stop_processing_guards_later_rules() {
        let rules = [
            rule(vec![subject("a")], vec![Flag::Seen], None, true),
            rule(
                vec![subject("b")],
                vec![],
                Some(Moving::FileInto("B".into())),
                true,
            ),
            rule(
                vec![subject("c")],
                vec![],
                Some(Moving::FileInto("C".into())),
                false,
            ),
        ];
        let script = compile(&rules, &ALL).expect("compiles");
        assert!(script.contains(
            "elsif allof(header :contains \"subject\" \"c\", not anyof(header :contains \"subject\" \"a\", header :contains \"subject\" \"b\")) {"
        ));
        assert!(script.contains(
            "if allof(header :contains \"subject\" \"b\", not header :contains \"subject\" \"a\") {"
        ));
    }

    #[test]
    fn quotes_backslashes_and_non_ascii_are_escaped() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(quote("Ünïcödé/Ordner 📁"), "\"Ünïcödé/Ordner 📁\"");
        assert_eq!(quote("x\"; discard;\r\n#"), "\"x\\\"; discard;#\"");
        let rules = [rule(
            vec![subject(r#"say "hi" \o/"#)],
            vec![],
            Some(Moving::FileInto("Été/\"Quoted\" \\ box".into())),
            false,
        )];
        let script = compile(&rules, &ALL).expect("compiles");
        assert!(script.contains(r#"header :contains "subject" "say \"hi\" \\o/""#));
        assert!(script.contains(r#"fileinto "Été/\"Quoted\" \\ box";"#));
    }

    #[test]
    fn unicode_casemap_is_used_when_offered() {
        let ext = Extensions {
            unicode_casemap: true,
            ..ALL
        };
        let script = compile(
            &[rule(vec![subject("ä")], vec![Flag::Seen], None, false)],
            &ext,
        )
        .expect("compiles");
        assert!(script.contains("\"comparator-i;unicode-casemap\"]"));
        assert!(
            script.contains("header :contains :comparator \"i;unicode-casemap\" \"subject\" \"ä\"")
        );
    }

    #[test]
    fn missing_extensions_are_reported() {
        let none = Extensions::default();
        let move_rule = rule(
            vec![subject("x")],
            vec![],
            Some(Moving::FileInto("X".into())),
            false,
        );
        let flag_rule = rule(vec![subject("x")], vec![Flag::Seen], None, false);
        let discard_rule = rule(vec![subject("x")], vec![], Some(Moving::Discard), false);
        assert!(!move_rule.supported(&none) && !flag_rule.supported(&none));
        assert!(discard_rule.supported(&none));
        assert_eq!(compile(&[move_rule], &none), Err(ScriptError::Unsupported));
        let script = compile(&[discard_rule], &none).expect("core Sieve only");
        assert!(!script.contains("require"));
    }

    #[test]
    fn empty_rules_and_texts_never_match() {
        let script = compile(&[rule(vec![], vec![Flag::Seen], None, false)], &ALL).expect("ok");
        assert_eq!(script, HEADER);
        let script = compile(
            &[rule(vec![subject("  ")], vec![Flag::Seen], None, false)],
            &ALL,
        )
        .expect("ok");
        assert!(script.contains("if false {"));
    }

    #[test]
    fn huge_scripts_are_refused() {
        let big = "x".repeat(MAX_SCRIPT);
        let rules = [rule(vec![subject(&big)], vec![Flag::Seen], None, false)];
        assert_eq!(compile(&rules, &ALL), Err(ScriptError::TooLarge));
    }

    proptest! {
        /// Whatever the text, the quoted string ends exactly at its last
        /// character: no unescaped quote inside, no line break.
        #[test]
        fn quoted_strings_stay_one_string(s in "\\PC{0,80}|[\"\\\\\r\n ;{}]{0,20}") {
            let q = quote(&s);
            prop_assert!(q.starts_with('"') && q.ends_with('"') && q.len() >= 2);
            let inner = &q[1..q.len() - 1];
            prop_assert!(!inner.contains('\r') && !inner.contains('\n'));
            let mut escaped = false;
            for c in inner.chars() {
                if escaped { escaped = false; continue; }
                if c == '\\' { escaped = true; continue; }
                prop_assert!(c != '"');
            }
            prop_assert!(!escaped);
        }
    }
}
