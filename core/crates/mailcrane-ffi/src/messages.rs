//! Reading stored messages, as the apps call it: `list_messages` returns one
//! page of a folder's message list, newest first; `get_message` returns one
//! message ready to show, and `get_inline_part` the `cid:` images in it.

use mailcrane_core::phishing;
use mailcrane_mime::{parse_message, sanitize_html, SanitizeOptions, TrackerHit, TrackerKind};
use mailcrane_store::{
    BodyState, FolderId, MessageHeader, MessageId, Store, StoreError, ACCOUNT_COLOR_COUNT,
};
use std::collections::HashMap;

use crate::sync::{SyncErrorKind, SyncFailure};

/// The most rows one `list_messages` call returns, whatever the app asks for.
pub const MAX_PAGE: u32 = 500;

/// The longest subject, sender name or snippet passed to the apps, in chars.
/// Longer values are cut; a list row never shows more.
const MAX_TEXT_CHARS: usize = 998;

/// One row of a message list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageSummary {
    /// Row id of the message.
    pub id: i64,
    /// Sender display name; empty when the message has none.
    pub from_name: String,
    /// Sender address; empty when the message has none.
    pub from_addr: String,
    /// Subject; empty when the message has none.
    pub subject: String,
    /// Short preview of the text; empty until known.
    pub snippet: String,
    /// Header date, unix seconds UTC.
    pub date: i64,
    /// Whether the message has been read (`\Seen`).
    pub is_seen: bool,
    /// Whether the message is flagged (`\Flagged`).
    pub is_flagged: bool,
    /// Whether the message has attachments.
    pub has_attachments: bool,
    /// Its account, in the Unified Inbox (M4-01); 0 in single-folder lists.
    #[uniffi(default = 0)]
    pub account_id: i64,
    /// A colour for the account's marker (0–7), in the Unified Inbox.
    #[uniffi(default = 0)]
    pub account_color: u32,
}

/// Colours for account markers.
pub const ACCOUNT_COLORS: u32 = ACCOUNT_COLOR_COUNT;

/// The automatic marker colour of an account (stable: follows the account
/// id). Used until the user chooses one (M4-20).
#[uniffi::export]
pub fn account_color(account_id: i64) -> u32 {
    u32::try_from(account_id.rem_euclid(i64::from(ACCOUNT_COLORS))).unwrap_or(0)
}

/// Every account's marker colour: the one the user chose, else the
/// automatic one. Accounts missing from the map use [`account_color`].
pub(crate) fn marker_colors(store: &Store) -> Result<HashMap<i64, u32>, StoreError> {
    Ok(store
        .list_account_colors()?
        .into_iter()
        .map(|(id, chosen)| (id.0, chosen.unwrap_or_else(|| account_color(id.0))))
        .collect())
}

/// The marker colour for `account_id` from a [`marker_colors`] map.
pub(crate) fn marker_color(colors: &HashMap<i64, u32>, account_id: i64) -> u32 {
    colors
        .get(&account_id)
        .copied()
        .unwrap_or_else(|| account_color(account_id))
}

