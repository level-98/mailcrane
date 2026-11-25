-- Schema v20 (file 0021: 0020 is held by #318, not merged yet): the
-- account's personal IMAP namespace prefix (RFC 2342), such as 'INBOX.', as
-- the server reported it (#325). NULL: none, or the server has no NAMESPACE.
-- Top-level folders are created under it.
ALTER TABLE accounts ADD COLUMN personal_namespace TEXT
    CHECK (personal_namespace IS NULL OR length(personal_namespace) BETWEEN 1 AND 200);
