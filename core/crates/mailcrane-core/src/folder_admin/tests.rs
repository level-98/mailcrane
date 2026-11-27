use super::*;
use mailcrane_store::FolderRole;

const ACCOUNT: AccountId = AccountId(1);

fn folder(id: i64, remote: &str, role: Option<FolderRole>) -> Folder {
    Folder {
        id: FolderId(id),
        details: NewFolder {
            account_id: ACCOUNT,
            remote_name: remote.into(),
            display_name: decode(remote),
            delimiter: Some("/".into()),
            role,
            uidvalidity: Some(1),
            uidnext: Some(1),
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
            virtual_role: None,
        },
    }
}

/// INBOX, Sent, Trash, Projects, Projects/2026, Projects/2026/Q1, Notes,
/// Snoozed, and Gmail-style [Gmail]/Spam.
fn folders() -> Vec<Folder> {
    vec![
        folder(1, "INBOX", Some(FolderRole::Inbox)),
        folder(2, "Sent", Some(FolderRole::Sent)),
        folder(3, "Trash", Some(FolderRole::Trash)),
        folder(4, "Projects", None),
        folder(5, "Projects/2026", None),
        folder(6, "Projects/2026/Q1", None),
        folder(7, "Notes", None),
        folder(8, "Snoozed", None),
        folder(9, "[Gmail]", None),
        folder(10, "[Gmail]/Spam", Some(FolderRole::Spam)),
    ]
}

#[test]
fn names_are_checked() {
    assert_eq!(
        clean_folder_name("  Receipts ", Some('/')),
        Ok("Receipts".into())
    );
    assert_eq!(
        clean_folder_name("Café ☕", Some('/')),
        Ok("Café ☕".into())
    );
    for (name, want) in [
        ("", FolderNameError::Empty),
        ("   ", FolderNameError::Empty),
        ("a/b", FolderNameError::Delimiter('/')),
        ("50%", FolderNameError::BadCharacter),
        ("a*", FolderNameError::BadCharacter),
        ("a\tb", FolderNameError::BadCharacter),
        ("inbox", FolderNameError::Reserved),
        ("..", FolderNameError::Reserved),
    ] {
        assert_eq!(clean_folder_name(name, Some('/')), Err(want), "{name:?}");
    }
    assert_eq!(
        clean_folder_name(&"x".repeat(MAX_FOLDER_NAME_CHARS + 1), Some('/')),
        Err(FolderNameError::TooLong)
    );
    assert_eq!(
        clean_folder_name("a.b", Some('.')),
        Err(FolderNameError::Delimiter('.'))
    );
    assert_eq!(clean_folder_name("a/b", None), Ok("a/b".into()));
}

#[test]
fn create_at_the_top_and_under_a_folder() {
    let all = folders();
    let p = plan_create(&all, ACCOUNT, None, " Receipts ", None).expect("plan");
    assert_eq!(
        (p.remote_name.as_str(), p.delimiter),
        ("Receipts", Some('/'))
    );
    let p = plan_create(&all, ACCOUNT, Some(FolderId(5)), "Q2", None).expect("plan");
    assert_eq!(p.remote_name, "Projects/2026/Q2");
    // Non-ASCII names are sent in modified UTF-7 and stored decoded.
    let p = plan_create(&all, ACCOUNT, None, "Привет", None).expect("plan");
    assert_eq!(p.remote_name, mutf7::encode("Привет"));
    assert_eq!(p.display_name, "Привет");
}

#[test]
fn create_refuses_taken_names_and_unknown_parents() {
    let all = folders();
    assert!(matches!(
        plan_create(&all, ACCOUNT, None, "Notes", None),
        Err(FolderAdminError::AlreadyExists)
    ));
    assert!(matches!(
        plan_create(&all, ACCOUNT, Some(FolderId(4)), "2026", None),
        Err(FolderAdminError::AlreadyExists)
    ));
    assert!(matches!(
        plan_create(&all, ACCOUNT, Some(FolderId(99)), "X", None),
        Err(FolderAdminError::NotFound)
    ));
    assert!(matches!(
        plan_create(&all, ACCOUNT, None, "a/b", None),
        Err(FolderAdminError::InvalidName(FolderNameError::Delimiter(
            '/'
        )))
    ));
}

