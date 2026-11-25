//! Past correspondents (M2-08). See `migrations/0010_contacts.sql`.

use super::narrow;
use crate::{AccountId, Store, StoreError};
use rusqlite::params;

/// Someone the user has written to or heard from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondent {
    pub account_id: AccountId,
    /// Lower-case address.
    pub email: String,
    /// The last display name seen; may be empty.
    pub name: String,
    pub sent_to_count: u32,
    pub received_from_count: u32,
    /// Unix seconds.
    pub last_seen_at: i64,
}

impl Store {
    /// Counts one sent message to each of `recipients` (address, name).
    /// Addresses are lower-cased; ones without `@` are skipped.
    pub fn record_sent_to(
        &self,
        account: AccountId,
        recipients: &[(String, String)],
        at: i64,
    ) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        for (email, name) in recipients {
            let email = email.trim().to_lowercase();
            if !email.contains('@') {
                continue;
            }
            tx.execute(
                "INSERT INTO correspondents (account_id, email, name, sent_to_count, last_seen_at) \
                 VALUES (?1, ?2, ?3, 1, ?4) \
                 ON CONFLICT (account_id, email) DO UPDATE SET \
                 sent_to_count = sent_to_count + 1, \
                 last_seen_at = max(last_seen_at, excluded.last_seen_at), \
                 name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE name END",
                params![account.0, email, name.trim(), at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every correspondent of `account` (`None`: of every account) with at
    /// least one message counted.
    pub fn list_correspondents(
        &self,
        account: Option<AccountId>,
    ) -> Result<Vec<Correspondent>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT account_id, email, name, sent_to_count, received_from_count, last_seen_at \
             FROM correspondents WHERE (?1 IS NULL OR account_id = ?1) \
             AND (sent_to_count > 0 OR received_from_count > 0)",
        )?;
        let rows = stmt.query_map(params![account.map(|a| a.0)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (account, email, name, sent, received, seen) = row?;
            out.push(Correspondent {
                account_id: AccountId(account),
                email,
                name,
                sent_to_count: narrow(sent, "sent_to_count")?,
                received_from_count: narrow(received, "received_from_count")?,
                last_seen_at: seen,
            });
        }
        Ok(out)
    }
}
