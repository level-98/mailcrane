-- Schema v2: message actions (M1-14) applied locally first and queued for
-- the server.

-- A message that was moved or deleted locally but not yet on the server.
-- Hidden messages are left out of lists and unread counts, but their UIDs
-- stay known so a sync doesn't fetch them again.
ALTER TABLE messages ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1));

-- One queued action on messages of one folder, replayed in id order per
-- account. `step` records progress through a multi-command action (COPY,
-- then STORE \Deleted, then UID EXPUNGE), so a replay after a dropped
-- connection never repeats the COPY.
CREATE TABLE pending_ops (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN
                        ('mark_read', 'mark_unread', 'flag', 'unflag', 'move', 'expunge')),
    folder_id       INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    dest_folder_id  INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    step            INTEGER NOT NULL DEFAULT 0,
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    -- An error code such as 'network', never server text.
    last_error      TEXT
);

CREATE INDEX idx_pending_ops_account ON pending_ops (account_id, id);

-- The messages an action applies to. `message_id` has no foreign key: a sync
-- may delete the local row (for example when the server already expunged
-- it) and the action must still be replayed by UID.
CREATE TABLE pending_op_messages (
    op_id      INTEGER NOT NULL REFERENCES pending_ops(id) ON DELETE CASCADE,
    message_id INTEGER NOT NULL,
    uid        INTEGER NOT NULL,
    PRIMARY KEY (op_id, message_id)
);

CREATE INDEX idx_pending_op_messages_message ON pending_op_messages (message_id);