#[test]
fn a_server_without_hierarchy_only_allows_top_level_folders() {
    let mut flat = vec![
        folder(1, "INBOX", Some(FolderRole::Inbox)),
        folder(2, "Notes", None),
    ];
    for f in &mut flat {
        f.details.delimiter = None;
    }
    assert_eq!(
        plan_create(&flat, ACCOUNT, None, "a/b", None)
            .expect("plan")
            .remote_name,
        "a/b"
    );
    assert!(matches!(
        plan_create(&flat, ACCOUNT, Some(FolderId(2)), "X", None),
        Err(FolderAdminError::NoSubfolders)
    ));
    assert!(matches!(
        plan_move(&flat, FolderId(2), Some(FolderId(1))),
        Err(FolderAdminError::SystemFolder | FolderAdminError::NoSubfolders)
    ));
    // A local-only account with no delimiter yet gets the default.
    let local = vec![folder(1, "INBOX", Some(FolderRole::Inbox))];
    let mut local = local;
    local[0].details.delimiter = None;
    let p = plan_create(&local, ACCOUNT, Some(FolderId(1)), "Sub", Some('/')).expect("plan");
    assert_eq!(p.remote_name, "INBOX/Sub");
}

#[test]
fn standard_folders_and_folders_holding_them_are_protected() {
    let all = folders();
    for id in [1, 2, 3, 8, 9, 10] {
        let id = FolderId(id);
        assert!(
            matches!(
                plan_rename(&all, id, "X"),
                Err(FolderAdminError::SystemFolder)
            ),
            "{id:?}"
        );
        assert!(
            matches!(
                plan_move(&all, id, None),
                Err(FolderAdminError::SystemFolder)
            ),
            "{id:?}"
        );
        assert!(
            matches!(plan_delete(&all, id), Err(FolderAdminError::SystemFolder)),
            "{id:?}"
        );
    }
    assert!(can_change(&all, &all[3]));
    assert!(!can_change(&all, &all[8]));
}

#[test]
fn rename_takes_the_subfolders_along() {
    let all = folders();
    let p = plan_rename(&all, FolderId(4), "Work").expect("plan");
    assert_eq!(
        (p.old_remote.as_str(), p.new_remote.as_str()),
        ("Projects", "Work")
    );
    assert_eq!(
        p.rows,
        vec![
            (FolderId(4), "Work".into(), "Work".into()),
            (FolderId(5), "Work/2026".into(), "Work/2026".into()),
            (FolderId(6), "Work/2026/Q1".into(), "Work/2026/Q1".into()),
        ]
    );
    let p = plan_rename(&all, FolderId(6), "First quarter").expect("plan");
    assert_eq!(p.new_remote, "Projects/2026/First quarter");
    assert_eq!(p.rows.len(), 1);
    // The same name is a no-op, a sibling's name is refused.
    assert!(plan_rename(&all, FolderId(7), " Notes ")
        .expect("plan")
        .is_noop());
    assert!(matches!(
        plan_rename(&all, FolderId(7), "Projects"),
        Err(FolderAdminError::AlreadyExists)
    ));
    assert!(matches!(
        plan_rename(&all, FolderId(7), "inbox"),
        Err(FolderAdminError::InvalidName(_))
    ));
}

