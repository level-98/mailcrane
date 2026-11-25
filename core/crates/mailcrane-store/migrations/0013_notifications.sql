-- Schema v13: new-mail notifications (M4-05).
--
-- `notify_accounts` has a row once an account's inbox was first looked at
-- for notifications. That first look only records what is already there
-- (nobody wants a notification for every unread message on a new account).
--
-- `notified` holds what was notified (or recorded at that first look), so a
-- message is notified at most once. The key is the Message-ID header, which
-- survives a resync, a UIDVALIDITY reset and moving the message back to the
-- inbox; messages without one use "uid:<folder id>:<uid>". Old rows are
-- pruned after 90 days; by then the message is too old to notify anyway.
CREATE TABLE notify_accounts (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    primed_at  INTEGER NOT NULL
);

CREATE TABLE notified (
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    notified_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, key)
) WITHOUT ROWID;
CREATE INDEX idx_notified_at ON notified (notified_at);
