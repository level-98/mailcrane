-- Schema v4: drafts (M2-05), saved here first and uploaded to the server's
-- Drafts folder.

CREATE TABLE drafts (
    id                INTEGER PRIMARY KEY,
    account_id        INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Recipient entries as typed, one per line.
    to_list           TEXT NOT NULL DEFAULT '',
    cc_list           TEXT NOT NULL DEFAULT '',
    bcc_list          TEXT NOT NULL DEFAULT '',
    subject           TEXT NOT NULL DEFAULT '',
    body              TEXT NOT NULL DEFAULT '',
    in_reply_to       TEXT,
    -- Message-IDs separated by spaces.
    references_list   TEXT NOT NULL DEFAULT '',
    -- Kept across versions, so the server copy can be found and replaced.
    message_id_header TEXT NOT NULL UNIQUE,
    -- Where the newest uploaded copy is, if any.
    server_folder_id  INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    server_uid        INTEGER,
    -- Changed since the last upload.
    dirty             INTEGER NOT NULL DEFAULT 1 CHECK (dirty IN (0, 1)),
    attempts          INTEGER NOT NULL DEFAULT 0,
    next_attempt_at   INTEGER NOT NULL DEFAULT 0,
    -- Bumped on every save; an upload only clears `dirty` for the version it sent.
    version           INTEGER NOT NULL DEFAULT 1,
    updated_at        INTEGER NOT NULL,
    -- A version changed on another device, kept beside ours rather than overwritten.
    is_conflict       INTEGER NOT NULL DEFAULT 0 CHECK (is_conflict IN (0, 1))
);

CREATE INDEX idx_drafts_account ON drafts (account_id, updated_at DESC);