#[test]
fn move_keeps_the_name_and_refuses_cycles() {
    let all = folders();
    let p = plan_move(&all, FolderId(7), Some(FolderId(5))).expect("plan");
    assert_eq!(p.new_remote, "Projects/2026/Notes");
    let p = plan_move(&all, FolderId(5), None).expect("plan");
    assert_eq!(
        p.rows,
        vec![
            (FolderId(5), "2026".into(), "2026".into()),
            (FolderId(6), "2026/Q1".into(), "2026/Q1".into()),
        ]
    );
    assert!(plan_move(&all, FolderId(5), Some(FolderId(4)))
        .expect("plan")
        .is_noop());
    for target in [4, 5, 6] {
        assert!(matches!(
            plan_move(&all, FolderId(4), Some(FolderId(target))),
            Err(FolderAdminError::IntoItself)
        ));
    }
    // A sibling of the same name at the target is refused.
    let mut taken = all.clone();
    taken.push(folder(11, "Projects/Notes", None));
    assert!(matches!(
        plan_move(&taken, FolderId(7), Some(FolderId(4))),
        Err(FolderAdminError::AlreadyExists)
    ));
    // A folder can go under a standard folder (if the server allows it).
    assert_eq!(
        plan_move(&all, FolderId(7), Some(FolderId(1)))
            .expect("plan")
            .new_remote,
        "INBOX/Notes"
    );
}

#[test]
fn delete_needs_an_empty_folder() {
    let all = folders();
    assert!(matches!(
        plan_delete(&all, FolderId(4)),
        Err(FolderAdminError::HasSubfolders)
    ));
    assert!(matches!(
        plan_delete(&all, FolderId(5)),
        Err(FolderAdminError::HasSubfolders)
    ));
    assert_eq!(
        plan_delete(&all, FolderId(6)).expect("plan").id,
        FolderId(6)
    );
    assert!(matches!(
        plan_delete(&all, FolderId(99)),
        Err(FolderAdminError::NotFound)
    ));
}

#[test]
fn similar_names_are_not_subfolders() {
    // "Projects2" is not under "Projects".
    let mut all = folders();
    all.push(folder(11, "Projects2", None));
    let p = plan_rename(&all, FolderId(4), "Work").expect("plan");
    assert!(p.rows.iter().all(|(id, ..)| *id != FolderId(11)));
    assert!(plan_delete(&all, FolderId(11)).is_ok());
}

#[test]
fn saving_keeps_messages_and_sync_state() {
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&mailcrane_store::NewAccount {
            display_name: "A".into(),
            email: "a@example.org".into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            imap_security: mailcrane_store::Security::Tls,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 465,
            smtp_security: mailcrane_store::Security::Tls,
            username: "a".into(),
        })
        .expect("account");
    let listed = store.list_folders(account).expect("list");
    let plan = plan_create(&listed, account, None, "Projects", Some('/')).expect("plan");
    let parent = save_created(&store, &plan).expect("save");
    let listed = store.list_folders(account).expect("list");
    let child = save_created(
        &store,
        &plan_create(&listed, account, Some(parent), "2026", Some('/')).expect("plan"),
    )
    .expect("save");
    store.reset_folder(child, 7, 42).expect("state");

    let listed = store.list_folders(account).expect("list");
    save_renamed(&store, &plan_rename(&listed, parent, "Work").expect("plan")).expect("rename");
    let moved = store.get_folder(child).expect("get").expect("folder");
    assert_eq!(moved.details.remote_name, "Work/2026");
    assert_eq!(moved.details.uidvalidity, Some(7));

    let listed = store.list_folders(account).expect("list");
    save_deleted(&store, &plan_delete(&listed, child).expect("plan")).expect("delete");
    assert!(store.get_folder(child).expect("get").is_none());
    assert!(store.get_folder(parent).expect("get").is_some());
}

