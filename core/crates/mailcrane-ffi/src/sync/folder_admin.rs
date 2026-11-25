//! Creating, renaming, moving and deleting folders from the apps (M4-22).
//! The core decides everything (`mailcrane_core::folder_admin`); these calls
//! connect, make the change on the server, then store it. Local-only
//! accounts change only the store. JMAP accounts aren't supported yet.

use super::*;
use mailcrane_core::folder_admin::{self as admin, FolderAdminError, FolderNameError};
use mailcrane_store::Protocol;

/// Why a folder name isn't allowed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FolderNameProblem {
    /// Empty.
    Empty,
    /// Longer than 100 characters.
    TooLong,
    /// A control character, `%` or `*`.
    BadCharacter,
    /// The server's folder separator, such as `/`.
    Separator {
        /// The separator, to show.
        separator: String,
    },
    /// `INBOX`, `.` or `..`.
    Reserved,
}

/// Why a folder change didn't happen. Nothing was changed, except after
/// `Sync`, where the connection failed and the server may have made it
/// (the next folder refresh shows).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FolderFailure {
    /// The name isn't allowed.
    #[error("invalid folder name")]
    InvalidName {
        /// What's wrong with it.
        problem: FolderNameProblem,
    },
    /// Standard folders (and folders holding them) can't be changed.
    #[error("standard folder")]
    SystemFolder,
    /// Delete the subfolders first.
    #[error("folder has subfolders")]
    HasSubfolders,
    /// A folder with that name is already there.
    #[error("folder exists")]
    AlreadyExists,
    /// A folder can't move into itself or one of its subfolders.
    #[error("folder can't move into itself")]
    IntoItself,
    /// The server has no subfolders.
    #[error("no subfolders on this server")]
    NoSubfolders,
    /// No such folder or account.
    #[error("not found")]
    NotFound,
    /// The server refused the change.
    #[error("server refused")]
    Refused,
    /// Not available for this account (JMAP, for now).
    #[error("not supported")]
    Unsupported,
    /// Connecting or talking to the server failed.
    #[error("sync failure")]
    Sync {
        /// What kind of failure.
        kind: SyncErrorKind,
    },
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

impl From<FolderAdminError> for FolderFailure {
    fn from(e: FolderAdminError) -> Self {
        match e {
            FolderAdminError::InvalidName(n) => Self::InvalidName {
                problem: match n {
                    FolderNameError::Empty => FolderNameProblem::Empty,
                    FolderNameError::TooLong => FolderNameProblem::TooLong,
                    FolderNameError::BadCharacter => FolderNameProblem::BadCharacter,
                    FolderNameError::Delimiter(d) => FolderNameProblem::Separator {
                        separator: d.to_string(),
                    },
                    FolderNameError::Reserved => FolderNameProblem::Reserved,
                },
            },
            FolderAdminError::SystemFolder => Self::SystemFolder,
            FolderAdminError::HasSubfolders => Self::HasSubfolders,
            FolderAdminError::AlreadyExists => Self::AlreadyExists,
            FolderAdminError::IntoItself => Self::IntoItself,
            FolderAdminError::NoSubfolders => Self::NoSubfolders,
            FolderAdminError::NotFound => Self::NotFound,
            FolderAdminError::Refused { .. } => Self::Refused,
            FolderAdminError::Imap(e) => {
                let e = CoreSyncError::from(e);
                log_failure("folder change", &e, None);
                Self::Sync {
                    kind: e.kind().into(),
                }
            }
            FolderAdminError::Store(_) => Self::Storage,
        }
    }
}

/// Folder separator for local-only accounts that have none yet.
const LOCAL_SEPARATOR: char = '/';

enum Kind {
    Local,
    Imap,
}

#[uniffi::export]
impl SyncEngine {
    /// Creates a folder `name` under `parent_id`, or at the top with `None`,
    /// and returns its id.
    pub fn create_folder(
        &self,
        account_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> Result<i64, FolderFailure> {
        let account = AccountId(account_id);
        let (store, kind) = self.admin_open(account)?;
        let folders = list(&store, account)?;
        let namespace = namespace(&store, account)?;
        let plan = admin::plan_create_in(
            &folders,
            account,
            parent_id.map(FolderId),
            &name,
            Some(LOCAL_SEPARATOR),
            namespace.as_deref(),
        )?;
        if let Kind::Imap = kind {
            let target = plan.clone();
            self.on_server(&store, account, |s| {
                Box::pin(async move { admin::create_on_server(s, &target).await })
            })?;
        }
        let id = admin::save_created(&store, &plan)?;
        self.folder_changed(id);
        Ok(id.0)
    }

    /// Renames the folder, keeping its place. Its subfolders follow.
    pub fn rename_folder(&self, folder_id: i64, name: String) -> Result<(), FolderFailure> {
        self.change(folder_id, |folders, id, _| {
            admin::plan_rename(folders, id, &name)
        })
    }

    /// Moves the folder under `parent_id`, or to the top with `None`.
    pub fn move_folder(&self, folder_id: i64, parent_id: Option<i64>) -> Result<(), FolderFailure> {
        self.change(folder_id, |folders, id, namespace| {
            admin::plan_move_in(folders, id, parent_id.map(FolderId), namespace)
        })
    }

    /// Deletes the folder and its messages, on the server and here. The
    /// apps ask the user first.
    pub fn delete_folder(&self, folder_id: i64) -> Result<(), FolderFailure> {
        let (store, account, kind) = self.admin_folder(folder_id)?;
        let folders = list(&store, account)?;
        let folder = admin::plan_delete(&folders, FolderId(folder_id))?;
        if let Kind::Imap = kind {
            let target = folder.clone();
            self.on_server(&store, account, |s| {
                Box::pin(async move { admin::delete_on_server(s, &target).await })
            })?;
        }
        admin::save_deleted(&store, &folder)?;
        self.folder_changed(folder.id);
        Ok(())
    }
}

type AdminFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), FolderAdminError>> + 'a>>;

