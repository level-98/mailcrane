//! "Load test mail" (#289): **debug builds only**. Compiled only with the
//! `debug-fixtures` feature, which the Android build turns on for the debug
//! variant alone; `cargo xtask ci` and the Gradle build check that release
//! builds never get it.
//!
//! The app reads the files from its debug assets (a copy of
//! `tests/fixtures`) and hands them over; the core imports them into the
//! Inbox of a local-only account that never syncs or connects.

use mailcrane_core::events;
use mailcrane_core::fixtures as core;
use mailcrane_store::Store;

use crate::sync::{failed, SyncFailure};

/// One fixture file: its path under `tests/fixtures` (with `/`) and bytes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FixtureFile {
    /// Path such as `html/link-mismatch.html`.
    pub path: String,
    /// The file's bytes.
    pub bytes: Vec<u8>,
}

/// What "Load test mail" did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TestMailReport {
    /// The local-only fixtures account (`fixtures@example.org`).
    pub account_id: i64,
    /// Its Inbox.
    pub inbox_folder_id: i64,
    /// Messages now in the Inbox.
    pub imported: u32,
    /// Files left out (not `.eml`/`.html`, or too large).
    pub skipped: u32,
}

/// Replaces the fixtures account's Inbox with `files`, creating the account
/// the first time. Running it again replaces rather than duplicates.
/// Blocking; call it off the main thread.
#[uniffi::export]
pub fn load_test_mail(
    db_path: String,
    files: Vec<FixtureFile>,
) -> Result<TestMailReport, SyncFailure> {
    let store = Store::open(&db_path).map_err(|_| failed(events::SyncErrorKind::Storage))?;
    let files: Vec<core::FixtureFile> = files
        .into_iter()
        .map(|f| core::FixtureFile {
            path: f.path,
            bytes: f.bytes,
        })
        .collect();
    let report = core::import_fixtures(&store, &files, crate::actions::now())
        .map_err(|_| failed(events::SyncErrorKind::Storage))?;
    // Counts only; never names or content.
    tracing::info!(
        imported = report.imported,
        skipped = report.skipped,
        "test mail loaded"
    );
    Ok(TestMailReport {
        account_id: report.account_id.0,
        inbox_folder_id: report.inbox.0,
        imported: u32::try_from(report.imported).unwrap_or(u32::MAX),
        skipped: u32::try_from(report.skipped).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_twice_without_duplicates() {
        let dir =
            std::env::temp_dir().join(format!("mailcrane-ffi-fixtures-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("m.db");
        let _ = std::fs::remove_file(&path);
        let db = path.to_string_lossy().into_owned();
        let files = vec![
            FixtureFile {
                path: "html/link-mismatch.html".into(),
                bytes: b"<a href=\"https://example.net/login\">https://example.com/login</a>"
                    .to_vec(),
            },
            FixtureFile {
                path: "mime/plain.eml".into(),
                bytes: b"Subject: Hi\r\nFrom: ann@example.org\r\n\r\nHello\r\n".to_vec(),
            },
        ];
        let first = load_test_mail(db.clone(), files.clone()).expect("first");
        let second = load_test_mail(db.clone(), files).expect("second");
        assert_eq!(first, second);
        assert_eq!(second.imported, 2);
        let store = Store::open(&path).expect("store");
        assert!(store
            .is_local_account(mailcrane_store::AccountId(second.account_id))
            .expect("local"));
        assert_eq!(store.list_accounts().expect("accounts").len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
