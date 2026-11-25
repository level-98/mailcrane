-- Schema v16: local-only accounts (#289).
--
-- A local-only account lives only on this device. It never syncs and never
-- connects anywhere: the core refuses every IMAP login and SMTP send for it.
-- Only debug builds create one (the "Load test mail" fixtures account), so in
-- release builds every account keeps the default 0.
ALTER TABLE accounts ADD COLUMN local_only INTEGER NOT NULL DEFAULT 0
    CHECK (local_only IN (0, 1));
