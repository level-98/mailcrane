//! Turning what the server sent into rows for the store. Never logs
//! subjects, addresses or body text.

use mailcrane_imap::{parse_internal_date, EnvelopeAddress, FetchData};
use mailcrane_store::{Flags, FolderId, NewMessageHeader, RemoteFlags};

/// Decodes header text with RFC 2047 encoded words by running it through the
/// MIME parser as a `Subject` header. CR, LF and NUL never get through.
pub(crate) fn decode_text(raw: &[u8]) -> String {
    let mut line = b"Subject: ".to_vec();
    line.extend(raw.iter().map(|&b| {
        if matches!(b, b'\r' | b'\n' | 0) {
            b' '
        } else {
            b
        }
    }));
    line.extend_from_slice(b"\r\n\r\n");
    match mailcrane_mime::parse_message(&line) {
        Ok(parsed) => parsed.subject.unwrap_or_default(),
        Err(_) => String::from_utf8_lossy(raw)
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
            .collect(),
    }
}

fn decode_date(raw: &[u8]) -> Option<i64> {
    let mut line = b"Date: ".to_vec();
    line.extend(raw.iter().map(|&b| {
        if matches!(b, b'\r' | b'\n' | 0) {
            b' '
        } else {
            b
        }
    }));
    line.extend_from_slice(b"\r\n\r\n");
    mailcrane_mime::parse_message(&line).ok()?.date
}

pub(crate) fn json_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub(crate) fn json_strings(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_string(item, &mut out);
    }
    out.push(']');
    out
}

/// The real addresses of an ENVELOPE address list, with their indexes:
/// `mailbox@host`, or the bare mailbox for an unqualified one. RFC 9051
/// §7.5.2 group markers are left out (#433): the end marker has no mailbox,
/// and the start marker has a NIL host *and* an end marker after it. A NIL
/// host with no end marker later in the list is an unqualified address
/// that some servers send that way (`MAILER-DAEMON`, `root`), and is kept,
/// as is one followed by another start before the next end marker.
/// An empty mailbox or host counts as NIL.
fn envelope_emails(list: &[EnvelopeAddress]) -> Vec<(usize, String)> {
    let part = |v: &Option<Vec<u8>>| {
        v.as_deref()
            .map(decode_text)
            .filter(|t| !t.trim().is_empty())
    };
    let is_end = |a: &EnvelopeAddress| part(&a.mailbox).is_none() && part(&a.host).is_none();
    let mut out = Vec::new();
    for (i, a) in list.iter().enumerate() {
        let Some(mailbox) = part(&a.mailbox) else {
            continue;
        };
        match part(&a.host) {
            Some(host) => out.push((i, format!("{mailbox}@{host}"))),
            None => {
                // A group start only if the next end marker closes this
                // group, with no other start in between: a lone address
                // before a separate group is kept.
                let starts_group = list
                    .get(i.saturating_add(1)..)
                    .unwrap_or_default()
                    .iter()
                    .find(|b| is_end(b) || (part(&b.mailbox).is_some() && part(&b.host).is_none()))
                    .is_some_and(is_end);
                if !starts_group {
                    out.push((i, mailbox));
                }
            }
        }
    }
    out
}

/// `[{"name":..,"email":..}, ...]`; `name` is `null` if there is none.
fn json_addresses(list: &[EnvelopeAddress]) -> String {
    let mut out = String::from("[");
    let mut first = true;
    for (i, email) in envelope_emails(list) {
        let Some(a) = list.get(i) else {
            continue;
        };
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str("{\"name\":");
        match a.name.as_deref().map(decode_text).filter(|n| !n.is_empty()) {
            Some(name) => json_string(&name, &mut out),
            None => out.push_str("null"),
        }
        out.push_str(",\"email\":");
        json_string(&email, &mut out);
        out.push('}');
    }
    out.push(']');
    out
}

fn strip_brackets(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string()
}

fn flags(list: &[String]) -> (Flags, Vec<String>) {
    let mut f = Flags::default();
    let mut keywords = Vec::new();
    for flag in list {
        match flag.to_ascii_lowercase().as_str() {
            "\\seen" => f.seen = true,
            "\\flagged" => f.flagged = true,
            "\\answered" => f.answered = true,
            "\\draft" => f.draft = true,
            "\\deleted" => f.deleted = true,
            "\\recent" => {}
            _ if flag.starts_with('\\') => {}
            _ => keywords.push(flag.clone()),
        }
    }
    (f, keywords)
}

