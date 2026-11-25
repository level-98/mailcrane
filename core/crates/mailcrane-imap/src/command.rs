//! Building commands: tags, quoting and literals (RFC 9051 §2.2.1, §4.3).

use crate::error::ImapError;
use crate::parser::is_atom_char;

/// Hands out unique command tags: `A0001`, `A0002`, ...
#[derive(Debug, Default)]
pub(crate) struct Tags {
    next: u32,
}

impl Tags {
    pub(crate) fn next(&mut self) -> String {
        self.next = self.next.wrapping_add(1);
        format!("A{:04}", self.next)
    }
}

/// One command argument.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Arg<'a> {
    /// Protocol syntax written exactly as given, e.g. `RETURN (SPECIAL-USE)`.
    /// Only for fixed strings in our own code, never for user data.
    Raw(&'static str),
    /// Protocol syntax our own code built from numbers and fixed keywords,
    /// e.g. `(CHANGEDSINCE 42)` or `4097:*`. Checked before sending: only
    /// letters, digits, space, `:`, `,`, `*`, `(` and `)` are allowed.
    Built(&'a str),
    /// A flag keyword, sent as `(<keyword>)`. Checked before sending to be
    /// a keyword atom (RFC 9051 §9).
    KeywordList(&'a str),
    /// A string from data: sent as an atom, a quoted string or a literal,
    /// whichever fits.
    Str(&'a [u8]),
    /// Data always sent as a literal, such as a whole message for `APPEND`
    /// (RFC 9051 §4.3). May not contain NUL.
    Literal(&'a [u8]),
}

/// A command ready to send. Every segment but the last ends with a
/// synchronizing literal header `{n}\r\n`; the client must wait for a `+`
/// continuation before sending the next segment (RFC 9051 §4.3).
#[derive(Debug)]
pub(crate) struct Command {
    pub(crate) tag: String,
    pub(crate) segments: Vec<Vec<u8>>,
}

/// Builds `<tag> <name> <args...>\r\n`.
pub(crate) fn build(tag: String, name: &str, args: &[Arg<'_>]) -> Result<Command, ImapError> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    current.extend_from_slice(tag.as_bytes());
    current.push(b' ');
    current.extend_from_slice(name.as_bytes());
    for arg in args {
        current.push(b' ');
        match arg {
            Arg::Raw(text) => current.extend_from_slice(text.as_bytes()),
            Arg::Built(text) => {
                let safe = !text.is_empty()
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b" :,*()".contains(&b));
                if !safe {
                    return Err(ImapError::InvalidInput);
                }
                current.extend_from_slice(text.as_bytes());
            }
            Arg::KeywordList(keyword) => {
                if !crate::store_cmds::is_keyword_atom(keyword) {
                    return Err(ImapError::InvalidInput);
                }
                current.push(b'(');
                current.extend_from_slice(keyword.as_bytes());
                current.push(b')');
            }
            Arg::Str(bytes) => {
                if let Some(literal) = push_string(bytes, &mut current)? {
                    current.extend_from_slice(format!("{{{}}}\r\n", literal.len()).as_bytes());
                    segments.push(std::mem::take(&mut current));
                    current.extend_from_slice(literal);
                }
            }
            Arg::Literal(bytes) => {
                if bytes.contains(&0) {
                    return Err(ImapError::InvalidInput);
                }
                current.extend_from_slice(format!("{{{}}}\r\n", bytes.len()).as_bytes());
                segments.push(std::mem::take(&mut current));
                current.extend_from_slice(bytes);
            }
        }
    }
    current.extend_from_slice(b"\r\n");
    segments.push(current);
    Ok(Command { tag, segments })
}

/// Writes `bytes` as an atom or quoted string, or returns them back when
/// they need a literal.
fn push_string<'a>(bytes: &'a [u8], out: &mut Vec<u8>) -> Result<Option<&'a [u8]>, ImapError> {
    // CHAR8 excludes NUL, so not even a literal can carry it.
    if bytes.contains(&0) {
        return Err(ImapError::InvalidInput);
    }
    if !bytes.is_empty() && bytes.iter().all(|&b| is_atom_char(b) || b == b']') {
        out.extend_from_slice(bytes);
        return Ok(None);
    }
    if bytes.iter().all(|&b| (0x20..=0x7e).contains(&b)) {
        out.push(b'"');
        for &b in bytes {
            if b == b'"' || b == b'\\' {
                out.push(b'\\');
            }
            out.push(b);
        }
        out.push(b'"');
        return Ok(None);
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(args: &[Arg<'_>]) -> Vec<Vec<u8>> {
        build("A0001".into(), "LOGIN", args)
            .map(|c| c.segments)
            .unwrap_or_default()
    }

    #[test]
    fn literal_args_are_always_literals() {
        let segments = build(
            "A1".into(),
            "APPEND",
            &[Arg::Str(b"Drafts"), Arg::Literal(b"ab")],
        )
        .map(|c| c.segments)
        .unwrap_or_default();
        assert_eq!(
            segments,
            [b"A1 APPEND Drafts {2}\r\n".to_vec(), b"ab\r\n".to_vec()]
        );
        assert!(build("A1".into(), "APPEND", &[Arg::Literal(b"a\0b")]).is_err());
    }

    #[test]
    fn tags_are_unique() {
        let mut tags = Tags::default();
        assert_eq!(tags.next(), "A0001");
        assert_eq!(tags.next(), "A0002");
    }

    #[test]
    fn atoms_quoted_and_literals() {
        assert_eq!(
            one(&[Arg::Str(b"alice@example.org"), Arg::Str(b"pw")]),
            [b"A0001 LOGIN alice@example.org pw\r\n".to_vec()]
        );
        assert_eq!(
            one(&[Arg::Str(b""), Arg::Str(b"a \"b\" \\c*")]),
            [b"A0001 LOGIN \"\" \"a \\\"b\\\" \\\\c*\"\r\n".to_vec()]
        );
        assert_eq!(
            one(&[Arg::Str("p\u{e4}ss".as_bytes()), Arg::Raw("X")]),
            [
                b"A0001 LOGIN {5}\r\n".to_vec(),
                b"p\xc3\xa4ss X\r\n".to_vec()
            ]
        );
        // CR and LF can only travel in a literal.
        assert_eq!(
            one(&[Arg::Str(b"a\r\nb")]),
            [b"A0001 LOGIN {4}\r\n".to_vec(), b"a\r\nb\r\n".to_vec()]
        );
    }

    #[test]
    fn built_syntax_is_checked() {
        assert_eq!(
            one(&[Arg::Built("(CHANGEDSINCE 42)")]),
            [b"A0001 LOGIN (CHANGEDSINCE 42)\r\n".to_vec()]
        );
        assert_eq!(
            one(&[Arg::Built("4097:*")]),
            [b"A0001 LOGIN 4097:*\r\n".to_vec()]
        );
        for bad in ["", "1\r\nA2 LOGOUT", "\"x\"", "{5}"] {
            assert!(matches!(
                build("A1".into(), "X", &[Arg::Built(bad)]),
                Err(ImapError::InvalidInput)
            ));
        }
    }

    #[test]
    fn nul_is_rejected() {
        assert!(matches!(
            build("A1".into(), "LOGIN", &[Arg::Str(b"a\0b")]),
            Err(ImapError::InvalidInput)
        ));
    }
}
