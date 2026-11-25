//! The folder list, as the apps show it in the folder drawer: `list_folders`
//! returns an account's folders already in display order, with their depth
//! and unread counts.

use std::cmp::Ordering;
use std::collections::HashSet;

use mailcrane_store::{AccountId, Folder, FolderRole as StoreRole, Store};

use crate::messages::one_line;
use crate::sync::{SyncErrorKind, SyncFailure};

/// Folders nested deeper than this are shown at this depth, so a hostile
/// server can't push names off the screen.
pub const MAX_DEPTH: u32 = 8;

/// What a standard folder is for. The apps show a translated name and an
/// icon for these instead of the server's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
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
    AllMail,
}

impl From<StoreRole> for FolderRole {
    fn from(role: StoreRole) -> Self {
        match role {
            StoreRole::Inbox => Self::Inbox,
            StoreRole::Drafts => Self::Drafts,
            StoreRole::Sent => Self::Sent,
            StoreRole::Archive => Self::Archive,
            StoreRole::Spam => Self::Spam,
            StoreRole::Trash => Self::Trash,
            StoreRole::All => Self::AllMail,
        }
    }
}

impl FolderRole {
    /// Position among the standard folders, which come before all others.
    fn rank(self) -> u8 {
        match self {
            Self::Inbox => 0,
            Self::Drafts => 1,
            Self::Sent => 2,
            Self::Archive => 3,
            Self::Spam => 4,
            Self::Trash => 5,
            Self::AllMail => 6,
        }
    }
}

/// One row of the folder list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FolderSummary {
    /// Row id of the folder.
    pub id: i64,
    /// Decoded name to show, relative to the folder it's nested under. For
    /// standard folders the apps show a translated name instead.
    pub display_name: String,
    /// Standard role, if the folder has one.
    pub role: Option<FolderRole>,
    /// How far to indent: 0 for top-level and standard folders, at most
    /// [`MAX_DEPTH`].
    pub depth: u32,
    /// Messages in the folder that are not `\Seen`.
    pub unread_count: u32,
}

/// Lists the account's folders in display order: Inbox, Drafts, Sent,
/// Archive, Spam, Trash, All mail, then every other folder alphabetically
/// (ignoring case) with children right under their parent. An unknown
/// account gives an empty list. Reads only the local database.
///
/// `db_path` is the same database file the app gives `SyncEngine`.
#[uniffi::export]
pub fn list_folders(db_path: String, account_id: i64) -> Result<Vec<FolderSummary>, SyncFailure> {
    let storage = || SyncFailure::Failed {
        kind: SyncErrorKind::Storage,
    };
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let folders = store
        .list_folders(AccountId(account_id))
        .map_err(|_| storage())?;
    let mut rows = Vec::with_capacity(folders.len());
    for (folder, place) in arrange(&folders) {
        let unread = store.count_unread(folder.id).map_err(|_| storage())?;
        rows.push(FolderSummary {
            id: folder.id.0,
            display_name: one_line(&place.name),
            role: folder.details.role.map(FolderRole::from),
            depth: place.depth,
            unread_count: u32::try_from(unread).unwrap_or(u32::MAX),
        });
    }
    Ok(rows)
}

/// Where a folder sits in the list: its indent and the name to show there.
#[derive(Debug, PartialEq, Eq)]
struct Place {
    depth: u32,
    name: String,
}

/// The folder's decoded path split at its delimiter.
fn segments(folder: &Folder) -> Vec<&str> {
    let name = folder.details.display_name.as_str();
    match folder
        .details
        .delimiter
        .as_deref()
        .filter(|d| !d.is_empty())
    {
        Some(d) => name.split(d).collect(),
        None => vec![name],
    }
}

