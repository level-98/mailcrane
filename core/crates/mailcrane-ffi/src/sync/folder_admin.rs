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
    /// The server won't delete a folder that still has messages (JMAP):
    /// move or delete them first.
    #[error("folder has messages")]
    HasMessages,
    /// A folder with that name is already there.
    #[error("folder exists")]
    AlreadyExists,
    /// A folder can't move into itself or one of its subfolders.
    #[error("folder can't move into itself")]
    IntoItself,
    /// The server has no subfolders.
    #[error("no subfolders on this server")]
    NoSubfolders,
    /// The chosen parent can't hold folders, such as Gmail's Starred (#461).
    #[error("not a parent")]
    NotAParent,
    /// No such folder or account.
    #[error("not found")]
    NotFound,
    /// The server refused the change.
    #[error("server refused")]
    Refused,
    /// Not available for this account.
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
            FolderAdminError::HasMessages => Self::HasMessages,
            FolderAdminError::AlreadyExists => Self::AlreadyExists,
            FolderAdminError::IntoItself => Self::IntoItself,
            FolderAdminError::NoSubfolders => Self::NoSubfolders,
            FolderAdminError::NotAParent => Self::NotAParent,
            FolderAdminError::NotFound => Self::NotFound,
            FolderAdminError::Refused { .. } => Self::Refused,
            FolderAdminError::Imap(e) => {
                let e = CoreSyncError::from(e);
                log_failure("folder change", &e, None);
                Self::Sync {
                    kind: e.kind().into(),
                }
            }
            FolderAdminError::Jmap(e) => {
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
    /// M4-26: through `Mailbox/set`.
    Jmap,
}

#[uniffi::export]
impl SyncEngine {
    /// Creates a folder `name` under `parent_id`, or at the top with `None`,
    /// and returns its id. On a server with a personal namespace (such as
    /// `INBOX.`, #325), a top-level folder goes under it.
    pub fn create_folder(
        &self,
        account_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> Result<i64, FolderFailure> {
        let account = AccountId(account_id);
        let (store, kind) = self.admin_open(account)?;
        let folders = list(&store, account)?;
        let derived = derived_namespace(&kind, &folders);
        let plan_with = |namespace: Option<&str>| {
            admin::plan_create_in(
                &folders,
                account,
                parent_id.map(FolderId),
                &name,
                Some(LOCAL_SEPARATOR),
                namespace,
            )
        };
        // Bad names, unknown parents and taken names are refused before
        // connecting.
        let mut plan = plan_with(derived.as_deref())?;
        let id = match kind {
            Kind::Jmap => {
                let (target, parent) = (plan.clone(), parent_id.map(FolderId));
                self.jmap_admin(account, move |s| {
                    Box::pin(async move { Ok(s.create_folder(&target, parent).await) })
                })?
            }
            Kind::Imap => {
                plan = self.on_server(account, async |s| {
                    let namespace = server_namespace(s).await.or(derived.clone());
                    let plan = plan_with(namespace.as_deref())?;
                    admin::create_on_server(s, &plan).await?;
                    Ok(plan)
                })?;
                admin::save_created(&store, &plan)?
            }
            Kind::Local => admin::save_created(&store, &plan)?,
        };
        self.folder_changed(id);
        Ok(id.0)
    }

    /// Renames the folder, keeping its place. Its subfolders follow.
    pub fn rename_folder(&self, folder_id: i64, name: String) -> Result<(), FolderFailure> {
        self.change(folder_id, |folders, id, _| {
            admin::plan_rename(folders, id, &name)
        })
    }

    /// Moves the folder under `parent_id`, or to the top with `None` (under
    /// the personal namespace, if the server has one).
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
        match kind {
            Kind::Jmap => {
                let target = folder.clone();
                self.jmap_admin(account, move |s| {
                    Box::pin(async move { Ok(s.delete_folder(&target).await) })
                })?;
            }
            Kind::Imap => {
                self.on_server(account, async |s| admin::delete_on_server(s, &folder).await)?;
                admin::save_deleted(&store, &folder)?;
            }
            Kind::Local => admin::save_deleted(&store, &folder)?,
        }
        self.folder_changed(folder.id);
        Ok(())
    }
}

