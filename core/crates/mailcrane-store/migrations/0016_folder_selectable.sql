-- Schema v16: whether a folder can be opened (issue #282).
-- 0 for a \Noselect folder (RFC 9051 §7.3.1), such as Gmail's "[Gmail]"
-- parent: it only holds other folders, so it is shown in the tree but never
-- SELECTed. Existing rows default to 1 until the next folder list.
ALTER TABLE folders ADD COLUMN selectable INTEGER NOT NULL DEFAULT 1
    CHECK (selectable IN (0, 1));