/// Sorts the folders into display order and works out each one's place.
fn arrange(folders: &[Folder]) -> Vec<(&Folder, Place)> {
    let (mut standard, mut other): (Vec<&Folder>, Vec<&Folder>) =
        folders.iter().partition(|f| f.details.role.is_some());

    standard.sort_by(|a, b| {
        let rank = |f: &Folder| f.details.role.map(|r| FolderRole::from(r).rank());
        rank(a)
            .cmp(&rank(b))
            .then_with(|| by_path(a, b))
            .then_with(|| a.id.cmp(&b.id))
    });
    // Comparing paths segment by segment puts each child right after its
    // parent and before the parent's next sibling.
    other.sort_by(|a, b| by_path(a, b).then_with(|| a.id.cmp(&b.id)));

    // A folder is indented under another only if that one is listed among
    // the other folders; a standard parent (as in `INBOX/Receipts`) is
    // shown at the top, so its children are not indented under it.
    let shown: HashSet<Vec<&str>> = other.iter().map(|f| segments(f)).collect();

    let mut out = Vec::with_capacity(folders.len());
    for f in standard {
        let name = segments(f)
            .last()
            .filter(|s| !s.is_empty())
            .map_or_else(|| f.details.display_name.clone(), |s| (*s).to_owned());
        out.push((f, Place { depth: 0, name }));
    }
    for f in other {
        let path = segments(f);
        let ancestors: Vec<usize> = (1..path.len())
            .filter(|&n| path.get(..n).is_some_and(|p| shown.contains(p)))
            .collect();
        let depth = u32::try_from(ancestors.len())
            .unwrap_or(u32::MAX)
            .min(MAX_DEPTH);
        // Shown relative to the nearest listed ancestor; if folders in
        // between are missing, their names stay in the label.
        let start = ancestors.last().copied().unwrap_or(0);
        let delimiter = f.details.delimiter.as_deref().unwrap_or_default();
        let name = path.get(start..).map(|rest| rest.join(delimiter));
        let name = name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| f.details.display_name.clone());
        out.push((f, Place { depth, name }));
    }
    out
}

