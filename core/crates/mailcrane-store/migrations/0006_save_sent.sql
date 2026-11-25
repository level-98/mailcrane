-- Schema v6: saving sent messages to the Sent folder (M2-07).

-- Per account: 'auto' (skip for servers that file sent mail themselves),
-- 'always' or 'never'.
ALTER TABLE accounts ADD COLUMN save_sent_copy TEXT NOT NULL DEFAULT 'auto'
    CHECK (save_sent_copy IN ('auto', 'always', 'never'));

-- For a sent outbox message, its copy in Sent: 'none' (not sent yet),
-- 'pending', 'saved', 'skipped' (the server saves it, or the user said no)
-- or 'failed' ("Sent, not uploaded"). A failed copy never resends the
-- message.
ALTER TABLE outbox ADD COLUMN sent_copy TEXT NOT NULL DEFAULT 'none'
    CHECK (sent_copy IN ('none', 'pending', 'saved', 'skipped', 'failed'));
ALTER TABLE outbox ADD COLUMN copy_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE outbox ADD COLUMN copy_next_attempt_at INTEGER NOT NULL DEFAULT 0;
