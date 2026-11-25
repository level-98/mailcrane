//! ManageSieve responses (RFC 5804 §4).
//!
//! A response is zero or more data lines, then a status line:
//! `OK` / `NO` / `BYE`, an optional response code in parentheses and an
//! optional human-readable string. Lines are made of atoms, quoted strings
//! and literals (`{n}` CRLF, then n octets), separated by spaces.
//!
//! Everything here is hostile input: every read is bounds-checked, sizes
//! are limited by [`Limits`], and nothing panics.

/// Size limits for one response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest line, CRLF included, not counting literal contents.
    /// Default 64 KiB, as for IMAP.
    pub max_line: usize,
    /// Largest literal (a script). Default 1 MiB; servers usually allow
    /// far less for one script.
    pub max_literal: usize,
    /// Most lines in one response (`LISTSCRIPTS`). Default 10 000.
    pub max_lines: usize,
    /// Largest whole response. Default 4 MiB.
    pub max_response: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_line: 64 * 1024,
            max_literal: 1024 * 1024,
            max_lines: 10_000,
            max_response: 4 * 1024 * 1024,
        }
    }
}

/// Why bytes could not be parsed as a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// More bytes are needed to finish the response.
    #[error("incomplete response")]
    Incomplete,
    /// A line is longer than [`Limits::max_line`].
    #[error("response line too long")]
    LineTooLong,
    /// A literal, the line count or the whole response is over its limit.
    #[error("response too large")]
    TooLarge,
    /// Not valid ManageSieve.
    #[error("malformed response")]
    Invalid,
}

/// One item of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A bare word such as `OK`, `ACTIVE` or `QUOTA/MAXSIZE`.
    Atom(String),
    /// A quoted string or a literal, unescaped. Always valid UTF-8.
    Str(String),
    Open,
    Close,
}

impl Token {
    /// The text of an atom or string.
    pub fn text(&self) -> Option<&str> {
        match self {
            Token::Atom(s) | Token::Str(s) => Some(s),
            Token::Open | Token::Close => None,
        }
    }
}

/// How a command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    No,
    Bye,
}

/// A whole response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The lines before the status line.
    pub data: Vec<Vec<Token>>,
    pub status: Status,
    /// The response code's items, without the parentheses (empty if none),
    /// e.g. `[Atom("QUOTA/MAXSIZE")]`.
    pub code: Vec<Token>,
    /// The server's human-readable text. Never shown or logged as is.
    pub text: Option<String>,
}

impl Response {
    /// The response code's first word, upper case (`"QUOTA/MAXSIZE"`).
    pub fn code_name(&self) -> Option<String> {
        match self.code.first() {
            Some(Token::Atom(a)) => Some(a.to_ascii_uppercase()),
            _ => None,
        }
    }
}

/// Parses one response from the start of `buf` with the default limits.
/// Returns it and how many bytes it used.
pub fn parse_response(buf: &[u8]) -> Result<(Response, usize), ParseError> {
    parse_response_with(buf, &Limits::default())
}

/// [`parse_response`] with explicit limits.
pub fn parse_response_with(buf: &[u8], limits: &Limits) -> Result<(Response, usize), ParseError> {
    let mut at = 0usize;
    let mut data = Vec::new();
    loop {
        if at > limits.max_response {
            return Err(ParseError::TooLarge);
        }
        let (line, next) = parse_line(buf, at, limits)?;
        if next > limits.max_response {
            return Err(ParseError::TooLarge);
        }
        at = next;
        if let Some(Token::Atom(word)) = line.first() {
            let status = if word.eq_ignore_ascii_case("OK") {
                Some(Status::Ok)
            } else if word.eq_ignore_ascii_case("NO") {
                Some(Status::No)
            } else if word.eq_ignore_ascii_case("BYE") {
                Some(Status::Bye)
            } else {
                None
            };
            if let Some(status) = status {
                let (code, text) = status_tail(line.get(1..).unwrap_or_default())?;
                return Ok((
                    Response {
                        data,
                        status,
                        code,
                        text,
                    },
                    at,
                ));
            }
        }
        if data.len() >= limits.max_lines {
            return Err(ParseError::TooLarge);
        }
        data.push(line);
    }
}

/// After the status word: `[SP "(" code ")"] [SP string]`.
fn status_tail(rest: &[Token]) -> Result<(Vec<Token>, Option<String>), ParseError> {
    let mut rest = rest;
    let mut code = Vec::new();
    if rest.first() == Some(&Token::Open) {
        let close = rest
            .iter()
            .position(|t| *t == Token::Close)
            .ok_or(ParseError::Invalid)?;
        code = rest.get(1..close).unwrap_or_default().to_vec();
        if code.contains(&Token::Open) || code.is_empty() {
            return Err(ParseError::Invalid);
        }
        rest = rest.get(close.saturating_add(1)..).unwrap_or_default();
    }
    match rest {
        [] => Ok((code, None)),
        [Token::Str(s)] => Ok((code, Some(s.clone()))),
        _ => Err(ParseError::Invalid),
    }
}

