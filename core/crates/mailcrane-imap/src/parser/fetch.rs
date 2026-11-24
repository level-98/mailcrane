//! `FETCH` and `SEARCH` responses (RFC 9051 §7.5.2 and §7.3.2).
//!
//! `ENVELOPE` and `BODYSTRUCTURE` are read into a generic nested list first,
//! with limits on depth and size, then checked for the shape the RFC gives.
//! A `FETCH` response that does not fit is reported as
//! [`Untagged::FetchMalformed`] so one bad message does not spoil the rest
//! of a batch.

use super::{Cursor, ParseError, Untagged};

/// Deepest nesting accepted in `ENVELOPE` and `BODYSTRUCTURE`.
const MAX_DEPTH: usize = 32;
/// Most list items and strings accepted in one `FETCH` response.
const MAX_NODES: usize = 20_000;

/// One address of an `ENVELOPE` (RFC 9051 §7.5.2). Text is as sent: it can
/// hold RFC 2047 encoded words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvelopeAddress {
    /// Display name.
    pub name: Option<Vec<u8>>,
    /// Local part (the mailbox name).
    pub mailbox: Option<Vec<u8>>,
    /// Domain.
    pub host: Option<Vec<u8>>,
}

/// An `ENVELOPE` (RFC 9051 §7.5.2). Text is as sent, not decoded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Envelope {
    /// The `Date` header.
    pub date: Option<Vec<u8>>,
    /// The `Subject` header.
    pub subject: Option<Vec<u8>>,
    /// `From`.
    pub from: Vec<EnvelopeAddress>,
    /// `To`.
    pub to: Vec<EnvelopeAddress>,
    /// `Cc`.
    pub cc: Vec<EnvelopeAddress>,
    /// `In-Reply-To`.
    pub in_reply_to: Option<Vec<u8>>,
    /// `Message-ID`.
    pub message_id: Option<Vec<u8>>,
}

/// The data of one `* n FETCH (...)` response.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchData {
    /// Message sequence number.
    pub seq: u32,
    /// `UID`.
    pub uid: Option<u32>,
    /// `FLAGS`, as sent (`\Seen`, keywords).
    pub flags: Option<Vec<String>>,
    /// `INTERNALDATE`, as sent (`24-Nov-2026 10:46:51 +0000`).
    pub internal_date: Option<String>,
    /// `RFC822.SIZE`.
    pub size: Option<u64>,
    /// `ENVELOPE`.
    pub envelope: Option<Envelope>,
    /// Whether `BODYSTRUCTURE` has a part marked as an attachment.
    pub has_attachment: Option<bool>,
    /// `BODY[HEADER.FIELDS (...)]`: the raw header lines.
    pub header_fields: Option<Vec<u8>>,
    /// `BODY[]`: the whole raw message.
    pub body: Option<Vec<u8>>,
}

/// A generic parenthesised value.
enum Value {
    Nil,
    Text(Vec<u8>),
    List(Vec<Value>),
}

impl Value {
    fn nstring(&self) -> Result<Option<Vec<u8>>, ()> {
        match self {
            Value::Nil => Ok(None),
            Value::Text(t) => Ok(Some(t.clone())),
            Value::List(_) => Err(()),
        }
    }

    fn text_is(&self, expected: &str) -> bool {
        matches!(self, Value::Text(t) if t.eq_ignore_ascii_case(expected.as_bytes()))
    }
}

fn addresses(value: &Value) -> Result<Vec<EnvelopeAddress>, ()> {
    let list = match value {
        Value::Nil => return Ok(Vec::new()),
        Value::List(list) => list,
        Value::Text(_) => return Err(()),
    };
    list.iter()
        .map(|item| match item {
            Value::List(f) if f.len() == 4 => Ok(EnvelopeAddress {
                name: f.first().ok_or(())?.nstring()?,
                mailbox: f.get(2).ok_or(())?.nstring()?,
                host: f.get(3).ok_or(())?.nstring()?,
            }),
            _ => Err(()),
        })
        .collect()
}

