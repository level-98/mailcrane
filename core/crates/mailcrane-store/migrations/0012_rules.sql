-- Schema v12: local rules (M4-09).
--
-- Conditions and actions are rows of their own rather than JSON (the store
-- has no JSON library). Matching is plain "contains", never a regular
-- expression.
CREATE TABLE rules (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER REFERENCES accounts(id) ON DELETE CASCADE,  -- NULL: every account
    name            TEXT NOT NULL,
    position        INTEGER NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    match_all       INTEGER NOT NULL DEFAULT 1 CHECK (match_all IN (0, 1)),
    stop_processing INTEGER NOT NULL DEFAULT 0 CHECK (stop_processing IN (0, 1))
);

CREATE TABLE rule_conditions (
    rule_id  INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind     TEXT NOT NULL CHECK (kind IN
             ('from', 'to_cc', 'subject', 'has_attachment', 'list_id', 'older_than_days')),
    value    TEXT NOT NULL DEFAULT ''
);

CREATE TABLE rule_actions (
    rule_id   INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    kind      TEXT NOT NULL CHECK (kind IN ('move', 'mark_read', 'flag', 'archive', 'delete')),
    folder_id INTEGER REFERENCES folders(id) ON DELETE CASCADE
);
CREATE INDEX idx_rule_conditions_rule ON rule_conditions (rule_id);
CREATE INDEX idx_rule_actions_rule ON rule_actions (rule_id);

-- The List-Id of a message (RFC 2919), the id part, lower-case.
ALTER TABLE messages ADD COLUMN list_id TEXT;

-- Whether rules have looked at this message. Mail already stored is done:
-- rules only act on new mail (and on old mail when asked to).
ALTER TABLE messages ADD COLUMN rules_done INTEGER NOT NULL DEFAULT 0 CHECK (rules_done IN (0, 1));
UPDATE messages SET rules_done = 1;
CREATE INDEX idx_messages_rules_pending ON messages (folder_id) WHERE rules_done = 0;

-- Messages rules have handled, by Message-ID, so a resync (which stores
-- them again) doesn't handle them twice.
CREATE TABLE rules_seen (
    account_id        INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id_header TEXT NOT NULL,
    PRIMARY KEY (account_id, message_id_header)
);