/// Header text as one safe line: CR, LF and NUL become spaces, and the
/// result is at most [`MAX_TEXT_CHARS`] chars.
pub(crate) fn one_line(text: &str) -> String {
    text.chars()
        .take(MAX_TEXT_CHARS)
        .map(|c| {
            if matches!(c, '\r' | '\n' | '\0') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// [`one_line`] for text the app shows as a line of its own: subjects and
/// attachment names. Bidi controls are removed too, so an override can't
/// reverse what's shown (`invoice\u{202E}fdp.exe` showing as
/// `invoiceexe.pdf`, #315).
pub(crate) fn display_line(text: &str) -> String {
    one_line(&mailcrane_mime::strip_bidi(text))
}

impl From<MessageHeader> for MessageSummary {
    fn from(m: MessageHeader) -> Self {
        let d = m.details;
        Self {
            id: m.id.0,
            from_name: one_line(d.from_name.as_deref().unwrap_or_default()),
            from_addr: one_line(d.from_addr.as_deref().unwrap_or_default()),
            subject: display_line(&d.subject),
            snippet: one_line(&d.snippet),
            date: d.date,
            is_seen: d.flags.seen,
            is_flagged: d.flags.flagged,
            has_attachments: d.has_attachments,
            account_id: 0,
            account_color: 0,
        }
    }
}

/// Lists up to `limit` messages of the folder, newest first, skipping the
/// first `offset`. Pages with the same `limit` and consecutive offsets have
/// no duplicates or gaps. `limit` is capped at [`MAX_PAGE`]. An unknown
/// folder gives an empty list. Reads only the local database.
///
/// `db_path` is the same database file the app gives `SyncEngine`.
#[uniffi::export]
pub fn list_messages(
    db_path: String,
    folder_id: i64,
    offset: u32,
    limit: u32,
) -> Result<Vec<MessageSummary>, SyncFailure> {
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let rows = store
        .list_messages(FolderId(folder_id), limit.min(MAX_PAGE), offset)
        .map_err(|_| storage())?;
    Ok(rows.into_iter().map(MessageSummary::from).collect())
}

/// The Unified Inbox (M4-01): every account's inbox, newest first, `limit`
/// (capped at [`MAX_PAGE`]) after skipping `offset`. Each row names its
/// account and marker colour. Reads only the local database.
#[uniffi::export]
pub fn list_unified_inbox(
    db_path: String,
    offset: u32,
    limit: u32,
) -> Result<Vec<MessageSummary>, SyncFailure> {
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let rows = store
        .list_unified_inbox(limit.min(MAX_PAGE), offset)
        .map_err(|_| storage())?;
    let colors = marker_colors(&store).map_err(|_| storage())?;
    Ok(rows
        .into_iter()
        .map(|(header, account)| {
            let mut summary = MessageSummary::from(header);
            summary.account_id = account.0;
            summary.account_color = marker_color(&colors, account.0);
            summary
        })
        .collect())
}

/// Unread messages across every account's inbox.
#[uniffi::export]
pub fn unified_unread_count(db_path: String) -> Result<u64, SyncFailure> {
    let store = Store::open(&db_path).map_err(|_| storage())?;
    store.count_unified_unread().map_err(|_| storage())
}

/// The scheme `cid:` images are rewritten to. The app serves it from
/// [`get_inline_part`] and never from the network.
pub const CID_SCHEME: &str = "mailcrane-cid";

/// What a `cid:` reference becomes: this prefix plus the percent-encoded
/// content id, e.g. `mailcrane-cid://part/logo%40example.org`.
pub const CID_URL_PREFIX: &str = "mailcrane-cid://part/";

/// The longest plain-text body passed to the apps, in chars. Longer text is
/// cut; a phone can't usefully show more as one text block.
const MAX_PLAIN_CHARS: usize = 1_000_000;

/// Image types `get_inline_part` serves. SVG is not one of them.
const INLINE_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/gif",
    "image/webp",
];

/// A sender or recipient.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageAddress {
    /// Display name; empty when the message has none.
    pub name: String,
    /// The address; may be empty for a malformed header.
    pub email: String,
}

/// An attached file, for the list under a message.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttachmentInfo {
    /// The file name; empty when the sender gave none.
    pub name: String,
    /// Lower-case `type/subtype`.
    pub mime_type: String,
    /// Size in bytes after decoding.
    pub size: u64,
    /// Which part it is, for `save_attachment`.
    #[uniffi(default = "")]
    pub part_id: String,
}

/// The body of a message, ready to show.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MessageBody {
    /// Plain text. Show it as text, never as HTML.
    Plain {
        /// The text.
        text: String,
    },
    /// A complete HTML document: the sanitised message HTML with a strict
    /// Content-Security-Policy. Show it only in a web view with JavaScript
    /// off and network loads blocked unless remote content was allowed.
    Html {
        /// The document.
        html: String,
        /// Remote images that were removed because remote content is off.
        remote_blocked: u32,
        /// Tracking pixels removed (in both modes).
        trackers_removed: u32,
        /// The trackers removed, one per domain, for the "N trackers
        /// blocked" sheet (M5-01).
        trackers: Vec<TrackerInfo>,
        /// Each link and the text shown for it, to compare in the link
        /// preview (M5-07). At most 500.
        #[uniffi(default = [])]
        links: Vec<LinkText>,
    },
    /// The body has not been downloaded yet; call `SyncEngine::fetch_body`.
    NotDownloaded,
}

/// A link in the message and the text the reader sees for it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LinkText {
    /// The `href`, exactly as the web view reports it when tapped.
    pub href: String,
    /// The visible text, whitespace collapsed; may be empty.
    pub text: String,
}

/// A tracker removed from a message (M5-01).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrackerInfo {
    /// The known service's name, or `None` for an unlisted tiny or hidden
    /// image.
    pub service: Option<String>,
    /// The tracker's domain; empty when the image had no web address.
    pub domain: String,
}

impl From<TrackerHit> for TrackerInfo {
    fn from(hit: TrackerHit) -> Self {
        let service = match hit.kind {
            TrackerKind::KnownService(name) => Some(one_line(&name)),
            TrackerKind::Tiny | TrackerKind::Hidden => None,
        };
        Self {
            service,
            domain: one_line(&hit.domain),
        }
    }
}

/// One message, ready to show.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageDetail {
    /// Row id of the message.
    pub id: i64,
    /// Row id of its folder.
    pub folder_id: i64,
    /// Sender.
    pub from: MessageAddress,
    /// Recipients; empty until the body is downloaded.
    pub to: Vec<MessageAddress>,
    /// Copy recipients; empty until the body is downloaded.
    pub cc: Vec<MessageAddress>,
    /// Header date, unix seconds UTC.
    pub date: i64,
    /// Subject; empty when the message has none.
    pub subject: String,
    /// Whether the message has been read (`\Seen`).
    pub is_seen: bool,
    /// Whether the message is flagged (`\Flagged`).
    pub is_flagged: bool,
    /// The body.
    pub body: MessageBody,
    /// Attached files, not counting inline images shown in the body.
    pub attachments: Vec<AttachmentInfo>,
    /// What the user's provider found when it checked the sender (M5-07);
    /// `None` until the body is downloaded, or if it can't be parsed.
    #[uniffi(default = None)]
    pub sender_check: Option<SenderCheck>,
}

