use super::narrow;
use crate::{AccountId, Folder, FolderId, FolderRole, NewFolder, Store, StoreError};
use rusqlite::{params, Row};

fn from_row(r: &Row<'_>) -> Result<Folder, StoreError> {
    let role: Option<String> = r.get(5)?;
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
        },
    })
}

impl Store {
    /// Inserts the folder or, if `(account, remote_name)` exists, updates it.
    pub fn upsert_folder(&self, f: &NewFolder) -> Result<FolderId, StoreError> {
        let id: i64 = self.conn.query_row(
            "INSERT INTO folders (account_id, remote_name, display_name, delimiter, role, \
             uidvalidity, uidnext, highestmodseq, is_subscribed) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
             ON CONFLICT (account_id, remote_name) DO UPDATE SET \
               display_name = excluded.display_name, delimiter = excluded.delimiter, \
               role = excluded.role, uidvalidity = excluded.uidvalidity, \
               uidnext = excluded.uidnext, highestmodseq = excluded.highestmodseq, \
               is_subscribed = excluded.is_subscribed \
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
                f.is_subscribed
            ],
            |r| r.get(0),
        )?;
        Ok(FolderId(id))
    }

    /// Upserts every folder in one transaction: either all are saved or,
    /// on error, none are.
    pub fn upsert_folders(&self, folders: &[NewFolder]) -> Result<Vec<FolderId>, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = Vec::with_capacity(folders.len());
        for f in folders {
            ids.push(self.upsert_folder(f)?);
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Lists an account's folders ordered by remote name.
    pub fn list_folders(&self, account: AccountId) -> Result<Vec<Folder>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, remote_name, display_name, delimiter, role, uidvalidity, \
             uidnext, highestmodseq, is_subscribed FROM folders WHERE account_id = ?1 \
             ORDER BY remote_name",
        )?;
        let mut rows = stmt.query(params![account.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }

    /// Counts messages in the folder that are not `\Seen`.
    pub fn count_unread(&self, folder: FolderId) -> Result<u64, StoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM messages WHERE folder_id = ?1 AND seen = 0",
            params![folder.0],
            |r| r.get(0),
        )?;
        narrow(n, "count")
    }
}
