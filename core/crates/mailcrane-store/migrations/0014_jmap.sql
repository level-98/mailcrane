-- Schema v14: JMAP accounts (M3-04, RFC 8620 and RFC 8621).
--
-- An account speaks IMAP+SMTP (the default, every existing account) or
-- JMAP. A JMAP account keeps its session URL here; its imap_*/smtp_* columns
-- hold empty values and are never used.
ALTER TABLE accounts ADD COLUMN protocol TEXT NOT NULL DEFAULT 'imap'
    CHECK (protocol IN ('imap', 'jmap'));
ALTER TABLE accounts ADD COLUMN jmap_session_url TEXT;

-- Which JMAP mailbox each folder is. Folders of JMAP accounts use the
-- mailbox's name path as remote_name, so the rest of the app sees an
-- ordinary folder.
CREATE TABLE jmap_mailboxes (
    folder_id  INTEGER PRIMARY KEY REFERENCES folders(id) ON DELETE CASCADE,
    mailbox_id TEXT NOT NULL
);

-- Which JMAP email each message row is. An email in several mailboxes has
-- one row per mailbox (as Gmail labels look over IMAP); each row gets a
-- per-folder number in `messages.uid`, taken from folders.uidnext, so
-- (folder_id, uid) stays unique.
CREATE TABLE jmap_emails (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    email_id   TEXT NOT NULL,
    blob_id    TEXT NOT NULL
);
CREATE INDEX idx_jmap_emails_email ON jmap_emails (email_id);

-- The last state string per data type (RFC 8620 §5.2), the JMAP
-- counterpart of HIGHESTMODSEQ: "Mailbox" and "Email".
CREATE TABLE jmap_state (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    type       TEXT NOT NULL,
    state      TEXT NOT NULL,
    PRIMARY KEY (account_id, type)
) WITHOUT ROWID;
