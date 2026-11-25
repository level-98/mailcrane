-- Schema v15: a colour the user chose for the account's marker (M4-20).
-- NULL means automatic: the colour follows the account id, as before.
-- Values are indexes into the apps' palette of 8 marker colours.
ALTER TABLE accounts ADD COLUMN color INTEGER
    CHECK (color IS NULL OR (color >= 0 AND color < 8));
