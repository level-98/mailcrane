# 0009: The exact UID of a snooze's copy (`snoozes.copy_uid`)

Status: accepted
Date: 2026-11-27

## Context

A snooze or wake moves a message between the Snoozed folder and another folder. After a dropped connection the next sync has to find the copy it made, to finish the move or roll it back. Until now it found the copy by Message-ID at or above the target's UIDNEXT, recorded just before the MOVE or COPY was sent (migration 0023, #423).

That search can match another install's copy. If install A records step 1 and drops before sending its MOVE, and install B wakes the same message meanwhile and the user reads it, A's retry finds B's copy and marks it unread (#450).

A server with UIDPLUS answers UID COPY, and UID MOVE, with a `COPYUID` response code naming the copy's UID in the target (RFC 4315 §3; for MOVE, RFC 6851 §4.3). The fleet lead decided on the schema change under docs/fleet.md, "Who decides" (G6, #459 and #450).

## Decision

- Store migration 0025 adds `snoozes.copy_uid INTEGER`, **nullable**. It is written together with the move's step (2 after a COPY, 3 after a MOVE) when the server sent `COPYUID` naming exactly one message. It is cleared when a move starts again (step 1) or ends (the row is finished, rolled back to step 0, or deleted).
- With `copy_uid` set, the clean-up after a move touches only that UID. Without it (no UIDPLUS, or a reply that never came), the UIDNEXT floor is the fallback, as before.
- `\Seen` is cleared on a woken copy only when it is surely this install's: `copy_uid` is set, or this install's COPY or MOVE got its reply (step 2 or 3). Resuming at step 1, the copy found could be another install's, so its read state is left alone. At worst our own woken message stays read; it never marks someone else's read message unread.
- `copy_uid` is one of the columns every guarded snooze write compares (invariant 5 in `snooze/server.rs`), with `IS`, so NULL matches NULL.
- Like every migration, 0025 is append-only: it can't be undone once released. A later change to this column needs a new migration and a decision that replaces this one.

## Why

- `COPYUID` is the server's own statement of which message is the copy, so nothing has to be guessed. The floor search stays only for servers that can't say.
- A nullable column on the existing row keeps the move's state in one place, written in the same statement as its step.

Options we didn't pick:

- **Search by Message-ID and a marker keyword on the copy:** it needs custom keywords, which not every server keeps, and another install could carry the same keyword.
- **Never clear `\Seen` on a wake:** it would break "wake as unread" for the normal case, where the copy is ours.

## Consequences

- On UIDPLUS servers (most of them), #450 can't happen. On others, a wake resumed at step 1 leaves the read state alone, and the rest is as before.
- The IMAP client gains `uid_move_returning` and `uid_copy_returning`, which return the copy's UID. `uid_move` and `uid_copy` are unchanged for the archive and move actions.
- Related decisions on G6, recorded in the `snooze/server.rs` doc: a move or delete of a message whose wake is past step 0 is refused, in the same transaction that reads the row (as #431 does for re-snoozing). A wake held up by dropped connections isn't given up.