impl SyncEngine {
    /// A rename or move. `plan` gets the personal namespace prefix: the
    /// server's (NAMESPACE, on the connection that makes the change) when it
    /// has one, otherwise the one the folder list suggests.
    fn change(
        &self,
        folder_id: i64,
        plan: impl Fn(
                &[mailcrane_store::Folder],
                FolderId,
                Option<&str>,
            ) -> Result<admin::RenamePlan, FolderAdminError>
            + Sync,
    ) -> Result<(), FolderFailure> {
        let (store, account, kind) = self.admin_folder(folder_id)?;
        let folders = list(&store, account)?;
        let derived = derived_namespace(&kind, &folders);
        let id = FolderId(folder_id);
        // Refused moves and renames fail before connecting.
        let mut p = plan(&folders, id, derived.as_deref())?;
        match kind {
            // On IMAP the server's namespace may make a move that looks like
            // a no-op here a real one (#325): plan again on the connection.
            Kind::Jmap | Kind::Local if p.is_noop() => return Ok(()),
            Kind::Jmap => {
                let target = p.clone();
                self.jmap_admin(account, move |s| {
                    Box::pin(async move { Ok(s.rename_folder(id, &target).await) })
                })?;
            }
            Kind::Imap => {
                p = self.on_server(account, async |s| {
                    let namespace = server_namespace(s).await.or(derived.clone());
                    let p = plan(&folders, id, namespace.as_deref())?;
                    admin::rename_on_server(s, &p).await?;
                    Ok(p)
                })?;
                if p.is_noop() {
                    return Ok(());
                }
                admin::save_renamed(&store, &p)?;
            }
            Kind::Local => admin::save_renamed(&store, &p)?,
        }
        for (id, ..) in &p.rows {
            self.folder_changed(*id);
        }
        Ok(())
    }

    /// Runs a folder change on a JMAP account (M4-26) through the sync
    /// engine's JMAP session. The job stores what it changed.
    fn jmap_admin<T: Send>(
        &self,
        account: AccountId,
        job: impl for<'a> FnOnce(
                &'a mut mailcrane_core::sync::JmapSyncer<'_>,
            ) -> SyncFuture<'a, Result<T, FolderAdminError>>
            + Send,
    ) -> Result<T, FolderFailure> {
        match self.run_jmap(account, job) {
            Some(Ok(result)) => Ok(result?),
            Some(Err(SyncFailure::Failed { kind })) => Err(FolderFailure::Sync { kind }),
            None => Err(FolderFailure::Unsupported),
        }
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
            Ok(Some(Protocol::Jmap { .. })) => Ok((store, Kind::Jmap)),
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
    /// runs at the same time. Runs on a thread with a stack this crate sets
    /// (#462), never on the caller's.
    fn on_server<T: Send>(
        &self,
        account: AccountId,
        job: impl AsyncFnOnce(&mut Session) -> Result<T, FolderAdminError> + Send,
    ) -> Result<T, FolderFailure> {
        let sync = |kind: events::SyncErrorKind| FolderFailure::Sync { kind: kind.into() };
        crate::worker::on_job_thread(|| {
            let lock = self.account_lock(account);
            let _guard = lock.lock();
            let store = self.open_store().map_err(|_| FolderFailure::Storage)?;
            let runtime = runtime().map_err(sync)?;
            runtime.block_on(async {
                let mut session = self.connect(&store, account).await.map_err(sync)?;
                let result = job(&mut session).await;
                session.logout().await;
                Ok(result?)
            })
        })
        .unwrap_or_else(|| {
            tracing::error!(
                stage = "job thread",
                "the folder change couldn't run on a thread of its own"
            );
            Err(sync(events::SyncErrorKind::Network))
        })
    }
}

/// The personal namespace the folder list suggests (#325). JMAP has no
/// namespaces, and its Inbox is stored as `Inbox`, so it gets none.
fn derived_namespace(kind: &Kind, folders: &[mailcrane_store::Folder]) -> Option<String> {
    match kind {
        Kind::Jmap => None,
        Kind::Imap | Kind::Local => admin::derive_namespace(folders),
    }
}

/// The server's personal namespace prefix (#325), asked on this
/// connection. `None` without NAMESPACE, without a personal namespace, or
/// if asking failed (the change itself then reports a broken connection).
async fn server_namespace(session: &mut Session) -> Option<String> {
    session.personal_namespace().await.ok().flatten()
}

fn list(store: &Store, account: AccountId) -> Result<Vec<mailcrane_store::Folder>, FolderFailure> {
    store
        .list_folders(account)
        .map_err(|_| FolderFailure::Storage)
}

#[cfg(test)]
mod tests;
