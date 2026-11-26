-- Schema v24: snoozing a message again while its wake is half done (#431).
-- A `to_wake` row with `step` > 0 has moved, or started moving, the message
-- back. Snoozing it again doesn't restart that move (it would leave the
-- message in two folders): the wake finishes first, and the new wake time
-- waits here. When the wake is done, the row turns into `to_snooze` with
-- this time, at step 0.
ALTER TABLE snoozes ADD COLUMN pending_resnooze_at INTEGER;
