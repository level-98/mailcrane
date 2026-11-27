-- Schema v24: a virtual folder's kind (issue #390), such as Gmail's
-- "[Gmail]/Starred" (RFC 6154 \Flagged) or "[Gmail]/Important". 'starred'
-- or 'important'; NULL for every other folder, so existing rows are
-- unchanged. It only gives the folder a name, an icon and a place among the
-- system folders. Such a folder's `role` stays NULL, so nothing that moves,
-- archives or deletes by role can ever pick it. No CHECK: the app reads an
-- unknown value as none, so a newer app's value never breaks an older one.
ALTER TABLE folders ADD COLUMN virtual_role TEXT;
