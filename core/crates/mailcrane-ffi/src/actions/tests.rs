use super::*;
use mailcrane_core::events::SyncErrorKind as CoreKind;

#[test]
fn action_failed_crosses_the_boundary_with_ids_only() {
    let event = events::CoreEvent::ActionFailed {
        account_id: AccountId(3),
        action: events::ActionKind::Delete,
        message_ids: vec![MessageId(7), MessageId(9)],
        reason: CoreKind::NotFound,
    };
    assert_eq!(
        CoreEvent::from(event),
        CoreEvent::ActionFailed {
            account_id: 3,
            action: ActionKind::Delete,
            message_ids: vec![7, 9],
            reason: SyncErrorKind::NotFound,
        }
    );
}

#[test]
fn errors_map_one_to_one() {
    use mailcrane_store::StoreError;
    assert_eq!(
        ActionFailure::from(ActionError::NoArchiveFolder),
        ActionFailure::NoArchiveFolder
    );
    assert_eq!(
        ActionFailure::from(ActionError::UnknownMessage),
        ActionFailure::NotFound
    );
    assert_eq!(
        ActionFailure::from(ActionError::UnknownFolder),
        ActionFailure::NotFound
    );
    assert_eq!(
        ActionFailure::from(ActionError::MixedFolders),
        ActionFailure::MixedFolders
    );
    assert_eq!(
        ActionFailure::from(ActionError::Store(StoreError::Invalid { field: "x" })),
        ActionFailure::Storage
    );
}

#[test]
fn every_action_maps_to_the_core() {
    assert_eq!(Action::from(MessageAction::MarkRead), Action::MarkRead);
    assert_eq!(Action::from(MessageAction::MarkUnread), Action::MarkUnread);
    assert_eq!(Action::from(MessageAction::Flag), Action::Flag);
    assert_eq!(Action::from(MessageAction::Unflag), Action::Unflag);
    assert_eq!(Action::from(MessageAction::Archive), Action::Archive);
    assert_eq!(Action::from(MessageAction::Delete), Action::Delete);
    assert_eq!(
        Action::from(MessageAction::Move { folder_id: 4 }),
        Action::Move { to: FolderId(4) }
    );
}

#[test]
fn applying_to_an_unopenable_database_is_a_storage_error() {
    #[derive(Default)]
    struct Quiet;
    impl crate::sync::CoreEventListener for Quiet {
        fn on_event(&self, _event: CoreEvent) {}
    }
    let dir = std::env::temp_dir().join("mailcrane-ffi-actions-no-such-dir");
    let engine = SyncEngine::new(
        dir.join("nested")
            .join("m.db")
            .to_string_lossy()
            .into_owned(),
        std::sync::Arc::new(Quiet),
    );
    assert_eq!(
        engine.apply_action(vec![1], MessageAction::Flag),
        Err(ActionFailure::Storage)
    );
}

#[test]
fn bulk_errors_map_one_to_one() {
    assert_eq!(
        ActionFailure::from(ActionError::MixedAccounts),
        ActionFailure::MixedAccounts
    );
    assert_eq!(
        ActionFailure::from(ActionError::TooMany),
        ActionFailure::TooMany
    );
}

/// Multi-select (M4-14) through the FFI, on a real database file with two
/// accounts. Only account A has an Archive folder.
mod bulk {
    use super::*;
    use mailcrane_store::{
        Flags, FolderRole, NewAccount, NewFolder, NewMessageHeader, Security, Store,
    };

    struct Quiet;
    impl crate::sync::CoreEventListener for Quiet {
        fn on_event(&self, _event: CoreEvent) {}
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        path: String,
        engine: std::sync::Arc<SyncEngine>,
        a_inbox: FolderId,
        a_trash: FolderId,
        a_work: FolderId,
        b_inbox: FolderId,
        /// A's inbox: a two-message conversation, then one more message.
        a: Vec<i64>,
        /// B's inbox: one message.
        b: Vec<i64>,
        /// Account C has no Trash folder: its inbox's two messages.
        c_inbox: FolderId,
        c: Vec<i64>,
    }

