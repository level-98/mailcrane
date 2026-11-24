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
