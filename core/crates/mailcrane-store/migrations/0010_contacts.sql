-- Schema v10: past correspondents for recipient suggestions (M2-08).
--
-- `received_from_count` follows the stored messages: triggers count the
-- From address of every message stored outside Sent, Drafts, Trash, Spam and
-- All Mail, and take it off again when the message row goes (so a resync or a
-- move doesn't count twice). No-reply addresses are never counted.
-- `sent_to_count` is raised by the core when the outbox sends a message.
CREATE TABLE correspondents (
    id                  INTEGER PRIMARY KEY,
    account_id          INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email               TEXT NOT NULL,          -- lower-case
    name                TEXT NOT NULL DEFAULT '',
    sent_to_count       INTEGER NOT NULL DEFAULT 0,
    received_from_count INTEGER NOT NULL DEFAULT 0,
    last_seen_at        INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, email)
);

CREATE TRIGGER correspondents_received AFTER INSERT ON messages
WHEN NEW.from_addr IS NOT NULL AND trim(NEW.from_addr) <> ''
    AND instr(NEW.from_addr, '@') > 0
    AND COALESCE((SELECT role FROM folders WHERE id = NEW.folder_id), '')
        NOT IN ('sent', 'drafts', 'trash', 'spam', 'all')
    AND lower(NEW.from_addr) NOT LIKE '%noreply%'
    AND lower(NEW.from_addr) NOT LIKE '%no-reply%'
    AND lower(NEW.from_addr) NOT LIKE '%no!_reply%' ESCAPE '!'
    AND lower(NEW.from_addr) NOT LIKE '%donotreply%'
    AND lower(NEW.from_addr) NOT LIKE '%do-not-reply%'
    AND lower(NEW.from_addr) NOT LIKE 'mailer-daemon@%'
BEGIN
    INSERT INTO correspondents (account_id, email, name, received_from_count, last_seen_at)
    VALUES (
        (SELECT account_id FROM folders WHERE id = NEW.folder_id),
        lower(trim(NEW.from_addr)),
        COALESCE(trim(NEW.from_name), ''),
        1,
        NEW.date
    )
    ON CONFLICT (account_id, email) DO UPDATE SET
        received_from_count = received_from_count + 1,
        last_seen_at = max(last_seen_at, excluded.last_seen_at),
        name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE name END;
END;

CREATE TRIGGER correspondents_received_gone AFTER DELETE ON messages
WHEN OLD.from_addr IS NOT NULL
BEGIN
    UPDATE correspondents SET received_from_count = max(received_from_count - 1, 0)
    WHERE email = lower(trim(OLD.from_addr))
        AND account_id = (SELECT account_id FROM folders WHERE id = OLD.folder_id)
        AND COALESCE((SELECT role FROM folders WHERE id = OLD.folder_id), '')
            NOT IN ('sent', 'drafts', 'trash', 'spam', 'all');
END;

-- Existing mail.
INSERT INTO correspondents (account_id, email, name, received_from_count, last_seen_at)
SELECT f.account_id, lower(trim(m.from_addr)), COALESCE(max(trim(m.from_name)), ''), count(*), max(m.date)
FROM messages m JOIN folders f ON f.id = m.folder_id
WHERE m.from_addr IS NOT NULL AND trim(m.from_addr) <> '' AND instr(m.from_addr, '@') > 0
    AND COALESCE(f.role, '') NOT IN ('sent', 'drafts', 'trash', 'spam', 'all')
    AND lower(m.from_addr) NOT LIKE '%noreply%'
    AND lower(m.from_addr) NOT LIKE '%no-reply%'
    AND lower(m.from_addr) NOT LIKE '%no!_reply%' ESCAPE '!'
    AND lower(m.from_addr) NOT LIKE '%donotreply%'
    AND lower(m.from_addr) NOT LIKE '%do-not-reply%'
    AND lower(m.from_addr) NOT LIKE 'mailer-daemon@%'
GROUP BY f.account_id, lower(trim(m.from_addr));