/// Orders folders by their path, segment by segment, ignoring case; the
/// exact server name breaks ties so the order is stable.
fn by_path(a: &Folder, b: &Folder) -> Ordering {
    let key =
        |f: &Folder| -> Vec<String> { segments(f).iter().map(|s| s.to_lowercase()).collect() };
    key(a)
        .cmp(&key(b))
        .then_with(|| a.details.remote_name.cmp(&b.details.remote_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{Flags, FolderId, NewAccount, NewFolder, NewMessageHeader, Security};

    fn new_account(store: &Store) -> AccountId {
        store
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
            .expect("account")
    }

    fn folder(account: AccountId, name: &str, role: Option<StoreRole>) -> NewFolder {
        NewFolder {
            account_id: account,
            remote_name: name.into(),
            display_name: name.into(),
            delimiter: Some("/".into()),
            role,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        }
    }

    fn header(folder: FolderId, uid: u32, seen: bool) -> NewMessageHeader {
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
            date: 1_000,
            internal_date: 1_000,
            size: 100,
            flags: Flags {
                seen,
                ..Flags::default()
            },
            keywords_json: "[]".into(),
            has_attachments: false,
            snippet: "Hello".into(),
        }
    }

    /// A database holding one account with `folders` (name, role); returns
    /// the guard, path, account and folder ids in the same order.
    fn setup(
        folders: &[(&str, Option<StoreRole>)],
    ) -> (tempfile::TempDir, String, AccountId, Vec<FolderId>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let account = new_account(&store);
        let rows: Vec<NewFolder> = folders
            .iter()
            .map(|(name, role)| folder(account, name, *role))
            .collect();
        let ids = store.upsert_folders(&rows).expect("folders");
        (dir, path, account, ids)
    }

    fn names_and_depths(rows: &[FolderSummary]) -> Vec<(String, u32)> {
        rows.iter()
            .map(|r| (r.display_name.clone(), r.depth))
            .collect()
    }

    #[test]
    fn sorts_standard_folders_first_then_others_nested_alphabetically() {
        // Deliberately shuffled, with mixed case and a standard folder
        // nested under INBOX.
        let (_dir, path, account, _) = setup(&[
            ("Work/b-team", None),
            ("Trash", Some(StoreRole::Trash)),
            ("archive-2020", None),
            ("INBOX/Sent", Some(StoreRole::Sent)),
            ("Work", None),
            ("Spam", Some(StoreRole::Spam)),
            ("Work/A-team/2026", None),
            ("Drafts", Some(StoreRole::Drafts)),
            ("Archive", Some(StoreRole::Archive)),
            ("INBOX", Some(StoreRole::Inbox)),
            ("Work/A-team", None),
            ("Workshop", None),
            ("[Gmail]/All Mail", Some(StoreRole::All)),
            ("Banana", None),
        ]);
        let rows = list_folders(path, account.0).expect("list");
        assert_eq!(
            names_and_depths(&rows),
            [
                ("INBOX".to_string(), 0),
                ("Drafts".to_string(), 0),
                ("Sent".to_string(), 0),
                ("Archive".to_string(), 0),
                ("Spam".to_string(), 0),
                ("Trash".to_string(), 0),
                ("All Mail".to_string(), 0),
                ("archive-2020".to_string(), 0),
                ("Banana".to_string(), 0),
                ("Work".to_string(), 0),
                ("A-team".to_string(), 1),
                ("2026".to_string(), 2),
                ("b-team".to_string(), 1),
                ("Workshop".to_string(), 0),
            ]
        );
        let roles: Vec<_> = rows.iter().map(|r| r.role).collect();
        assert_eq!(
            roles.get(..8),
            Some(
                &[
                    Some(FolderRole::Inbox),
                    Some(FolderRole::Drafts),
                    Some(FolderRole::Sent),
                    Some(FolderRole::Archive),
                    Some(FolderRole::Spam),
                    Some(FolderRole::Trash),
                    Some(FolderRole::AllMail),
                    None,
                ][..]
            )
        );
    }

    #[test]
    fn children_of_unlisted_or_standard_parents_are_not_indented() {
        let (_dir, path, account, _) = setup(&[
            ("INBOX", Some(StoreRole::Inbox)),
            ("INBOX/Receipts", None),
            ("Projects/2026/Q1", None),
            ("Projects", None),
        ]);
        let rows = list_folders(path, account.0).expect("list");
        assert_eq!(
            names_and_depths(&rows),
            [
                ("INBOX".to_string(), 0),
                ("INBOX/Receipts".to_string(), 0),
                ("Projects".to_string(), 0),
                // "Projects/2026" isn't listed, so its name stays in the label.
                ("2026/Q1".to_string(), 1),
            ]
        );
    }

    #[test]
    fn counts_unread_per_folder() {
        let (_dir, path, account, ids) =
            setup(&[("INBOX", Some(StoreRole::Inbox)), ("Lists", None)]);
        let (inbox, lists) = (ids[0], ids[1]);
        let store = Store::open(&path).expect("store");
        store
            .upsert_message_headers(&[
                header(inbox, 1, false),
                header(inbox, 2, true),
                header(inbox, 3, false),
                header(lists, 1, true),
            ])
            .expect("messages");
        drop(store);

        let rows = list_folders(path, account.0).expect("list");
        let unread: Vec<_> = rows.iter().map(|r| (r.id, r.unread_count)).collect();
        assert_eq!(unread, [(inbox.0, 2), (lists.0, 0)]);
    }

    #[test]
    fn hostile_names_are_one_line_and_depth_is_capped() {
        let deep: Vec<String> = (1..=20)
            .map(|n| {
                (1..=n)
                    .map(|i| format!("d{i:02}"))
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        let mut folders: Vec<(&str, Option<StoreRole>)> =
            deep.iter().map(|n| (n.as_str(), None)).collect();
        folders.push(("Evil\r\nName\0", None));
        folders.push(("Trailing/", None));
        let (_dir, path, account, _) = setup(&folders);

        let rows = list_folders(path, account.0).expect("list");
        assert!(rows.iter().all(|r| r.depth <= MAX_DEPTH));
        assert_eq!(rows.iter().map(|r| r.depth).max(), Some(MAX_DEPTH));
        assert!(rows
            .iter()
            .all(|r| !r.display_name.contains(['\r', '\n', '\0'])));
        assert!(rows.iter().any(|r| r.display_name == "Evil  Name "));
        // An empty last segment keeps the whole name.
        assert!(rows.iter().any(|r| r.display_name == "Trailing/"));
    }

    #[test]
    fn folders_without_a_delimiter_are_flat() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let account = new_account(&store);
        let mut a = folder(account, "a/b", None);
        a.delimiter = None;
        let mut b = folder(account, "a", None);
        b.delimiter = None;
        store.upsert_folders(&[a, b]).expect("folders");
        drop(store);

        let rows = list_folders(path, account.0).expect("list");
        assert_eq!(
            names_and_depths(&rows),
            [("a".to_string(), 0), ("a/b".to_string(), 0)]
        );
    }

    #[test]
    fn unknown_account_is_empty_and_bad_database_is_a_storage_error() {
        let (_dir, path, _, _) = setup(&[("INBOX", Some(StoreRole::Inbox))]);
        assert_eq!(list_folders(path, 9_999), Ok(Vec::new()));

        let dir = tempfile::tempdir().expect("tempdir");
        let not_a_db = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            list_folders(not_a_db, 1),
            Err(SyncFailure::Failed {
                kind: SyncErrorKind::Storage
            })
        );
    }
}