mod imap {
    use super::*;
    use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret};
    use mailcrane_testkit::FakeImap;
    use std::time::Duration;

    const USER: &str = "ann@example.org";
    const PASSWORD: &str = "app-password";

    async fn connect(imap: &FakeImap) -> Session {
        let mut config =
            ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
                .with_extra_root_cert(imap.root_cert_der().to_vec());
        config.command_timeout = Duration::from_millis(500);
        Session::connect(&config, USER, &Secret::new(PASSWORD))
            .await
            .expect("connects")
    }

    fn account(store: &Store) -> AccountId {
        store
            .insert_account(&mailcrane_store::NewAccount {
                display_name: "Ann".into(),
                email: USER.into(),
                imap_host: "localhost".into(),
                imap_port: 993,
                imap_security: mailcrane_store::Security::Tls,
                smtp_host: "localhost".into(),
                smtp_port: 465,
                smtp_security: mailcrane_store::Security::Tls,
                username: USER.into(),
            })
            .expect("account")
    }

    #[tokio::test]
    async fn create_rename_move_and_delete_on_the_server() {
        let imap = FakeImap::builder()
            .user(USER, PASSWORD)
            .folder("INBOX", Vec::new())
            .folder("Projects", Vec::new())
            .folder("Projects/2026", Vec::new())
            .folder("Notes", Vec::new())
            .start()
            .await
            .expect("fake imap starts");
        let store = Store::open_in_memory().expect("store");
        let account = account(&store);
        let mut session = connect(&imap).await;
        save_folders(
            &store,
            account,
            &session.list_folders().await.expect("list"),
        )
        .expect("save");
        let id = |store: &Store, remote: &str| {
            store
                .list_folders(account)
                .expect("list")
                .into_iter()
                .find(|f| f.details.remote_name == remote)
                .map(|f| f.id)
        };

        // Create.
        let listed = store.list_folders(account).expect("list");
        let plan =
            plan_create(&listed, account, id(&store, "Projects"), "Receipts", None).expect("plan");
        create_on_server(&mut session, &plan).await.expect("create");
        save_created(&store, &plan).expect("save");

        // Rename, with the subfolders.
        let listed = store.list_folders(account).expect("list");
        let projects = id(&store, "Projects").expect("projects");
        let plan = plan_rename(&listed, projects, "Work").expect("plan");
        rename_on_server(&mut session, &plan).await.expect("rename");
        save_renamed(&store, &plan).expect("save");

        // Move to the top.
        let listed = store.list_folders(account).expect("list");
        let receipts = id(&store, "Work/Receipts").expect("receipts");
        let plan = plan_move(&listed, receipts, None).expect("plan");
        rename_on_server(&mut session, &plan).await.expect("move");
        save_renamed(&store, &plan).expect("save");

        // Delete.
        let listed = store.list_folders(account).expect("list");
        let notes = plan_delete(&listed, id(&store, "Notes").expect("notes")).expect("plan");
        delete_on_server(&mut session, &notes)
            .await
            .expect("delete");
        save_deleted(&store, &notes).expect("save");

        let mut server = imap.mailbox().folder_names().await;
        server.sort();
        assert_eq!(server, ["INBOX", "Receipts", "Work", "Work/2026"]);
        let mut local: Vec<String> = store
            .list_folders(account)
            .expect("list")
            .into_iter()
            .map(|f| f.details.remote_name)
            .collect();
        local.sort();
        assert_eq!(local, server);
        // The renamed folder kept its row.
        assert_eq!(id(&store, "Work"), Some(projects));

        // A name taken on the server but not known locally is refused.
        imap.mailbox().create_folder("Archive2").await;
        let listed = store.list_folders(account).expect("list");
        let plan = plan_rename(&listed, receipts, "Archive2").expect("plan");
        assert!(matches!(
            rename_on_server(&mut session, &plan).await,
            Err(FolderAdminError::AlreadyExists)
        ));
        session.logout().await;
    }
}

#[test]
fn move_targets_leave_out_the_folder_its_subfolders_and_where_it_is() {
    let all = folders();
    let (top, under) = move_targets(&all, FolderId(5));
    assert!(top);
    // Not 4 (already there), 5 or 6 (itself, its subfolder).
    assert_eq!(under, [1, 2, 3, 7, 8, 9, 10].map(FolderId).to_vec());
    let (top, under) = move_targets(&all, FolderId(7));
    assert!(!top);
    assert!(under.contains(&FolderId(4)) && !under.contains(&FolderId(7)));
    // A standard folder can't move anywhere.
    assert_eq!(move_targets(&all, FolderId(1)), (false, Vec::new()));
}