/// The store's view of the flags the server sent for `uid`.
pub(crate) fn remote_flags(uid: u32, list: &[String]) -> RemoteFlags {
    let (flags, keywords) = flags(list);
    RemoteFlags {
        uid,
        flags,
        keywords_json: json_strings(&keywords),
    }
}

/// Builds the store row, or `None` if the response lacks the UID or the
/// envelope.
pub(crate) fn header_row(folder: FolderId, data: &FetchData) -> Option<NewMessageHeader> {
    let uid = data.uid?;
    let envelope = data.envelope.as_ref()?;
    // `References` and `In-Reply-To` come from the header lines we asked for.
    let parsed = data
        .header_fields
        .as_deref()
        .and_then(|h| mailcrane_mime::parse_message(h).ok());
    let references = parsed
        .as_ref()
        .map(|p| p.references.clone())
        .unwrap_or_default();
    let in_reply_to = parsed
        .as_ref()
        .and_then(|p| p.in_reply_to.first().cloned())
        .or_else(|| {
            envelope
                .in_reply_to
                .as_deref()
                .map(|v| strip_brackets(&decode_text(v)))
                .filter(|v| !v.is_empty())
        });
    let internal_date = data.internal_date.as_deref().and_then(parse_internal_date);
    let date = envelope.date.as_deref().and_then(decode_date);
    let first_from = envelope_emails(&envelope.from)
        .into_iter()
        .next()
        .and_then(|(i, email)| {
            envelope.from.get(i).map(|a| {
                let name = a.name.as_deref().map(decode_text).filter(|n| !n.is_empty());
                (name, email)
            })
        });
    let (flag_set, keywords) = flags(data.flags.as_deref().unwrap_or_default());
    Some(NewMessageHeader {
        folder_id: folder,
        uid,
        message_id_header: envelope
            .message_id
            .as_deref()
            .map(|v| strip_brackets(&decode_text(v)))
            .filter(|v| !v.is_empty()),
        in_reply_to,
        references_json: json_strings(&references),
        thread_id: None,
        subject: envelope
            .subject
            .as_deref()
            .map(decode_text)
            .unwrap_or_default(),
        from_name: first_from.as_ref().and_then(|(n, _)| n.clone()),
        from_addr: first_from.map(|(_, e)| e),
        to_json: json_addresses(&envelope.to),
        cc_json: json_addresses(&envelope.cc),
        date: date.or(internal_date).unwrap_or(0),
        internal_date: internal_date.or(date).unwrap_or(0),
        size: data.size.and_then(|s| i64::try_from(s).ok()).unwrap_or(0),
        flags: flag_set,
        keywords_json: json_strings(&keywords),
        has_attachments: data.has_attachment.unwrap_or(false),
        snippet: String::new(),
    })
}

/// `[{"name":..,"email":..}, ...]` from JMAP addresses (RFC 8621 §4.1.2.3),
/// the same shape as [`json_addresses`] writes for IMAP.
fn json_jmap_addresses(list: &[mailcrane_jmap::JmapAddress]) -> String {
    let mut out = String::from("[");
    for (i, a) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        match a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => json_string(name, &mut out),
            None => out.push_str("null"),
        }
        out.push_str(",\"email\":");
        json_string(&a.email, &mut out);
        out.push('}');
    }
    out.push(']');
    out
}

