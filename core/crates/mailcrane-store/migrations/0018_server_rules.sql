-- Schema v18: server-side rules through ManageSieve (M4-23, ADR 0004).
-- One row per account once the server has been checked.
--   state 'unavailable':  no usable ManageSieve server; checked again later
--   state 'other_active': another script is active, so rules stay local
--   state 'active':       Mailcrane's script "mailcrane" is the active one
CREATE TABLE server_rules (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK (state IN ('unavailable', 'other_active', 'active')),
    host TEXT,
    port INTEGER,
    -- The server's Sieve extensions, space separated.
    extensions TEXT NOT NULL DEFAULT '',
    -- The other active script's name (state 'other_active'), as the server sent it.
    other_script TEXT,
    -- Hash of the script now on the server (state 'active').
    script_hash TEXT,
    checked_at INTEGER NOT NULL,
    -- The user asked to make Mailcrane's script the active one.
    switch_requested INTEGER NOT NULL DEFAULT 0 CHECK (switch_requested IN (0, 1))
);

-- The rules in the account's server script, with a fingerprint of each
-- rule's settings as uploaded. A rule edited since then runs locally until
-- the new script is up.
CREATE TABLE server_rules_deployed (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    rule_id INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    PRIMARY KEY (account_id, rule_id)
);
