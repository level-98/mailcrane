//! Creating, renaming, moving and deleting folders (M4-22).
//!
//! Every decision is made here, from the stored folder list: whether a name
//! is allowed, which folders may be changed, and the new server names. The
//! server change is then made with [`create_on_server`], [`rename_on_server`]
//! or [`delete_on_server`], and only after it succeeded is the store changed
//! with [`save_created`], [`save_renamed`] or [`save_deleted`]. Local-only
//! accounts skip the server step.
//!
//! Rules:
//! - Standard folders (Inbox, Drafts, Sent, Archive, Spam, Trash, All mail),
//!   Mailcrane's own Snoozed folder, and any folder that holds one of them
//!   (such as Gmail's `[Gmail]`) can't be renamed, moved or deleted.
//! - A folder with subfolders can't be deleted: delete those first.
//! - A name is one level: it can't contain the server's hierarchy
//!   delimiter, the IMAP wildcards `%` and `*`, or control characters.
//! - A folder can't move into itself or into one of its own subfolders.
//! - Two folders under the same parent can't have the same name.

use mailcrane_imap::{mutf7, ChangeError, FolderCommandError, Session};
use mailcrane_store::{AccountId, Folder, FolderId, NewFolder, Store, StoreError};

use crate::snooze::SNOOZED_FOLDER_NAME;

/// The longest folder name accepted, in characters.
pub const MAX_FOLDER_NAME_CHARS: usize = 100;

/// Why a folder name isn't allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderNameError {
    /// Nothing but spaces.
    Empty,
    /// Longer than [`MAX_FOLDER_NAME_CHARS`].
    TooLong,
    /// Contains a control character, `%` or `*`.
    BadCharacter,
    /// Contains the server's hierarchy delimiter (often `/` or `.`).
    Delimiter(char),
    /// `INBOX`, `.` or `..`.
    Reserved,
}

/// Why a folder change can't be made.
#[derive(Debug, thiserror::Error)]
pub enum FolderAdminError {
    /// The name isn't allowed.
    #[error("the folder name isn't allowed")]
    InvalidName(FolderNameError),
    /// A standard folder, or one that holds one.
    #[error("standard folders can't be changed")]
    SystemFolder,
    /// Delete its subfolders first.
    #[error("the folder has subfolders")]
    HasSubfolders,
    /// A folder with that name is already there.
    #[error("a folder with that name exists")]
    AlreadyExists,
    /// A folder can't move into itself or one of its subfolders.
    #[error("a folder can't move into itself")]
    IntoItself,
    /// The server has no subfolders.
    #[error("the server has no subfolders")]
    NoSubfolders,
    /// The folder (or the chosen parent) isn't in this account.
    #[error("no such folder")]
    NotFound,
    /// The server refused. `transient`: worth trying again later.
    #[error("the server refused")]
    Refused {
        /// Worth trying again later.
        transient: bool,
    },
    /// The connection failed; whether the change happened is unknown.
    #[error(transparent)]
    Imap(#[from] mailcrane_imap::ImapError),
    /// The local database failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<ChangeError> for FolderAdminError {
    fn from(e: ChangeError) -> Self {
        match e {
            ChangeError::Refused { transient } => Self::Refused { transient },
            ChangeError::Imap(e) => Self::Imap(e),
        }
    }
}

impl From<FolderCommandError> for FolderAdminError {
    fn from(e: FolderCommandError) -> Self {
        match e {
            FolderCommandError::AlreadyExists => Self::AlreadyExists,
            FolderCommandError::Refused { transient } => Self::Refused { transient },
            FolderCommandError::Imap(e) => Self::Imap(e),
        }
    }
}

/// A folder to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlan {
    /// The account.
    pub account: AccountId,
    /// The full server name.
    pub remote_name: String,
    /// The decoded full name, as stored.
    pub display_name: String,
    /// The hierarchy delimiter, if known.
    pub delimiter: Option<char>,
}

/// A rename or move: the server's `RENAME old new`, and every stored folder
/// it changes (the folder and its subfolders) with their new names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    /// The folder's server name now.
    pub old_remote: String,
    /// Its new server name.
    pub new_remote: String,
    /// `(folder, remote_name, display_name)` for the folder and each
    /// subfolder.
    pub rows: Vec<(FolderId, String, String)>,
}

impl RenamePlan {
    /// Nothing would change (the same name, or the same parent).
    pub fn is_noop(&self) -> bool {
        self.old_remote == self.new_remote
    }
}

