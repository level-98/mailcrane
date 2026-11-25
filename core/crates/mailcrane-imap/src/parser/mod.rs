//! IMAP response parser (RFC 9051 §7 and the formal syntax in §9).
//!
//! A small hand-written parser rather than `imap-codec`: we only need the
//! response shapes used for connecting, logging in and listing folders,
//! the fuzzing entry point needs a `(Response, usize)` signature over raw
//! bytes, and framing must enforce our own line and literal limits before
//! anything is buffered. Responses we do not understand yet are framed
//! correctly (literals included) and returned as [`Untagged::Other`].

use std::fmt;

mod fetch;
pub use fetch::{Envelope, EnvelopeAddress, FetchData};

/// Size limits applied while reading responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest response line, CRLF included. Default 64 KiB.
    pub max_line: usize,
    /// Largest literal, and the largest whole response is this plus
    /// `max_line`. Default 64 MiB.
    pub max_literal: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_line: 64 * 1024,
            max_literal: 64 * 1024 * 1024,
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
    /// A literal or the whole response is over the limit.
    #[error("response too large")]
    TooLarge,
    /// The bytes are not a valid response; `offset` is where it went wrong.
    #[error("invalid response at byte {offset}")]
    Invalid {
        /// Byte offset of the problem.
        offset: usize,
    },
}

/// `OK`, `NO` or `BAD` (RFC 9051 §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Ok,
    No,
    Bad,
}

/// A status condition on an untagged response, which adds `PREAUTH` and
/// `BYE` (RFC 9051 §7.1.4 and §7.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Ok,
    No,
    Bad,
    PreAuth,
    Bye,
}

/// A response code in square brackets (RFC 9051 §7.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Code {
    /// `[CAPABILITY ...]`.
    Capability(Vec<String>),
    /// Any other code, with its argument text if present.
    Other { name: String, args: Option<String> },
}

/// Response code and human-readable text. The text is never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct RespText {
    pub code: Option<Code>,
    pub text: String,
}

impl fmt::Debug for RespText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Server text can echo credentials; keep it out of Debug output.
        f.debug_struct("RespText")
            .field("code", &self.code)
            .field("text", &format_args!("<{} bytes>", self.text.len()))
            .finish()
    }
}

/// One `LIST` response (RFC 9051 §7.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// Attributes such as `\HasChildren` or `\Sent`, as sent.
    pub attributes: Vec<String>,
    /// Hierarchy delimiter, or `None` for `NIL`.
    pub delimiter: Option<char>,
    /// Mailbox name exactly as sent (modified UTF-7 or UTF-8).
    pub name: Vec<u8>,
}

/// Untagged response data (RFC 9051 §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Untagged {
    Condition {
        condition: Condition,
        text: RespText,
    },
    Capability(Vec<String>),
    Enabled(Vec<String>),
    List(ListItem),
    /// `* n FETCH (...)` (RFC 9051 §7.5.2).
    Fetch(Box<FetchData>),
    /// A `FETCH` response that could not be read. `uid` is set when it
    /// could still be found. The rest of the command's responses are fine.
    FetchMalformed {
        uid: Option<u32>,
    },
    /// `* n EXISTS` (RFC 9051 §7.4.1).
    Exists(u32),
    /// `* n EXPUNGE` (RFC 9051 §7.5.1): the message with sequence number
    /// `n` is gone. Never zero.
    Expunge(u32),
    /// `* SEARCH ...` (RFC 9051 §7.3.2): UIDs or sequence numbers.
    Search(Vec<u32>),
    /// `* VANISHED [(EARLIER)] <uids>` (RFC 7162 §3.2.10): UIDs that were
    /// expunged, as inclusive ranges `(low, high)` with `low <= high`. Kept
    /// as ranges so a hostile `1:4294967295` costs nothing.
    Vanished {
        /// `(EARLIER)`: an answer to QRESYNC rather than a new expunge.
        earlier: bool,
        uids: Vec<(u32, u32)>,
    },
    /// Something this parser does not interpret yet, named by its keyword.
    Other {
        name: String,
    },
}

/// One complete server response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// `+ ...` (RFC 9051 §7.5).
    Continuation { text: String },
    /// `* ...`.
    Untagged(Untagged),
    /// `<tag> OK|NO|BAD ...`, completing a command.
    Tagged {
        tag: String,
        status: StatusKind,
        text: RespText,
    },
}

