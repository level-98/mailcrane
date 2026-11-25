-- Schema v7: scheduled send (M4-07). A scheduled message is an outbox
-- message held back until `not_before`.

-- 'none' (not scheduled), 'pending' (scheduled; who sends it isn't known
-- yet), 'server' (handed to the server with HOLDUNTIL, RFC 4865: it goes out
-- on time even with the phone off, and can't be cancelled from here) or
-- 'device' (this device sends it at the time).
ALTER TABLE outbox ADD COLUMN schedule_mode TEXT NOT NULL DEFAULT 'none'
    CHECK (schedule_mode IN ('none', 'pending', 'server', 'device'));