fn is_atom_char(b: u8) -> bool {
    (0x21..=0x7e).contains(&b) && !matches!(b, b'"' | b'(' | b')' | b'{' | b'}' | b'\\')
}

/// Parses one line starting at `start`; returns its tokens and where the
/// next line starts.
fn parse_line(
    buf: &[u8],
    start: usize,
    limits: &Limits,
) -> Result<(Vec<Token>, usize), ParseError> {
    let mut tokens = Vec::new();
    let mut at = start;
    // Bytes of this line outside literals.
    let mut line_len = 0usize;
    let mut literal_total = 0usize;
    let incomplete = |line_len: usize| {
        if line_len > limits.max_line {
            ParseError::LineTooLong
        } else {
            ParseError::Incomplete
        }
    };
    // Whether the next token may start here without a space before it.
    let mut need_sep = false;
    loop {
        let Some(&b) = buf.get(at) else {
            return Err(incomplete(line_len));
        };
        match b {
            b'\r' => {
                match buf.get(at.saturating_add(1)) {
                    None => return Err(incomplete(line_len)),
                    Some(b'\n') => {}
                    Some(_) => return Err(ParseError::Invalid),
                }
                if tokens.is_empty() {
                    return Err(ParseError::Invalid);
                }
                line_len = line_len.saturating_add(2);
                if line_len > limits.max_line {
                    return Err(ParseError::LineTooLong);
                }
                return Ok((tokens, at.saturating_add(2)));
            }
            b' ' => {
                if tokens.is_empty() {
                    return Err(ParseError::Invalid);
                }
                at = at.saturating_add(1);
                line_len = line_len.saturating_add(1);
                need_sep = false;
                continue;
            }
            b')' => {
                tokens.push(Token::Close);
                at = at.saturating_add(1);
                line_len = line_len.saturating_add(1);
                need_sep = true;
            }
            _ if need_sep => return Err(ParseError::Invalid),
            b'(' => {
                tokens.push(Token::Open);
                at = at.saturating_add(1);
                line_len = line_len.saturating_add(1);
                // `(CODE` has no space after the parenthesis.
                need_sep = false;
                continue;
            }
            b'"' => {
                let (s, next) = quoted(buf, at, line_len, limits)?;
                line_len = line_len.saturating_add(next.saturating_sub(at));
                at = next;
                tokens.push(Token::Str(s));
                need_sep = true;
            }
            b'{' => {
                let (s, next, header_len) = literal(buf, at, line_len, limits)?;
                literal_total = literal_total.saturating_add(s.len());
                if literal_total > limits.max_response {
                    return Err(ParseError::TooLarge);
                }
                line_len = line_len.saturating_add(header_len);
                at = next;
                tokens.push(Token::Str(s));
                need_sep = true;
            }
            _ if is_atom_char(b) => {
                let begin = at;
                while buf.get(at).copied().is_some_and(is_atom_char) {
                    at = at.saturating_add(1);
                    if line_len.saturating_add(at.saturating_sub(begin)) > limits.max_line {
                        return Err(ParseError::LineTooLong);
                    }
                }
                let word = buf.get(begin..at).unwrap_or_default();
                line_len = line_len.saturating_add(word.len());
                // Atom characters are ASCII, so this never fails.
                let word = std::str::from_utf8(word).map_err(|_| ParseError::Invalid)?;
                tokens.push(Token::Atom(word.to_owned()));
                need_sep = true;
            }
            _ => return Err(ParseError::Invalid),
        }
        if line_len > limits.max_line {
            return Err(ParseError::LineTooLong);
        }
    }
}

