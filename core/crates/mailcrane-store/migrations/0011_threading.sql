-- Schema v11: conversation threading (M4-02).
--
-- Threads are the connected groups of messages linked by Message-ID,
-- In-Reply-To and References (a missing parent links its replies like a
-- JWZ placeholder), plus a narrow subject rule. `thread_keys` maps every
-- id seen in an account to its thread, so a new message joins (and, if it
-- links two, merges) threads without a rebuild.
CREATE TABLE thread_keys (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    thread_id  INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    PRIMARY KEY (account_id, key)
);
CREATE INDEX idx_thread_keys_thread ON thread_keys (thread_id);

-- The subject with Re:/Fwd: prefixes removed, lower-case, and whether the
-- message can join a thread by subject: 1 = an original (no prefix),
-- 2 = a reply or forward (prefix) without any References or In-Reply-To.
ALTER TABLE messages ADD COLUMN thread_subject TEXT;
ALTER TABLE messages ADD COLUMN subject_kind INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_messages_thread_subject ON messages (thread_subject) WHERE subject_kind > 0;
CREATE INDEX idx_threads_account ON threads (account_id);
