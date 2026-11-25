//! What links a message to others: its ids and its subject key.

use mailcrane_store::UnthreadedMessage;

/// Most References kept per message (the last ones: the nearest ancestors).
pub const MAX_REFERENCES: usize = 100;

/// How far apart a reply without references and its original may be.
pub const SUBJECT_WINDOW_SECS: i64 = 30 * 86_400;

/// Can't join by subject.
pub(super) const KIND_NONE: u8 = 0;
/// An original: a subject without a reply or forward prefix.
pub(super) const KIND_ORIGINAL: u8 = 1;
/// A reply or forward prefix, and no In-Reply-To or References.
pub(super) const KIND_REPLY: u8 = 2;

/// Reply and forward prefixes in common languages (lower-case).
const PREFIXES: &[&str] = &[
    "re", "fwd", "fw", "aw", "wg", "sv", "vs", "antw", "tr", "rif", "r", "res", "enc", "odp", "pd",
    "ynt", "atb",
];

/// The links of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadLinks {
    /// Its own Message-ID, In-Reply-To and References (valid ones, without
    /// repeats).
    pub keys: Vec<String>,
    /// The subject without prefixes, lower-case, spaces collapsed.
    pub subject_key: String,
    /// 0 none, 1 original, 2 reply without references.
    pub subject_kind: u8,
}

/// The links of `m`.
pub fn thread_links(m: &UnthreadedMessage) -> ThreadLinks {
    let mut keys: Vec<String> = Vec::new();
    let mut push = |id: &str| {
        let id = clean_id(id);
        if is_valid_id(id) && !keys.iter().any(|k| k == id) {
            keys.push(id.to_owned());
        }
    };
    if let Some(id) = &m.message_id_header {
        push(id);
    }
    let in_reply_to = m
        .in_reply_to
        .as_deref()
        .filter(|v| is_valid_id(clean_id(v)));
    if let Some(id) = in_reply_to {
        push(id);
    }
    let refs = json_strings(&m.references_json);
    let valid_refs: Vec<&String> = refs.iter().filter(|r| is_valid_id(clean_id(r))).collect();
    let start = valid_refs.len().saturating_sub(MAX_REFERENCES);
    for r in valid_refs.get(start..).unwrap_or_default() {
        push(r);
    }
    let (subject_key, prefixed) = normalize_subject(&m.subject);
    let has_refs = in_reply_to.is_some() || !valid_refs.is_empty();
    let subject_kind = if subject_key.is_empty() {
        KIND_NONE
    } else if !prefixed {
        KIND_ORIGINAL
    } else if !has_refs {
        KIND_REPLY
    } else {
        KIND_NONE
    };
    ThreadLinks {
        keys,
        subject_key,
        subject_kind,
    }
}

fn clean_id(id: &str) -> &str {
    id.trim().trim_start_matches('<').trim_end_matches('>')
}

/// A plausible Message-ID: `local@domain`, no spaces or brackets, not too long.
fn is_valid_id(id: &str) -> bool {
    (3..=250).contains(&id.len())
        && id.contains('@')
        && !id.starts_with('@')
        && !id.ends_with('@')
        && !id
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>')
}

/// The subject with any number of `Re:`, `Fwd:`, `Re[2]:`, `AW:` … prefixes
/// removed, lower-case, whitespace collapsed; and whether a prefix was
/// removed.
pub fn normalize_subject(subject: &str) -> (String, bool) {
    let mut rest = subject.trim();
    let mut prefixed = false;
    while let Some((head, tail)) = rest.split_once(':') {
        let head = head.trim();
        // `Re[2]` and `Re(2)` count too.
        let (word, counter) = match head.find(['[', '(']) {
            Some(i) => head.split_at(i),
            None => (head, ""),
        };
        let word = word.trim().to_lowercase();
        let counter_ok = counter
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '[' | ']' | '(' | ')' | ' '));
        if PREFIXES.contains(&word.as_str()) && counter_ok {
            rest = tail.trim_start();
            prefixed = true;
        } else {
            break;
        }
    }
    let key = rest
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    (key, prefixed)
}

/// The strings in a JSON array of strings (as the store writes them).
/// Anything else gives what was read so far.
pub(super) fn json_strings(json: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = json.trim().chars().peekable();
    if chars.next() != Some('[') {
        return out;
    }
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        if chars.next() != Some('"') {
            return out;
        }
        let mut s = String::new();
        loop {
            match chars.next() {
                Some('"') => break,
                Some('\\') => match chars.next() {
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            s.push(c);
                        }
                    }
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some(c) => s.push(c),
                    None => return out,
                },
                Some(c) => s.push(c),
                None => return out,
            }
        }
        out.push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects() {
        assert_eq!(normalize_subject("Hello"), ("hello".into(), false));
        assert_eq!(normalize_subject("Re: Hello"), ("hello".into(), true));
        assert_eq!(
            normalize_subject("RE: Fwd:  re[2]: AW: Hello   world"),
            ("hello world".into(), true)
        );
        assert_eq!(normalize_subject("Re(3): Hi"), ("hi".into(), true));
        assert_eq!(
            normalize_subject("Meeting: notes"),
            ("meeting: notes".into(), false)
        );
        assert_eq!(normalize_subject("Re:"), (String::new(), true));
        assert_eq!(normalize_subject(""), (String::new(), false));
    }

    #[test]
    fn json_arrays() {
        assert_eq!(
            json_strings(r#"["a@x", "b\"c@y", "A@z"]"#),
            ["a@x", "b\"c@y", "A@z"]
        );
        assert!(json_strings("garbage").is_empty());
        assert_eq!(json_strings(r#"["a@x", 5]"#), ["a@x"]);
        assert!(json_strings("[]").is_empty());
    }

    fn msg(id: Option<&str>, irt: Option<&str>, refs: &str, subject: &str) -> UnthreadedMessage {
        UnthreadedMessage {
            id: mailcrane_store::MessageId(1),
            account_id: mailcrane_store::AccountId(1),
            message_id_header: id.map(Into::into),
            in_reply_to: irt.map(Into::into),
            references_json: refs.into(),
            subject: subject.into(),
            date: 0,
        }
    }

    #[test]
    fn links_are_cleaned_and_capped() {
        let l = thread_links(&msg(
            Some("a@x"),
            Some("<b@x>"),
            r#"["c@x", "bad id", "a@x", "@x", "d@x"]"#,
            "Re: Hi",
        ));
        assert_eq!(l.keys, ["a@x", "b@x", "c@x", "d@x"]);
        assert_eq!(l.subject_kind, KIND_NONE, "a reply with references");
        let many: Vec<String> = (0..500).map(|i| format!("\"r{i}@x\"")).collect();
        let l = thread_links(&msg(
            Some("m@x"),
            None,
            &format!("[{}]", many.join(",")),
            "x",
        ));
        assert_eq!(l.keys.len(), 1 + MAX_REFERENCES);
        assert_eq!(l.keys.last().map(String::as_str), Some("r499@x"));
        assert_eq!(l.keys.get(1).map(String::as_str), Some("r400@x"));
        let l = thread_links(&msg(None, None, "[]", "Re: Hello"));
        assert_eq!(l.subject_kind, KIND_REPLY);
        let l = thread_links(&msg(None, None, "[]", "Hello"));
        assert_eq!(l.subject_kind, KIND_ORIGINAL);
    }
}