/// Courier-style: every personal folder under `INBOX.`, `.` as delimiter.
fn namespaced() -> Vec<Folder> {
    let mut all = vec![
        folder(1, "INBOX", Some(FolderRole::Inbox)),
        folder(2, "INBOX.Sent", Some(FolderRole::Sent)),
        folder(3, "INBOX.Projects", None),
        folder(4, "INBOX.Projects.2026", None),
        folder(5, "INBOX.Notes", None),
    ];
    for f in &mut all {
        f.details.delimiter = Some(".".into());
    }
    all
}

const NS: Option<&str> = Some("INBOX.");

#[test]
fn top_level_create_goes_under_the_namespace() {
    let all = namespaced();
    let p = plan_create_in(&all, ACCOUNT, None, "Receipts", None, NS).expect("plan");
    assert_eq!(p.remote_name, "INBOX.Receipts");
    // Under a folder it's as before; without a namespace, as before.
    let p = plan_create_in(&all, ACCOUNT, Some(FolderId(3)), "Q1", None, NS).expect("plan");
    assert_eq!(p.remote_name, "INBOX.Projects.Q1");
    assert_eq!(
        plan_create(&all, ACCOUNT, None, "Receipts", None)
            .expect("plan")
            .remote_name,
        "Receipts"
    );
    assert!(matches!(
        plan_create_in(&all, ACCOUNT, None, "Notes", None, NS),
        Err(FolderAdminError::AlreadyExists)
    ));
    assert!(matches!(
        plan_create_in(&all, ACCOUNT, None, "a.b", None, NS),
        Err(FolderAdminError::InvalidName(FolderNameError::Delimiter(
            '.'
        )))
    ));
}

#[test]
fn moving_to_the_top_stays_in_the_namespace() {
    let all = namespaced();
    let p = plan_move_in(&all, FolderId(4), None, NS).expect("plan");
    assert_eq!(p.new_remote, "INBOX.2026");
    // Already at the top of the namespace.
    assert!(plan_move_in(&all, FolderId(3), None, NS)
        .expect("plan")
        .is_noop());
    // Renames keep their place.
    assert_eq!(
        plan_rename(&all, FolderId(3), "Work").expect("plan").rows,
        vec![
            (FolderId(3), "INBOX.Work".into(), "INBOX.Work".into()),
            (
                FolderId(4),
                "INBOX.Work.2026".into(),
                "INBOX.Work.2026".into()
            ),
        ]
    );
}

#[test]
fn the_namespace_folder_is_the_top_in_the_move_picker() {
    let all = namespaced();
    assert_eq!(namespace_root(&all, "INBOX."), Some(FolderId(1)));
    assert_eq!(namespace_root(&all, "Other/"), None);
    let (top, under) = move_targets_in(&all, FolderId(4), NS);
    assert!(top);
    assert!(
        !under.contains(&FolderId(1)),
        "INBOX is the top, not a target"
    );
    assert!(under.contains(&FolderId(5)));
    let (top, under) = move_targets_in(&all, FolderId(5), NS);
    assert!(!top, "already at the top");
    assert!(!under.contains(&FolderId(1)));
}

mod namespace_imap {
    use super::*;
    use mailcrane_imap::{save_folders, ImapConfig, ImapSecurity, Secret};
    use mailcrane_testkit::{FakeImap, Quirk};
    use std::time::Duration;

    const USER: &str = "ann@example.org";
    const PASSWORD: &str = "app-password";

    async fn connect(imap: &FakeImap) -> Session {
        let mut config =
            ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
                .with_extra_root_cert(imap.root_cert_der().to_vec());
        config.command_timeout = Duration::from_millis(500);
        Session::connect(&config, USER, &Secret::new(PASSWORD))
            .await
            .expect("connects")
    }

