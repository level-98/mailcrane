use super::*;
use crate::folders::{can_manage_folders, list_folders, move_targets};
use mailcrane_store::{FolderRole as Role, NewAccount, NewFolder};

#[derive(Default)]
struct Quiet;

impl CoreEventListener for Quiet {
    fn on_event(&self, _event: CoreEvent) {}
}

/// A local-only account (like the debug Fixtures account) with an Inbox,
/// and its database path.
fn local_account() -> (tempfile::TempDir, String, i64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("f.db").to_string_lossy().into_owned();
    let store = Store::open(&path).expect("store");
    let account = store
        .insert_local_account(&NewAccount {
            display_name: "Fixtures".into(),
            email: "fixtures@example.org".into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: "fixtures@example.org".into(),
        })
        .expect("account");
    store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "INBOX".into(),
            delimiter: None,
            role: Some(Role::Inbox),
            uidvalidity: Some(1),
            uidnext: Some(1),
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("inbox");
    (dir, path, account.0)
}

fn names(path: &str, account: i64) -> Vec<(String, u32, bool)> {
    list_folders(path.to_owned(), account)
        .expect("list")
        .into_iter()
        .map(|f| (f.display_name, f.depth, f.can_change))
        .collect()
}

#[test]
fn a_local_account_creates_renames_moves_and_deletes_folders() {
    let (_dir, path, account) = local_account();
    let engine = SyncEngine::new(path.clone(), Arc::new(Quiet));
    assert_eq!(can_manage_folders(path.clone(), account), Ok(true));
    // The Inbox can't be changed.
    assert_eq!(names(&path, account), [("INBOX".into(), 0, false)]);

    let projects = engine
        .create_folder(account, None, " Projects ".into())
        .expect("create");
    let year = engine
        .create_folder(account, Some(projects), "2026".into())
        .expect("create");
    let notes = engine
        .create_folder(account, None, "Notes".into())
        .expect("create");
    assert_eq!(
        names(&path, account),
        [
            ("INBOX".into(), 0, false),
            ("Notes".into(), 0, true),
            ("Projects".into(), 0, true),
            ("2026".into(), 1, true),
        ]
    );

    engine
        .rename_folder(projects, "Work".into())
        .expect("rename");
    let targets = move_targets(path.clone(), notes).expect("targets");
    assert!(!targets.to_top);
    let ids: Vec<i64> = targets.folders.iter().map(|f| f.id).collect();
    assert!(ids.contains(&projects) && ids.contains(&year) && !ids.contains(&notes));
    engine.move_folder(notes, Some(year)).expect("move");
    assert_eq!(
        names(&path, account),
        [
            ("INBOX".into(), 0, false),
            ("Work".into(), 0, true),
            ("2026".into(), 1, true),
            ("Notes".into(), 2, true),
        ]
    );

    // Refusals change nothing.
    assert_eq!(
        engine.delete_folder(projects),
        Err(FolderFailure::HasSubfolders)
    );
    assert_eq!(
        engine.move_folder(projects, Some(notes)),
        Err(FolderFailure::IntoItself)
    );
    assert_eq!(
        engine.create_folder(account, Some(year), "Notes".into()),
        Err(FolderFailure::AlreadyExists)
    );
    assert_eq!(
        engine.create_folder(account, None, "a/b".into()),
        Err(FolderFailure::InvalidName {
            problem: FolderNameProblem::Separator {
                separator: "/".into()
            }
        })
    );
    let inbox = list_folders(path.clone(), account).expect("list")[0].id;
    assert_eq!(
        engine.delete_folder(inbox),
        Err(FolderFailure::SystemFolder)
    );
    assert_eq!(
        engine.rename_folder(999, "X".into()),
        Err(FolderFailure::NotFound)
    );

    engine.delete_folder(notes).expect("delete");
    engine.delete_folder(year).expect("delete");
    engine.delete_folder(projects).expect("delete");
    assert_eq!(names(&path, account), [("INBOX".into(), 0, false)]);
}

#[test]
fn an_inbox_namespace_is_the_top_level() {
    // #325: every personal folder under INBOX., "." as delimiter.
    let (_dir, path, account) = local_account();
    let store = Store::open(&path).expect("store");
    let inbox = store.list_folders(AccountId(account)).expect("list")[0].id;
    let mut details = store
        .get_folder(inbox)
        .expect("get")
        .expect("inbox")
        .details;
    details.delimiter = Some(".".into());
    store.upsert_folder(&details).expect("delimiter");
    // A folder already under INBOX.: the list suggests the namespace.
    details.remote_name = "INBOX.Archive".into();
    details.display_name = "INBOX.Archive".into();
    details.role = None;
    store.upsert_folder(&details).expect("archive");
    drop(store);
    let engine = SyncEngine::new(path.clone(), Arc::new(Quiet));

    let projects = engine
        .create_folder(account, None, "Projects".into())
        .expect("create");
    let year = engine
        .create_folder(account, Some(projects), "2026".into())
        .expect("create");
    let store = Store::open(&path).expect("store");
    let remote = |id: i64| {
        store
            .get_folder(FolderId(id))
            .expect("get")
            .expect("folder")
            .details
            .remote_name
    };
    assert_eq!(remote(projects), "INBOX.Projects");
    assert_eq!(remote(year), "INBOX.Projects.2026");
    // Shown without the namespace: Projects at the top, 2026 under it.
    assert_eq!(
        names(&path, account),
        [
            ("INBOX".into(), 0, false),
            ("Archive".into(), 0, true),
            ("Projects".into(), 0, true),
            ("2026".into(), 1, true),
        ]
    );
    // INBOX is the top, not a place of its own; moving to the top stays
    // in the namespace.
    let targets = move_targets(path.clone(), year).expect("targets");
    assert!(targets.to_top);
    assert!(targets.folders.iter().all(|f| f.id != inbox.0));
    engine.move_folder(year, None).expect("move");
    assert_eq!(remote(year), "INBOX.2026");
}