/// The result of one authentication method (RFC 8601).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AuthResult {
    /// It passed.
    Pass,
    /// It failed (for SPF, also a soft fail).
    Fail,
    /// The domain makes no claim either way.
    Neutral,
    /// Not checked, or not in the header.
    None,
    /// A temporary error while checking.
    TempError,
    /// A permanent error, such as a broken record.
    PermError,
    /// Some other result.
    Unknown,
}

impl From<phishing::AuthResult> for AuthResult {
    fn from(r: phishing::AuthResult) -> Self {
        use phishing::AuthResult as R;
        match r {
            R::Pass => Self::Pass,
            R::Fail => Self::Fail,
            R::Neutral => Self::Neutral,
            R::None => Self::None,
            R::TempError => Self::TempError,
            R::PermError => Self::PermError,
            R::Unknown => Self::Unknown,
        }
    }
}

/// What the authentication chip next to the sender says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AuthVerdict {
    /// "Verified".
    Verified,
    /// "Not verified".
    NotVerified,
    /// "Failed".
    Failed,
}

/// The provider's SPF, DKIM and DMARC results for a message. Only the
/// `Authentication-Results` header the user's provider added is read.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SenderCheck {
    /// Whether the provider's header was found. When `false`, show no chip.
    pub trusted: bool,
    /// The chip to show; `None` exactly when `trusted` is `false`.
    pub verdict: Option<AuthVerdict>,
    /// SPF.
    pub spf: AuthResult,
    /// The domain SPF checked.
    pub spf_domain: Option<String>,
    /// DKIM.
    pub dkim: AuthResult,
    /// The domain that signed the message.
    pub dkim_domain: Option<String>,
    /// DMARC.
    pub dmarc: AuthResult,
    /// The From domain DMARC checked.
    pub dmarc_domain: Option<String>,
    /// The From name shows an address or domain from another organisation
    /// than the real From address; show a warning banner above the body.
    /// Set whether or not `trusted` is.
    #[uniffi(default = None)]
    pub display_name_warning: Option<DisplayNameWarning>,
}

/// A From name that claims to be someone the message isn't from, such as
/// `"ceo@example.com" <x@example.net>`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DisplayNameWarning {
    /// The address or domain the name shows (lowercase, tricks removed).
    pub claimed: String,
    /// The address the message is really from.
    pub actual: String,
}

impl From<phishing::SenderCheck> for SenderCheck {
    fn from(c: phishing::SenderCheck) -> Self {
        let domain = |d: Option<String>| d.map(|d| one_line(&d)).filter(|d| !d.is_empty());
        Self {
            trusted: c.trusted,
            verdict: c.verdict.map(|v| match v {
                phishing::AuthVerdict::Verified => AuthVerdict::Verified,
                phishing::AuthVerdict::NotVerified => AuthVerdict::NotVerified,
                phishing::AuthVerdict::Failed => AuthVerdict::Failed,
            }),
            spf: c.auth.spf.into(),
            spf_domain: domain(c.auth.spf_domain),
            dkim: c.auth.dkim.into(),
            dkim_domain: domain(c.auth.dkim_domain),
            dmarc: c.auth.dmarc.into(),
            dmarc_domain: domain(c.auth.dmarc_domain),
            display_name_warning: c.display_name_warning.map(|w| DisplayNameWarning {
                claimed: one_line(&w.claimed),
                actual: one_line(&w.actual),
            }),
        }
    }
}

/// The bytes of an inline image.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct InlinePart {
    /// Lower-case `type/subtype`; always an image type.
    pub mime_type: String,
    /// The decoded bytes.
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for InlinePart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print message content (AGENTS.md §4.4).
        f.debug_struct("InlinePart")
            .field("mime_type", &self.mime_type)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

fn storage() -> SyncFailure {
    SyncFailure::Failed {
        kind: SyncErrorKind::Storage,
    }
}

fn not_found() -> SyncFailure {
    SyncFailure::Failed {
        kind: SyncErrorKind::NotFound,
    }
}

/// Percent-encodes everything except RFC 3986 unreserved characters, so a
/// content id is one safe path segment.
fn percent_encode(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
    }
    out
}

fn cid_url(content_id: &str) -> String {
    format!("{CID_URL_PREFIX}{}", percent_encode(content_id))
}

/// Wraps sanitised HTML in a document whose Content-Security-Policy allows
/// nothing but inline styles, `cid:` and `data:` images, and, only when
/// `allow_remote`, `https:` images. No referrer is ever sent.
///
/// The viewport is the device width with no `initial-scale`, so the app's
/// web view can zoom a fixed-width (often 600 px) newsletter out to fit the
/// screen; `initial-scale=1` kept it at full size, cut off on the right
/// (#342).
fn html_document(body: &str, allow_remote: bool) -> String {
    let remote = if allow_remote { " https:" } else { "" };
    format!(
        "<!DOCTYPE html><html><head>\
         <meta charset=\"utf-8\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         img-src {CID_SCHEME}: data:{remote}; style-src 'unsafe-inline'\">\
         <meta name=\"viewport\" content=\"width=device-width\">\
         <style>body{{margin:16px;overflow-wrap:break-word}}\
         img{{max-width:100%;height:auto}}</style>\
         </head><body>{body}</body></html>"
    )
}

