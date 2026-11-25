//! Search for the apps (M4-03): local results at once with
//! `search_messages`, then `SyncEngine::search_server` to fetch matches that
//! were never downloaded; after it, search locally again.

use std::collections::HashMap;

use crate::messages::{marker_color, marker_colors, MessageSummary};
use crate::sync::{failed, SyncEngine, SyncFailure};
use mailcrane_core::events;
use mailcrane_core::search::{
    format_day, local_search, local_search_all, parse_query, with_filter, Filter,
};
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

/// Searches the stored mail of every account (M4-21) for `query`, which
/// may use `account:` to narrow it to accounts whose address or name
/// contains the value. Each row names its account and marker colour.
/// Newest first, at most `limit` (capped at 500). Reads only the database.
#[uniffi::export]
pub fn search_all_accounts(
    db_path: String,
    query: String,
    limit: u32,
) -> Result<Vec<MessageSummary>, SyncFailure> {
    let storage = |_| failed(events::SyncErrorKind::Storage);
    let store = Store::open(&db_path).map_err(storage)?;
    let found =
        local_search_all(&store, &parse_query(&query), limit.min(MAX_RESULTS)).map_err(storage)?;
    let colors = marker_colors(&store).map_err(storage)?;
    // Which account each folder belongs to.
    let mut owner: HashMap<i64, i64> = HashMap::new();
    for account in store.list_accounts().map_err(storage)? {
        for folder in store.list_folders(account.id).map_err(storage)? {
            owner.insert(folder.id.0, account.id.0);
        }
    }
    Ok(found
        .into_iter()
        .map(|header| {
            let account = owner.get(&header.details.folder_id.0).copied().unwrap_or(0);
            let mut summary = MessageSummary::from(header);
            summary.account_id = account;
            summary.account_color = marker_color(&colors, account);
            summary
        })
        .collect())
}

/// The filters in a search query, for the search chips (M4-21).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchFilters {
    /// `from:` values.
    pub from: Vec<String>,
    pub has_attachment: bool,
    pub unread: bool,
    pub flagged: bool,
    /// `after:`, as days since 1970-01-01 (inclusive).
    pub after_day: Option<i64>,
    /// `before:`, as days since 1970-01-01 (exclusive).
    pub before_day: Option<i64>,
    /// `account:`.
    pub account: Option<String>,
}

/// The filters `query` uses. Never fails.
#[uniffi::export]
pub fn search_filters(query: String) -> SearchFilters {
    let q = parse_query(&query);
    SearchFilters {
        from: q.from,
        has_attachment: q.has_attachment,
        unread: q.unread,
        flagged: q.flagged,
        after_day: q.after.map(|s| s.div_euclid(86_400)),
        before_day: q.before.map(|s| s.div_euclid(86_400)),
        account: q.account,
    }
}

/// An on/off search filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SearchFlag {
    HasAttachment,
    Unread,
    Flagged,
}

/// `query` with `flag` turned on or off; the rest is kept.
#[uniffi::export]
pub fn search_with_flag(query: String, flag: SearchFlag, on: bool) -> String {
    let filter = match flag {
        SearchFlag::HasAttachment => Filter::HasAttachment,
        SearchFlag::Unread => Filter::Unread,
        SearchFlag::Flagged => Filter::Flagged,
    };
    with_filter(&query, filter, on.then_some(""))
}

/// `query` without any `from:` terms.
#[uniffi::export]
pub fn search_without_from(query: String) -> String {
    with_filter(&query, Filter::From, None)
}

/// `query` limited to dates from `after_day` (inclusive) to `before_day`
/// (exclusive), both days since 1970-01-01; `None` removes that end. A day
/// outside years 1–9999 removes that end too.
#[uniffi::export]
pub fn search_with_dates(query: String, after_day: Option<i64>, before_day: Option<i64>) -> String {
    let after = after_day.and_then(format_day);
    let before = before_day.and_then(format_day);
    let query = with_filter(&query, Filter::After, after.as_deref());
    with_filter(&query, Filter::Before, before.as_deref())
}

/// `query` limited to accounts matching `account` (an address or name), or
/// to every account with `None`.
#[uniffi::export]
pub fn search_with_account(query: String, account: Option<String>) -> String {
    with_filter(&query, Filter::Account, account.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_read_and_edit_the_query_text() {
        let q = search_with_flag("lunch".into(), SearchFlag::Unread, true);
        let q = search_with_flag(q, SearchFlag::HasAttachment, true);
        let q = search_with_dates(q, Some(20_757), Some(20_788));
        let q = search_with_account(q, Some("ann@example.net".into()));
        assert_eq!(
            q,
            "lunch is:unread has:attachment after:2026-10-31 before:2026-12-01 \
             account:ann@example.net"
        );
        let f = search_filters(format!("{q} from:ana"));
        assert_eq!(
            f,
            SearchFilters {
                from: vec!["ana".into()],
                has_attachment: true,
                unread: true,
                flagged: false,
                after_day: Some(20_757),
                before_day: Some(20_788),
                account: Some("ann@example.net".into()),
            }
        );
        let q = search_with_flag(q, SearchFlag::Unread, false);
        let q = search_with_dates(q, None, None);
        let q = search_with_account(q, None);
        assert_eq!(q, "lunch has:attachment");
        assert_eq!(search_without_from("from:ana lunch from:".into()), "lunch");
        assert_eq!(
            search_with_dates("x".into(), Some(i64::MAX), Some(-1)),
            "x before:1969-12-31"
        );
    }
}
