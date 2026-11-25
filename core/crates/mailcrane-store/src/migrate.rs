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
    include_str!("../migrations/0012_rules.sql"),
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