/// The store row for a JMAP email in `folder` (RFC 8621 §4.1). `uid` is
/// filled in by the store. Keywords map to flags the way IMAP flags do
/// (RFC 8621 §4.1.1: `$seen` is `\Seen` and so on).
pub(crate) fn jmap_header_row(folder: FolderId, e: &mailcrane_jmap::JmapEmail) -> NewMessageHeader {
    let mut f = Flags::default();
    let mut keywords = Vec::new();
    for k in &e.keywords {
        match k.to_ascii_lowercase().as_str() {
            "$seen" => f.seen = true,
            "$flagged" => f.flagged = true,
            "$answered" => f.answered = true,
            "$draft" => f.draft = true,
            _ => keywords.push(k.clone()),
        }
    }
    let from = e.from.first();
    NewMessageHeader {
        folder_id: folder,
        uid: 0,
        message_id_header: e.message_id.clone().filter(|m| !m.is_empty()),
        in_reply_to: e.in_reply_to.clone().filter(|m| !m.is_empty()),
        references_json: json_strings(&e.references),
        thread_id: None,
        subject: e.subject.chars().filter(|c| !c.is_control()).collect(),
        from_name: from
            .and_then(|a| a.name.clone())
            .filter(|n| !n.trim().is_empty()),
        from_addr: from.map(|a| a.email.clone()),
        to_json: json_jmap_addresses(&e.to),
        cc_json: json_jmap_addresses(&e.cc),
        date: e.sent_at.unwrap_or(e.received_at),
        internal_date: e.received_at,
        size: i64::try_from(e.size).unwrap_or(i64::MAX),
        flags: f,
        keywords_json: json_strings(&keywords),
        has_attachments: e.has_attachment,
        snippet: e
            .preview
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_words_are_decoded_and_control_characters_removed() {
        assert_eq!(decode_text(b"=?UTF-8?Q?Gr=C3=BC=C3=9Fe?="), "Grüße");
        assert_eq!(
            decode_text(b"a\r\nBcc: x@example.org"),
            "a  Bcc: x@example.org"
        );
    }

    /// The FETCH data a server sends for one message, parsed as a session
    /// would parse it.
    fn fetched(internal_date: &str, envelope_date: &str) -> FetchData {
        let line = format!(
            "* 1 FETCH (UID 7 INTERNALDATE \"{internal_date}\" ENVELOPE ({envelope_date} \
             \"Hello\" ((\"Ana\" NIL \"ana\" \"example.org\")) NIL NIL NIL NIL NIL NIL \
             \"<m1@example.org>\"))\r\n"
        );
        match mailcrane_imap::parse_response(line.as_bytes()) {
            Ok((
                mailcrane_imap::parser::Response::Untagged(
                    mailcrane_imap::parser::Untagged::Fetch(data),
                ),
                _,
            )) => *data,
            other => panic!("not a FETCH: {other:?}"),
        }
    }

    /// A FETCH whose ENVELOPE has these From and To address lists (RFC
    /// 9051 §7.5.2 syntax), parsed as a session would parse it.
    fn fetched_addresses(from: &str, to: &str) -> FetchData {
        let line = format!(
            "* 1 FETCH (UID 7 ENVELOPE (\"Tue, 24 Nov 2026 10:46:51 +0000\" \"Hello\" \
             {from} NIL NIL {to} NIL NIL NIL \"<m1@example.org>\"))\r\n"
        );
        match mailcrane_imap::parse_response(line.as_bytes()) {
            Ok((
                mailcrane_imap::parser::Response::Untagged(
                    mailcrane_imap::parser::Untagged::Fetch(data),
                ),
                _,
            )) => *data,
            other => panic!("not a FETCH: {other:?}"),
        }
    }

    const ANA: &str = "((\"Ana\" NIL \"ana\" \"example.org\"))";

    /// The FETCH responses of `tests/fixtures/imap/<name>`, in order.
    fn fixture_fetches(name: &str) -> Vec<FetchData> {
        let path = format!(
            "{}/../../../tests/fixtures/imap/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = std::fs::read(&path).expect("fixture");
        let mut rest = bytes.as_slice();
        let mut out = Vec::new();
        while let Ok((response, used)) = mailcrane_imap::parse_response(rest) {
            if let mailcrane_imap::parser::Response::Untagged(
                mailcrane_imap::parser::Untagged::Fetch(data),
            ) = response
            {
                out.push(*data);
            }
            match rest.get(used..) {
                Some(next) if used > 0 => rest = next,
                _ => break,
            }
        }
        out
    }

    /// #433 with the fixture: an empty group, a group with members, and a
    /// lone unqualified address (no end marker after it), which is kept.
    #[test]
    fn the_envelope_groups_fixture() {
        let rows: Vec<_> = fixture_fetches("fetch-envelope-groups.imap")
            .iter()
            .map(|d| header_row(FolderId(1), d).expect("row"))
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].to_json, "[]");
        assert_eq!(
            rows[1].to_json,
            "[{\"name\":\"Bo\",\"email\":\"bo@example.net\"},\
             {\"name\":null,\"email\":\"carl@example.com\"}]"
        );
        assert_eq!(rows[2].from_addr.as_deref(), Some("MAILER-DAEMON"));
        assert_eq!(
            rows[2].to_json,
            "[{\"name\":null,\"email\":\"root\"},\
             {\"name\":\"Bo\",\"email\":\"bo@example.net\"}]"
        );
    }

    /// A lone unqualified address before a separate group is kept; only the
    /// group's own start marker is dropped.
    #[test]
    fn a_lone_address_before_a_group_is_kept() {
        let data = fetched_addresses(
            ANA,
            "((NIL NIL \"root\" NIL)(NIL NIL \"team\" NIL)(\"Bo\" NIL \"bo\" \"example.net\")(NIL NIL NIL NIL))",
        );
        let row = header_row(FolderId(1), &data).expect("row");
        assert_eq!(
            row.to_json,
            "[{\"name\":null,\"email\":\"root\"},{\"name\":\"Bo\",\"email\":\"bo@example.net\"}]"
        );
    }

    /// An empty mailbox or host (`""`) counts as NIL: never `name@` or
    /// `@host`.
    #[test]
    fn empty_address_parts_are_not_addresses() {
        let data = fetched_addresses(
            "((NIL NIL \"\" \"example.org\"))",
            "((NIL NIL \"team\" \"\")(NIL NIL NIL NIL)(NIL NIL \"bo\" \"example.net\"))",
        );
        let row = header_row(FolderId(1), &data).expect("row");
        assert_eq!(row.from_addr, None);
        assert_eq!(
            row.to_json,
            "[{\"name\":null,\"email\":\"bo@example.net\"}]"
        );
    }

    /// #433: `To: undisclosed-recipients:;` arrives as a group-start marker
    /// (mailbox set, host NIL) and an all-NIL end marker (RFC 9051 §7.5.2).
    /// Neither is a recipient.
    #[test]
    fn an_empty_group_gives_no_recipients() {
        let data = fetched_addresses(
            ANA,
            "((NIL NIL \"undisclosed-recipients\" NIL)(NIL NIL NIL NIL))",
        );
        let row = header_row(FolderId(1), &data).expect("row");
        assert_eq!(row.to_json, "[]");
    }

    /// The members between the markers are kept, and only they.
    #[test]
    fn a_group_keeps_exactly_its_members() {
        let data = fetched_addresses(
            ANA,
            "((NIL NIL \"team\" NIL)(\"Bo\" NIL \"bo\" \"example.net\")\
             (NIL NIL \"carl\" \"example.com\")(NIL NIL NIL NIL))",
        );
        let row = header_row(FolderId(1), &data).expect("row");
        assert_eq!(
            row.to_json,
            "[{\"name\":\"Bo\",\"email\":\"bo@example.net\"},\
             {\"name\":null,\"email\":\"carl@example.com\"}]"
        );
    }

    /// A From that is only a group has no sender (the app shows its usual
    /// unknown sender), not the group's name as an address; a group with a
    /// member gives that member.
    #[test]
    fn a_from_group_is_not_a_sender() {
        let empty = fetched_addresses("((NIL NIL \"list\" NIL)(NIL NIL NIL NIL))", "NIL");
        let row = header_row(FolderId(1), &empty).expect("row");
        assert_eq!((row.from_name, row.from_addr), (None, None));

        let member = fetched_addresses(
            "((NIL NIL \"list\" NIL)(\"Ana\" NIL \"ana\" \"example.org\")(NIL NIL NIL NIL))",
            "NIL",
        );
        let row = header_row(FolderId(1), &member).expect("row");
        assert_eq!(row.from_addr.as_deref(), Some("ana@example.org"));
        assert_eq!(row.from_name.as_deref(), Some("Ana"));
    }

    /// #410: an INTERNALDATE with an impossible year doesn't crash the sync
    /// or drop the message. The message is stored, dated from its `Date`
    /// header, or 0 when it has none, the fallback for a missing date.
    #[test]
    fn a_bad_internal_date_still_gives_a_row() {
        for bad in [
            "01-Jan-9223372036854775807 00:00:00 +0000",
            "01-Mar-99999999999999999 00:00:00 +0000",
            "01-Jan-0000 00:00:00 +0000",
        ] {
            let data = fetched(bad, "\"Tue, 24 Nov 2026 10:46:51 +0000\"");
            assert_eq!(data.internal_date.as_deref(), Some(bad));
            let row = header_row(FolderId(1), &data).expect("row");
            assert_eq!(row.uid, 7);
            assert_eq!(row.internal_date, 1_795_517_211, "{bad}");
            assert_eq!(row.date, 1_795_517_211, "{bad}");

            let row = header_row(FolderId(1), &fetched(bad, "NIL")).expect("row, no Date");
            assert_eq!((row.internal_date, row.date), (0, 0), "{bad}");
        }
    }

    #[test]
    fn json_is_escaped() {
        assert_eq!(
            json_strings(&["a\"b\\c\n".into()]),
            "[\"a\\\"b\\\\c\\u000a\"]"
        );
    }
}