/// Parses one response from the start of `input` using the default
/// [`Limits`]. Returns the response and the number of bytes it used.
///
/// This is the fuzzing entry point: it must never panic, whatever the bytes.
pub fn parse_response(input: &[u8]) -> Result<(Response, usize), ParseError> {
    parse_response_with(input, &Limits::default())
}

/// [`parse_response`] with explicit limits.
pub fn parse_response_with(input: &[u8], limits: &Limits) -> Result<(Response, usize), ParseError> {
    let end = frame_end(input, limits)?.ok_or(ParseError::Incomplete)?;
    let frame = input.get(..end).ok_or(ParseError::Incomplete)?;
    let mut c = Cursor { buf: frame, pos: 0 };
    let response = c.response()?;
    Ok((response, end))
}

/// Finds where the first complete response ends: after the CRLF of its
/// last line, following any `{n}` literals. `Ok(None)` means more bytes
/// are needed. Limits are checked on what has arrived so far, so a hostile
/// server cannot make us buffer more than the limits allow.
pub(crate) fn frame_end(buf: &[u8], limits: &Limits) -> Result<Option<usize>, ParseError> {
    let max_frame = limits.max_literal.saturating_add(limits.max_line);
    let mut pos = 0usize;
    loop {
        let rest = buf.get(pos..).unwrap_or_default();
        let Some(lf) = rest.iter().position(|&b| b == b'\n') else {
            if rest.len() >= limits.max_line {
                return Err(ParseError::LineTooLong);
            }
            if buf.len() > max_frame {
                return Err(ParseError::TooLarge);
            }
            return Ok(None);
        };
        if lf >= limits.max_line {
            return Err(ParseError::LineTooLong);
        }
        let line = rest.get(..lf).unwrap_or_default();
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let after_line = pos.saturating_add(lf).saturating_add(1);
        match literal_suffix(line) {
            Some(Some(n)) => {
                if n > limits.max_literal {
                    return Err(ParseError::TooLarge);
                }
                let end = after_line.checked_add(n).ok_or(ParseError::TooLarge)?;
                if end > max_frame {
                    return Err(ParseError::TooLarge);
                }
                if end > buf.len() {
                    return Ok(None);
                }
                pos = end;
            }
            // A literal size too big to even represent.
            Some(None) => return Err(ParseError::TooLarge),
            None => {
                if after_line > max_frame {
                    return Err(ParseError::TooLarge);
                }
                return Ok(Some(after_line));
            }
        }
    }
}

/// If `line` ends with `{n}` or `{n+}` (RFC 9051 §4.3 and RFC 7888),
/// returns `Some(Some(n))`, or `Some(None)` if `n` overflows.
fn literal_suffix(line: &[u8]) -> Option<Option<usize>> {
    let inner = line.strip_suffix(b"}")?;
    let open = inner.iter().rposition(|&b| b == b'{')?;
    let digits = inner.get(open + 1..)?;
    let digits = digits.strip_suffix(b"+").unwrap_or(digits);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut n: usize = 0;
    for &d in digits {
        let Some(next) = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(usize::from(d - b'0')))
        else {
            return Some(None);
        };
        n = next;
    }
    Some(Some(n))
}

/// `ATOM-CHAR`: any CHAR except atom-specials (RFC 9051 §9).
pub(crate) fn is_atom_char(b: u8) -> bool {
    (0x21..=0x7e).contains(&b) && !b"(){ %*\"\\]".contains(&b)
}

/// `ASTRING-CHAR`: `ATOM-CHAR` or `]`.
fn is_astring_char(b: u8) -> bool {
    is_atom_char(b) || b == b']'
}

