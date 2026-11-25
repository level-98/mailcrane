//! Store errors.

/// Everything that can go wrong in the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database file was written by a newer version of the app.
    #[error("database schema v{found} is newer than the newest known v{supported}")]
    NewerSchema {
        /// `user_version` found in the file.
        found: i64,
        /// Newest schema this build knows.
        supported: i64,
    },
    /// A stored value was not one the schema allows.
    #[error("corrupt value in column {column}")]
    Corrupt {
        /// Column that held the bad value.
        column: &'static str,
    },
    /// A value could not be stored (for example, out of range).
    #[error("invalid value for {field}")]
    Invalid {
        /// Field that was rejected.
        field: &'static str,
    },
    /// The underlying SQLite call failed.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl StoreError {
    /// The variant's name, e.g. `"Sqlite"`. Safe to log: a fixed string.
    pub fn name(&self) -> &'static str {
        match self {
            StoreError::NewerSchema { .. } => "NewerSchema",
            StoreError::Corrupt { .. } => "Corrupt",
            StoreError::Invalid { .. } => "Invalid",
            StoreError::Sqlite(_) => "Sqlite",
        }
    }

    /// The schema column or field named by `Corrupt` and `Invalid`. Safe to
    /// log: a fixed name from our own code, never a value.
    pub fn field(&self) -> Option<&'static str> {
        match self {
            StoreError::Corrupt { column } => Some(column),
            StoreError::Invalid { field } => Some(field),
            _ => None,
        }
    }

    /// What went wrong inside rusqlite or SQLite, as a fixed name: the
    /// SQLite extended result code (e.g. `"SQLITE_CONSTRAINT_UNIQUE"`) or
    /// the rusqlite error kind (e.g. `"ToSqlConversionFailure"`). Never the
    /// message, which can quote values or SQL.
    pub fn sqlite_detail(&self) -> Option<&'static str> {
        let StoreError::Sqlite(error) = self else {
            return None;
        };
        Some(match error {
            rusqlite::Error::SqliteFailure(e, _) => sqlite_code_name(e.extended_code),
            rusqlite::Error::FromSqlConversionFailure(..) => "FromSqlConversionFailure",
            rusqlite::Error::IntegralValueOutOfRange(..) => "IntegralValueOutOfRange",
            rusqlite::Error::ToSqlConversionFailure(_) => "ToSqlConversionFailure",
            rusqlite::Error::InvalidColumnType(..) => "InvalidColumnType",
            rusqlite::Error::QueryReturnedNoRows => "QueryReturnedNoRows",
            rusqlite::Error::QueryReturnedMoreThanOneRow => "QueryReturnedMoreThanOneRow",
            rusqlite::Error::NulError(_) => "NulError",
            rusqlite::Error::Utf8Error(..) => "Utf8Error",
            _ => "OtherRusqliteError",
        })
    }

    /// The SQLite extended result code as a number, for codes
    /// [`sqlite_code_name`] doesn't know.
    pub fn sqlite_code(&self) -> Option<i32> {
        match self {
            StoreError::Sqlite(rusqlite::Error::SqliteFailure(e, _)) => Some(e.extended_code),
            _ => None,
        }
    }
}

/// The name of an SQLite (extended) result code (sqlite.org/rescode.html).
/// Unknown codes become `"SQLITE_OTHER"`; log the number alongside.
pub fn sqlite_code_name(code: i32) -> &'static str {
    use rusqlite::ffi;
    match code {
        ffi::SQLITE_CONSTRAINT_UNIQUE => "SQLITE_CONSTRAINT_UNIQUE",
        ffi::SQLITE_CONSTRAINT_PRIMARYKEY => "SQLITE_CONSTRAINT_PRIMARYKEY",
        ffi::SQLITE_CONSTRAINT_CHECK => "SQLITE_CONSTRAINT_CHECK",
        ffi::SQLITE_CONSTRAINT_NOTNULL => "SQLITE_CONSTRAINT_NOTNULL",
        ffi::SQLITE_CONSTRAINT_FOREIGNKEY => "SQLITE_CONSTRAINT_FOREIGNKEY",
        ffi::SQLITE_CONSTRAINT_TRIGGER => "SQLITE_CONSTRAINT_TRIGGER",
        ffi::SQLITE_CONSTRAINT_DATATYPE => "SQLITE_CONSTRAINT_DATATYPE",
        ffi::SQLITE_CONSTRAINT => "SQLITE_CONSTRAINT",
        ffi::SQLITE_TOOBIG => "SQLITE_TOOBIG",
        ffi::SQLITE_FULL => "SQLITE_FULL",
        ffi::SQLITE_BUSY => "SQLITE_BUSY",
        ffi::SQLITE_LOCKED => "SQLITE_LOCKED",
        ffi::SQLITE_READONLY => "SQLITE_READONLY",
        ffi::SQLITE_CORRUPT => "SQLITE_CORRUPT",
        ffi::SQLITE_NOMEM => "SQLITE_NOMEM",
        ffi::SQLITE_MISMATCH => "SQLITE_MISMATCH",
        ffi::SQLITE_RANGE => "SQLITE_RANGE",
        ffi::SQLITE_CANTOPEN => "SQLITE_CANTOPEN",
        ffi::SQLITE_ERROR => "SQLITE_ERROR",
        c if c & 0xff == ffi::SQLITE_IOERR => "SQLITE_IOERR",
        c if c & 0xff == ffi::SQLITE_CONSTRAINT => "SQLITE_CONSTRAINT",
        _ => "SQLITE_OTHER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn failure(sql: &str) -> StoreError {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch("CREATE TABLE t (a INTEGER NOT NULL UNIQUE CHECK (a < 10), b TEXT);")
            .expect("schema");
        conn.execute_batch("INSERT INTO t (a, b) VALUES (1, 'secret-value');")
            .expect("row");
        StoreError::from(conn.execute_batch(sql).expect_err("fails"))
    }

    #[test]
    fn constraint_failures_are_named_without_values() {
        for (sql, name) in [
            (
                "INSERT INTO t (a, b) VALUES (1, 'secret-value')",
                "SQLITE_CONSTRAINT_UNIQUE",
            ),
            (
                "INSERT INTO t (a, b) VALUES (99, 'secret-value')",
                "SQLITE_CONSTRAINT_CHECK",
            ),
            (
                "INSERT INTO t (a, b) VALUES (NULL, 'secret-value')",
                "SQLITE_CONSTRAINT_NOTNULL",
            ),
        ] {
            let error = failure(sql);
            assert_eq!(error.name(), "Sqlite");
            assert_eq!(error.sqlite_detail(), Some(name), "{sql}");
            assert!(error.sqlite_code().is_some());
            assert_eq!(error.field(), None);
        }
    }

    #[test]
    fn fields_and_names_are_fixed_strings() {
        let corrupt = StoreError::Corrupt { column: "role" };
        assert_eq!(
            (corrupt.name(), corrupt.field(), corrupt.sqlite_detail()),
            ("Corrupt", Some("role"), None)
        );
        let invalid = StoreError::Invalid { field: "size" };
        assert_eq!((invalid.name(), invalid.field()), ("Invalid", Some("size")));
        assert_eq!(sqlite_code_name(123_456), "SQLITE_OTHER");
        assert_eq!(
            sqlite_code_name(rusqlite::ffi::SQLITE_IOERR_READ),
            "SQLITE_IOERR"
        );
    }
}
