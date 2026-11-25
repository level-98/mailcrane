//! Migration runner using `PRAGMA user_version`.

use crate::error::StoreError;
use rusqlite::Connection;

/// Migrations in order. Entry `n` upgrades the schema from v`n` to v`n+1`.
pub(crate) const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_pending_ops.sql"),
    include_str!("../migrations/0003_account_settings.sql"),
    include_str!("../migrations/0004_drafts.sql"),
    include_str!("../migrations/0005_outbox.sql"),
    include_str!("../migrations/0006_save_sent.sql"),
    include_str!("../migrations/0007_scheduled_send.sql"),
    include_str!("../migrations/0008_search.sql"),
    include_str!("../migrations/0009_snooze.sql"),
    include_str!("../migrations/0010_contacts.sql"),
    include_str!("../migrations/0011_threading.sql"),
    include_str!("../migrations/0012_rules.sql"),
    include_str!("../migrations/0013_notifications.sql"),
    include_str!("../migrations/0014_jmap.sql"),
    include_str!("../migrations/0015_account_color.sql"),
    include_str!("../migrations/0016_folder_selectable.sql"),
    include_str!("../migrations/0017_local_accounts.sql"),
    include_str!("../migrations/0018_server_rules.sql"),
];

fn user_version(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Fails with `NewerSchema` if the file is newer than `migrations`.
/// Reads only; never modifies the database.
pub(crate) fn check_version(conn: &Connection, migrations: &[&str]) -> Result<i64, StoreError> {
    let found = user_version(conn)?;
    let supported = i64::try_from(migrations.len()).unwrap_or(i64::MAX);
    if found > supported || found < 0 {
        return Err(StoreError::NewerSchema { found, supported });
    }
    Ok(found)
}

/// Applies each pending migration in its own transaction, setting
/// `user_version` inside it. A failure rolls back that migration only.
pub(crate) fn run(conn: &mut Connection, migrations: &[&str]) -> Result<(), StoreError> {
    let current = check_version(conn, migrations)?;
    let start = usize::try_from(current).unwrap_or(usize::MAX);
    for (index, sql) in migrations.iter().enumerate().skip(start) {
        let target = i64::try_from(index)
            .ok()
            .and_then(|i| i.checked_add(1))
            .ok_or(StoreError::Invalid { field: "migration" })?;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // `PRAGMA user_version` does not accept bound parameters; `target` is an integer.
        tx.execute_batch(&format!("PRAGMA user_version = {target}"))?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    type R = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn v16_with_rows_upgrades_with_every_account_remote() -> R {
        // A database as main had it before #289: schema v16, with an account,
        // a folder and a message.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("m.db");
        let v16 = MIGRATIONS.get(..16).ok_or("fewer than 16 migrations")?;
        {
            let mut conn = Connection::open(&path)?;
            run(&mut conn, v16)?;
            assert_eq!(user_version(&conn)?, 16);
            conn.execute_batch(
                "INSERT INTO accounts (display_name, email, imap_host, imap_port, imap_security, \
                 smtp_host, smtp_port, smtp_security, username) VALUES \
                 ('A','a@example.org','imap.example.org',993,'tls','smtp.example.org',465,'tls','a'); \
                 INSERT INTO folders (account_id, remote_name, display_name, is_subscribed) \
                 VALUES (1, 'INBOX', 'Inbox', 1); \
                 INSERT INTO messages (folder_id, uid, subject, date, internal_date, size) \
                 VALUES (1, 7, 'Hello', 1790000000, 1790000000, 10);",
            )?;
        }

        let store = crate::Store::open(&path)?;
        // Every later migration runs too (v18 is server rules, M4-23).
        assert_eq!(store.schema_version()?, i64::try_from(MIGRATIONS.len())?);
        let accounts = store.list_accounts()?;
        assert_eq!(accounts.len(), 1);
        let id = accounts.first().ok_or("account")?.id;
        assert!(
            !store.is_local_account(id)?,
            "existing accounts stay remote"
        );
        let folders = store.list_folders(id)?;
        assert_eq!(folders.len(), 1);
        let folder = folders.first().ok_or("folder")?.id;
        assert_eq!(store.list_messages(folder, 10, 0)?.len(), 1);
        // The new column only takes 0 or 1.
        drop(store);
        let conn = Connection::open(&path)?;
        assert!(conn
            .execute("UPDATE accounts SET local_only = 2", [])
            .is_err());
        Ok(())
    }

    #[test]
    fn broken_second_migration_leaves_v1_and_data() -> R {
        let mut conn = Connection::open_in_memory()?;
        run(&mut conn, &["CREATE TABLE t (x INTEGER);"])?;
        conn.execute("INSERT INTO t VALUES (7)", [])?;

        let broken = [
            "CREATE TABLE t (x INTEGER);",
            "CREATE TABLE half (y INTEGER); INSERT INTO nope VALUES (1);",
        ];
        assert!(run(&mut conn, &broken).is_err());

        assert_eq!(user_version(&conn)?, 1);
        let x: i64 = conn.query_row("SELECT x FROM t", [], |r| r.get(0))?;
        assert_eq!(x, 7);
        let half: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'half'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(half, 0, "failed migration must be rolled back entirely");
        Ok(())
    }
}
