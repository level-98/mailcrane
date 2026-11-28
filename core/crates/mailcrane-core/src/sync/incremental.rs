//! Incremental sync: after the first sync, bring a folder up to date
//! cheaply, deletions included.
//!
//! Strategies, best first, picked by what the server supports:
//!
//! 1. **QRESYNC** (RFC 7162 §3.2): one `SELECT` with the last known
//!    `UIDVALIDITY`, `HIGHESTMODSEQ` and stored UID range returns the
//!    changed flags and the expunged UIDs (§3.2.5).
//! 2. **CONDSTORE** (RFC 7162 §3.1): `UID FETCH <stored range> (FLAGS)
//!    (CHANGEDSINCE <modseq>)` for flag changes (§3.1.4), and `UID SEARCH`
//!    over the stored range to find deletions.
//! 3. **Neither**: `UID FETCH <stored range> (FLAGS)`, comparing UIDs and
//!    flags with the store.
//!
//! Each then fetches headers for UIDs above the highest stored one,
//! oldest first, so a failure leaves no gap. `HIGHESTMODSEQ` is saved only
//! after every change up to it is stored (RFC 7162 §3.1.2.1): a failure
//! part way means the next sync asks for the same changes again. A
//! `NOMODSEQ` answer (§3.1.2.2) or a `HIGHESTMODSEQ` lower than the stored
//! one (the server lost its mod-sequences) falls back to strategy 3 for
//! that folder.

use super::convert::remote_flags;
use super::initial::{FolderSyncer, SyncError};
use crate::events::{CoreEvent, FolderId};
use mailcrane_imap::{FetchedFlags, QresyncParams, SelectInfo};
use mailcrane_store::RemoteFlags;
use std::collections::HashSet;

/// How a folder was brought up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Nothing was stored yet, or the server renumbered the folder: the
    /// first-sync path ran instead.
    Initial,
    /// `SELECT ... (QRESYNC ...)` (RFC 7162 §3.2.5).
    Qresync,
    /// `CHANGEDSINCE` and `UID SEARCH` (RFC 7162 §3.1.4).
    Condstore,
    /// A full comparison of UIDs and flags.
    Compare,
    /// The folder can't be opened (`\Noselect`, e.g. Gmail's `[Gmail]`
    /// parent): nothing was sent to the server.
    NotSelectable,
}

/// What [`FolderSyncer::sync_changes`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    /// The strategy used.
    pub strategy: Strategy,
    /// New messages stored.
    pub new: usize,
    /// Stored messages that were deleted or had their flags changed.
    pub changed: usize,
}

/// The stored UID range and what we know about the folder's state.
struct Cached {
    remote_name: String,
    uidvalidity: i64,
    modseq: Option<u64>,
    low: u32,
    high: u32,
}

/// Stored UIDs that fall in any of the inclusive `ranges`.
fn in_ranges(stored: &[u32], ranges: &[(u32, u32)]) -> Vec<u32> {
    stored
        .iter()
        .copied()
        .filter(|uid| ranges.iter().any(|(a, b)| (*a..=*b).contains(uid)))
        .collect()
}

fn to_rows(fetched: &FetchedFlags, high: u32) -> Vec<RemoteFlags> {
    fetched
        .updates
        .iter()
        // Above `high` are new messages, stored with their headers.
        .filter(|u| u.uid <= high)
        .map(|u| remote_flags(u.uid, &u.flags))
        .collect()
}

fn log_skipped(fetched: &FetchedFlags) {
    for uid in &fetched.skipped {
        // Only the UID is logged, never what the server sent.
        tracing::warn!(uid = ?uid, "skipped an unreadable flags response");
    }
}

