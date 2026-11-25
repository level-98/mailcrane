-- Schema v5: the outbox (M2-06). A message waits here until the server
-- accepts it, so pressing Send works offline.

CREATE TABLE outbox (
    id                INTEGER PRIMARY KEY,
    account_id        INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- The exact bytes to send (no Bcc header).
    raw               BLOB NOT NULL,
    envelope_from     TEXT NOT NULL,
    -- SMTP recipients, one per line (including Bcc).
    envelope_to       TEXT NOT NULL,
    -- Without angle brackets; used to check the Sent folder after a crash.
    message_id_header TEXT NOT NULL,
    -- What the user typed, so a failed message can be edited and sent again.
    to_list           TEXT NOT NULL DEFAULT '',
    cc_list           TEXT NOT NULL DEFAULT '',
    bcc_list          TEXT NOT NULL DEFAULT '',
    subject           TEXT NOT NULL DEFAULT '',
    body              TEXT NOT NULL DEFAULT '',
    in_reply_to       TEXT,
    references_list   TEXT NOT NULL DEFAULT '',
    state             TEXT NOT NULL DEFAULT 'queued'
                          CHECK (state IN ('queued', 'sending', 'sent', 'failed')),
    attempts          INTEGER NOT NULL DEFAULT 0,
    next_attempt_at   INTEGER NOT NULL,
    -- Held back until then (undo send, M4-08; scheduled send, M4-07).
    not_before        INTEGER NOT NULL DEFAULT 0,
    -- An error code such as 'network', never server text.
    last_error        TEXT,
    -- Recipients the server refused while accepting the rest, one per line.
    rejected          TEXT NOT NULL DEFAULT '',
    created_at        INTEGER NOT NULL,
    sent_at           INTEGER
);

CREATE INDEX idx_outbox_account_state ON outbox (account_id, state, next_attempt_at);
