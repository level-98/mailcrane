-- Schema v9: snooze (M4-06).
--
-- One row per snoozed message, keyed by its Message-ID header: moving it to
-- the Snoozed folder and back gives it new UIDs and new local rows.
--   state 'to_snooze': snoozed here; the server move (and keyword) is still
--                      to do. `local_message_id` is the row hidden until then.
--   state 'snoozed':   in the Snoozed folder on the server.
--   state 'to_wake':   due (or unsnoozed); the move back is still to do.
-- `on_server` is 1 when the wake time is also in a `$snoozed-until-...`
-- keyword, so other installs know it; 0 means this device only.
CREATE TABLE snoozes (
    id                 INTEGER PRIMARY KEY,
    account_id         INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id_header  TEXT NOT NULL,
    original_folder_id INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    local_message_id   INTEGER,
    wake_at            INTEGER NOT NULL,
    state              TEXT NOT NULL CHECK (state IN ('to_snooze', 'snoozed', 'to_wake')),
    on_server          INTEGER NOT NULL DEFAULT 0 CHECK (on_server IN (0, 1)),
    mark_unread        INTEGER NOT NULL DEFAULT 1 CHECK (mark_unread IN (0, 1)),
    attempts           INTEGER NOT NULL DEFAULT 0,
    next_attempt_at    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, message_id_header)
);
CREATE INDEX idx_snoozes_state_wake ON snoozes (state, wake_at);