/// Trims `name` and checks it against the rules above. `delimiter` is the
/// server's hierarchy delimiter, if known.
pub fn clean_folder_name(name: &str, delimiter: Option<char>) -> Result<String, FolderNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FolderNameError::Empty);
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(FolderNameError::TooLong);
    }
    if name.chars().any(|c| c.is_control() || c == '%' || c == '*') {
        return Err(FolderNameError::BadCharacter);
    }
    if let Some(d) = delimiter.filter(|d| name.contains(*d)) {
        return Err(FolderNameError::Delimiter(d));
    }
    if name.eq_ignore_ascii_case("INBOX") || name == "." || name == ".." {
        return Err(FolderNameError::Reserved);
    }
    Ok(name.to_owned())
}

fn delimiter_of(f: &Folder) -> Option<char> {
    f.details
        .delimiter
        .as_deref()
        .and_then(|d| d.chars().next())
}

/// The account's hierarchy delimiter: the first one a folder has.
/// `local_default` is used when none is known (local-only accounts).
fn account_delimiter(folders: &[Folder], local_default: Option<char>) -> Option<char> {
    folders.iter().find_map(delimiter_of).or(local_default)
}

/// The decoded form of a server name, as stored for display.
fn decode(remote: &str) -> String {
    mutf7::decode(remote).unwrap_or_else(|_| remote.to_owned())
}

fn last_segment(remote: &str, delimiter: Option<char>) -> &str {
    match delimiter {
        Some(d) => remote.rsplit(d).next().unwrap_or(remote),
        None => remote,
    }
}

/// Whether `child` is under `parent` (not equal to it).
fn is_under(child: &str, parent: &str, delimiter: Option<char>) -> bool {
    let Some(d) = delimiter else { return false };
    child
        .strip_prefix(parent)
        .is_some_and(|rest| rest.starts_with(d))
}

/// A standard folder or Mailcrane's own Snoozed folder.
pub fn is_system_folder(f: &Folder) -> bool {
    let remote = &f.details.remote_name;
    f.details.role.is_some()
        || remote.eq_ignore_ascii_case("INBOX")
        || decode(last_segment(remote, delimiter_of(f))) == SNOOZED_FOLDER_NAME
}

/// Whether `f` may be renamed, moved or deleted (apart from the subfolder
/// rule for deleting): not a standard folder and holding none.
pub fn can_change(folders: &[Folder], f: &Folder) -> bool {
    let d = delimiter_of(f);
    !is_system_folder(f)
        && !folders.iter().any(|o| {
            is_under(&o.details.remote_name, &f.details.remote_name, d) && is_system_folder(o)
        })
}

fn find(folders: &[Folder], id: FolderId) -> Result<&Folder, FolderAdminError> {
    folders
        .iter()
        .find(|f| f.id == id)
        .ok_or(FolderAdminError::NotFound)
}

fn taken(folders: &[Folder], remote: &str, except: Option<FolderId>) -> bool {
    folders.iter().any(|f| {
        Some(f.id) != except
            && (f.details.remote_name == remote
                || (remote.eq_ignore_ascii_case("INBOX")
                    && f.details.remote_name.eq_ignore_ascii_case("INBOX")))
    })
}

fn join(
    parent: Option<&str>,
    leaf: &str,
    delimiter: Option<char>,
) -> Result<String, FolderAdminError> {
    let leaf = mutf7::encode(leaf);
    match (parent, delimiter) {
        (None, _) => Ok(leaf),
        (Some(p), Some(d)) => Ok(format!("{p}{d}{leaf}")),
        (Some(_), None) => Err(FolderAdminError::NoSubfolders),
    }
}

/// Plans a new folder `name` in `account`, under `parent` or at the top.
/// `folders` is the account's stored folder list. `local_default` is the
/// delimiter to use when none is known (local-only accounts).
pub fn plan_create(
    folders: &[Folder],
    account: AccountId,
    parent: Option<FolderId>,
    name: &str,
    local_default: Option<char>,
) -> Result<CreatePlan, FolderAdminError> {
    plan_create_in(folders, account, parent, name, local_default, None)
}

