//! Local search over the FTS5 index.

use mailcrane_store::{AccountId, FolderId, MessageHeader, SearchFilter, Store, StoreError};

use super::query::SearchQuery;

/// `term` as an FTS5 string: in double quotes, with quotes doubled. Inside
/// quotes, FTS5 reads nothing as syntax. `None` for a term with no letters
/// or digits, which would be an empty phrase.
fn quoted(term: &str) -> Option<String> {
    if !term.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(format!("\"{}\"", term.replace('"', "\"\"")))
}

/// The FTS5 `MATCH` expression for `q`, or `None` if it has no text terms.
/// Terms are ANDed; the last free word matches as a prefix (search as you
/// type).
pub fn fts_expression(q: &SearchQuery) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let last = q.words.len().checked_sub(1);
    for (i, word) in q.words.iter().enumerate() {
        if let Some(t) = quoted(word) {
            parts.push(if Some(i) == last { format!("{t} *") } else { t });
        }
    }
    parts.extend(q.phrases.iter().filter_map(|p| quoted(p)));
    for (column, values) in [
        ("from_text", &q.from),
        ("to_text", &q.to),
        ("subject", &q.subject),
    ] {
        parts.extend(
            values
                .iter()
                .filter_map(|v| quoted(v))
                .map(|t| format!("{column} : {t}")),
        );
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Searches the stored messages of `account` (or of one `folder`), newest
/// first. Works offline.
pub fn local_search(
    store: &Store,
    account: AccountId,
    folder: Option<FolderId>,
    q: &SearchQuery,
    limit: u32,
) -> Result<Vec<MessageHeader>, StoreError> {
    if q.is_empty() {
        return Ok(Vec::new());
    }
    store.search_messages(&SearchFilter {
        account,
        folder,
        folder_name: q.folder.clone(),
        fts: fts_expression(q),
        unread: q.unread,
        flagged: q.flagged,
        has_attachment: q.has_attachment,
        after: q.after,
        before: q.before,
        limit,
    })
}

#[cfg(test)]
mod tests;
