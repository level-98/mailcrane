//! Parsing messages into a safe, simple structure.
//!
//! [`parse_message`] turns raw message bytes into a [`ParsedMessage`]. It
//! never panics and enforces the limits in [`limits`].

#![forbid(unsafe_code)]

mod headers;
pub mod limits;
mod model;
mod snippet;

use mail_parser::{MessageParser, PartFlags, PartKind};

pub use model::{Address, AttachmentInfo, Disposition, ParsedMessage, UnsanitisedHtml};

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Why a message could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MimeError {
    /// The input is larger than [`limits::MAX_INPUT_BYTES`].
    #[error("message is larger than {} bytes", limits::MAX_INPUT_BYTES)]
    TooLarge,
    /// MIME parts nest deeper than [`limits::MAX_DEPTH`].
    #[error("message nests deeper than {} levels", limits::MAX_DEPTH)]
    TooDeep,
    /// The message has more than [`limits::MAX_PARTS`] parts.
    #[error("message has more than {} parts", limits::MAX_PARTS)]
    TooManyParts,
    /// The parser found no message in the input (for example, it is empty).
    #[error("no message found in input")]
    Empty,
}

/// Parses raw message bytes.
pub fn parse_message(raw: &[u8]) -> Result<ParsedMessage, MimeError> {
    if raw.len() > limits::MAX_INPUT_BYTES {
        return Err(MimeError::TooLarge);
    }
    // The parser counts the top-level message as one level and stops at its
    // limit, so ask for one more than we allow and detect the stop. The part
    // limit works the same way.
    let parser = MessageParser::new()
        .max_depth(limits::MAX_DEPTH.saturating_add(1))
        .max_parts(limits::MAX_PARTS.saturating_add(1));
    let message = parser.parse(raw).ok_or(MimeError::Empty)?;

    if message.parts().len() > limits::MAX_PARTS {
        return Err(MimeError::TooManyParts);
    }
    // A container left unopened at the depth limit is flagged.
    if message
        .parts()
        .any(|p| p.flags().contains(PartFlags::LIMIT_REACHED))
    {
        return Err(MimeError::TooDeep);
    }

    let root = message.root();

    let mut text_plain: Option<String> = None;
    let mut html: Option<String> = None;
    for part in root.text_body() {
        if matches!(part.kind(), PartKind::Text) {
            // Invalid charsets fall back to lossy UTF-8 inside `text()`.
            if let Some(t) = part.text() {
                text_plain.get_or_insert_with(String::new).push_str(&t);
            }
        }
    }
    for part in root.html_body() {
        if matches!(part.kind(), PartKind::Html) {
            if let Some(t) = part.html_utf8() {
                html.get_or_insert_with(String::new).push_str(&t);
            }
        }
    }
    if text_plain.is_none() {
        if let Some(h) = &html {
            text_plain = Some(mail_parser::html_to_text(h));
        }
    }

    let attachments = root
        .attachments()
        .map(|part| {
            let content_type = part.content_type();
            let mime_type = content_type
                .map(|ct| match ct.subtype() {
                    Some(sub) => format!("{}/{}", ct.ctype(), sub),
                    None => ct.ctype().to_owned(),
                })
                .map(|s| headers::clean(&s).to_ascii_lowercase())
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            let disposition = if part.is_inline() {
                Disposition::Inline
            } else {
                Disposition::Attachment
            };
            AttachmentInfo {
                part_id: part.id().to_string(),
                filename: headers::clean_opt(part.attachment_name()),
                mime_type,
                size: part.decoded_len(),
                content_id: headers::clean_opt(part.content_id()),
                disposition,
            }
        })
        .collect();

    let hdrs = root.headers();
    let raw_header = |name: &str| {
        hdrs.get(name)
            .and_then(|h| headers::raw_value(h.raw_value()))
    };

    let snippet = snippet::snippet(text_plain.as_deref().unwrap_or(""));

    Ok(ParsedMessage {
        subject: headers::clean_opt(root.subject()),
        from: headers::addresses(root.from()),
        sender: headers::addresses(root.sender()),
        reply_to: headers::addresses(root.reply_to()),
        to: headers::addresses(root.to()),
        cc: headers::addresses(root.cc()),
        bcc: headers::addresses(root.bcc()),
        date: root.date().map(|d| d.to_timestamp()),
        message_id: headers::clean_opt(root.message_id()),
        in_reply_to: root
            .in_reply_to()
            .map(|l| headers::clean_list(l.iter()))
            .unwrap_or_default(),
        references: root
            .references()
            .map(|l| headers::clean_list(l.iter()))
            .unwrap_or_default(),
        list_unsubscribe: raw_header("List-Unsubscribe"),
        list_unsubscribe_post: raw_header("List-Unsubscribe-Post"),
        text_plain,
        text_html: html.map(UnsanitisedHtml::new),
        attachments,
        snippet,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches() {
        assert_eq!(CRATE_NAME, "mailcrane-mime");
    }
}