/// A quoted string at `at` (the opening quote). RFC 5804 §4: inside, only
/// `\"` and `\\` are escapes, and CR, LF and NUL are not allowed.
fn quoted(
    buf: &[u8],
    at: usize,
    line_len: usize,
    limits: &Limits,
) -> Result<(String, usize), ParseError> {
    let mut out = Vec::new();
    let mut i = at.saturating_add(1);
    loop {
        if line_len.saturating_add(i.saturating_sub(at)) > limits.max_line {
            return Err(ParseError::LineTooLong);
        }
        let Some(&b) = buf.get(i) else {
            return Err(ParseError::Incomplete);
        };
        match b {
            b'"' => break,
            b'\\' => {
                let Some(&e) = buf.get(i.saturating_add(1)) else {
                    return Err(ParseError::Incomplete);
                };
                if e != b'"' && e != b'\\' {
                    return Err(ParseError::Invalid);
                }
                out.push(e);
                i = i.saturating_add(2);
            }
            b'\r' | b'\n' | 0 => return Err(ParseError::Invalid),
            _ => {
                out.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    let s = String::from_utf8(out).map_err(|_| ParseError::Invalid)?;
    Ok((s, i.saturating_add(1)))
}

/// A literal at `at` (the `{`): `{n}` or `{n+}`, CRLF, then n octets.
/// Returns the string, where it ends and the length of its `{n}` CRLF
/// header (which counts toward the line).
fn literal(
    buf: &[u8],
    at: usize,
    line_len: usize,
    limits: &Limits,
) -> Result<(String, usize, usize), ParseError> {
    let mut i = at.saturating_add(1);
    let mut n: usize = 0;
    let mut digits = 0usize;
    while let Some(&b) = buf.get(i) {
        if !b.is_ascii_digit() {
            break;
        }
        n = n
            .checked_mul(10)
            .and_then(|v| v.checked_add(usize::from(b - b'0')))
            .ok_or(ParseError::TooLarge)?;
        digits = digits.saturating_add(1);
        if digits > 10 || n > limits.max_literal {
            return Err(ParseError::TooLarge);
        }
        i = i.saturating_add(1);
    }
    let incomplete = |i: usize| {
        if line_len.saturating_add(i.saturating_sub(at)) > limits.max_line {
            ParseError::LineTooLong
        } else {
            ParseError::Incomplete
        }
    };
    if digits == 0 {
        return match buf.get(i) {
            None => Err(incomplete(i)),
            Some(_) => Err(ParseError::Invalid),
        };
    }
    if buf.get(i) == Some(&b'+') {
        i = i.saturating_add(1);
    }
    for want in *b"}\r\n" {
        match buf.get(i) {
            None => return Err(incomplete(i)),
            Some(&b) if b == want => i = i.saturating_add(1),
            Some(_) => return Err(ParseError::Invalid),
        }
    }
    let header_len = i.saturating_sub(at);
    let end = i.checked_add(n).ok_or(ParseError::TooLarge)?;
    let bytes = buf.get(i..end).ok_or(ParseError::Incomplete)?;
    let s = String::from_utf8(bytes.to_vec()).map_err(|_| ParseError::Invalid)?;
    Ok((s, end, header_len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn parse(s: &str) -> Result<(Response, usize), ParseError> {
        parse_response(s.as_bytes())
    }

    #[test]
    fn greeting_capabilities() {
        // RFC 5804 §1.7, the capability response.
        let text = "\"IMPlemENTATION\" \"Example1 ManageSieved v001\"\r\n\
                    \"SASL\" \"DIGEST-MD5 GSSAPI\"\r\n\
                    \"SIeVE\" \"fileinto vacation\"\r\n\
                    \"StartTLS\"\r\n\
                    \"NOTIFY\" \"xmpp mailto\"\r\n\
                    \"MAXREDIRECTS\" \"5\"\r\n\
                    \"VERSION\" \"1.0\"\r\n\
                    OK\r\n";
        let (r, used) = parse(text).expect("parses");
        assert_eq!(used, text.len());
        assert_eq!(r.status, Status::Ok);
        assert_eq!(r.data.len(), 7);
        assert_eq!(r.data[3], vec![Token::Str("StartTLS".into())]);
    }

    #[test]
    fn listscripts_and_literal() {
        let (r, _) = parse("\"summer_script\"\r\n\"vac\" ACTIVE\r\n{5}\r\nx\"y\\z\r\nOK\r\n")
            .expect("parses");
        assert_eq!(
            r.data[1],
            vec![Token::Str("vac".into()), Token::Atom("ACTIVE".into())]
        );
        assert_eq!(r.data[2], vec![Token::Str("x\"y\\z".into())]);
    }

    #[test]
    fn status_codes_and_text() {
        let (r, _) = parse("NO (QUOTA/MAXSIZE) \"Quota exceeded\"\r\n").expect("parses");
        assert_eq!(r.status, Status::No);
        assert_eq!(r.code_name().as_deref(), Some("QUOTA/MAXSIZE"));
        assert_eq!(r.text.as_deref(), Some("Quota exceeded"));
        let (r, _) =
            parse("OK (SASL \"cnNwYXV0aD1lYTQwZjYwMzM1YzQyN2I1NTI3Yjg0ZGJhYmNkZmZmZA==\")\r\n")
                .expect("parses");
        assert_eq!(r.code_name().as_deref(), Some("SASL"));
        assert_eq!(r.code.len(), 2);
        let (r, _) = parse("BYE \"Too many failures\"\r\n").expect("parses");
        assert_eq!(r.status, Status::Bye);
        let (r, _) = parse("ok {3}\r\nabc\r\n").expect("literal text");
        assert_eq!(r.text.as_deref(), Some("abc"));
    }

    #[test]
    fn incomplete_responses_ask_for_more() {
        for partial in [
            "",
            "\"SIEVE\"",
            "\"SIEVE\"\r\n",
            "{10}\r\nabc",
            "OK",
            "OK\r",
            "\"a\\",
        ] {
            assert_eq!(
                parse(partial).err(),
                Some(ParseError::Incomplete),
                "{partial:?}"
            );
        }
    }

    #[test]
    fn malformed_responses_are_refused() {
        for bad in [
            "\r\n",
            " OK\r\n",
            "OK\n",
            "\"a\nb\"\r\nOK\r\n",
            "\"a\\nb\"\r\nOK\r\n",
            "\"a\"\"b\"\r\nOK\r\n",
            "OK (\r\n",
            "OK ()\r\n",
            "OK (A (B))\r\n",
            "OK \"a\" \"b\"\r\n",
            "{x}\r\nOK\r\n",
            "{2}x\r\nOK\r\n",
            "\x01\r\nOK\r\n",
        ] {
            assert_eq!(parse(bad).err(), Some(ParseError::Invalid), "{bad:?}");
        }
        assert_eq!(
            parse_response(b"{2}\r\n\xff\xfe\r\nOK\r\n").err(),
            Some(ParseError::Invalid),
            "not UTF-8"
        );
    }

    #[test]
    fn limits_are_enforced() {
        let limits = Limits {
            max_line: 32,
            max_literal: 8,
            max_lines: 2,
            max_response: 64,
        };
        let long = format!("\"{}\"\r\nOK\r\n", "a".repeat(40));
        assert_eq!(
            parse_response_with(long.as_bytes(), &limits).err(),
            Some(ParseError::LineTooLong)
        );
        // A long line that hasn't ended yet is refused without waiting.
        let partial = "a".repeat(40);
        assert_eq!(
            parse_response_with(partial.as_bytes(), &limits).err(),
            Some(ParseError::LineTooLong)
        );
        assert_eq!(
            parse_response_with(b"{9}\r\n", &limits).err(),
            Some(ParseError::TooLarge)
        );
        assert_eq!(
            parse_response_with(b"{99999999999999999999}\r\n", &limits).err(),
            Some(ParseError::TooLarge)
        );
        assert_eq!(
            parse_response_with(b"\"a\"\r\n\"b\"\r\n\"c\"\r\nOK\r\n", &limits).err(),
            Some(ParseError::TooLarge)
        );
        // A literal fits even when it is longer than a line.
        let limits = Limits {
            max_line: 16,
            max_literal: 64,
            ..limits
        };
        let lit = format!("{{40}}\r\n{}\r\nOK\r\n", "b".repeat(40));
        assert!(parse_response_with(lit.as_bytes(), &limits).is_ok());
    }

    #[test]
    fn two_responses_in_a_row() {
        let buf = b"OK\r\nNO\r\n";
        let (r, used) = parse_response(buf).expect("first");
        assert_eq!((r.status, used), (Status::Ok, 4));
        let (r, _) = parse_response(&buf[used..]).expect("second");
        assert_eq!(r.status, Status::No);
    }

    proptest! {
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse_response(&bytes);
        }

        #[test]
        fn used_is_within_the_input(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            if let Ok((_, used)) = parse_response(&bytes) {
                prop_assert!(used > 0 && used <= bytes.len());
            }
        }

        #[test]
        fn token_like_input_never_panics(parts in proptest::collection::vec(
            prop_oneof![
                Just("OK".to_string()), Just("NO".to_string()), Just("BYE".to_string()),
                Just(" ".to_string()), Just("\r\n".to_string()), Just("(".to_string()),
                Just(")".to_string()), Just("\"".to_string()), Just("\\".to_string()),
                Just("{3}\r\n".to_string()), Just("{3+}\r\n".to_string()),
                "[a-zA-Z0-9/]{0,6}".prop_map(|s| s),
            ], 0..40)) {
            let s: String = parts.concat();
            let _ = parse_response(s.as_bytes());
        }

        #[test]
        fn quoted_strings_round_trip(s in "[^\r\n\u{0}]{0,64}") {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            let wire = format!("\"{escaped}\"\r\nOK\r\n");
            let (r, _) = parse_response(wire.as_bytes()).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(&r.data[0], &vec![Token::Str(s)]);
        }

        #[test]
        fn literals_round_trip(s in "\\PC{0,64}") {
            let wire = format!("{{{}}}\r\n{s}\r\nOK\r\n", s.len());
            let (r, _) = parse_response(wire.as_bytes()).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(&r.data[0], &vec![Token::Str(s)]);
        }
    }
}