fn address(a: mailcrane_mime::Address) -> MessageAddress {
    MessageAddress {
        name: one_line(a.name.as_deref().unwrap_or_default()),
        email: one_line(&a.email),
    }
}

/// Text for display: NUL removed, at most [`MAX_PLAIN_CHARS`] chars.
fn plain_text(text: &str) -> String {
    text.chars()
        .filter(|c| *c != '\0')
        .take(MAX_PLAIN_CHARS)
        .collect()
}

/// Returns one message, ready to show. HTML goes through the sanitiser and
/// comes back as a complete document (see [`MessageBody::Html`]); remote
/// images stay blocked unless `allow_remote`. A body that can't be parsed is
/// shown as its raw text. Reads only the local database.
#[uniffi::export]
pub fn get_message(
    db_path: String,
    message_id: i64,
    allow_remote: bool,
) -> Result<MessageDetail, SyncFailure> {
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let id = MessageId(message_id);
    let header = store
        .get_message_header(id)
        .map_err(|_| storage())?
        .ok_or_else(not_found)?;
    let raw = match header.body_state {
        BodyState::Fetched => store.get_body(id).map_err(|_| storage())?,
        BodyState::None => None,
    };
    let d = &header.details;
    let mut detail = MessageDetail {
        id: message_id,
        folder_id: d.folder_id.0,
        from: MessageAddress {
            name: one_line(d.from_name.as_deref().unwrap_or_default()),
            email: one_line(d.from_addr.as_deref().unwrap_or_default()),
        },
        to: Vec::new(),
        cc: Vec::new(),
        date: d.date,
        subject: display_line(&d.subject),
        is_seen: d.flags.seen,
        is_flagged: d.flags.flagged,
        body: MessageBody::NotDownloaded,
        attachments: Vec::new(),
        sender_check: None,
    };
    let Some(raw) = raw else {
        return Ok(detail);
    };
    detail.attachments = store
        .list_attachments(id)
        .map_err(|_| storage())?
        .into_iter()
        .map(|a| a.details)
        // Inline images are shown in the body, not listed.
        .filter(|a| !(a.is_inline && a.content_id.is_some() && a.mime_type.starts_with("image/")))
        .map(|a| AttachmentInfo {
            name: display_line(a.filename.as_deref().unwrap_or_default()),
            mime_type: one_line(&a.mime_type),
            size: u64::try_from(a.size).unwrap_or(0),
            part_id: a.part_id,
        })
        .collect();
    detail.body = match parse_message(&raw) {
        Ok(parsed) => {
            detail.sender_check = Some(phishing::sender_check(&parsed).into());
            detail.to = parsed.to.into_iter().map(address).collect();
            detail.cc = parsed.cc.into_iter().map(address).collect();
            match parsed.text_html {
                Some(html) => {
                    let opts = SanitizeOptions {
                        allow_remote_content: allow_remote,
                        cid_url,
                    };
                    let clean = sanitize_html(&html, &opts);
                    MessageBody::Html {
                        html: html_document(&clean.html, allow_remote),
                        remote_blocked: clean.remote_resources_blocked,
                        trackers_removed: clean.trackers_removed,
                        trackers: clean.trackers.into_iter().map(TrackerInfo::from).collect(),
                        links: clean
                            .links
                            .into_iter()
                            .map(|l| LinkText {
                                href: l.href,
                                text: one_line(&l.text),
                            })
                            .collect(),
                    }
                }
                None => MessageBody::Plain {
                    text: plain_text(parsed.text_plain.as_deref().unwrap_or_default()),
                },
            }
        }
        // Better to show the source as text than nothing.
        Err(_) => MessageBody::Plain {
            text: plain_text(&String::from_utf8_lossy(&raw)),
        },
    };
    Ok(detail)
}

