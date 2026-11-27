//! Snooze steps for moves without MOVE (#423).

use mailcrane_store::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn account(store: &Store) -> Result<AccountId, StoreError> {
    store.insert_account(&NewAccount {
        display_name: "A".into(),
        email: "a@example.org".into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 465,
        smtp_security: Security::Tls,
        username: "a".into(),
    })
}

fn new(account: AccountId) -> NewSnooze {
    NewSnooze {
        account_id: account,
        message_id_header: "m1@example.org".into(),
        original_folder_id: None,
        hide_message: None,
        wake_at: 1_795_600_000,
    }
}

#[test]
fn steps_are_saved_and_cleared_when_the_move_finishes() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?.ok_or("waking")?;
    store.set_snooze_step(id, 1, Some(42))?;
    store.set_snooze_step(id, 2, None)?;
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!((s.step, s.target_uidnext), (2, Some(42)), "uidnext kept");
    store.finish_snooze(id, true)?;
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!(
        (s.state, s.step, s.target_uidnext),
        (SnoozeState::Snoozed, 0, None)
    );
    Ok(())
}

#[test]
fn unsnoozing_mid_move_keeps_the_row_and_wakes_it_at_once() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?.ok_or("waking")?;
    store.set_snooze_step(id, 2, Some(7))?;
    // Dropping the row now would leave the copy in Snoozed.
    assert!(store.request_unsnooze(a, "m1@example.org", 1_795_500_000)?);
    let s = store.get_snooze_by_id(id)?.ok_or("kept")?;
    assert_eq!(s.state, SnoozeState::ToSnooze);
    assert_eq!(
        (s.step, s.wake_at, s.mark_unread),
        (2, 1_795_500_000, false)
    );

    // Before any server step it is simply dropped, as before.
    let other = store
        .snooze_message(&NewSnooze {
            message_id_header: "m2@example.org".into(),
            ..new(a)
        })?
        .ok_or("waking")?;
    assert!(store.request_unsnooze(a, "m2@example.org", 1_795_500_000)?);
    assert!(store.get_snooze_by_id(other)?.is_none());
    Ok(())
}

#[test]
fn a_new_wake_starts_at_step_zero_but_a_wake_under_way_keeps_its_step() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?.ok_or("waking")?;
    store.finish_snooze(id, true)?;
    assert_eq!(store.take_due_snoozes(1_795_600_000)?.len(), 1);
    store.set_snooze_step(id, 2, Some(9))?;
    // Unsnoozing while the wake is mid-move must not restart it.
    assert!(store.request_unsnooze(a, "m1@example.org", 1_795_600_100)?);
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!(
        (s.state, s.step, s.target_uidnext),
        (SnoozeState::ToWake, 2, Some(9))
    );
    Ok(())
}

#[test]
fn a_rollback_keeps_its_start_time_and_unsnooze_shows_the_message() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?.ok_or("waking")?;
    store.start_snooze_rollback(id, 100)?;
    store.start_snooze_rollback(id, 200)?;
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!(
        (s.step, s.rollback_since),
        (4, Some(100)),
        "first refusal kept"
    );
    store.defer_snooze(id, 9_999)?;
    assert!(store.request_unsnooze(a, "m1@example.org", 300)?);
    let s = store.get_snooze_by_id(id)?.ok_or("kept for the rollback")?;
    assert_eq!(
        (s.step, s.attempts, s.next_attempt_at),
        (4, 0, 0),
        "no back-off wait"
    );
    Ok(())
}

/// #431 (lead's decision on #455): snoozing several messages is all or
/// nothing. One being woken past step 0 refuses the lot, with nothing
/// written or hidden.
#[test]
fn snoozing_is_refused_while_a_wake_is_moving_the_message() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?.ok_or("waking")?;
    store.finish_snooze(id, true)?;
    assert_eq!(store.take_due_snoozes(1_796_000_000)?.len(), 1);
    // At step 0 nothing has moved: a new snooze replaces the wake.
    let fresh = NewSnooze {
        wake_at: 1_797_000_000,
        ..new(a)
    };
    assert!(store.snooze_message(&fresh)?.is_some());
    store.finish_snooze(id, true)?;
    assert_eq!(store.take_due_snoozes(1_798_000_000)?.len(), 1);
    // Past step 0 the wake is moving it: refused, nothing changed.
    store.set_snooze_step(id, 1, Some(42))?;
    let before = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    let other = NewSnooze {
        message_id_header: "m2@example.org".into(),
        ..new(a)
    };
    assert_eq!(
        store.snooze_messages(&[other.clone(), new(a)])?,
        Snoozing::Waking
    );
    assert_eq!(store.get_snooze_by_id(id)?, Some(before));
    assert!(
        store
            .list_snoozes(Some(a))?
            .iter()
            .all(|s| s.message_id_header != "m2@example.org"),
        "the other message wasn't snoozed either"
    );
    Ok(())
}

/// Review of #455: a sync starts a move (step 1) only while the row is
/// still what it read. A wake at step 0 the user replaced by snoozing again
/// isn't started, so nothing is sent and the new snooze is kept.
#[test]
fn a_move_starts_only_while_the_row_is_unchanged() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?.ok_or("waking")?;
    store.finish_snooze(id, true)?;
    assert_eq!(store.take_due_snoozes(1_796_000_000)?.len(), 1);
    // The user snoozes it again while the sync is about to wake it.
    assert!(store
        .snooze_message(&NewSnooze {
            wake_at: 1_797_000_000,
            ..new(a)
        })?
        .is_some());
    assert!(!store.start_snooze_move(id, SnoozeState::ToWake, 9)?);
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!(
        (s.state, s.step, s.wake_at),
        (SnoozeState::ToSnooze, 0, 1_797_000_000)
    );
    // Unchanged, it starts; and a resumed start at step 1 too.
    assert!(store.start_snooze_move(id, SnoozeState::ToSnooze, 9)?);
    assert!(store.start_snooze_move(id, SnoozeState::ToSnooze, 9)?);
    store.set_snooze_step(id, 2, None)?;
    assert!(!store.start_snooze_move(id, SnoozeState::ToSnooze, 9)?);
    Ok(())
}
