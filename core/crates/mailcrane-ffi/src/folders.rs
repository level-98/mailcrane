//! The folder list, as the apps show it in the folder drawer: `list_folders`
//! returns an account's folders already in display order, with their depth
//! and unread counts.

use std::cmp::Ordering;
use std::collections::HashSet;

use mailcrane_store::{AccountId, Folder, FolderRole as StoreRole, NewFolder, Store, VirtualRole};

use crate::messages::one_line;
use crate::sync::{SyncErrorKind, SyncFailure};
use mailcrane_core::folder_admin;

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
    /// Starred (flagged) mail, a virtual folder such as Gmail's (#390).
    Starred,
    /// Gmail's Important, a virtual folder (#390).
    Important,
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
    /// What the apps show the folder as: its role, or for a virtual folder
    /// such as Gmail's Starred, that (#390). The store keeps the two apart,
    /// so a virtual folder never has a role that actions look up.
    fn of(details: &NewFolder) -> Option<Self> {
        details.role.map(Self::from).or(match details.virtual_role {
            Some(VirtualRole::Starred) => Some(Self::Starred),
            Some(VirtualRole::Important) => Some(Self::Important),
            None => None,
        })
    }

    /// Position among the standard folders, which come before all others.
    /// Starred and Important follow the inbox, where Gmail shows them (#390).
    fn rank(self) -> u8 {
        match self {
            Self::Inbox => 0,
            Self::Starred => 1,
            Self::Important => 2,
            Self::Drafts => 3,
            Self::Sent => 4,
            Self::Archive => 5,
            Self::Spam => 6,
            Self::Trash => 7,
            Self::AllMail => 8,
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
    /// What the apps show the folder as: its standard role, or for a virtual
    /// folder `Starred` or `Important` (#390). Not "can take moves": see
    /// [`FolderSummary::can_take_moves`].
    pub role: Option<FolderRole>,
    /// How far to indent: 0 for top-level and standard folders, at most
    /// [`MAX_DEPTH`].
    pub depth: u32,
    /// Messages in the folder that are not `\Seen`.
    pub unread_count: u32,
    /// Whether it can be renamed, moved and deleted (M4-22): not a standard
    /// folder or one holding one, and the account supports it.
    #[uniffi(default = false)]
    pub can_change: bool,
    /// Whether messages can be moved into it. `false` for a virtual folder
    /// such as Gmail's Starred or Important (#390): the core refuses such a
    /// move or rule, and the apps leave it out of their pickers.
    #[uniffi(default = true)]
    pub can_take_moves: bool,
}

/// Lists the account's folders in display order: Inbox, Starred, Important,
/// Drafts, Sent, Archive, Spam, Trash, All mail, then every other folder alphabetically
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
    let manageable = folders_manageable(&store, AccountId(account_id)).map_err(|_| storage())?;
    let mut rows = Vec::with_capacity(folders.len());
    // Shown without the personal namespace prefix (#325), as the folder
    // list suggests it; nothing is asked of the server here.
    let namespace = imap_namespace(&store, &folders);
    for (folder, place) in arrange(&folders, namespace.as_deref()) {
        let unread = store.count_unread(folder.id).map_err(|_| storage())?;
        rows.push(FolderSummary {
            id: folder.id.0,
            display_name: one_line(&place.name),
            role: FolderRole::of(&folder.details),
            can_take_moves: folder.details.virtual_role.is_none(),
            depth: place.depth,
            unread_count: u32::try_from(unread).unwrap_or(u32::MAX),
            can_change: manageable && folder_admin::can_change(&folders, folder),
        });
    }
    Ok(rows)
}

/// Whether folders can be created, renamed, moved and deleted in this
/// account (M4-22): IMAP and local-only accounts, not JMAP ones yet.
fn folders_manageable(
    store: &Store,
    account: AccountId,
) -> Result<bool, mailcrane_store::StoreError> {
    if store.get_account(account)?.is_none() {
        return Ok(false);
    }
    if store.is_local_account(account)? {
        return Ok(true);
    }
    // IMAP, and JMAP through Mailbox/set (M4-26).
    Ok(true)
}

/// Whether the apps should offer "New folder" for this account (M4-22).
#[uniffi::export]
pub fn can_manage_folders(db_path: String, account_id: i64) -> Result<bool, SyncFailure> {
    let store = Store::open(&db_path).map_err(|_| SyncFailure::Failed {
        kind: SyncErrorKind::Storage,
    })?;
    folders_manageable(&store, AccountId(account_id)).map_err(|_| SyncFailure::Failed {
        kind: SyncErrorKind::Storage,
    })
}

