//! CONDSTORE and QRESYNC (RFC 7162): after the first sync, ask the server
//! only for what changed in a folder since last time.
//!
//! - CONDSTORE gives every message a mod-sequence that grows whenever its
//!   flags change, and every folder a `HIGHESTMODSEQ` (RFC 7162 §3.1).
//! - QRESYNC builds on it: one `SELECT` with the last known state returns
//!   the changed flags and the expunged UIDs (RFC 7162 §3.2).

use crate::command::Arg;
use crate::error::ImapError;
use crate::fetch::{select_info, SelectInfo};
use crate::parser::Untagged;
use crate::session::{expect_ok, has_cap, Session};

/// The flags of one message, as reported by a `FETCH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagUpdate {
    /// The message's UID.
    pub uid: u32,
    /// Its flags, as sent (`\Seen`, keywords).
    pub flags: Vec<String>,
    /// Its mod-sequence, when the server sent one (RFC 7162 §3.1.4.1).
    pub modseq: Option<u64>,
}

/// The result of a flags-only `UID FETCH`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchedFlags {
    /// Messages whose flags were read, each UID once.
    pub updates: Vec<FlagUpdate>,
    /// Responses that could not be read, with the UID where it was found.
    /// When one has no UID, the caller cannot tell which message it was.
    pub skipped: Vec<Option<u32>>,
}

/// What we last knew about a folder, sent with `SELECT` (RFC 7162 §3.2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QresyncParams {
    /// The `UIDVALIDITY` the state belongs to.
    pub uidvalidity: u32,
    /// The `HIGHESTMODSEQ` we last stored.
    pub modseq: u64,
    /// The range of UIDs we have stored, `(lowest, highest)`, if any.
    pub known_uids: Option<(u32, u32)>,
}

/// What a `SELECT ... (QRESYNC ...)` returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QresyncSelect {
    /// The usual `SELECT` data, including the new `HIGHESTMODSEQ`.
    pub info: SelectInfo,
    /// UIDs expunged since `modseq`, as inclusive ranges (`VANISHED
    /// (EARLIER)`, RFC 7162 §3.2.5.1). Can include UIDs we never had.
    pub vanished: Vec<(u32, u32)>,
    /// Messages whose flags changed since `modseq`.
    pub changed: FetchedFlags,
}

fn collect_flags(untagged: Vec<Untagged>, wanted: impl Fn(u32) -> bool) -> FetchedFlags {
    let mut out = FetchedFlags::default();
    for u in untagged {
        match u {
            Untagged::Fetch(data) => match (data.uid, data.flags) {
                (Some(uid), Some(flags)) if wanted(uid) => {
                    // A server may send one message twice; the last wins.
                    out.updates.retain(|f| f.uid != uid);
                    out.updates.push(FlagUpdate {
                        uid,
                        flags,
                        modseq: data.modseq,
                    });
                }
                // Outside what we asked for, or a FETCH without FLAGS (for
                // example an unsolicited one): nothing to apply.
                (Some(_), _) => {}
                (None, _) => out.skipped.push(None),
            },
            Untagged::FetchMalformed { uid } => out.skipped.push(uid),
            _ => {}
        }
    }
    out
}

impl Session {
    /// Whether the server supports CONDSTORE. QRESYNC implies it (RFC 7162
    /// §3.2).
    pub fn supports_condstore(&self) -> bool {
        has_cap(self.capabilities(), "CONDSTORE") || self.supports_qresync()
    }

    /// Whether the server supports QRESYNC (RFC 7162 §3.2).
    pub fn supports_qresync(&self) -> bool {
        has_cap(self.capabilities(), "QRESYNC")
    }

    /// `ENABLE QRESYNC` (RFC 7162 §3.2.3, RFC 5161), once per session.
    /// Returns whether QRESYNC is now on. A server without QRESYNC is not
    /// asked, and a refusal is `Ok(false)`, not an error.
    pub async fn enable_qresync(&mut self) -> Result<bool, ImapError> {
        if self.qresync_enabled {
            return Ok(true);
        }
        if !self.supports_qresync() {
            return Ok(false);
        }
        let done = self.run_command("ENABLE", &[Arg::Raw("QRESYNC")]).await?;
        // RFC 5161 §3.2: only what the ENABLED response lists is on.
        let enabled = done.untagged.iter().any(|u| {
            matches!(u, Untagged::Enabled(caps)
                if caps.iter().any(|c| c.eq_ignore_ascii_case("QRESYNC")))
        });
        self.qresync_enabled = expect_ok(&done).is_ok() && enabled;
        Ok(self.qresync_enabled)
    }

    /// `SELECT <folder> (QRESYNC (<uidvalidity> <modseq> [<known-uids>]))`
    /// (RFC 7162 §3.2.5). Needs [`Session::enable_qresync`] first.
    ///
    /// If the server's `UIDVALIDITY` differs from `params.uidvalidity`,
    /// it sends no changes (§3.2.5.2): check `info.uidvalidity` before
    /// using the result.
    pub async fn select_qresync(
        &mut self,
        raw_name: &str,
        params: QresyncParams,
    ) -> Result<QresyncSelect, ImapError> {
        if !self.qresync_enabled || params.uidvalidity == 0 || params.modseq == 0 {
            // §3.2.5: SELECT with QRESYNC before ENABLE is a BAD. Nothing sent.
            return Err(ImapError::InvalidInput);
        }
        let known = match params.known_uids {
            Some((low, high)) if low > 0 && low <= high => format!(" {low}:{high}"),
            _ => String::new(),
        };
        let qresync = format!(
            "(QRESYNC ({} {}{known}))",
            params.uidvalidity, params.modseq
        );
        let done = self
            .run_command(
                "SELECT",
                &[Arg::Str(raw_name.as_bytes()), Arg::Built(&qresync)],
            )
            .await?;
        expect_ok(&done)?;
        let info = select_info(&done.untagged)?;
        let mut vanished = Vec::new();
        for u in &done.untagged {
            if let Untagged::Vanished { uids, .. } = u {
                vanished.extend_from_slice(uids);
            }
        }
        let changed = collect_flags(done.untagged, |_| true);
        Ok(QresyncSelect {
            info,
            vanished,
            changed,
        })
    }

    /// `UID FETCH <low>:<high> (FLAGS)` in the selected folder (RFC 9051
    /// §6.4.5), or with `changed_since`, `... (FLAGS) (CHANGEDSINCE <n>)`
    /// for only the messages whose mod-sequence is above `n` (RFC 7162
    /// §3.1.4). Only UIDs in `low..=high` are returned.
    pub async fn uid_fetch_flags(
        &mut self,
        low: u32,
        high: u32,
        changed_since: Option<u64>,
    ) -> Result<FetchedFlags, ImapError> {
        if low == 0 || low > high {
            return Ok(FetchedFlags::default());
        }
        let range = format!("{low}:{high}");
        let modifier = changed_since.map(|m| format!("(CHANGEDSINCE {m})"));
        let mut args = vec![
            Arg::Raw("FETCH"),
            Arg::Str(range.as_bytes()),
            Arg::Raw("(UID FLAGS)"),
        ];
        if let Some(modifier) = &modifier {
            args.push(Arg::Built(modifier));
        }
        let done = self.run_command("UID", &args).await?;
        expect_ok(&done)?;
        Ok(collect_flags(done.untagged, |uid| {
            (low..=high).contains(&uid)
        }))
    }
}
