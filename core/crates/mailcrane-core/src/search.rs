//! Search (M4-03): instant and offline over the local FTS5 index, plus the
//! server (IMAP `SEARCH`) for messages never downloaded.
//!
//! Queries are parsed here, never handed to SQLite: every user term goes
//! into the FTS5 expression as a quoted string, so FTS syntax in a query
//! (`OR`, `NEAR(`, `*`, `-`, quotes, column names) is just text.

mod local;
mod query;

pub use local::{fts_expression, local_search};
pub use query::{parse_query, SearchQuery};