/// Where a folder can be moved to (M4-22).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MoveTargets {
    /// It can move to the top level.
    pub to_top: bool,
    /// The folders it can move under, in display order.
    pub folders: Vec<FolderSummary>,
}

/// The places `folder_id` can move to, for the move picker. Places it is
/// already in, itself and its subfolders are left out.
#[uniffi::export]
pub fn move_targets(db_path: String, folder_id: i64) -> Result<MoveTargets, SyncFailure> {
    let storage = || SyncFailure::Failed {
        kind: SyncErrorKind::Storage,
    };
    let store = Store::open(&db_path).map_err(|_| storage())?;
    let Some(folder) = store
        .get_folder(mailcrane_store::FolderId(folder_id))
        .map_err(|_| storage())?
    else {
        return Err(SyncFailure::Failed {
            kind: SyncErrorKind::NotFound,
        });
    };
    let account = folder.details.account_id;
    let all = list_folders(db_path, account.0)?;
    let folders = store.list_folders(account).map_err(|_| storage())?;
    let namespace = imap_namespace(&store, &folders);
    let (to_top, under) = folder_admin::move_targets_in(&folders, folder.id, namespace.as_deref());
    Ok(MoveTargets {
        to_top,
        folders: all
            .into_iter()
            .filter(|f| under.contains(&mailcrane_store::FolderId(f.id)))
            .collect(),
    })
}

/// How many leading path segments of `f` are the namespace prefix: 1 for
/// `INBOX.Projects` under `INBOX.`, 0 when it isn't under the prefix (or
/// would be nothing but the prefix).
fn namespace_levels(f: &Folder, namespace: Option<&str>) -> usize {
    let (Some(prefix), Some(d)) = (namespace, f.details.delimiter.as_deref()) else {
        return 0;
    };
    let remote = f.details.remote_name.as_str();
    match remote.strip_prefix(prefix) {
        Some(rest) if !rest.is_empty() && !d.is_empty() => {
            prefix.trim_end_matches(d).split(d).count()
        }
        _ => 0,
    }
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
/// `namespace` is the account's personal namespace prefix (#325, such as
/// `INBOX.`): folders under it are labelled without it.
fn arrange<'a>(folders: &'a [Folder], namespace: Option<&str>) -> Vec<(&'a Folder, Place)> {
    let (mut standard, mut other): (Vec<&Folder>, Vec<&Folder>) = folders
        .iter()
        .partition(|f| FolderRole::of(&f.details).is_some());

    standard.sort_by(|a, b| {
        let rank = |f: &Folder| FolderRole::of(&f.details).map(FolderRole::rank);
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
        // between are missing, their names stay in the label. The personal
        // namespace's own levels (`INBOX` in `INBOX.Projects`) never do.
        let start = ancestors
            .last()
            .copied()
            .unwrap_or(0)
            .max(namespace_levels(f, namespace));
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

/// The personal namespace the folder list suggests (#325), for IMAP and
/// local accounts. JMAP has none (its Inbox is stored as `Inbox`).
fn imap_namespace(store: &Store, folders: &[Folder]) -> Option<String> {
    let account = folders.first()?.details.account_id;
    match store.account_protocol(account) {
        Ok(Some(mailcrane_store::Protocol::Jmap { .. })) => None,
        _ => folder_admin::derive_namespace(folders),
    }
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
            virtual_role: None,
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
    fn gmails_starred_and_important_come_straight_after_the_inbox() {
        // #390: with roles they're system folders, named by the app, not
        // "[Gmail]/Starred" sorted in among the user's own folders.
        let (_dir, path, account, _) = setup(&[
            ("Work", None),
            ("[Gmail]/Important", None),
            ("[Gmail]/Sent Mail", Some(StoreRole::Sent)),
            ("[Gmail]/Starred", None),
            ("INBOX", Some(StoreRole::Inbox)),
        ]);
        let store = Store::open(&path).expect("store");
        for (name, kind) in [
            ("[Gmail]/Starred", VirtualRole::Starred),
            ("[Gmail]/Important", VirtualRole::Important),
        ] {
            let mut row = folder(account, name, None);
            row.virtual_role = Some(kind);
            store.upsert_folder(&row).expect("virtual folder");
        }
        drop(store);
        let rows = list_folders(path, account.0).expect("list");
        let roles: Vec<_> = rows.iter().map(|r| r.role).collect();
        assert_eq!(
            roles,
            [
                Some(FolderRole::Inbox),
                Some(FolderRole::Starred),
                Some(FolderRole::Important),
                Some(FolderRole::Sent),
                None,
            ]
        );
        assert!(rows.iter().all(|r| r.depth == 0));
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
