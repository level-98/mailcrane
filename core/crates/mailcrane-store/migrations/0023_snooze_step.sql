-- Schema v23: a snooze or wake on a server without MOVE resumes where it
-- stopped (#423). The fallback is UID COPY, STORE +FLAGS (\Deleted), UID
-- EXPUNGE (RFC 6851 §3.3), and `step` records how far it got:
--   0  nothing yet;
--   1  about to COPY, with `target_uidnext` recorded;
--   2  copied;
--   3  the original is flagged \Deleted (only the expunge is left);
--   4  the server refused after the COPY: the copy still has to be removed
--      from the target. Retried on every sync until it is, never dropped.
-- `target_uidnext` is the target folder's UIDNEXT just before the COPY. The
-- copy is the message with this Message-ID at or above it, so an older
-- message with the same Message-ID is never touched.
ALTER TABLE snoozes ADD COLUMN step INTEGER NOT NULL DEFAULT 0 CHECK (step BETWEEN 0 AND 4);
ALTER TABLE snoozes ADD COLUMN target_uidnext INTEGER;
