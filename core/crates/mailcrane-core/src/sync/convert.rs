//! Turning what the server sent into rows for the store. Never logs
//! subjects, addresses or body text.

use mailcrane_imap::{parse_internal_date, EnvelopeAddress, FetchData};
use mailcrane_store::{Flags, FolderId, NewMessageHeader};

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

fn json_string(text: &str, out: &mut String) {
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

fn json_strings(items: &[String]) -> String {
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

fn address_email(a: &EnvelopeAddress) -> Option<String> {
    let mailbox = decode_text(a.mailbox.as_deref()?);
    match a.host.as_deref() {
        Some(host) => Some(format!("{mailbox}@{}", decode_text(host))),
        None => Some(mailbox),
    }
}

/// `[{"name":..,"email":..}, ...]`; `name` is `null` if there is none.
fn json_addresses(list: &[EnvelopeAddress]) -> String {
    let mut out = String::from("[");
    let mut first = true;
    for a in list {
        // Group markers have no mailbox; skip them.
        let Some(email) = address_email(a) else {
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
    let first_from = envelope.from.iter().find_map(|a| {
        address_email(a).map(|email| {
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

    #[test]
    fn json_is_escaped() {
        assert_eq!(
            json_strings(&["a\"b\\c\n".into()]),
            "[\"a\\\"b\\\\c\\u000a\"]"
        );
    }
}
