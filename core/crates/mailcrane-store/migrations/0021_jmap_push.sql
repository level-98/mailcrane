-- Schema v21: JMAP push subscriptions (M4-27, #367, RFC 8620 §7.2). At
-- most one per account: the server's id for it, this install's
-- deviceClientId, the Web Push URL it was made for (a UnifiedPush
-- endpoint), the client's public key when one was sent (never the auth
-- secret), when the server will drop it, and whether the verification push
-- has been answered. Removing the account removes its row.
CREATE TABLE jmap_push_subscriptions (
    account_id       INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    subscription_id  TEXT NOT NULL CHECK (length(subscription_id) BETWEEN 1 AND 255),
    device_client_id TEXT NOT NULL CHECK (length(device_client_id) BETWEEN 1 AND 255),
    url              TEXT NOT NULL CHECK (length(url) BETWEEN 1 AND 2048),
    p256dh           TEXT CHECK (p256dh IS NULL OR length(p256dh) BETWEEN 1 AND 255),
    expires          INTEGER,
    verified         INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
    created_at       INTEGER NOT NULL
);