/// [`plan_create`] for an account whose personal namespace prefix is
/// `namespace` (RFC 2342, such as `INBOX.`, #325): a top-level folder is
/// created under it.
pub fn plan_create_in(
    folders: &[Folder],
    account: AccountId,
    parent: Option<FolderId>,
    name: &str,
    local_default: Option<char>,
    namespace: Option<&str>,
) -> Result<CreatePlan, FolderAdminError> {
    let parent = parent.map(|p| find(folders, p)).transpose()?;
    let delimiter = parent
        .and_then(delimiter_of)
        .or_else(|| account_delimiter(folders, local_default));
    let name = clean_folder_name(name, delimiter).map_err(FolderAdminError::InvalidName)?;
    let remote = match (parent, namespace) {
        (None, Some(prefix)) => format!("{prefix}{}", mutf7::encode(&name)),
        _ => join(
            parent.map(|p| p.details.remote_name.as_str()),
            &name,
            delimiter,
        )?,
    };
    if taken(folders, &remote, None) {
        return Err(FolderAdminError::AlreadyExists);
    }
    Ok(CreatePlan {
        account,
        display_name: decode(&remote),
        remote_name: remote,
        delimiter,
    })
}

/// The rows a rename from `old` to `new` changes: `folder` and everything
/// under it.
fn rename_rows(folders: &[Folder], folder: &Folder, new_remote: &str) -> RenamePlan {
    let old = folder.details.remote_name.clone();
    let d = delimiter_of(folder);
    let rows = folders
        .iter()
        .filter(|f| f.id == folder.id || is_under(&f.details.remote_name, &old, d))
        .map(|f| {
            let rest = f.details.remote_name.strip_prefix(&old).unwrap_or_default();
            let remote = format!("{new_remote}{rest}");
            (f.id, decode(&remote), remote)
        })
        .map(|(id, display, remote)| (id, remote, display))
        .collect();
    RenamePlan {
        old_remote: old,
        new_remote: new_remote.to_owned(),
        rows,
    }
}

fn check_rename_target(
    folders: &[Folder],
    plan: &RenamePlan,
    folder: FolderId,
) -> Result<(), FolderAdminError> {
    if !plan.is_noop() && taken(folders, &plan.new_remote, Some(folder)) {
        return Err(FolderAdminError::AlreadyExists);
    }
    Ok(())
}

/// Plans renaming `folder` to `name`, keeping its place.
pub fn plan_rename(
    folders: &[Folder],
    folder: FolderId,
    name: &str,
) -> Result<RenamePlan, FolderAdminError> {
    let f = find(folders, folder)?;
    if !can_change(folders, f) {
        return Err(FolderAdminError::SystemFolder);
    }
    let d = delimiter_of(f);
    let name = clean_folder_name(name, d).map_err(FolderAdminError::InvalidName)?;
    let remote = &f.details.remote_name;
    let parent = d.and_then(|d| remote.rsplit_once(d)).map(|(p, _)| p);
    let new_remote = join(parent, &name, d)?;
    let plan = rename_rows(folders, f, &new_remote);
    check_rename_target(folders, &plan, folder)?;
    Ok(plan)
}

/// Plans moving `folder` under `parent`, or to the top with `None`,
/// keeping its name.
pub fn plan_move(
    folders: &[Folder],
    folder: FolderId,
    parent: Option<FolderId>,
) -> Result<RenamePlan, FolderAdminError> {
    plan_move_in(folders, folder, parent, None)
}

/// [`plan_move`] for an account with personal namespace prefix `namespace`
/// (#325): "to the top" means directly under the prefix.
pub fn plan_move_in(
    folders: &[Folder],
    folder: FolderId,
    parent: Option<FolderId>,
    namespace: Option<&str>,
) -> Result<RenamePlan, FolderAdminError> {
    let f = find(folders, folder)?;
    if !can_change(folders, f) {
        return Err(FolderAdminError::SystemFolder);
    }
    let d = delimiter_of(f).or_else(|| account_delimiter(folders, None));
    let remote = &f.details.remote_name;
    let leaf = last_segment(remote, d);
    let new_remote = match parent {
        None => format!("{}{leaf}", namespace.unwrap_or_default()),
        Some(p) => {
            let p = find(folders, p)?;
            let pr = &p.details.remote_name;
            if p.id == f.id || is_under(pr, remote, d) {
                return Err(FolderAdminError::IntoItself);
            }
            let d = d.ok_or(FolderAdminError::NoSubfolders)?;
            format!("{pr}{d}{leaf}")
        }
    };
    let plan = rename_rows(folders, f, &new_remote);
    check_rename_target(folders, &plan, folder)?;
    Ok(plan)
}

/// Where `folder` can move to, for the move picker: whether to the top,
/// and under which folders. A place it already is, or one a move would be
/// refused for, is left out. Empty when the folder can't be moved at all.
pub fn move_targets(folders: &[Folder], folder: FolderId) -> (bool, Vec<FolderId>) {
    move_targets_in(folders, folder, None)
}

