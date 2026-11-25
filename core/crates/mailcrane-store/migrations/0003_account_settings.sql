-- Schema v3: per-account settings (M2-10 signatures). Preferences only;
-- never credentials (AGENTS.md §4.3).

-- The signature added to new messages, replies and forwards, as plain text.
ALTER TABLE accounts ADD COLUMN signature_text TEXT NOT NULL DEFAULT '';
-- Whether to add it. An empty signature adds nothing either way.
ALTER TABLE accounts ADD COLUMN signature_enabled INTEGER NOT NULL DEFAULT 1
    CHECK (signature_enabled IN (0, 1));
