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