    fn account(store: &Store, name: &str) -> AccountId {
        let email = format!("{name}@example.org");
        store
            .insert_account(&NewAccount {
                display_name: name.into(),
                email: email.clone(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 465,
                smtp_security: Security::Tls,
                username: email,
            })
            .expect("account")
    }

    fn folder(store: &Store, account: AccountId, name: &str, role: Option<FolderRole>) -> FolderId {
        store
            .upsert_folder(&NewFolder {
                account_id: account,
                remote_name: name.into(),
                display_name: name.into(),
                delimiter: None,
                role,
                uidvalidity: None,
                uidnext: None,
                highestmodseq: None,
                is_subscribed: true,
                selectable: true,
            })
            .expect("folder")
    }

    fn message(store: &Store, folder: FolderId, uid: u32, id: &str, refs: &str) -> i64 {
        store
            .upsert_message_header(&NewMessageHeader {
                folder_id: folder,
                uid,
                message_id_header: Some(id.into()),
                in_reply_to: None,
                references_json: refs.into(),
                thread_id: None,
                subject: format!("Subject {uid}"),
                from_name: None,
                from_addr: Some(format!("p{uid}@example.net")),
                to_json: "[]".into(),
                cc_json: "[]".into(),
                date: 1_000 + i64::from(uid),
                internal_date: 1_000,
                size: 1,
                flags: Flags::default(),
                keywords_json: "[]".into(),
                has_attachments: false,
                snippet: String::new(),
            })
            .expect("message")
            .0
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.db").to_string_lossy().into_owned();
        let store = Store::open(&path).expect("store");
        let a_account = account(&store, "a");
        let b_account = account(&store, "b");
        let a_inbox = folder(&store, a_account, "INBOX", Some(FolderRole::Inbox));
        let a_trash = folder(&store, a_account, "Trash", Some(FolderRole::Trash));
        folder(&store, a_account, "Archive", Some(FolderRole::Archive));
        let a_work = folder(&store, a_account, "Work", None);
        let b_inbox = folder(&store, b_account, "INBOX", Some(FolderRole::Inbox));
        folder(&store, b_account, "Trash", Some(FolderRole::Trash));
        let a = vec![
            message(&store, a_inbox, 1, "a1@x", "[]"),
            message(&store, a_inbox, 2, "a2@x", "[\"a1@x\"]"),
            message(&store, a_inbox, 3, "a3@x", "[]"),
        ];
        let b = vec![message(&store, b_inbox, 1, "b1@x", "[]")];
        let c_account = account(&store, "c");
        let c_inbox = folder(&store, c_account, "INBOX", Some(FolderRole::Inbox));
        let c = vec![
            message(&store, c_inbox, 1, "c1@x", "[]"),
            message(&store, c_inbox, 2, "c2@x", "[]"),
        ];
        drop(store);
        let engine = SyncEngine::new(path.clone(), std::sync::Arc::new(Quiet));
        Fixture {
            _dir: dir,
            path,
            engine,
            a_inbox,
            a_trash,
            a_work,
            b_inbox,
            a,
            b,
            c_inbox,
            c,
        }
    }

    impl Fixture {
        fn visible(&self, folder: FolderId) -> usize {
            Store::open(&self.path)
                .expect("store")
                .list_messages(folder, 100, 0)
                .expect("list")
                .len()
        }

        fn select(&self, ids: &[i64]) -> BulkSelection {
            BulkSelection {
                message_ids: ids.to_vec(),
                folder_id: 0,
                thread_ids: Vec::new(),
            }
        }

        fn all(&self) -> Vec<i64> {
            self.a.iter().chain(&self.b).copied().collect()
        }
    }

    #[test]
    fn delete_across_accounts_then_undo_everything() {
        let f = fixture();
        let applied = f
            .engine
            .apply_bulk_action(f.select(&f.all()), MessageAction::Delete)
            .expect("applies");
        assert_eq!(applied.account_ids.len(), 2, "both queues need a replay");
        assert_eq!(applied.message_ids.len(), 4);
        assert!(!applied.deleted_for_good, "both accounts have a Trash");
        assert_eq!(f.visible(f.a_inbox), 0);
        assert_eq!(f.visible(f.b_inbox), 0);

        let undo = f
            .engine
            .undo_bulk_action(applied.message_ids)
            .expect("undoes");
        assert_eq!(
            undo,
            BulkUndo {
                undone: 4,
                too_late: 0
            }
        );
        assert_eq!(f.visible(f.a_inbox), 3);
        assert_eq!(f.visible(f.b_inbox), 1);
    }

    #[test]
    fn one_failing_group_changes_nothing() {
        let f = fixture();
        // B has no Archive folder: A's messages must not be archived either.
        assert_eq!(
            f.engine
                .apply_bulk_action(f.select(&f.all()), MessageAction::Archive),
            Err(ActionFailure::NoArchiveFolder)
        );
        assert_eq!(f.visible(f.a_inbox), 3);
        assert_eq!(f.visible(f.b_inbox), 1);
        assert_eq!(
            f.engine
                .apply_bulk_action(f.select(&[f.a[0], 999_999]), MessageAction::Flag),
            Err(ActionFailure::NotFound)
        );
    }

    #[test]
    fn move_needs_one_account() {
        let f = fixture();
        assert_eq!(
            f.engine.apply_bulk_action(
                f.select(&f.all()),
                MessageAction::Move {
                    folder_id: f.a_work.0
                }
            ),
            Err(ActionFailure::MixedAccounts)
        );
        assert_eq!(f.visible(f.b_inbox), 1);
        let applied = f
            .engine
            .apply_bulk_action(
                f.select(&f.a),
                MessageAction::Move {
                    folder_id: f.a_work.0,
                },
            )
            .expect("moves");
        assert_eq!(applied.message_ids.len(), 3);
        assert_eq!(
            f.visible(f.a_work),
            0,
            "shown there once the server confirms"
        );
        assert_eq!(f.visible(f.a_inbox), 0);
    }

    #[test]
    fn a_conversation_row_stands_for_its_messages_in_that_folder() {
        let f = fixture();
        let threads =
            crate::threads::list_threads(f.path.clone(), f.a_inbox.0, 0, 50).expect("threads");
        let pair = threads
            .iter()
            .find(|t| t.message_count == 2)
            .expect("the conversation");
        let applied = f
            .engine
            .apply_bulk_action(
                BulkSelection {
                    message_ids: f.b.clone(),
                    folder_id: f.a_inbox.0,
                    thread_ids: vec![pair.thread_id],
                },
                MessageAction::MarkRead,
            )
            .expect("applies");
        let mut changed = applied.message_ids.clone();
        changed.sort_unstable();
        let mut want = vec![f.a[0], f.a[1], f.b[0]];
        want.sort_unstable();
        assert_eq!(changed, want, "the third inbox message is untouched");
    }

    #[test]
    fn delete_in_trash_is_for_good_and_nothing_selected_does_nothing() {
        let f = fixture();
        let store = Store::open(&f.path).expect("store");
        let in_trash = message(&store, f.a_trash, 50, "t@x", "[]");
        drop(store);
        let gone = f
            .engine
            .apply_bulk_action(f.select(&[in_trash]), MessageAction::Delete)
            .expect("expunges");
        assert!(gone.deleted_for_good);
        let none = f
            .engine
            .apply_bulk_action(f.select(&[]), MessageAction::Delete)
            .expect("empty");
        assert!(none.message_ids.is_empty() && none.account_ids.is_empty());
    }

    #[test]
    fn too_many_is_refused() {
        let f = fixture();
        let ids = vec![f.a[0]; mailcrane_core::actions::MAX_BULK + 1];
        assert_eq!(
            f.engine
                .apply_bulk_action(f.select(&ids), MessageAction::Flag),
            Err(ActionFailure::TooMany)
        );
    }

    #[test]
    fn the_app_can_tell_ahead_which_deletes_are_for_good() {
        let f = fixture();
        // A and B have a Trash: nothing would be deleted for good.
        assert_eq!(f.engine.count_deleted_for_good(f.select(&f.all())), Ok(0));
        // C has none, so its messages would be.
        let mixed: Vec<i64> = f.a.iter().chain(&f.c).copied().collect();
        assert_eq!(f.engine.count_deleted_for_good(f.select(&mixed)), Ok(2));
        // In Trash, a delete is for good too.
        let store = Store::open(&f.path).expect("store");
        let in_trash = message(&store, f.a_trash, 60, "t2@x", "[]");
        drop(store);
        assert_eq!(
            f.engine.count_deleted_for_good(f.select(&[in_trash])),
            Ok(1)
        );
        // Only a question: nothing changed.
        assert_eq!(f.visible(f.c_inbox), 2);
        assert_eq!(f.visible(f.a_inbox), 3);
    }

    #[test]
    fn undo_of_a_pending_delete_for_good_brings_the_rows_back() {
        let f = fixture();
        let applied = f
            .engine
            .apply_bulk_action(f.select(&f.c), MessageAction::Delete)
            .expect("deletes");
        assert!(applied.deleted_for_good, "C has no Trash");
        assert_eq!(f.visible(f.c_inbox), 0, "hidden at once");
        // The expunge is still queued (never replayed): undo puts both back.
        let undo = f
            .engine
            .undo_bulk_action(applied.message_ids)
            .expect("undoes");
        assert_eq!(
            undo,
            BulkUndo {
                undone: 2,
                too_late: 0
            }
        );
        assert_eq!(f.visible(f.c_inbox), 2);
        let store = Store::open(&f.path).expect("store");
        let c_account = store
            .get_folder(f.c_inbox)
            .expect("read")
            .expect("folder")
            .details
            .account_id;
        assert_eq!(store.count_pending_ops(c_account).expect("count"), 0);
    }
}
