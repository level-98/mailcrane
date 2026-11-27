//! Downloading recent messages' bodies in the background (M4-25), so most
//! messages open at once and offline.
//!
//! After a sync of an inbox, the newest messages without a stored body are
//! downloaded in small batches and saved through the same path as opening a
//! message ([`save_body`]): raw bytes, snippet, attachment list and search
//! text. Nothing on the server changes: IMAP uses `BODY.PEEK[]`, JMAP blob
//! downloads never touch `$seen`. Only complete bodies are saved, each in
//! one store transaction, so an interrupted run leaves no partial body and
//! the next run carries on with what is still missing.
//!
//! When to run ([`PrefetchConditions`]) is decided from facts the app
//! reports (network, Battery Saver, free storage, the user's setting); what
//! to fetch ([`PrefetchLimits`], [`candidates`]) from the store. The caller
//! owns the connection and checks between batches whether a message is
//! being opened, and gives way to it.

use super::initial::{save_body, SyncError};
use crate::events::FolderId;
use mailcrane_imap::{Session, PREFETCH_MAX_OCTETS};
use mailcrane_store::{MessageId, RowIdentity, Store};
use std::collections::HashSet;

/// The "Download messages in the background" setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrefetchMode {
    /// Only on an unmetered network (Wi-Fi or Ethernet). The default.
    #[default]
    UnmeteredOnly,
    /// On any network, mobile data included.
    Always,
    /// Never.
    Never,
}

/// Free storage needed before anything is downloaded ahead: more than
/// 500 MB.
pub const MIN_FREE_BYTES: u64 = 500 * 1000 * 1000;

/// What the app reports about the device, and the user's setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefetchConditions {
    /// The user's setting.
    pub mode: PrefetchMode,
    /// Whether the current network is unmetered.
    pub unmetered: bool,
    /// Whether Battery Saver is on.
    pub battery_saver: bool,
    /// Free bytes on the storage holding the database.
    pub free_bytes: u64,
}

impl PrefetchConditions {
    /// Nothing is downloaded: what applies until the app has reported.
    pub const UNKNOWN: Self = Self {
        mode: PrefetchMode::Never,
        unmetered: false,
        battery_saver: false,
        free_bytes: 0,
    };

    /// Whether bodies may be downloaded ahead now. Battery Saver and low
    /// storage stop it whatever the setting says.
    pub fn allows(&self) -> bool {
        let network = match self.mode {
            PrefetchMode::Never => false,
            PrefetchMode::UnmeteredOnly => self.unmetered,
            PrefetchMode::Always => true,
        };
        network && !self.battery_saver && self.free_bytes > MIN_FREE_BYTES
    }
}

impl Default for PrefetchConditions {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

/// Which messages are downloaded ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefetchLimits {
    /// Only messages that arrived within this many seconds. Default 14 days.
    pub max_age_secs: i64,
    /// Only among the folder's newest this many messages. Default 200.
    pub newest: u32,
    /// Only messages of at most this many bytes. Default 1 MiB, which is
    /// also the most a single IMAP prefetch returns.
    pub max_size: u32,
    /// Messages per batch (one `UID FETCH`, or as many JMAP downloads).
    /// Opening a message waits for at most one batch. Default 20.
    pub batch: usize,
}

impl Default for PrefetchLimits {
    fn default() -> Self {
        Self {
            max_age_secs: 14 * 24 * 60 * 60,
            newest: 200,
            max_size: PREFETCH_MAX_OCTETS,
            batch: 20,
        }
    }
}

/// A message to download: its row id and UID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub id: MessageId,
    pub uid: u32,
}

/// The folder's messages still to download, newest first, leaving out
/// those in `skip` (already tried in this run). `now` is unix seconds.
pub fn candidates(
    store: &Store,
    folder: FolderId,
    now: i64,
    limits: &PrefetchLimits,
    skip: &HashSet<MessageId>,
) -> Result<Vec<Candidate>, SyncError> {
    let since = now.saturating_sub(limits.max_age_secs);
    let rows =
        store.prefetch_candidates(folder, since, limits.newest, i64::from(limits.max_size))?;
    Ok(rows
        .into_iter()
        .filter(|(id, _)| !skip.contains(id))
        .map(|(id, uid)| Candidate { id, uid })
        .collect())
}

/// What one batch did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchOutcome {
    /// Bodies saved.
    pub saved: usize,
    /// Every message of the batch, saved or not, so a run doesn't ask for
    /// one the server didn't return (gone, too big) again.
    pub tried: Vec<MessageId>,
}

/// Downloads one batch over an IMAP session that has the batch's `folder`
/// selected with `uidvalidity`, and saves each complete body, only into a
/// row that still holds that UID of that folder (#503). A message the
/// server doesn't return, or returns incomplete or over the size limit, is
/// left as it was.
pub async fn imap_batch(
    store: &Store,
    session: &mut Session,
    folder: FolderId,
    uidvalidity: u32,
    batch: &[Candidate],
) -> Result<BatchOutcome, SyncError> {
    let uids: Vec<u32> = batch.iter().map(|c| c.uid).collect();
    let bodies = session.uid_fetch_complete_bodies(&uids).await?;
    let mut outcome = BatchOutcome {
        saved: 0,
        tried: batch.iter().map(|c| c.id).collect(),
    };
    for (uid, raw) in bodies {
        let Some(candidate) = batch.iter().find(|c| c.uid == uid) else {
            continue;
        };
        let identity = RowIdentity::Imap {
            folder,
            uidvalidity: i64::from(uidvalidity),
            uid,
        };
        match save_body(store, candidate.id, Some(identity), &raw) {
            Ok(()) => outcome.saved = outcome.saved.saturating_add(1),
            // Deleted locally meanwhile (an expunge, a folder reset), maybe
            // with its id reused: skip it.
            Err(SyncError::UnknownMessage) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "prefetch_tests.rs"]
mod tests;