impl FolderSyncer<'_> {
    /// Brings a folder that was synced before up to date: new messages,
    /// flag changes made elsewhere and expunged messages, using the best
    /// strategy the server supports. A folder with nothing stored yet, or
    /// whose `UIDVALIDITY` changed, gets a first sync instead. On failure
    /// everything already stored is kept, and a `SyncError` event is sent.
    pub async fn sync_changes(&mut self, folder: FolderId) -> Result<SyncReport, SyncError> {
        let result = self.sync_changes_inner(folder).await;
        self.report(result)
    }

    async fn sync_changes_inner(&mut self, folder: FolderId) -> Result<SyncReport, SyncError> {
        if !self.selectable(folder)? {
            return Ok(SyncReport {
                strategy: Strategy::NotSelectable,
                new: 0,
                changed: 0,
            });
        }
        let Some(cached) = self.cached(folder)? else {
            let (_, info, _) = self.select(folder).await?;
            return self.initial(folder, &info).await;
        };

        if let Some(modseq) = cached.modseq {
            let uidvalidity = u32::try_from(cached.uidvalidity).ok();
            if let Some(uidvalidity) = uidvalidity.filter(|_| self.session.supports_qresync()) {
                if self.session.enable_qresync().await? {
                    return self.qresync(folder, &cached, uidvalidity, modseq).await;
                }
            }
        }

        let info = self.session.select(&cached.remote_name).await?;
        if self
            .reset_if_renumbered(folder, Some(cached.uidvalidity), &info)
            .await?
        {
            return self.initial(folder, &info).await;
        }
        match (cached.modseq, info.highestmodseq) {
            (Some(modseq), Some(highest))
                if self.session.supports_condstore() && highest >= modseq =>
            {
                self.condstore(folder, &cached, &info, modseq, highest)
                    .await
            }
            _ => self.compare(folder, &cached, &info).await,
        }
    }

    /// The folder's stored state, or `None` if it has never been synced or
    /// has no messages stored.
    fn cached(&self, folder: FolderId) -> Result<Option<Cached>, SyncError> {
        let row = self
            .store
            .get_folder(folder)?
            .ok_or(SyncError::UnknownFolder)?;
        let (Some(uidvalidity), Some(low), Some(high)) = (
            row.details.uidvalidity,
            self.store.oldest_uid(folder)?,
            self.store.newest_uid(folder)?,
        ) else {
            return Ok(None);
        };
        Ok(Some(Cached {
            remote_name: row.details.remote_name,
            uidvalidity,
            modseq: row
                .details
                .highestmodseq
                .and_then(|m| u64::try_from(m).ok())
                .filter(|m| *m > 0),
            low,
            high,
        }))
    }

    async fn initial(
        &mut self,
        folder: FolderId,
        info: &SelectInfo,
    ) -> Result<SyncReport, SyncError> {
        let new = self.fill_newest(folder, info).await?;
        Ok(SyncReport {
            strategy: Strategy::Initial,
            new,
            changed: 0,
        })
    }

    /// Strategy 1 (RFC 7162 §3.2.5).
    async fn qresync(
        &mut self,
        folder: FolderId,
        cached: &Cached,
        uidvalidity: u32,
        modseq: u64,
    ) -> Result<SyncReport, SyncError> {
        let params = QresyncParams {
            uidvalidity,
            modseq,
            known_uids: Some((cached.low, cached.high)),
        };
        let answer = self
            .session
            .select_qresync(&cached.remote_name, params)
            .await?;
        // §3.2.5.2: with another UIDVALIDITY the server sent no changes.
        if self
            .reset_if_renumbered(folder, Some(cached.uidvalidity), &answer.info)
            .await?
        {
            return self.initial(folder, &answer.info).await;
        }
        let highest = match answer.info.highestmodseq {
            Some(highest) if highest >= modseq => highest,
            // §3.1.2.2 NOMODSEQ, or the server's mod-sequences went back:
            // the answer cannot be trusted, so compare everything.
            _ => return self.compare(folder, cached, &answer.info).await,
        };
        log_skipped(&answer.changed);
        let stored = self.store.list_uids(folder)?;
        let expunged = in_ranges(&stored, &answer.vanished);
        let rows = to_rows(&answer.changed, cached.high);
        let changed = self.apply(folder, &expunged, &rows)?;
        let new = self.fetch_new(folder, cached.high, &answer.info).await?;
        // Re-ask for the same changes next time if any response was lost.
        let keep = if answer.changed.skipped.is_empty() {
            highest
        } else {
            modseq
        };
        self.save_state(folder, &answer.info, Some(keep))?;
        Ok(SyncReport {
            strategy: Strategy::Qresync,
            new,
            changed,
        })
    }

    /// Strategy 2 (RFC 7162 §3.1.4).
    async fn condstore(
        &mut self,
        folder: FolderId,
        cached: &Cached,
        info: &SelectInfo,
        modseq: u64,
        highest: u64,
    ) -> Result<SyncReport, SyncError> {
        // An unchanged HIGHESTMODSEQ means no flag changed (§3.1.2.1).
        let fetched = if highest == modseq {
            FetchedFlags::default()
        } else {
            self.session
                .uid_fetch_flags(cached.low, cached.high, Some(modseq))
                .await?
        };
        log_skipped(&fetched);
        let present: HashSet<u32> = self
            .session
            .uid_search_newest(info.exists, cached.low, cached.high, &[], usize::MAX)
            .await?
            .into_iter()
            .collect();
        let expunged: Vec<u32> = self
            .store
            .list_uids(folder)?
            .into_iter()
            .filter(|uid| !present.contains(uid))
            .collect();
        let rows = to_rows(&fetched, cached.high);
        let changed = self.apply(folder, &expunged, &rows)?;
        let new = self.fetch_new(folder, cached.high, info).await?;
        let keep = if fetched.skipped.is_empty() {
            highest
        } else {
            modseq
        };
        self.save_state(folder, info, Some(keep))?;
        Ok(SyncReport {
            strategy: Strategy::Condstore,
            new,
            changed,
        })
    }

    /// Strategy 3: compare every stored UID and its flags.
    async fn compare(
        &mut self,
        folder: FolderId,
        cached: &Cached,
        info: &SelectInfo,
    ) -> Result<SyncReport, SyncError> {
        let fetched = self
            .session
            .uid_fetch_flags(cached.low, cached.high, None)
            .await?;
        log_skipped(&fetched);
        // A response without a UID could be any message: delete nothing
        // rather than delete one that still exists.
        let complete = !fetched.skipped.contains(&None);
        let expunged: Vec<u32> = if complete {
            let present: HashSet<u32> = fetched
                .updates
                .iter()
                .map(|u| u.uid)
                .chain(fetched.skipped.iter().flatten().copied())
                .collect();
            self.store
                .list_uids(folder)?
                .into_iter()
                .filter(|uid| !present.contains(uid))
                .collect()
        } else {
            Vec::new()
        };
        let rows = to_rows(&fetched, cached.high);
        let changed = self.apply(folder, &expunged, &rows)?;
        let new = self.fetch_new(folder, cached.high, info).await?;
        // The flags were read after SELECT reported this HIGHESTMODSEQ, so
        // every change up to it is stored. If anything was unreadable,
        // store none, so the next sync compares again.
        let highest = if fetched.skipped.is_empty() {
            info.highestmodseq
        } else {
            None
        };
        self.save_state(folder, info, highest)?;
        Ok(SyncReport {
            strategy: Strategy::Compare,
            new,
            changed,
        })
    }

    /// Writes deletions and flag changes in one store transaction.
    fn apply(
        &self,
        folder: FolderId,
        expunged: &[u32],
        rows: &[RemoteFlags],
    ) -> Result<usize, SyncError> {
        if expunged.is_empty() && rows.is_empty() {
            return Ok(0);
        }
        let changed = self.store.apply_remote_changes(folder, expunged, rows)?;
        if changed > 0 {
            tracing::debug!(
                folder = folder.0,
                expunged = expunged.len(),
                changed,
                "applied server changes"
            );
            self.listener
                .on_event(CoreEvent::MessagesChanged { folder_id: folder });
        }
        Ok(changed)
    }

    /// Fetches headers for messages above `high`, oldest first, so that a
    /// failure part way leaves no gap below the highest stored UID.
    async fn fetch_new(
        &mut self,
        folder: FolderId,
        high: u32,
        info: &SelectInfo,
    ) -> Result<usize, SyncError> {
        let top = info.uidnext.saturating_sub(1);
        if top <= high {
            return Ok(0);
        }
        let mut uids = self
            .session
            .uid_search_newest(info.exists, high.saturating_add(1), top, &[], usize::MAX)
            .await?;
        // Oldest first, so an interrupted fetch leaves no gap below the
        // highest stored UID, which is where the next sync starts.
        uids.reverse();
        self.fetch_batches(folder, &uids).await
    }

    /// Records `UIDNEXT` and `HIGHESTMODSEQ`, after the changes are stored.
    fn save_state(
        &self,
        folder: FolderId,
        info: &SelectInfo,
        highestmodseq: Option<u64>,
    ) -> Result<(), SyncError> {
        let highestmodseq = highestmodseq.and_then(|m| i64::try_from(m).ok());
        if !self
            .store
            .set_folder_sync_state(folder, i64::from(info.uidnext), highestmodseq)?
        {
            return Err(SyncError::UnknownFolder);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "incremental_tests.rs"]
mod tests;
