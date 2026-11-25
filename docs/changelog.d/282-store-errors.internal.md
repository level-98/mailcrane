When a sync fails because the local database refused a change, the log now names the database error (for example `SQLITE_CONSTRAINT_UNIQUE`) and the column involved, never the values.