fn envelope(value: &Value) -> Result<Envelope, ()> {
    let Value::List(f) = value else {
        return Err(());
    };
    if f.len() != 10 {
        return Err(());
    }
    let get = |i: usize| f.get(i).ok_or(());
    Ok(Envelope {
        date: get(0)?.nstring()?,
        subject: get(1)?.nstring()?,
        from: addresses(get(2)?)?,
        to: addresses(get(5)?)?,
        cc: addresses(get(6)?)?,
        in_reply_to: get(8)?.nstring()?,
        message_id: get(9)?.nstring()?,
    })
}

/// Checks the shape of a `BODYSTRUCTURE` and reports whether any part is
/// marked `ATTACHMENT`.
fn structure(value: &Value, depth: usize) -> Result<bool, ()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    let Value::List(items) = value else {
        return Err(());
    };
    if matches!(items.first(), Some(Value::List(_))) {
        // Multipart: one or more bodies, then the subtype and extensions.
        let bodies = items
            .iter()
            .take_while(|i| matches!(i, Value::List(_)))
            .count();
        let mut attached = false;
        for item in items.iter().take(bodies) {
            attached |= structure(item, depth + 1)?;
        }
        match items.get(bodies) {
            Some(Value::Text(_)) => Ok(attached),
            _ => Err(()),
        }
    } else {
        // Single part: type, subtype, parameters, id, description,
        // encoding, size, then type-specific and extension fields.
        if items.len() < 7 || !matches!(items.first(), Some(Value::Text(_))) {
            return Err(());
        }
        if !matches!(items.get(1), Some(Value::Text(_))) {
            return Err(());
        }
        Ok(items.iter().skip(7).any(|item| match item {
            Value::List(d) => d.first().is_some_and(|k| k.text_is("ATTACHMENT")),
            _ => false,
        }))
    }
}