/// Returns the inline image with Content-ID `content_id` from the stored
/// body of the message, for the app's `mailcrane-cid:` handler. `None` if the
/// body isn't downloaded, no part has that id, or the part is not a
/// supported image type. Reads only the local database.
#[uniffi::export]
pub fn get_inline_part(
    db_path: String,
    message_id: i64,
    content_id: String,
) -> Result<Option<InlinePart>, SyncFailure> {
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let Some(raw) = store
        .get_body(MessageId(message_id))
        .map_err(|_| storage())?
    else {
        return Ok(None);
    };
    let part = mailcrane_mime::inline_part(&raw, &content_id).unwrap_or(None);
    Ok(part
        .filter(|p| INLINE_IMAGE_TYPES.contains(&p.mime_type.as_str()))
        .map(|p| InlinePart {
            mime_type: p.mime_type,
            bytes: p.bytes,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{Flags, NewAccount, NewFolder, NewMessageHeader, Security};

    fn header(folder: FolderId, uid: u32, date: i64) -> NewMessageHeader {
        NewMessageHeader {
            folder_id: folder,
            uid,
            message_id_header: None,
            in_reply_to: None,
            references_json: "[]".into(),
            thread_id: None,
            subject: format!("Message {uid}"),
            from_name: Some("Ana".into()),
            from_addr: Some("ana@example.org".into()),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date,
            internal_date: date,
            size: 100,
            flags: Flags::default(),
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: "Hello".into(),
        }
    }

    /// A database with one account and one folder; returns its path and the
    /// folder. The file is removed when the guard drops.
    fn setup() -> (tempfile::TempDir, String, FolderId) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let account = store
            .insert_account(&NewAccount {
                display_name: "Alice".into(),
                email: "alice@example.org".into(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 587,
                smtp_security: Security::Starttls,
                username: "alice@example.org".into(),
            })
            .expect("account");
        let folder = store
            .upsert_folder(&NewFolder {
                account_id: account,
                remote_name: "INBOX".into(),
                display_name: "Inbox".into(),
                delimiter: Some("/".into()),
                role: None,
                uidvalidity: None,
                uidnext: None,
                highestmodseq: None,
                is_subscribed: true,
                selectable: true,
            })
            .expect("folder");
        (dir, path, folder)
    }

    #[test]
    fn lists_newest_first_in_pages_without_gaps() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let rows: Vec<_> = (1..=7)
            .map(|n| header(folder, n, 1_000 + i64::from(n)))
            .collect();
        store.upsert_message_headers(&rows).expect("insert");
        drop(store);

        let first = list_messages(path.clone(), folder.0, 0, 3).expect("page 1");
        let second = list_messages(path.clone(), folder.0, 3, 3).expect("page 2");
        let third = list_messages(path, folder.0, 6, 3).expect("page 3");
        let subjects: Vec<_> = first
            .iter()
            .chain(&second)
            .chain(&third)
            .map(|m| m.subject.as_str())
            .collect();
        assert_eq!(
            subjects,
            [
                "Message 7",
                "Message 6",
                "Message 5",
                "Message 4",
                "Message 3",
                "Message 2",
                "Message 1"
            ]
        );
    }

    #[test]
    fn maps_flags_attachments_and_missing_sender() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let mut m = header(folder, 1, 1_000);
        m.flags = Flags {
            seen: true,
            flagged: true,
            ..Flags::default()
        };
        m.has_attachments = true;
        m.from_name = None;
        m.from_addr = None;
        store.upsert_message_header(&m).expect("insert");
        drop(store);

        let rows = list_messages(path, folder.0, 0, 10).expect("list");
        let row = rows.first().expect("one row");
        assert!(row.is_seen && row.is_flagged && row.has_attachments);
        assert_eq!(row.from_name, "");
        assert_eq!(row.from_addr, "");
        assert_eq!(row.date, 1_000);
    }

    #[test]
    fn header_text_becomes_one_bounded_line() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let mut m = header(folder, 1, 1_000);
        m.subject = format!("Line one\r\nLine two\0{}", "x".repeat(5_000));
        m.from_name = Some("Ana\nExample".into());
        store.upsert_message_header(&m).expect("insert");
        drop(store);

        let rows = list_messages(path, folder.0, 0, 10).expect("list");
        let row = rows.first().expect("one row");
        assert!(row.subject.starts_with("Line one  Line two "));
        assert!(!row.subject.contains(['\r', '\n', '\0']));
        assert_eq!(row.subject.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(row.from_name, "Ana Example");
    }

    #[test]
    fn limit_is_capped_and_unknown_folder_is_empty() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let rows: Vec<_> = (1..=600).map(|n| header(folder, n, i64::from(n))).collect();
        store.upsert_message_headers(&rows).expect("insert");
        drop(store);

        let page = list_messages(path.clone(), folder.0, 0, u32::MAX).expect("list");
        assert_eq!(page.len(), MAX_PAGE as usize);
        assert_eq!(list_messages(path, 9_999, 0, 10), Ok(Vec::new()));
    }

    #[test]
    fn unopenable_database_is_a_storage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory is not a database file.
        let path = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            list_messages(path, 1, 0, 10),
            Err(SyncFailure::Failed {
                kind: SyncErrorKind::Storage
            })
        );
    }

    // ---- get_message / get_inline_part / mark_read (M1-12) ----

    use crate::sync::{CoreEvent, CoreEventListener, SyncEngine};
    use mailcrane_store::NewAttachment;
    use std::sync::{Arc, Mutex};

    /// Stores `raw` as the body of a new message in `folder`, with the
    /// attachment metadata `parse_message` finds, as `fetch_body` does.
    fn with_body(path: &str, folder: FolderId, uid: u32, raw: &[u8]) -> i64 {
        with_header_and_body(path, &header(folder, uid, 1_000), raw)
    }

    /// [`with_body`] with a header row of the caller's.
    fn with_header_and_body(path: &str, row: &NewMessageHeader, raw: &[u8]) -> i64 {
        let store = Store::open(path).expect("store");
        let id = store.upsert_message_header(row).expect("insert");
        let attachments: Vec<NewAttachment> = parse_message(raw)
            .map(|p| p.attachments)
            .unwrap_or_default()
            .into_iter()
            .map(|a| NewAttachment {
                part_id: a.part_id,
                filename: a.filename,
                mime_type: a.mime_type,
                size: i64::try_from(a.size).unwrap_or(i64::MAX),
                content_id: a.content_id,
                is_inline: a.disposition == mailcrane_mime::Disposition::Inline,
            })
            .collect();
        assert!(store
            .store_parsed_body(id, raw, 1, "", &attachments)
            .expect("body"));
        id.0
    }

    fn fixture(dir: &str, name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../../../tests/fixtures/{dir}/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(path).expect("fixture")
    }

    fn html_message(html: &str) -> Vec<u8> {
        format!(
            "From: Ana <ana@example.org>\r\nTo: Bob <bob@example.com>\r\n\
             Cc: Cy <cy@example.net>\r\nSubject: Hi\r\nMIME-Version: 1.0\r\n\
             Content-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n"
        )
        .into_bytes()
    }

    fn html_of(detail: &MessageDetail) -> (&str, u32) {
        match &detail.body {
            MessageBody::Html {
                html,
                remote_blocked,
                ..
            } => (html.as_str(), *remote_blocked),
            other => panic!("expected HTML, got {other:?}"),
        }
    }

    #[test]
    fn body_not_downloaded_yet() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let id = store
            .upsert_message_header(&header(folder, 1, 1_000))
            .expect("insert");
        drop(store);
        let m = get_message(path, id.0, false).expect("message");
        assert_eq!(m.body, MessageBody::NotDownloaded);
        assert_eq!(m.subject, "Message 1");
        assert_eq!(m.from.email, "ana@example.org");
        assert_eq!(m.folder_id, folder.0);
        assert!(!m.is_seen);
    }

    #[test]
    fn plain_text_message() {
        let (_dir, path, folder) = setup();
        let id = with_body(&path, folder, 1, &fixture("mime", "plain-ascii.eml"));
        let m = get_message(path, id, false).expect("message");
        match &m.body {
            MessageBody::Plain { text } => assert!(text.starts_with("Hello Bob,")),
            other => panic!("expected plain text, got {other:?}"),
        }
        assert_eq!(m.to.len(), 1);
        assert_eq!(m.to[0].email, "bob@example.com");
        assert!(m.attachments.is_empty());
    }

    #[test]
    fn html_is_sanitised_in_a_locked_down_document() {
        let (_dir, path, folder) = setup();
        let raw = html_message(
            "<body onload=\"alert(1)\"><p>Hi</p><script>alert(2)</script>\
             <img src=\"https://example.com/a.png\" width=\"100\" height=\"50\"></body>",
        );
        let id = with_body(&path, folder, 1, &raw);
        let m = get_message(path.clone(), id, false).expect("message");
        let (html, blocked) = html_of(&m);
        assert!(html.contains("<p>Hi</p>"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("alert"));
        assert!(!html.contains("onload"));
        assert!(
            !html.contains("example.com"),
            "remote image must be removed"
        );
        assert_eq!(blocked, 1);
        assert!(html.contains("<meta name=\"referrer\" content=\"no-referrer\">"));
        // #342: no initial-scale, so wide mail can be zoomed out to fit.
        assert!(html.contains("<meta name=\"viewport\" content=\"width=device-width\">"));
        assert!(!html.contains("initial-scale"));
        assert!(html.contains(
            "content=\"default-src 'none'; img-src mailcrane-cid: data:; \
             style-src 'unsafe-inline'\""
        ));
        assert!(!html.contains("https:"));
        assert_eq!(m.cc[0].email, "cy@example.net");

        let m = get_message(path, id, true).expect("message");
        let (html, blocked) = html_of(&m);
        assert_eq!(blocked, 0);
        assert!(html.contains("src=\"https://example.com/a.png\""));
        assert!(html.contains("img-src mailcrane-cid: data: https:;"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn sender_check_reads_only_the_providers_header() {
        let (_dir, path, folder) = setup();
        // `above`: what the provider added above its Received header;
        // `below`: what the sender wrote.
        let raw = |above: &str, below: &str| {
            format!(
                "{above}Received: from out.example.net by mx2.example.org with ESMTPS; \
                 Wed, 25 Nov 2026 08:00:00 +0000\r\n{below}\
                 From: Ana <ana@example.net>\r\nTo: bob@example.org\r\n\
                 Subject: Hi\r\n\r\nHello\r\n"
            )
            .into_bytes()
        };
        let forged = "Authentication-Results: mx.example.org; dmarc=pass\r\n";
        let id = with_body(
            &path,
            folder,
            1,
            &raw(
                "Authentication-Results: mx1.example.org; spf=pass smtp.mailfrom=example.net; \
                 dkim=fail header.d=example.net; dmarc=fail header.from=example.net\r\n",
                forged,
            ),
        );
        let check = get_message(path.clone(), id, false)
            .expect("message")
            .sender_check
            .expect("checked");
        assert_eq!(
            check,
            SenderCheck {
                trusted: true,
                verdict: Some(AuthVerdict::Failed),
                spf: AuthResult::Pass,
                spf_domain: Some("example.net".into()),
                dkim: AuthResult::Fail,
                dkim_domain: Some("example.net".into()),
                dmarc: AuthResult::Fail,
                dmarc_domain: Some("example.net".into()),
                display_name_warning: None,
            }
        );
        // The provider added nothing: the sender's header is not trusted.
        let id = with_body(&path, folder, 2, &raw("", forged));
        let check = get_message(path.clone(), id, false)
            .expect("message")
            .sender_check
            .expect("checked");
        assert!(!check.trusted);
        assert_eq!(check.verdict, None);
    }

    #[test]
    fn display_name_warning_reaches_the_apps() {
        let (_dir, path, folder) = setup();
        let raw = b"From: \"ceo@example.com\" <attacker@example.net>\r\nTo: bob@example.org\r\n\
                    Subject: Hi\r\n\r\nHello\r\n";
        let id = with_body(&path, folder, 1, raw);
        let check = get_message(path, id, false)
            .expect("message")
            .sender_check
            .expect("checked");
        // No Authentication-Results: no chip, but the name is still checked.
        assert_eq!(check.verdict, None);
        assert_eq!(
            check.display_name_warning,
            Some(DisplayNameWarning {
                claimed: "ceo@example.com".into(),
                actual: "attacker@example.net".into(),
            })
        );
    }

    #[test]
    fn links_come_with_their_text() {
        let (_dir, path, folder) = setup();
        let raw = html_message(
            "<p><a href=\"https://example.net/login\">https://example.com/login</a>\
             <a href=\"javascript:alert(1)\">bad</a></p>",
        );
        let id = with_body(&path, folder, 1, &raw);
        let m = get_message(path, id, false).expect("message");
        let MessageBody::Html { links, .. } = &m.body else {
            panic!("expected HTML");
        };
        assert_eq!(
            links,
            &vec![LinkText {
                href: "https://example.net/login".into(),
                text: "https://example.com/login".into(),
            }]
        );
    }

    #[test]
    fn trackers_are_listed_for_the_apps() {
        let (_dir, path, folder) = setup();
        let raw = html_message(
            "<p>Hi</p><img src=\"https://us5.list-manage.com/track/open.php?u=1\">             <img src=\"https://us5.list-manage.com/track/open.php?u=2\">             <img src=\"https://stats.example.net/o.gif\" width=\"1\" height=\"1\">",
        );
        let id = with_body(&path, folder, 1, &raw);
        for allow_remote in [false, true] {
            let m = get_message(path.clone(), id, allow_remote).expect("message");
            let MessageBody::Html {
                html,
                trackers,
                trackers_removed,
                ..
            } = &m.body
            else {
                panic!("expected HTML");
            };
            assert!(!html.contains("list-manage"));
            assert_eq!(*trackers_removed, 3);
            assert_eq!(
                trackers,
                &vec![
                    TrackerInfo {
                        service: Some("Mailchimp".into()),
                        domain: "list-manage.com".into(),
                    },
                    TrackerInfo {
                        service: None,
                        domain: "stats.example.net".into(),
                    },
                ]
            );
        }
    }

    #[test]
    fn every_xss_fixture_comes_out_without_script() {
        let (_dir, path, folder) = setup();
        let dir = format!(
            "{}/../../../tests/fixtures/html",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut uid = 0;
        for entry in std::fs::read_dir(dir).expect("fixtures") {
            let entry = entry.expect("entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("xss-") {
                continue;
            }
            uid += 1;
            let html = std::fs::read_to_string(entry.path()).expect("read");
            let id = with_body(&path, folder, uid, &html_message(&html));
            for allow_remote in [false, true] {
                let m = get_message(path.clone(), id, allow_remote).expect("message");
                let (doc, _) = html_of(&m);
                let lower = doc.to_ascii_lowercase();
                for bad in ["<script", "javascript:", "onload=", "onerror=", "<iframe"] {
                    assert!(!lower.contains(bad), "{name}: {bad} survived");
                }
            }
        }
        assert!(uid >= 40, "only {uid} fixtures");
    }

    #[test]
    fn cid_images_are_rewritten_and_served_from_the_body() {
        let (_dir, path, folder) = setup();
        let id = with_body(
            &path,
            folder,
            1,
            &fixture("mime", "multipart-related-cid.eml"),
        );
        let m = get_message(path.clone(), id, false).expect("message");
        let (html, blocked) = html_of(&m);
        assert!(html.contains("src=\"mailcrane-cid://part/logo%40example.org\""));
        assert_eq!(blocked, 0);
        // The inline image is shown in the body, not listed as an attachment.
        assert!(m.attachments.is_empty());

        let part = get_inline_part(path.clone(), id, "logo@example.org".into())
            .expect("read")
            .expect("part");
        assert_eq!(part.mime_type, "image/png");
        assert_eq!(part.bytes, b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            get_inline_part(path.clone(), id, "nope@example.org".into()),
            Ok(None)
        );
        // Another message's id never finds this message's parts.
        assert_eq!(
            get_inline_part(path, id + 1, "logo@example.org".into()),
            Ok(None)
        );
    }

    #[test]
    fn inline_part_serves_only_raster_images() {
        let (_dir, path, folder) = setup();
        let raw = b"Subject: x\r\nMIME-Version: 1.0\r\n\
            Content-Type: multipart/related; boundary=\"b\"\r\n\r\n\
            --b\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:s@example.org\">\r\n\
            --b\r\nContent-Type: image/svg+xml\r\nContent-Disposition: inline\r\n\
            Content-ID: <s@example.org>\r\n\r\n<svg onload=\"alert(1)\"/>\r\n\
            --b\r\nContent-Type: text/html\r\nContent-Disposition: inline\r\n\
            Content-ID: <h@example.org>\r\n\r\n<script>alert(1)</script>\r\n--b--\r\n";
        let id = with_body(&path, folder, 1, raw);
        assert_eq!(
            get_inline_part(path.clone(), id, "s@example.org".into()),
            Ok(None)
        );
        assert_eq!(get_inline_part(path, id, "h@example.org".into()), Ok(None));
    }

    #[test]
    fn attachments_are_listed_with_name_type_and_size() {
        let (_dir, path, folder) = setup();
        let id = with_body(
            &path,
            folder,
            1,
            &fixture("mime", "multipart-mixed-pdf.eml"),
        );
        let m = get_message(path, id, false).expect("message");
        assert_eq!(m.attachments.len(), 1);
        let a = &m.attachments[0];
        assert_eq!(a.mime_type, "application/pdf");
        assert!(a.name.ends_with(".pdf"));
        assert!(a.size > 0);
    }

    #[test]
    fn display_line_removes_bidi_controls() {
        assert_eq!(display_line("invoice\u{202e}fdp.exe"), "invoicefdp.exe");
        assert_eq!(display_line("a\u{2067}b\u{2069}\r\nc"), "ab  c");
        assert_eq!(display_line("שלום"), "שלום", "right-to-left letters stay");
    }

    #[test]
    fn bidi_controls_never_reach_the_subject_or_attachment_name() {
        // #315: an RLO in the subject or file name would reverse what the
        // list row, the message header and notifications show.
        let (_dir, path, folder) = setup();
        let raw = fixture("mime", "bidi-subject-attachment.eml");
        let parsed = parse_message(&raw).expect("parse");
        let subject = parsed.subject.expect("subject");
        assert!(
            subject.chars().any(mailcrane_mime::is_bidi_control),
            "stored as sent"
        );
        let mut row = header(folder, 1, 1_000);
        row.subject = subject;
        let id = with_header_and_body(&path, &row, &raw);

        let want = "Invoice fdp.exe for March 2026 paid";
        let m = get_message(path.clone(), id, false).expect("message");
        assert_eq!(m.subject, want);
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].name, "invoicefdp.exe");
        let list = list_messages(path, folder.0, 0, 10).expect("list");
        assert_eq!(list[0].subject, want);
    }

    #[test]
    fn unparseable_body_is_shown_as_text() {
        let (_dir, path, folder) = setup();
        let id = with_body(&path, folder, 1, b"");
        let m = get_message(path, id, false).expect("message");
        assert_eq!(
            m.body,
            MessageBody::Plain {
                text: String::new()
            }
        );
    }

    #[test]
    fn unknown_message_is_not_found() {
        let (_dir, path, _folder) = setup();
        assert_eq!(get_message(path.clone(), 999, false), Err(not_found()));
        assert_eq!(get_inline_part(path, 999, "a@example.org".into()), Ok(None));
    }

    #[test]
    fn percent_encoding_keeps_one_safe_segment() {
        assert_eq!(percent_encode("a.b-c_d~e"), "a.b-c_d~e");
        assert_eq!(
            percent_encode("x@y/z?#\"<> %"),
            "x%40y%2Fz%3F%23%22%3C%3E%20%25"
        );
        assert_eq!(percent_encode("é"), "%C3%A9");
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<CoreEvent>>);

    impl CoreEventListener for Events {
        fn on_event(&self, event: CoreEvent) {
            if let Ok(mut e) = self.0.lock() {
                e.push(event);
            }
        }
    }

    #[test]
    fn mark_read_sets_seen_once_and_notifies() {
        let (_dir, path, folder) = setup();
        let store = Store::open(&path).expect("store");
        let id = store
            .upsert_message_header(&header(folder, 1, 1_000))
            .expect("insert");
        drop(store);
        let events = Arc::new(Events::default());
        let engine = SyncEngine::new(path.clone(), events.clone());

        assert_eq!(engine.mark_read(id.0), Ok(()));
        assert!(get_message(path.clone(), id.0, false).expect("m").is_seen);
        // Already read: nothing changes and no event.
        assert_eq!(engine.mark_read(id.0), Ok(()));
        let seen = events.0.lock().expect("events").clone();
        assert_eq!(
            seen,
            [CoreEvent::MessagesChanged {
                folder_id: folder.0
            }]
        );
        assert_eq!(engine.mark_read(999), Err(not_found()));
    }
}
