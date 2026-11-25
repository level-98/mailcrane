//! Search for the apps (M4-03): local results at once with
//! `search_messages`, then `SyncEngine::search_server` to fetch matches that
//! were never downloaded; after it, search locally again.

use crate::messages::MessageSummary;
use crate::sync::{failed, SyncEngine, SyncFailure};
use mailcrane_core::events;
use mailcrane_core::search::{local_search, parse_query};
use mailcrane_store::{AccountId, FolderId, Store};

/// Most results returned at once.
const MAX_RESULTS: u32 = 500;

/// Searches the stored mail of `account_id` (or of one folder) for `query`
/// (words, `"phrases"`, `from:`, `to:`, `subject:`, `has:attachment`,
/// `is:unread`, `is:flagged`, `before:`/`after:YYYY-MM-DD`, `in:folder`).
/// Newest first, at most `limit` (capped at 500). Works offline; reads only
/// the database.
#[uniffi::export]
pub fn search_messages(
    db_path: String,
    account_id: i64,
    folder_id: Option<i64>,
    query: String,
    limit: u32,
) -> Result<Vec<MessageSummary>, SyncFailure> {
    let storage = |_| failed(events::SyncErrorKind::Storage);
    let store = Store::open(&db_path).map_err(storage)?;
    let found = local_search(
        &store,
        AccountId(account_id),
        folder_id.map(FolderId),
        &parse_query(&query),
        limit.min(MAX_RESULTS),
    )
    .map_err(storage)?;
    Ok(found.into_iter().map(MessageSummary::from).collect())
}

#[uniffi::export]
impl SyncEngine {
    /// Asks the server for messages in `folder_id` matching `query` and
    /// stores the headers of any not stored yet (at most 200). Returns how
    /// many were stored; then call `search_messages` again. Blocking.
    pub fn search_server(&self, folder_id: i64, query: String) -> Result<u32, SyncFailure> {
        let parsed = parse_query(&query);
        let stored = self.run(FolderId(folder_id), move |syncer| {
            Box::pin(async move { syncer.search_server(FolderId(folder_id), &parsed).await })
        })?;
        Ok(u32::try_from(stored).unwrap_or(u32::MAX))
    }
}