impl Cursor<'_> {
    fn node(&mut self, nodes: &mut usize) -> Result<(), ParseError> {
        *nodes += 1;
        if *nodes > MAX_NODES {
            return Err(self.err());
        }
        Ok(())
    }

    /// `NIL`, an atom or number, a quoted string, a literal or a list.
    fn value(&mut self, depth: usize, nodes: &mut usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err());
        }
        self.node(nodes)?;
        match self.peek() {
            Some(b'(') => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    if self.eat(b')') {
                        return Ok(Value::List(items));
                    }
                    // Items are separated by a space, except that nested lists
                    // may touch, as in `(...)(...) "MIXED"` (RFC 9051 §9
                    // `body-type-mpart`).
                    if let Some(prev) = items.last() {
                        let touching = matches!(prev, Value::List(_)) || self.peek() == Some(b'(');
                        if !self.eat(b' ') && !touching {
                            return Err(self.err());
                        }
                    }
                    items.push(self.value(depth + 1, nodes)?);
                }
            }
            Some(b'"') => Ok(Value::Text(self.quoted()?)),
            Some(b'{') => Ok(Value::Text(self.literal()?)),
            _ => {
                // Flags start with a backslash (`\Seen`).
                let backslash = self.eat(b'\\');
                let mut atom = self.atom()?;
                if backslash {
                    atom.insert(0, '\\');
                } else if atom.eq_ignore_ascii_case("NIL") {
                    return Ok(Value::Nil);
                }
                Ok(Value::Text(atom.into_bytes()))
            }
        }
    }

    fn nstring_value(&mut self) -> Result<Option<Vec<u8>>, ParseError> {
        match self.peek() {
            Some(b'"') => Ok(Some(self.quoted()?)),
            Some(b'{') => Ok(Some(self.literal()?)),
            _ => {
                let atom = self.atom()?;
                if atom.eq_ignore_ascii_case("NIL") {
                    Ok(None)
                } else {
                    Err(self.err())
                }
            }
        }
    }

    fn number(&mut self) -> Result<u64, ParseError> {
        let atom = self.atom()?;
        atom.parse::<u64>().map_err(|_| self.err())
    }

    /// The data item name, including a `[section]` and `<partial>` for `BODY`.
    fn fetch_key(&mut self) -> Result<String, ParseError> {
        let bytes = self.take_while(|b| b != b' ' && b != b'[' && b != b'(' && b != b')');
        let mut key = String::from_utf8_lossy(bytes).to_ascii_uppercase();
        if key.is_empty() {
            return Err(self.err());
        }
        if self.eat(b'[') {
            let section = self.take_while(|b| b != b']' && b != b'\r' && b != b'\n');
            key.push('[');
            key.push_str(&String::from_utf8_lossy(section).to_ascii_uppercase());
            self.expect(b']')?;
            key.push(']');
            if self.eat(b'<') {
                self.take_while(|b| b.is_ascii_digit());
                self.expect(b'>')?;
            }
        }
        Ok(key)
    }

    /// `FETCH (...)` after `* n FETCH`.
    fn fetch_items(&mut self, seq: u32) -> Result<FetchData, ParseError> {
        let mut data = FetchData {
            seq,
            ..FetchData::default()
        };
        let mut nodes = 0usize;
        self.expect(b' ')?;
        self.expect(b'(')?;
        let mut first = true;
        loop {
            if self.eat(b')') {
                break;
            }
            if !first {
                self.expect(b' ')?;
            }
            first = false;
            let key = self.fetch_key()?;
            self.expect(b' ')?;
            match key.as_str() {
                "UID" => data.uid = Some(u32::try_from(self.number()?).map_err(|_| self.err())?),
                "RFC822.SIZE" => data.size = Some(self.number()?),
                "FLAGS" => {
                    let value = self.value(0, &mut nodes)?;
                    let Value::List(items) = value else {
                        return Err(self.err());
                    };
                    let mut flags = Vec::new();
                    for item in items {
                        match item {
                            Value::Text(t) => flags.push(String::from_utf8_lossy(&t).into_owned()),
                            _ => return Err(self.err()),
                        }
                    }
                    data.flags = Some(flags);
                }
                "INTERNALDATE" => {
                    let date = self.nstring_value()?.ok_or_else(|| self.err())?;
                    data.internal_date = Some(String::from_utf8_lossy(&date).into_owned());
                }
                "ENVELOPE" => {
                    let value = self.value(0, &mut nodes)?;
                    data.envelope = Some(envelope(&value).map_err(|()| self.err())?);
                }
                "BODYSTRUCTURE" | "BODY" => {
                    let value = self.value(0, &mut nodes)?;
                    data.has_attachment = Some(structure(&value, 0).map_err(|()| self.err())?);
                }
                "BODY[]" => data.body = self.nstring_value()?,
                k if k.starts_with("BODY[HEADER.FIELDS") => {
                    data.header_fields = self.nstring_value()?;
                }
                // Anything else (MODSEQ, ...): read past it as one value.
                _ => {
                    self.value(0, &mut nodes)?;
                }
            }
        }
        self.crlf()?;
        Ok(data)
    }

    /// `* n FETCH ...`. A response that cannot be read becomes
    /// [`Untagged::FetchMalformed`], carrying the UID if one can be found.
    pub(super) fn fetch_response(&mut self, seq: u32) -> Result<Untagged, ParseError> {
        let start = self.pos;
        match self.fetch_items(seq) {
            Ok(data) => Ok(Untagged::Fetch(Box::new(data))),
            Err(ParseError::Invalid { .. }) => {
                let uid = find_uid(self.buf.get(start..).unwrap_or_default());
                self.skip_to_end();
                Ok(Untagged::FetchMalformed { uid })
            }
            Err(other) => Err(other),
        }
    }

    /// `* SEARCH 1 2 3` (RFC 9051 §7.3.2), after the keyword.
    pub(super) fn search_response(&mut self) -> Result<Untagged, ParseError> {
        let mut uids = Vec::new();
        while self.eat(b' ') {
            if self.at_eol() {
                break;
            }
            uids.push(u32::try_from(self.number()?).map_err(|_| self.err())?);
        }
        self.crlf()?;
        Ok(Untagged::Search(uids))
    }
}

/// Looks for `UID <number>` as a whole word before any literal or string.
fn find_uid(bytes: &[u8]) -> Option<u32> {
    let end = bytes
        .iter()
        .position(|&b| b == b'"' || b == b'{')
        .unwrap_or(bytes.len());
    let head = bytes.get(..end)?;
    let text = String::from_utf8_lossy(head).to_ascii_uppercase();
    let mut words = text.split([' ', '(', ')']).filter(|w| !w.is_empty());
    while let Some(word) = words.next() {
        if word == "UID" {
            return words.next()?.parse().ok();
        }
    }
    None
}