    #[tokio::test]
    async fn create_move_and_rename_under_an_inbox_namespace() {
        let imap = FakeImap::builder()
            .user(USER, PASSWORD)
            .namespace("INBOX.", '.')
            .folder("INBOX", Vec::new())
            .folder("INBOX.Projects", Vec::new())
            .folder("INBOX.Projects.2026", Vec::new())
            .start()
            .await
            .expect("fake imap starts");
        let store = Store::open_in_memory().expect("store");
        let account = store
            .insert_account(&mailcrane_store::NewAccount {
                display_name: "Ann".into(),
                email: USER.into(),
                imap_host: "localhost".into(),
                imap_port: 993,
                imap_security: mailcrane_store::Security::Tls,
                smtp_host: "localhost".into(),
                smtp_port: 465,
                smtp_security: mailcrane_store::Security::Tls,
                username: USER.into(),
            })
            .expect("account");
        let mut session = connect(&imap).await;
        let ns = session.personal_namespace().await.expect("namespace");
        assert_eq!(ns.as_deref(), Some("INBOX."));
        save_folders(
            &store,
            account,
            &session.list_folders().await.expect("list"),
        )
        .expect("save");
        let id = |remote: &str| {
            store
                .list_folders(account)
                .expect("list")
                .into_iter()
                .find(|f| f.details.remote_name == remote)
                .map(|f| f.id)
        };

        let listed = store.list_folders(account).expect("list");
        let plan =
            plan_create_in(&listed, account, None, "Receipts", None, ns.as_deref()).expect("plan");
        create_on_server(&mut session, &plan).await.expect("create");
        save_created(&store, &plan).expect("save");

        let listed = store.list_folders(account).expect("list");
        let year = id("INBOX.Projects.2026").expect("2026");
        let plan = plan_move_in(&listed, year, None, ns.as_deref()).expect("plan");
        rename_on_server(&mut session, &plan).await.expect("move");
        save_renamed(&store, &plan).expect("save");

        let listed = store.list_folders(account).expect("list");
        let plan =
            plan_rename(&listed, id("INBOX.Projects").expect("projects"), "Work").expect("plan");
        rename_on_server(&mut session, &plan).await.expect("rename");
        save_renamed(&store, &plan).expect("save");
        session.logout().await;

        let mut server = imap.mailbox().folder_names().await;
        server.sort();
        assert_eq!(
            server,
            ["INBOX", "INBOX.2026", "INBOX.Receipts", "INBOX.Work"]
        );
    }

    #[tokio::test]
    async fn servers_without_a_namespace_behave_as_before() {
        for builder in [
            // IMAP4rev2 without a personal namespace: NAMESPACE says NIL.
            FakeImap::builder(),
            // IMAP4rev1 without NAMESPACE: nothing is asked.
            FakeImap::builder().quirk(Quirk::NoMove),
        ] {
            let imap = builder
                .user(USER, PASSWORD)
                .folder("INBOX", Vec::new())
                .start()
                .await
                .expect("fake imap starts");
            let mut session = connect(&imap).await;
            assert_eq!(session.personal_namespace().await.expect("namespace"), None);
            session.logout().await;
        }
    }
}

#[test]
fn the_namespace_is_derived_from_the_folder_list() {
    assert_eq!(derive_namespace(&namespaced()).as_deref(), Some("INBOX."));
    // Folders outside INBOX: no namespace.
    assert_eq!(derive_namespace(&folders()), None);
    let mut mixed = namespaced();
    let mut outside = folder(9, "Archive", None);
    outside.details.delimiter = Some(".".into());
    mixed.push(outside);
    assert_eq!(derive_namespace(&mixed), None);
    // Only the Inbox, or an Inbox without a delimiter: nothing to go by.
    assert_eq!(derive_namespace(&namespaced()[..1]), None);
    let mut flat = namespaced();
    flat[0].details.delimiter = None;
    assert_eq!(derive_namespace(&flat), None);
    assert_eq!(derive_namespace(&[]), None);
}
