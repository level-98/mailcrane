use super::narrow;
use crate::{AccountId, Folder, FolderId, FolderRole, NewFolder, Store, StoreError, VirtualRole};
use rusqlite::{params, Row};

fn from_row(r: &Row<'_>) -> Result<Folder, StoreError> {
    let role: Option<String> = r.get(5)?;
    let virtual_role: Option<String> = r.get(11)?;
    Ok(Folder {
        id: FolderId(r.get(0)?),
        details: NewFolder {
            account_id: AccountId(r.get(1)?),
            remote_name: r.get(2)?,
            display_name: r.get(3)?,
            delimiter: r.get(4)?,
            role: role.as_deref().map(FolderRole::parse).transpose()?,
            uidvalidity: r.get(6)?,
            uidnext: r.get(7)?,
            highestmodseq: r.get(8)?,
            is_subscribed: narrow::<u8>(r.get(9)?, "is_subscribed")? != 0,
            selectable: narrow::<u8>(r.get(10)?, "selectable")? != 0,
            virtual_role: virtual_role.as_deref().and_then(VirtualRole::parse),
        },
    })
}

impl Store {
    /// Inserts the folder or, if `(account, remote_name)` exists, updates it.
    pub fn upsert_folder(&self, f: &NewFolder) -> Result<FolderId, StoreError> {
        let id: i64 = self.conn.query_row(
            "INSERT INTO folders (account_id, remote_name, display_name, delimiter, role, \
             uidvalidity, uidnext, highestmodseq, is_subscribed, selectable, virtual_role) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) \
             ON CONFLICT (account_id, remote_name) DO UPDATE SET \
               display_name = excluded.display_name, delimiter = excluded.delimiter, \
               role = excluded.role, uidvalidity = excluded.uidvalidity, \
               uidnext = excluded.uidnext, highestmodseq = excluded.highestmodseq, \
               is_subscribed = excluded.is_subscribed, selectable = excluded.selectable, \
               virtual_role = excluded.virtual_role \
             RETURNING id",
            params![
                f.account_id.0,
                f.remote_name,
                f.display_name,
                f.delimiter,
                f.role.map(FolderRole::as_str),
                f.uidvalidity,
                f.uidnext,
                f.highestmodseq,
                f.is_subscribed,
                f.selectable,
                f.virtual_role.map(VirtualRole::as_str)
            ],
            |r| r.get(0),
        )?;
        Ok(FolderId(id))
    }

