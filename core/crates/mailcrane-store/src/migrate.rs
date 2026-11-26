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
    include_str!("../migrations/0019_rules_missing_folder.sql"),
    include_str!("../migrations/0020_provider_authserv_id.sql"),
    include_str!("../migrations/0021_jmap_push.sql"),
    include_str!("../migrations/0022_oauth_provider.sql"),
    include_str!("../migrations/0023_snooze_step.sql"),
    include_str!("../migrations/0024_snooze_pending_resnooze.sql"),
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
    fn v17_with_rows_upgrades_to_v18_with_no_server_rules() -> R {
        // A database as main had it before M4-23: schema v17, with a
        // local-only account, a remote one, a folder, a message and a rule.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("m.db");
        let v17 = MIGRATIONS.get(..17).ok_or("fewer than 17 migrations")?;
        {
            let mut conn = Connection::open(&path)?;
            run(&mut conn, v17)?;
            assert_eq!(user_version(&conn)?, 17);
            conn.execute_batch(
                "INSERT INTO accounts (display_name, email, imap_host, imap_port, imap_security, \
                 smtp_host, smtp_port, smtp_security, username, local_only) VALUES \
                 ('A','a@example.org','imap.example.org',993,'tls','smtp.example.org',465,'tls','a',0), \
                 ('F','fixtures@example.org','localhost',993,'tls','localhost',465,'tls','f',1); \
                 INSERT INTO folders (account_id, remote_name, display_name, is_subscribed) \
                 VALUES (1, 'INBOX', 'Inbox', 1); \
                 INSERT INTO messages (folder_id, uid, subject, date, internal_date, size) \
                 VALUES (1, 7, 'Hello', 1790000000, 1790000000, 10); \
                 INSERT INTO rules (account_id, name, position, enabled, match_all, stop_processing) \
                 VALUES (NULL, 'Invoices', 0, 1, 1, 0); \
                 INSERT INTO rule_conditions (rule_id, position, kind, value) \
                 VALUES (1, 0, 'subject', 'Invoice'); \
                 INSERT INTO rule_actions (rule_id, position, kind, folder_id) \
                 VALUES (1, 0, 'flag', NULL);",
            )?;
        }

        let store = crate::Store::open(&path)?;
        assert_eq!(store.schema_version()?, i64::try_from(MIGRATIONS.len())?);
        let accounts = store.list_accounts()?;
        assert_eq!(accounts.len(), 2);
        let remote = accounts.first().ok_or("account")?.id;
        let local = accounts.get(1).ok_or("account")?.id;
        assert!(!store.is_local_account(remote)? && store.is_local_account(local)?);
        let folder = store.list_folders(remote)?.first().ok_or("folder")?.id;
        assert_eq!(store.list_messages(folder, 10, 0)?.len(), 1);
        let rules = store.list_rules(None)?;
        assert_eq!(rules.len(), 1);
        let rule = rules.first().ok_or("rule")?;
        assert_eq!(rule.rule.name, "Invoices");
        // No account has been checked yet, so every rule runs on the device.
        assert!(store.list_server_rules()?.is_empty());
        assert_eq!(store.server_rules(remote)?, None);
        // The new tables work on the upgraded file.
        store.save_server_rules(&crate::ServerRules {
            account_id: remote,
            state: crate::ServerRulesState::Active,
            host: Some("imap.example.org".into()),
            port: Some(4190),
            extensions: vec!["fileinto".into()],
            other_script: None,
            script_hash: Some("h".into()),
            checked_at: 1,
            switch_requested: false,
            deployed: vec![(rule.id, "f".into())],
        })?;
        assert_eq!(
            store.server_rules(remote)?.map(|s| s.deployed),
            Some(vec![(rule.id, "f".to_string())])
        );
        // The state column only takes the three known states.
        drop(store);
        let conn = Connection::open(&path)?;
        assert!(conn
            .execute("UPDATE server_rules SET state = 'other'", [])
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

    #[test]
    fn v22_snoozes_upgrade_with_no_step() -> R {
        // A snooze row as main had it before #423: schema v22, mid-move.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("m.db");
        let v22 = MIGRATIONS.get(..22).ok_or("fewer than 22 migrations")?;
        {
            let mut conn = Connection::open(&path)?;
            run(&mut conn, v22)?;
            assert_eq!(user_version(&conn)?, 22);
            conn.execute_batch(
                "INSERT INTO accounts (display_name, email, imap_host, imap_port, imap_security, \
                 smtp_host, smtp_port, smtp_security, username) VALUES \
                 ('A','a@example.org','imap.example.org',993,'tls','smtp.example.org',465,'tls','a'); \
                 INSERT INTO snoozes (account_id, message_id_header, wake_at, state) \
                 VALUES (1, 'm1@example.org', 1795600000, 'to_snooze');",
            )?;
        }

        let store = crate::Store::open(&path)?;
        assert_eq!(store.schema_version()?, i64::try_from(MIGRATIONS.len())?);
        let snoozes = store.list_snoozes(None)?;
        let old = snoozes.first().ok_or("snooze")?;
        assert_eq!((old.step, old.target_uidnext), (0, None));
        // The new column only takes the five known steps.
        drop(store);
        let conn = Connection::open(&path)?;
        assert!(conn.execute("UPDATE snoozes SET step = 5", []).is_err());
        Ok(())
    }
}
