-- Schema v19 (file 0020: 0019 is taken by an unmerged PR, #318): why a rule
-- was turned off by the app (#323). When a folder a rule moves mail to is
-- removed (deleted in Mailcrane or gone from the server), the rule is
-- disabled and this holds the folder's name, so the Rules screen can say
-- why. Cleared when the user edits or turns the rule back on.
ALTER TABLE rules ADD COLUMN missing_folder TEXT;