    /// Upserts every folder in one transaction: either all are saved or,
    /// on error, none are.
    pub fn upsert_folders(&self, folders: &[NewFolder]) -> Result<Vec<FolderId>, StoreError> {
        let tx = self.write_tx()?;
        let mut ids = Vec::with_capacity(folders.len());
        for f in folders {
            ids.push(self.upsert_folder(f)?);
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Gives folders new names, all in one transaction (M4-22): each
    /// `(folder, remote_name, display_name)`. A folder's messages and sync
    /// state stay with it. On error nothing changes, for example when a new
    /// name is taken in the account.
    pub fn rename_folders(&self, renames: &[(FolderId, String, String)]) -> Result<(), StoreError> {
        let tx = self.write_tx()?;
        for (id, remote, display) in renames {
            let n = self.conn.execute(
                "UPDATE folders SET remote_name = ?2, display_name = ?3 WHERE id = ?1",
                params![id.0, remote, display],
            )?;
            if n == 0 {
                return Err(StoreError::Invalid { field: "folder" });
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes a folder and, with it, its messages and everything else
    /// stored for it (M4-22). Rules that moved mail to it are turned off
    /// with a note (#323). Returns whether it existed.
    pub fn delete_folder(&self, id: FolderId) -> Result<bool, StoreError> {
        let tx = self.write_tx()?;
        let n = self.remove_folder(id)?;
        tx.commit()?;
        Ok(n)
    }

    /// Removes one folder inside the caller's transaction. Every rule with
    /// an action pointing at it is turned off and remembers the folder's
    /// name (`rules.missing_folder`); the action itself goes with the
    /// folder, as a move needs a folder.
    fn remove_folder(&self, id: FolderId) -> Result<bool, StoreError> {
        self.conn.execute(
            "UPDATE rules SET enabled = 0, \
             missing_folder = (SELECT display_name FROM folders WHERE id = ?1) \
             WHERE id IN (SELECT rule_id FROM rule_actions WHERE folder_id = ?1)",
            params![id.0],
        )?;
        let n = self
            .conn
            .execute("DELETE FROM folders WHERE id = ?1", params![id.0])?;
        Ok(n > 0)
    }

    /// Saves a folder list from the server in one transaction (#323):
    /// `renames` first (standard folders found again under a new name), then
    /// `rows` upserted, then `removed` taken out as [`Store::delete_folder`]
    /// does. Returns the ids of `rows`, in order. On error nothing changes.
    pub fn apply_folder_list(
        &self,
        renames: &[(FolderId, String, String)],
        rows: &[NewFolder],
        removed: &[FolderId],
    ) -> Result<Vec<FolderId>, StoreError> {
        let tx = self.write_tx()?;
        for (id, remote, display) in renames {
            self.conn.execute(
                "UPDATE folders SET remote_name = ?2, display_name = ?3 WHERE id = ?1",
                params![id.0, remote, display],
            )?;
        }
        let mut ids = Vec::with_capacity(rows.len());
        for f in rows {
            ids.push(self.upsert_folder(f)?);
        }
        for id in removed {
            self.remove_folder(*id)?;
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Returns the folder, or `None` if it doesn't exist.
    pub fn get_folder(&self, id: FolderId) -> Result<Option<Folder>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, remote_name, display_name, delimiter, role, uidvalidity, \
             uidnext, highestmodseq, is_subscribed, selectable, virtual_role FROM folders WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id.0])?;
        rows.next()?.map(from_row).transpose()
    }

    /// Deletes every cached message of the folder (with their bodies and
    /// attachment metadata) and records the server's new `UIDVALIDITY` and
    /// `UIDNEXT`, in one transaction (RFC 9051 §2.3.1.1). The folder's
    /// queued actions go too: their UIDs now name other messages (#517).
    /// Returns whether the folder exists; if not, nothing is changed.
    pub fn reset_folder(
        &self,
        id: FolderId,
        uidvalidity: i64,
        uidnext: i64,
    ) -> Result<bool, StoreError> {
        Ok(self
            .reset_folder_reporting(id, uidvalidity, uidnext)?
            .is_some())
    }

    /// [`Store::reset_folder`], returning the queued actions it dropped so
    /// the user can be told (#517); `None` if the folder doesn't exist (then
    /// nothing is changed).
    pub fn reset_folder_reporting(
        &self,
        id: FolderId,
        uidvalidity: i64,
        uidnext: i64,
    ) -> Result<Option<Vec<crate::DroppedOp>>, StoreError> {
        let tx = self.write_tx()?;
        let dropped = super::pending_ops::take_folder_ops(&tx, id)?;
        tx.execute("DELETE FROM messages WHERE folder_id = ?1", params![id.0])?;
        let n = tx.execute(
            "UPDATE folders SET uidvalidity = ?2, uidnext = ?3, highestmodseq = NULL \
             WHERE id = ?1",
            params![id.0, uidvalidity, uidnext],
        )?;
        if n == 0 {
            // Rolled back: no folder, nothing changed.
            return Ok(None);
        }
        tx.commit()?;
        Ok(Some(dropped))
    }

    /// Records the folder's `UIDNEXT` and `HIGHESTMODSEQ` after a sync.
    /// Call it only once the changes up to that `HIGHESTMODSEQ` are stored
    /// (RFC 7162 §3.1.2.1), so a failure never skips changes. Returns
    /// whether the folder exists.
    pub fn set_folder_sync_state(
        &self,
        id: FolderId,
        uidnext: i64,
        highestmodseq: Option<i64>,
    ) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE folders SET uidnext = ?2, highestmodseq = ?3 WHERE id = ?1",
            params![id.0, uidnext, highestmodseq],
        )?;
        Ok(n > 0)
    }

    /// Lists an account's folders ordered by remote name.
    pub fn list_folders(&self, account: AccountId) -> Result<Vec<Folder>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, remote_name, display_name, delimiter, role, uidvalidity, \
             uidnext, highestmodseq, is_subscribed, selectable, virtual_role FROM folders WHERE account_id = ?1 \
             ORDER BY remote_name",
        )?;
        let mut rows = stmt.query(params![account.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }

    /// Counts messages in the folder that are not `\Seen`, leaving out
    /// hidden ones and ones flagged `\Deleted` (#519).
    pub fn count_unread(&self, folder: FolderId) -> Result<u64, StoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM messages WHERE folder_id = ?1 AND seen = 0 AND hidden = 0 AND deleted = 0",
            params![folder.0],
            |r| r.get(0),
        )?;
        narrow(n, "count")
    }
}