impl SyncEngine {
    /// A rename or move.
    fn change(
        &self,
        folder_id: i64,
        plan: impl Fn(
            &[mailcrane_store::Folder],
            FolderId,
            Option<&str>,
        ) -> Result<admin::RenamePlan, FolderAdminError>,
    ) -> Result<(), FolderFailure> {
        let (store, account, kind) = self.admin_folder(folder_id)?;
        let folders = list(&store, account)?;
        let namespace = namespace(&store, account)?;
        let p = plan(&folders, FolderId(folder_id), namespace.as_deref())?;
        if p.is_noop() {
            return Ok(());
        }
        if let Kind::Imap = kind {
            let target = p.clone();
            self.on_server(&store, account, |s| {
                Box::pin(async move { admin::rename_on_server(s, &target).await })
            })?;
        }
        admin::save_renamed(&store, &p)?;
        for (id, ..) in &p.rows {
            self.folder_changed(*id);
        }
        Ok(())
    }

    /// Tells the apps the folder list changed, so they re-read it.
    fn folder_changed(&self, folder: FolderId) {
        self.listener
            .on_event(events::CoreEvent::FolderChanged { folder_id: folder }.into());
    }

    fn admin_open(&self, account: AccountId) -> Result<(Store, Kind), FolderFailure> {
        let store = self.open_store().map_err(|_| FolderFailure::Storage)?;
        if store
            .get_account(account)
            .map_err(|_| FolderFailure::Storage)?
            .is_none()
        {
            return Err(FolderFailure::NotFound);
        }
        if store
            .is_local_account(account)
            .map_err(|_| FolderFailure::Storage)?
        {
            return Ok((store, Kind::Local));
        }
        match store.account_protocol(account) {
            Ok(Some(Protocol::Jmap { .. })) => Err(FolderFailure::Unsupported),
            Ok(_) => Ok((store, Kind::Imap)),
            Err(_) => Err(FolderFailure::Storage),
        }
    }

    fn admin_folder(&self, folder_id: i64) -> Result<(Store, AccountId, Kind), FolderFailure> {
        let account = self
            .folder_account(FolderId(folder_id))
            .map_err(|e| match e {
                SyncFailure::Failed {
                    kind: SyncErrorKind::NotFound,
                } => FolderFailure::NotFound,
                _ => FolderFailure::Storage,
            })?;
        let (store, kind) = self.admin_open(account)?;
        Ok((store, account, kind))
    }

    /// Connects, runs `job`, logs out. Holds the account's lock so no sync
    /// runs at the same time.
    fn on_server(
        &self,
        store: &Store,
        account: AccountId,
        job: impl for<'a> FnOnce(&'a mut Session) -> AdminFuture<'a>,
    ) -> Result<(), FolderFailure> {
        let lock = self.account_lock(account);
        let _guard = lock.lock();
        let sync = |kind: events::SyncErrorKind| FolderFailure::Sync { kind: kind.into() };
        let runtime = runtime().map_err(sync)?;
        runtime.block_on(async {
            let mut session = self.connect(store, account).await.map_err(sync)?;
            let result = job(&mut session).await;
            session.logout().await;
            Ok(result?)
        })
    }
}

/// The account's personal namespace prefix (#325), as `refresh_folders`
/// last saw it.
fn namespace(store: &Store, account: AccountId) -> Result<Option<String>, FolderFailure> {
    store
        .get_personal_namespace(account)
        .map_err(|_| FolderFailure::Storage)
}

fn list(store: &Store, account: AccountId) -> Result<Vec<mailcrane_store::Folder>, FolderFailure> {
    store
        .list_folders(account)
        .map_err(|_| FolderFailure::Storage)
}

#[cfg(test)]
mod tests;
