//! Plain data types stored by the crate.

use crate::error::StoreError;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub i64);
    };
}

id_type!(
    /// Row id of an account.
    AccountId
);
id_type!(
    /// Row id of a folder.
    FolderId
);
id_type!(
    /// Row id of a thread.
    ThreadId
);
id_type!(
    /// Row id of a message.
    MessageId
);
id_type!(
    /// Row id of an attachment.
    AttachmentId
);

/// Connection security for IMAP and SMTP. Plain text is not an option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Implicit TLS.
    Tls,
    /// STARTTLS upgrade.
    Starttls,
}

impl Security {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Security::Tls => "tls",
            Security::Starttls => "starttls",
        }
    }
    pub(crate) fn parse(s: &str, column: &'static str) -> Result<Self, StoreError> {
        match s {
            "tls" => Ok(Security::Tls),
            "starttls" => Ok(Security::Starttls),
            _ => Err(StoreError::Corrupt { column }),
        }
    }
}

/// Special-use role of a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderRole {
    /// Inbox.
    Inbox,
    /// Drafts.
    Drafts,
    /// Sent mail.
    Sent,
    /// Archive.
    Archive,
    /// Spam or junk.
    Spam,
    /// Trash.
    Trash,
    /// All mail.
    All,
}

impl FolderRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Drafts => "drafts",
            FolderRole::Sent => "sent",
            FolderRole::Archive => "archive",
            FolderRole::Spam => "spam",
            FolderRole::Trash => "trash",
            FolderRole::All => "all",
        }
    }
    pub(crate) fn parse(s: &str) -> Result<Self, StoreError> {
        Ok(match s {
            "inbox" => FolderRole::Inbox,
            "drafts" => FolderRole::Drafts,
            "sent" => FolderRole::Sent,
            "archive" => FolderRole::Archive,
            "spam" => FolderRole::Spam,
            "trash" => FolderRole::Trash,
            "all" => FolderRole::All,
            _ => return Err(StoreError::Corrupt { column: "role" }),
        })
    }
}

/// An account to insert. Holds no password or token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    /// Name shown in the UI.
    pub display_name: String,
    /// Address of the account.
    pub email: String,
    /// IMAP host.
    pub imap_host: String,
    /// IMAP port.
    pub imap_port: u16,
    /// IMAP connection security.
    pub imap_security: Security,
    /// SMTP host.
    pub smtp_host: String,
    /// SMTP port.
    pub smtp_port: u16,
    /// SMTP connection security.
    pub smtp_security: Security,
    /// Login name.
    pub username: String,
}

/// A stored account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Row id.
    pub id: AccountId,
    /// Account fields.
    pub details: NewAccount,
    /// Unix seconds, UTC.
    pub created_at: i64,
}

/// A virtual folder's kind (#390): a folder whose contents follow from a flag
/// or the server's own judgement, such as Gmail's Starred and Important.
/// Kept apart from [`FolderRole`] on purpose: it only gives the folder a name,
/// an icon and a place among the system folders, and nothing that moves,
/// archives or deletes by role can pick it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VirtualRole {
    /// Starred (flagged) mail, RFC 6154 `\Flagged`.
    Starred,
    /// Gmail's Important, Gmail's own `\Important` attribute.
    Important,
}

impl VirtualRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            VirtualRole::Starred => "starred",
            VirtualRole::Important => "important",
        }
    }

    /// The stored text's kind. Unknown text (say, from a newer app) is none,
    /// never an error: the folder just shows as an ordinary one.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "starred" => Some(VirtualRole::Starred),
            "important" => Some(VirtualRole::Important),
            _ => None,
        }
    }
}

/// A folder to insert or update, keyed by `(account, remote_name)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFolder {
    /// Owning account.
    pub account_id: AccountId,
    /// Name exactly as on the server.
    pub remote_name: String,
    /// Decoded name for display.
    pub display_name: String,
    /// Hierarchy delimiter.
    pub delimiter: Option<String>,
    /// Special-use role.
    pub role: Option<FolderRole>,
    /// IMAP UIDVALIDITY.
    pub uidvalidity: Option<i64>,
    /// IMAP UIDNEXT.
    pub uidnext: Option<i64>,
    /// IMAP HIGHESTMODSEQ.
    pub highestmodseq: Option<i64>,
    /// Whether the folder is subscribed.
    pub is_subscribed: bool,
    /// Whether the folder can be opened. `false` for a `\Noselect` folder
    /// (RFC 9051 §7.3.1) such as Gmail's `[Gmail]` parent: it only holds
    /// other folders and is never `SELECT`ed.
    pub selectable: bool,
    /// A virtual folder's kind, such as Gmail's Starred (#390). Such a
    /// folder has no [`NewFolder::role`].
    pub virtual_role: Option<VirtualRole>,
}

/// A stored folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Row id.
    pub id: FolderId,
    /// Folder fields.
    pub details: NewFolder,
}

/// System flags of a message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    /// `\Seen`.
    pub seen: bool,
    /// `\Flagged`.
    pub flagged: bool,
    /// `\Answered`.
    pub answered: bool,
    /// `\Draft`.
    pub draft: bool,
    /// `\Deleted`.
    pub deleted: bool,
}

/// A message header to insert or update, keyed by `(folder, uid)`.
/// The `*_json` fields hold JSON text produced by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessageHeader {
    /// Folder holding the message.
    pub folder_id: FolderId,
    /// IMAP UID.
    pub uid: u32,
    /// `Message-ID` header.
    pub message_id_header: Option<String>,
    /// `In-Reply-To` header.
    pub in_reply_to: Option<String>,
    /// JSON array of referenced ids.
    pub references_json: String,
    /// Thread, if assigned.
    pub thread_id: Option<ThreadId>,
    /// Subject.
    pub subject: String,
    /// Sender display name.
    pub from_name: Option<String>,
    /// Sender address.
    pub from_addr: Option<String>,
    /// JSON array of recipients.
    pub to_json: String,
    /// JSON array of Cc recipients.
    pub cc_json: String,
    /// Header date, unix seconds UTC.
    pub date: i64,
    /// Server arrival date, unix seconds UTC.
    pub internal_date: i64,
    /// Size in bytes.
    pub size: i64,
    /// System flags.
    pub flags: Flags,
    /// JSON array of keywords.
    pub keywords_json: String,
    /// Whether the message has attachments.
    pub has_attachments: bool,
    /// Short preview text.
    pub snippet: String,
}

/// The flags the server now reports for a stored message, keyed by
/// `(folder, uid)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFlags {
    /// IMAP UID.
    pub uid: u32,
    /// System flags.
    pub flags: Flags,
    /// JSON array of keywords.
    pub keywords_json: String,
}

/// Whether the raw body is cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyState {
    /// Not downloaded.
    None,
    /// Cached in `message_bodies`.
    Fetched,
}

/// A stored message header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHeader {
    /// Row id.
    pub id: MessageId,
    /// Header fields.
    pub details: NewMessageHeader,
    /// Body cache state.
    pub body_state: BodyState,
}

/// Attachment metadata to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAttachment {
    /// MIME part path.
    pub part_id: String,
    /// File name, if any.
    pub filename: Option<String>,
    /// Content type.
    pub mime_type: String,
    /// Size in bytes.
    pub size: i64,
    /// Content-ID, if any.
    pub content_id: Option<String>,
    /// Whether the part is inline.
    pub is_inline: bool,
}

/// Stored attachment metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentMeta {
    /// Row id.
    pub id: AttachmentId,
    /// Owning message.
    pub message_id: MessageId,
    /// Attachment fields.
    pub details: NewAttachment,
}
