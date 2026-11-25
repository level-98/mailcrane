//! Conversation threading (M4-02).
//!
//! A thread is a connected group of messages in one account (across its
//! folders). Two messages are linked when:
//!
//! - one's Message-ID is in the other's In-Reply-To or References, or both
//!   refer to the same id (a missing parent links its replies, like JWZ's
//!   placeholder containers); or
//! - **subject rule** (JWZ step 5, narrowed): a reply or forward with a
//!   `Re:`/`Fwd:`-style prefix and *no* In-Reply-To or References has the
//!   same subject (prefixes removed) as an original without a prefix,
//!   dated up to 30 days before it. Two originals both titled "Hello" are
//!   never linked to each other.
//!
//! Because every link depends only on the two messages involved, the
//! groups don't depend on the order messages arrive in: [`assign_new`]
//! (run after each sync) gives the same threads as [`rebuild`]. References
//! are treated as hostile: at most the last 100 per message, malformed ids
//! ignored, and a message referring to itself (or a cycle) is harmless
//! because links are undirected.
//!
//! Deviation from the spec: this is not the JWZ tree algorithm. The tree is
//! only needed to indent replies; the conversation view shows messages by
//! date, so the grouping is computed directly, which is what makes
//! incremental assignment exact. Gmail's `X-GM-THRID` waits for M3-02.

mod links;

pub use links::{
    normalize_subject, thread_links, ThreadLinks, MAX_REFERENCES, SUBJECT_WINDOW_SECS,
};

use mailcrane_store::{AccountId, Store, StoreError, ThreadId, UnthreadedMessage};

/// Puts every message without a thread (of `account`, or every account)
/// into one, merging threads a message links. One transaction. Returns how
/// many messages were placed.
pub fn assign_new(store: &Store, account: Option<AccountId>) -> Result<usize, StoreError> {
    let pending = store.unthreaded_messages(account)?;
    if pending.is_empty() {
        return Ok(0);
    }
    store.in_transaction(|s| {
        for m in &pending {
            assign_one(s, m)?;
        }
        Ok(pending.len())
    })
}

/// Forgets the account's threads and threads every message again.
pub fn rebuild(store: &Store, account: AccountId) -> Result<usize, StoreError> {
    store.in_transaction(|s| s.clear_threads(account))?;
    assign_new(store, Some(account))
}

fn assign_one(store: &Store, m: &UnthreadedMessage) -> Result<ThreadId, StoreError> {
    let links = thread_links(m);
    let mut threads = store.threads_for_keys(m.account_id, &links.keys)?;
    match links.subject_kind {
        links::KIND_REPLY => threads.extend(store.threads_for_subject(
            m.account_id,
            &links.subject_key,
            links::KIND_ORIGINAL,
            m.date.saturating_sub(SUBJECT_WINDOW_SECS),
            m.date,
        )?),
        links::KIND_ORIGINAL => threads.extend(store.threads_for_subject(
            m.account_id,
            &links.subject_key,
            links::KIND_REPLY,
            m.date,
            m.date.saturating_add(SUBJECT_WINDOW_SECS),
        )?),
        _ => {}
    }
    threads.sort_by_key(|t| t.0);
    threads.dedup();
    let thread = match threads.split_first() {
        Some((winner, losers)) => {
            store.merge_threads(*winner, losers)?;
            *winner
        }
        None => store.create_thread(m.account_id, &m.subject, m.date)?,
    };
    store.bind_thread_keys(m.account_id, &links.keys, thread)?;
    store.set_message_thread(m.id, thread, &links.subject_key, links.subject_kind, m.date)?;
    Ok(thread)
}
