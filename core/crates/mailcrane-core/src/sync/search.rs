//! Searching the server (M4-03): finds messages that were never downloaded
//! and stores their headers, so the local search finds them too.

use mailcrane_imap::SearchKey;
use mailcrane_store::FolderId;

use super::initial::{FolderSyncer, SyncError};
use crate::search::SearchQuery;

/// At most this many new messages are fetched per server search.
const MAX_NEW: usize = 200;

impl FolderSyncer<'_> {
    /// Asks the server for messages in `folder` matching `query` and stores
    /// the headers of the ones not stored yet (newest first, at most 200).
    /// Returns how many were stored. Filters the server can't do (dates,
    /// attachments) are left to the local search that follows.
    pub async fn search_server(
        &mut self,
        folder: FolderId,
        query: &SearchQuery,
    ) -> Result<usize, SyncError> {
        let mut keys: Vec<SearchKey> = Vec::new();
        // The server matches words anywhere, so phrases are sent word by word.
        for word in query
            .words
            .iter()
            .chain(&query.phrases)
            .flat_map(|t| t.split_whitespace())
            .filter(|w| w.chars().any(char::is_alphanumeric))
        {
            keys.push(SearchKey::Text(word.to_owned()));
        }
        keys.extend(query.from.iter().map(|v| SearchKey::From(v.clone())));
        keys.extend(query.to.iter().map(|v| SearchKey::To(v.clone())));
        keys.extend(query.subject.iter().map(|v| SearchKey::Subject(v.clone())));
        if query.unread {
            keys.push(SearchKey::Unseen);
        }
        if query.flagged {
            keys.push(SearchKey::Flagged);
        }
        if keys.is_empty() {
            return Ok(0);
        }
        let result = self.search_server_inner(folder, &keys).await;
        self.report(result)
    }

    async fn search_server_inner(
        &mut self,
        folder: FolderId,
        keys: &[SearchKey],
    ) -> Result<usize, SyncError> {
        let (_, _, _) = self.select(folder).await?;
        let found = self.session.uid_search(keys).await?;
        let known: std::collections::HashSet<u32> =
            self.store.list_uids(folder)?.into_iter().collect();
        let mut missing: Vec<u32> = found
            .into_iter()
            .filter(|uid| !known.contains(uid))
            .collect();
        // Newest first: higher UIDs are newer.
        missing.sort_unstable_by(|a, b| b.cmp(a));
        missing.truncate(MAX_NEW);
        self.fetch_batches(folder, &missing).await
    }
}
