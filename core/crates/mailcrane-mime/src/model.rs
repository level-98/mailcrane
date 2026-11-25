//! The types `parse_message` returns. None of them expose `mail-parser`.

/// An email address with an optional display name. Both are decoded
/// (RFC 2047) and free of CR, LF and NUL.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Address {
    /// Display name, if any.
    pub name: Option<String>,
    /// The address, such as `ann@example.org`.
    pub email: String,
}

/// HTML exactly as the sender wrote it. **It has not been sanitised and must
/// never be displayed or run before it goes through the sanitiser.** The
/// type has no `Display` impl and no `Deref` to `str`, so it cannot be
/// passed where sanitised HTML is expected by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct UnsanitisedHtml(String);

impl UnsanitisedHtml {
    pub(crate) fn new(html: String) -> Self {
        Self(html)
    }

    /// The raw, unsanitised HTML. Only the sanitiser should call this.
    pub fn as_unsanitised_str(&self) -> &str {
        &self.0
    }

    /// Takes the raw, unsanitised HTML. Only the sanitiser should call this.
    pub fn into_unsanitised_string(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for UnsanitisedHtml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print message content (AGENTS.md §4.4).
        write!(f, "UnsanitisedHtml({} bytes)", self.0.len())
    }
}

/// How the sender marked an attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// An ordinary attachment.
    Attachment,
    /// Meant to show inside the message, usually referenced by `cid:`.
    Inline,
}

/// One attached or inline file. The bytes are not kept here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentInfo {
    /// Stable id of the part within this message: its position in document
    /// order.
    pub part_id: String,
    /// Decoded file name (RFC 2047 and RFC 2231), if the sender gave one.
    pub filename: Option<String>,
    /// Lower-case `type/subtype`, `application/octet-stream` if missing.
    pub mime_type: String,
    /// Size in bytes after transfer decoding.
    pub size: usize,
    /// The Content-ID without angle brackets, for `cid:` references.
    pub content_id: Option<String>,
    /// Attachment or inline.
    pub disposition: Disposition,
}

/// A message in the simple form the rest of the app uses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedMessage {
    /// Decoded Subject.
    pub subject: Option<String>,
    /// From.
    pub from: Vec<Address>,
    /// Sender.
    pub sender: Vec<Address>,
    /// Reply-To.
    pub reply_to: Vec<Address>,
    /// To.
    pub to: Vec<Address>,
    /// Cc.
    pub cc: Vec<Address>,
    /// Bcc.
    pub bcc: Vec<Address>,
    /// Date as seconds since the Unix epoch (UTC).
    pub date: Option<i64>,
    /// Message-ID without angle brackets.
    pub message_id: Option<String>,
    /// In-Reply-To ids without angle brackets.
    pub in_reply_to: Vec<String>,
    /// References ids without angle brackets.
    pub references: Vec<String>,
    /// List-Unsubscribe as written (RFC 2369), cleaned of CR, LF and NUL.
    pub list_unsubscribe: Option<String>,
    /// List-Unsubscribe-Post as written (RFC 8058), cleaned.
    pub list_unsubscribe_post: Option<String>,
    /// The id part of List-Id (RFC 2919), lower-case: `list.example.org`
    /// from `Name <list.example.org>`.
    pub list_id: Option<String>,
    /// The Received and Authentication-Results headers, cleaned, in their
    /// original order, topmost first (at most [`crate::limits::MAX_TRACE_HEADERS`]).
    /// Their order decides which results can be trusted: see
    /// [`crate::auth::trusted_results`].
    pub trace: Vec<crate::auth::TraceHeader>,
    /// The plain-text body, or text derived from the HTML if there is only
    /// HTML.
    pub text_plain: Option<String>,
    /// The HTML body, raw and unsanitised. `None` if the message has no
    /// HTML part.
    pub text_html: Option<UnsanitisedHtml>,
    /// Attached and inline files.
    pub attachments: Vec<AttachmentInfo>,
    /// First ~200 characters of readable text, whitespace collapsed.
    pub snippet: String,
}
