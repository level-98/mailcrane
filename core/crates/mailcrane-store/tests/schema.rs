//! Schema shape, migrations and file handling.

use mailcrane_store::{Store, StoreError};
use rusqlite::Connection;

type R = Result<(), Box<dyn std::error::Error>>;

fn names(conn: &Connection, kind: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([kind], |r| r.get(0))?;
    rows.collect()
}

#[test]
fn every_table_and_index_exists() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("m.db");
    drop(Store::open(&path)?);
    let conn = Connection::open(&path)?;
    assert_eq!(
        names(&conn, "table")?,
        [
            "accounts",
            "attachments",
            "drafts",
            "folders",
            "message_bodies",
            "messages",
            // FTS5 search (M4-03) and its shadow tables.
            "messages_fts",
            "messages_fts_config",
            "messages_fts_content",
            "messages_fts_data",
            "messages_fts_docsize",
            "messages_fts_idx",
            "outbox",
            "pending_op_messages",
            "pending_ops",
            "snoozes",
            "threads"
        ]
    );
    let indexes = names(&conn, "index")?;
    for want in [
        "idx_messages_folder_date",
        "idx_messages_message_id_header",
        "idx_messages_thread_id",
        "idx_pending_ops_account",
        "idx_pending_op_messages_message",
    ] {
        assert!(indexes.iter().any(|i| i == want), "missing index {want}");
    }
    Ok(())
}

#[test]
fn no_column_can_hold_a_secret() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("m.db");
    drop(Store::open(&path)?);
    let conn = Connection::open(&path)?;
    for table in names(&conn, "table")? {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get(1))?
            .collect::<Result<_, _>>()?;
        for c in cols {
            for bad in ["password", "passwd", "token", "secret", "credential"] {
                assert!(!c.contains(bad), "{table}.{c} looks like a secret");
            }
        }
    }
    Ok(())
}

#[test]
fn reopen_runs_no_migrations_and_keeps_data() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("m.db");
    let store = Store::open(&path)?;
    assert_eq!(store.schema_version()?, 9);
    drop(store);
    let conn = Connection::open(&path)?;
    conn.execute(
        "INSERT INTO accounts (display_name, email, imap_host, imap_port, imap_security, \
         smtp_host, smtp_port, smtp_security, username) \
         VALUES ('A','a@example.org','imap.example.org',993,'tls','smtp.example.org',465,'tls','a')",
        [],
    )?;
    drop(conn);
    let store = Store::open(&path)?;
    assert_eq!(store.schema_version()?, 9);
    assert_eq!(store.list_accounts()?.len(), 1);
    Ok(())
}

#[test]
fn newer_schema_is_refused_and_file_untouched() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("m.db");
    let conn = Connection::open(&path)?;
    conn.execute_batch("CREATE TABLE keep (x); PRAGMA user_version = 99;")?;
    drop(conn);
    let before = std::fs::read(&path)?;

    let err = Store::open(&path).err();
    assert!(
        matches!(err, Some(StoreError::NewerSchema { found: 99, .. })),
        "{err:?}"
    );
    assert_eq!(std::fs::read(&path)?, before);
    assert!(!dir.path().join("m.db-wal").exists());
    Ok(())
}

#[test]
fn connection_pragmas_are_set() -> R {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("m.db");
    drop(Store::open(&path)?);
    let conn = Connection::open(&path)?;
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    assert_eq!(mode, "wal");
    Ok(())
}
