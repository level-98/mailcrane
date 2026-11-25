-- Schema v8: offline full-text search (M4-03). One FTS5 row per message,
-- with the same rowid as the message. `remove_diacritics 2` makes "élodie"
-- and "Elodie" match each other.

CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject,
    from_text,
    to_text,
    body_text,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Headers are indexed as they are stored; body text when the body is
-- downloaded (Store::index_body_text). The address lists are JSON; the
-- tokenizer drops the punctuation, leaving names and addresses.
INSERT INTO messages_fts (rowid, subject, from_text, to_text, body_text)
    SELECT id, subject, coalesce(from_name, '') || ' ' || coalesce(from_addr, ''),
           to_json || ' ' || cc_json, ''
    FROM messages;

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, subject, from_text, to_text, body_text)
    VALUES (new.id, new.subject,
            coalesce(new.from_name, '') || ' ' || coalesce(new.from_addr, ''),
            new.to_json || ' ' || new.cc_json, '');
END;

CREATE TRIGGER messages_fts_update AFTER UPDATE OF subject, from_name, from_addr, to_json, cc_json
    ON messages BEGIN
    UPDATE messages_fts SET
        subject = new.subject,
        from_text = coalesce(new.from_name, '') || ' ' || coalesce(new.from_addr, ''),
        to_text = new.to_json || ' ' || new.cc_json
    WHERE rowid = new.id;
END;

CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = old.id;
END;
