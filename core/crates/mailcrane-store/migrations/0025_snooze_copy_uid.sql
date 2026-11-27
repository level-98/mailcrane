-- Schema v25: the exact UID of the copy a snooze or wake made (#450,
-- ADR 0009). A server with UIDPLUS answers UID MOVE and UID COPY with
-- COPYUID (RFC 4315 §3; RFC 6851 §4.3 for MOVE), naming the copy's UID in
-- the target. With it, the clean-up after a move touches that one message,
-- never another install's copy that a Message-ID search at or above
-- `target_uidnext` could also match. NULL when the server sent none (no
-- UIDPLUS, or the reply was lost); then the UIDNEXT floor is the fallback.
ALTER TABLE snoozes ADD COLUMN copy_uid INTEGER;
