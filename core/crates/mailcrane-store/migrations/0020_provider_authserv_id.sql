-- Schema v20: the authserv-id of the account's own provider (M5-08, #310).
-- NULL (the default) means unset: only Authentication-Results headers above
-- the point where a message entered the provider are trusted. When set, a
-- header with exactly this authserv-id directly below that point is trusted
-- too (Postfix with OpenDMARC writes it there). Lowercase host name.
ALTER TABLE accounts ADD COLUMN provider_authserv_id TEXT
    CHECK (provider_authserv_id IS NULL OR length(provider_authserv_id) BETWEEN 1 AND 253);