/// [`move_targets`] with a personal namespace prefix (#325). The folder the
/// namespace hangs under (`INBOX` for `INBOX.`) is the top level, so it's
/// offered as "to the top", not as a folder of its own.
pub fn move_targets_in(
    folders: &[Folder],
    folder: FolderId,
    namespace: Option<&str>,
) -> (bool, Vec<FolderId>) {
    let ok = |parent: Option<FolderId>| {
        plan_move_in(folders, folder, parent, namespace).is_ok_and(|plan| !plan.is_noop())
    };
    let root = namespace.and_then(|n| namespace_root(folders, n));
    let under = folders
        .iter()
        .map(|f| f.id)
        .filter(|id| Some(*id) != root && ok(Some(*id)))
        .collect();
    (ok(None), under)
}

/// The personal namespace prefix the folder list suggests, for servers
/// that don't answer NAMESPACE and for showing the list without connecting
/// (#325): `INBOX` plus its delimiter when the Inbox has one and every other
/// folder is under it, as on Courier-style servers. `None` otherwise,
/// including when the Inbox is the only folder.
pub fn derive_namespace(folders: &[Folder]) -> Option<String> {
    let inbox = folders
        .iter()
        .find(|f| f.details.remote_name.eq_ignore_ascii_case("INBOX"))?;
    let d = delimiter_of(inbox)?;
    let prefix = format!("{}{d}", inbox.details.remote_name);
    let mut others = folders.iter().filter(|f| f.id != inbox.id).peekable();
    others.peek()?;
    others
        .all(|f| f.details.remote_name.starts_with(&prefix))
        .then_some(prefix)
}

/// The folder a namespace prefix hangs under: `INBOX` for `INBOX.`, if it's
/// stored. `None` for a prefix with no such folder.
pub fn namespace_root(folders: &[Folder], namespace: &str) -> Option<FolderId> {
    folders
        .iter()
        .find(|f| {
            delimiter_of(f)
                .is_some_and(|d| namespace.strip_suffix(d) == Some(f.details.remote_name.as_str()))
        })
        .map(|f| f.id)
}

/// Checks that `folder` may be deleted and returns it.
pub fn plan_delete(folders: &[Folder], folder: FolderId) -> Result<Folder, FolderAdminError> {
    let f = find(folders, folder)?;
    if !can_change(folders, f) {
        return Err(FolderAdminError::SystemFolder);
    }
    let d = delimiter_of(f);
    if folders
        .iter()
        .any(|o| is_under(&o.details.remote_name, &f.details.remote_name, d))
    {
        return Err(FolderAdminError::HasSubfolders);
    }
    Ok(f.clone())
}

/// `CREATE` on the server.
pub async fn create_on_server(
    session: &mut Session,
    plan: &CreatePlan,
) -> Result<(), FolderAdminError> {
    Ok(session.create_folder(&plan.remote_name).await?)
}

/// `RENAME` on the server; nothing for a no-op.
pub async fn rename_on_server(
    session: &mut Session,
    plan: &RenamePlan,
) -> Result<(), FolderAdminError> {
    if plan.is_noop() {
        return Ok(());
    }
    Ok(session
        .rename_folder(&plan.old_remote, &plan.new_remote)
        .await?)
}

/// `DELETE` on the server.
pub async fn delete_on_server(
    session: &mut Session,
    folder: &Folder,
) -> Result<(), FolderAdminError> {
    Ok(session.delete_folder(&folder.details.remote_name).await?)
}

/// Stores a created folder (after the server created it, or at once for a
/// local-only account) and returns its id.
pub fn save_created(store: &Store, plan: &CreatePlan) -> Result<FolderId, FolderAdminError> {
    Ok(store.upsert_folder(&NewFolder {
        account_id: plan.account,
        remote_name: plan.remote_name.clone(),
        display_name: plan.display_name.clone(),
        delimiter: plan.delimiter.map(String::from),
        role: None,
        uidvalidity: None,
        uidnext: None,
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
    })?)
}

/// Stores a rename or move.
pub fn save_renamed(store: &Store, plan: &RenamePlan) -> Result<(), FolderAdminError> {
    if plan.is_noop() {
        return Ok(());
    }
    Ok(store.rename_folders(&plan.rows)?)
}

/// Removes a deleted folder, with its messages, from the store.
pub fn save_deleted(store: &Store, folder: &Folder) -> Result<(), FolderAdminError> {
    store.delete_folder(folder.id)?;
    Ok(())
}

#[cfg(test)]
mod tests;