/// A tag is `ASTRING-CHAR` except `+`.
fn is_tag_char(b: u8) -> bool {
    is_astring_char(b) && b != b'+'
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn err(&self) -> ParseError {
        ParseError::Invalid { offset: self.pos }
    }

    fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    fn bump(&mut self) {
        self.pos = self.pos.saturating_add(1);
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), ParseError> {
        if self.eat(b) {
            Ok(())
        } else {
            Err(self.err())
        }
    }

    fn take_while(&mut self, pred: impl Fn(u8) -> bool) -> &[u8] {
        let start = self.pos;
        while self.peek().is_some_and(&pred) {
            self.bump();
        }
        self.buf.get(start..self.pos).unwrap_or_default()
    }

    fn at_eol(&self) -> bool {
        matches!(self.peek(), Some(b'\r' | b'\n'))
    }

    fn crlf(&mut self) -> Result<(), ParseError> {
        // Be lenient about a bare LF; framing already found the line end.
        self.eat(b'\r');
        self.expect(b'\n')
    }

    fn atom(&mut self) -> Result<String, ParseError> {
        let bytes = self.take_while(is_atom_char);
        if bytes.is_empty() {
            return Err(self.err());
        }
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    /// Text up to the end of the line.
    fn text(&mut self) -> String {
        let bytes = self.take_while(|b| b != b'\r' && b != b'\n');
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn response(&mut self) -> Result<Response, ParseError> {
        let response = match self.peek() {
            Some(b'+') => {
                self.bump();
                self.eat(b' ');
                let text = self.text();
                self.crlf()?;
                Response::Continuation { text }
            }
            Some(b'*') => {
                self.bump();
                self.expect(b' ')?;
                Response::Untagged(self.untagged()?)
            }
            _ => {
                let tag = self.take_while(is_tag_char);
                if tag.is_empty() {
                    return Err(self.err());
                }
                let tag = String::from_utf8_lossy(tag).into_owned();
                self.expect(b' ')?;
                let status = match self.atom()?.to_ascii_uppercase().as_str() {
                    "OK" => StatusKind::Ok,
                    "NO" => StatusKind::No,
                    "BAD" => StatusKind::Bad,
                    _ => return Err(self.err()),
                };
                let text = self.resp_text()?;
                self.crlf()?;
                Response::Tagged { tag, status, text }
            }
        };
        if self.pos != self.buf.len() {
            return Err(self.err());
        }
        Ok(response)
    }

    fn untagged(&mut self) -> Result<Untagged, ParseError> {
        if self.peek().is_some_and(|b| b.is_ascii_digit()) {
            // "* 3 EXISTS", "* 1 FETCH (...)" and the like.
            let digits = self.take_while(|b| b.is_ascii_digit());
            let seq = String::from_utf8_lossy(digits).parse::<u32>().unwrap_or(0);
            self.expect(b' ')?;
            let name = self.atom()?.to_ascii_uppercase();
            if name == "FETCH" {
                return self.fetch_response(seq);
            }
            if name == "EXISTS" {
                self.skip_to_end();
                return Ok(Untagged::Exists(seq));
            }
            // RFC 9051 §7.5.1: the number is an nz-number; a zero (or one
            // too large to read) is kept as an unknown response.
            if name == "EXPUNGE" && seq > 0 {
                self.skip_to_end();
                return Ok(Untagged::Expunge(seq));
            }
            self.skip_to_end();
            return Ok(Untagged::Other { name });
        }
        let name = self.atom()?;
        let upper = name.to_ascii_uppercase();
        let condition = match upper.as_str() {
            "OK" => Some(Condition::Ok),
            "NO" => Some(Condition::No),
            "BAD" => Some(Condition::Bad),
            "PREAUTH" => Some(Condition::PreAuth),
            "BYE" => Some(Condition::Bye),
            _ => None,
        };
        if let Some(condition) = condition {
            let text = self.resp_text()?;
            self.crlf()?;
            return Ok(Untagged::Condition { condition, text });
        }
        match upper.as_str() {
            // RFC 9051 §7.2.2
            "CAPABILITY" => {
                let caps = self.atom_list()?;
                self.crlf()?;
                Ok(Untagged::Capability(caps))
            }
            // RFC 5161 §3.2
            "ENABLED" => {
                let caps = self.atom_list()?;
                self.crlf()?;
                Ok(Untagged::Enabled(caps))
            }
            // RFC 9051 §7.3.2
            "SEARCH" => self.search_response(),
            // RFC 7162 §3.2.10
            "VANISHED" => self.vanished_response(),
            // RFC 9051 §7.3.1
            "LIST" => {
                self.expect(b' ')?;
                let item = self.mailbox_list()?;
                Ok(Untagged::List(item))
            }
            _ => {
                self.skip_to_end();
                Ok(Untagged::Other { name: upper })
            }
        }
    }

    /// Zero or more space-separated atoms up to the end of the line.
    /// Tolerates a trailing space.
    fn atom_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut out = Vec::new();
        while self.eat(b' ') {
            if self.at_eol() {
                break;
            }
            out.push(self.atom()?);
        }
        if !self.at_eol() {
            return Err(self.err());
        }
        Ok(out)
    }

    /// The frame is already complete, so the rest of it (literals and all)
    /// belongs to this response.
    fn skip_to_end(&mut self) {
        self.pos = self.buf.len();
    }

    /// `resp-text = ["[" resp-text-code "]" SP] text` (RFC 9051 §9).
    /// Lenient: the SP and the text may both be missing.
    fn resp_text(&mut self) -> Result<RespText, ParseError> {
        if self.at_eol() {
            return Ok(RespText {
                code: None,
                text: String::new(),
            });
        }
        self.expect(b' ')?;
        let mut code = None;
        if self.eat(b'[') {
            let name = self.atom()?;
            if name.eq_ignore_ascii_case("CAPABILITY") {
                let mut caps = Vec::new();
                while self.eat(b' ') {
                    if self.peek() == Some(b']') {
                        break;
                    }
                    caps.push(self.atom()?);
                }
                code = Some(Code::Capability(caps));
            } else {
                let args = if self.eat(b' ') {
                    let bytes = self.take_while(|b| b != b']' && b != b'\r' && b != b'\n');
                    Some(String::from_utf8_lossy(bytes).into_owned())
                } else {
                    None
                };
                code = Some(Code::Other { name, args });
            }
            self.expect(b']')?;
            self.eat(b' ');
        }
        let text = self.text();
        Ok(RespText { code, text })
    }

    /// `mailbox-list` (RFC 9051 §9), then the line end. Extended data
    /// after the name (RFC 5258) is skipped.
    fn mailbox_list(&mut self) -> Result<ListItem, ParseError> {
        self.expect(b'(')?;
        let mut attributes = Vec::new();
        loop {
            if self.eat(b')') {
                break;
            }
            if !attributes.is_empty() {
                self.expect(b' ')?;
            }
            let backslash = self.eat(b'\\');
            let atom = self.atom()?;
            attributes.push(if backslash { format!("\\{atom}") } else { atom });
        }
        self.expect(b' ')?;
        let delimiter = if self.eat(b'"') {
            let c = match self.peek() {
                Some(b'\\') => {
                    self.bump();
                    match self.peek() {
                        Some(q @ (b'\\' | b'"')) => q,
                        _ => return Err(self.err()),
                    }
                }
                Some(b) if (0x20..=0x7e).contains(&b) && b != b'"' => b,
                _ => return Err(self.err()),
            };
            self.bump();
            self.expect(b'"')?;
            Some(char::from(c))
        } else {
            let nil = self.atom()?;
            if !nil.eq_ignore_ascii_case("NIL") {
                return Err(self.err());
            }
            None
        };
        self.expect(b' ')?;
        let name = self.astring()?;
        if self.eat(b' ') {
            // mbox-list-extended (RFC 5258 §6), not needed yet.
            self.skip_to_end();
            return Ok(ListItem {
                attributes,
                delimiter,
                name,
            });
        }
        self.crlf()?;
        Ok(ListItem {
            attributes,
            delimiter,
            name,
        })
    }

    /// `astring = 1*ASTRING-CHAR / string` (RFC 9051 §9).
    fn astring(&mut self) -> Result<Vec<u8>, ParseError> {
        match self.peek() {
            Some(b'"') => self.quoted(),
            Some(b'{') => self.literal(),
            _ => {
                let bytes = self.take_while(is_astring_char);
                if bytes.is_empty() {
                    return Err(self.err());
                }
                Ok(bytes.to_vec())
            }
        }
    }

    /// `quoted` with `\\` and `\"` escapes, no CR or LF.
    fn quoted(&mut self) -> Result<Vec<u8>, ParseError> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.bump();
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.bump();
                    match self.peek() {
                        Some(q @ (b'\\' | b'"')) => {
                            out.push(q);
                            self.bump();
                        }
                        _ => return Err(self.err()),
                    }
                }
                Some(b'\r' | b'\n') | None => return Err(self.err()),
                Some(b) => {
                    out.push(b);
                    self.bump();
                }
            }
        }
    }

    /// `literal = "{" number ["+"] "}" CRLF *CHAR8` (RFC 9051 §4.3).
    fn literal(&mut self) -> Result<Vec<u8>, ParseError> {
        self.expect(b'{')?;
        let digits = self.take_while(|b| b.is_ascii_digit());
        let mut n: usize = 0;
        for &d in digits {
            n = n
                .checked_mul(10)
                .and_then(|n| n.checked_add(usize::from(d - b'0')))
                .ok_or(ParseError::TooLarge)?;
        }
        if digits.is_empty() {
            return Err(self.err());
        }
        self.eat(b'+');
        self.expect(b'}')?;
        self.crlf()?;
        let end = self.pos.checked_add(n).ok_or(ParseError::TooLarge)?;
        let bytes = self.buf.get(self.pos..end).ok_or(ParseError::Incomplete)?;
        self.pos = end;
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests;
