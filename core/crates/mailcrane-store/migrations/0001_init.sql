-- Schema v1. No column here may hold a password or token: credentials live
-- in the OS keychain (AGENTS.md §4.3).

CREATE TABLE accounts (
    id            INTEGER PRIMARY KEY,
    display_name  TEXT NOT NULL,
    email         TEXT NOT NULL,
    imap_host     TEXT NOT NULL,
    imap_port     INTEGER NOT NULL,
    imap_security TEXT NOT NULL CHECK (imap_security IN ('tls', 'starttls')),
    smtp_host     TEXT NOT NULL,
    smtp_port     INTEGER NOT NULL,
    smtp_security TEXT NOT NULL CHECK (smtp_security IN ('tls', 'starttls')),
    username      TEXT NOT NULL,
    created_at    INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE TABLE folders (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_name   TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    delimiter     TEXT,
    role          TEXT CHECK (role IS NULL OR role IN
                      ('inbox', 'drafts', 'sent', 'archive', 'spam', 'trash', 'all')),
    uidvalidity   INTEGER,
    uidnext       INTEGER,
    highestmodseq INTEGER,
    is_subscribed INTEGER NOT NULL DEFAULT 1 CHECK (is_subscribed IN (0, 1)),
    UNIQUE (account_id, remote_name)
);

CREATE TABLE threads (
    id         INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject    TEXT NOT NULL,
    last_date  INTEGER NOT NULL
);

CREATE TABLE messages (
    id                INTEGER PRIMARY KEY,
    folder_id         INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid               INTEGER NOT NULL,
    message_id_header TEXT,
    in_reply_to       TEXT,
    references_json   TEXT NOT NULL DEFAULT '[]',
    thread_id         INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    subject           TEXT NOT NULL,
    from_name         TEXT,
    from_addr         TEXT,
    to_json           TEXT NOT NULL DEFAULT '[]',
    cc_json           TEXT NOT NULL DEFAULT '[]',
    date              INTEGER NOT NULL,
    internal_date     INTEGER NOT NULL,
    size              INTEGER NOT NULL,
    seen              INTEGER NOT NULL DEFAULT 0 CHECK (seen IN (0, 1)),
    flagged           INTEGER NOT NULL DEFAULT 0 CHECK (flagged IN (0, 1)),
    answered          INTEGER NOT NULL DEFAULT 0 CHECK (answered IN (0, 1)),
    draft             INTEGER NOT NULL DEFAULT 0 CHECK (draft IN (0, 1)),
    deleted           INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    keywords_json     TEXT NOT NULL DEFAULT '[]',
    has_attachments   INTEGER NOT NULL DEFAULT 0 CHECK (has_attachments IN (0, 1)),
    snippet           TEXT NOT NULL DEFAULT '',
    body_state        TEXT NOT NULL DEFAULT 'none' CHECK (body_state IN ('none', 'fetched')),
    UNIQUE (folder_id, uid)
);

CREATE INDEX idx_messages_folder_date ON messages (folder_id, date DESC);
CREATE INDEX idx_messages_message_id_header ON messages (message_id_header);
CREATE INDEX idx_messages_thread_id ON messages (thread_id);

CREATE TABLE message_bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw        BLOB NOT NULL,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE attachments (
    id         INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    part_id    TEXT NOT NULL,
    filename   TEXT,
    mime_type  TEXT NOT NULL,
    size       INTEGER NOT NULL,
    content_id TEXT,
    is_inline  INTEGER NOT NULL DEFAULT 0 CHECK (is_inline IN (0, 1))
);
