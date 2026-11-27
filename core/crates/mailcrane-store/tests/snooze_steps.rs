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
    let id = store.snooze_message(&new(a))?;
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
    let id = store.snooze_message(&new(a))?;
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
    let other = store.snooze_message(&NewSnooze {
        message_id_header: "m2@example.org".into(),
        ..new(a)
    })?;
    assert!(store.request_unsnooze(a, "m2@example.org", 1_795_500_000)?);
    assert!(store.get_snooze_by_id(other)?.is_none());
    Ok(())
}

#[test]
fn a_new_wake_starts_at_step_zero_but_a_wake_under_way_keeps_its_step() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?;
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
    let id = store.snooze_message(&new(a))?;
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

#[test]
fn a_resnooze_during_a_half_done_wake_waits_for_the_wake() -> R {
    let store = Store::open_in_memory()?;
    let a = account(&store)?;
    let id = store.snooze_message(&new(a))?;
    store.finish_snooze(id, true)?;
    assert_eq!(store.take_due_snoozes(1_795_600_000)?.len(), 1);
    // Re-snoozing at step 0 behaves as before: back to to_snooze at once.
    store.snooze_message(&NewSnooze {
        wake_at: 1_795_700_000,
        ..new(a)
    })?;
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!(
        (s.state, s.pending_resnooze_at),
        (SnoozeState::ToSnooze, None)
    );

    // Mid-wake (step > 0): the wake keeps going and the new time waits.
    store.finish_snooze(id, true)?;
    assert_eq!(store.take_due_snoozes(1_795_800_000)?.len(), 1);
    store.set_snooze_step(id, 2, Some(9))?;
    store.snooze_message(&NewSnooze {
        wake_at: 1_795_900_000,
        ..new(a)
    })?;
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!(
        (s.state, s.step, s.target_uidnext, s.pending_resnooze_at),
        (SnoozeState::ToWake, 2, Some(9), Some(1_795_900_000))
    );
    // The wake finishes: the re-snooze applies, at step 0.
    assert!(store.finish_wake(id, WakeLocal::Keep)?);
    let s = store.get_snooze_by_id(id)?.ok_or("snooze")?;
    assert_eq!(
        (s.state, s.step, s.wake_at, s.pending_resnooze_at),
        (SnoozeState::ToSnooze, 0, 1_795_900_000, None)
    );
    // Without a pending re-snooze, finishing a wake deletes the row.
    store.finish_snooze(id, true)?;
    assert_eq!(store.take_due_snoozes(1_796_000_000)?.len(), 1);
    assert!(!store.finish_wake(id, WakeLocal::Keep)?);
    assert!(store.get_snooze_by_id(id)?.is_none());
    Ok(())
}
