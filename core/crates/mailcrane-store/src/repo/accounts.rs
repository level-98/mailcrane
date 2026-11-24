use super::narrow;
use crate::{Account, AccountId, NewAccount, Security, Store, StoreError};
use rusqlite::{params, Row};

const COLS: &str = "id, display_name, email, imap_host, imap_port, imap_security, \
                    smtp_host, smtp_port, smtp_security, username, created_at";

fn from_row(r: &Row<'_>) -> Result<Account, StoreError> {
    Ok(Account {
        id: AccountId(r.get(0)?),
        details: NewAccount {
            display_name: r.get(1)?,
            email: r.get(2)?,
            imap_host: r.get(3)?,
            imap_port: narrow(r.get(4)?, "imap_port")?,
            imap_security: Security::parse(&r.get::<_, String>(5)?, "imap_security")?,
            smtp_host: r.get(6)?,
            smtp_port: narrow(r.get(7)?, "smtp_port")?,
            smtp_security: Security::parse(&r.get::<_, String>(8)?, "smtp_security")?,
            username: r.get(9)?,
        },
        created_at: r.get(10)?,
    })
}

impl Store {
    /// Inserts an account and returns its id.
    pub fn insert_account(&self, a: &NewAccount) -> Result<AccountId, StoreError> {
        self.conn.execute(
            "INSERT INTO accounts (display_name, email, imap_host, imap_port, imap_security, \
             smtp_host, smtp_port, smtp_security, username) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                a.display_name,
                a.email,
                a.imap_host,
                a.imap_port,
                a.imap_security.as_str(),
                a.smtp_host,
                a.smtp_port,
                a.smtp_security.as_str(),
                a.username
            ],
        )?;
        Ok(AccountId(self.conn.last_insert_rowid()))
    }

    /// Returns the account, or `None` if it doesn't exist.
    pub fn get_account(&self, id: AccountId) -> Result<Option<Account>, StoreError> {
        let sql = format!("SELECT {COLS} FROM accounts WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![id.0])?;
        rows.next()?.map(from_row).transpose()
    }

    /// Lists all accounts, oldest first.
    pub fn list_accounts(&self) -> Result<Vec<Account>, StoreError> {
        let sql = format!("SELECT {COLS} FROM accounts ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(from_row(r)?);
        }
        Ok(out)
    }

    /// Deletes an account and, by cascade, everything under it. Returns
    /// whether an account was deleted.
    pub fn delete_account(&self, id: AccountId) -> Result<bool, StoreError> {
        let n = self
            .conn
            .execute("DELETE FROM accounts WHERE id = ?1", params![id.0])?;
        Ok(n > 0)
    }
}
