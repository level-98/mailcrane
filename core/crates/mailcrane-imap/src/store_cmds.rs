//! Changing messages on the server: `UID STORE`, `UID COPY`, `UID MOVE` and
//! `UID EXPUNGE`, all in the selected folder.

use crate::command::Arg;
use crate::error::ImapError;
use crate::fetch::uid_set;
use crate::parser::{Code, StatusKind};
use crate::session::{has_cap, Done, Session};

/// A system flag the app changes (RFC 9051 §2.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemFlag {
    /// `\Seen`: read.
    Seen,
    /// `\Flagged`: starred.
    Flagged,
    /// `\Deleted`: marked for expunge.
    Deleted,
}

/// Why a command that changes messages did not succeed.
#[derive(Debug, thiserror::Error)]
pub enum ChangeError {
    /// The server answered `NO` or `BAD`. `transient` is true for refusals
    /// that may succeed later (RFC 5530 `UNAVAILABLE`, `INUSE`, `LIMIT`,
    /// `SERVERBUG`); anything else, such as a missing folder, is permanent.
    #[error("the server refused the change")]
    Refused {
        /// Worth trying again later.
        transient: bool,
    },
    /// The connection or protocol failed. Whether the change was applied is
    /// unknown.
    #[error(transparent)]
    Imap(#[from] ImapError),
}

impl ChangeError {
    /// Whether trying again later may work: connection trouble and transient
    /// refusals, but not a permanent `NO` or `BAD`.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Refused { transient } => *transient,
            // A refused SELECT (the folder is gone), bad input or a local
            // database failure won't get better by waiting.
            Self::Imap(
                ImapError::CommandFailed | ImapError::InvalidInput | ImapError::Store(_),
            ) => false,
            Self::Imap(_) => true,
        }
    }
}

/// RFC 5530 response codes that describe a temporary condition.
const TRANSIENT_CODES: &[&str] = &["UNAVAILABLE", "INUSE", "LIMIT", "SERVERBUG"];

fn check(done: &Done) -> Result<(), ChangeError> {
    if done.status == StatusKind::Ok {
        return Ok(());
    }
    let transient = done.status == StatusKind::No
        && matches!(&done.text.code, Some(Code::Other { name, .. })
            if TRANSIENT_CODES.iter().any(|c| name.eq_ignore_ascii_case(c)));
    Err(ChangeError::Refused { transient })
}

/// The fixed `+FLAGS.SILENT (...)` / `-FLAGS.SILENT (...)` arguments.
/// `.SILENT` because the next sync reads flags anyway (RFC 9051 §6.4.6).
fn flags_args(flag: SystemFlag, add: bool) -> (&'static str, &'static str) {
    let op = if add {
        "+FLAGS.SILENT"
    } else {
        "-FLAGS.SILENT"
    };
    let list = match flag {
        SystemFlag::Seen => "(\\Seen)",
        SystemFlag::Flagged => "(\\Flagged)",
        SystemFlag::Deleted => "(\\Deleted)",
    };
    (op, list)
}

impl Session {
    /// Whether the server supports `MOVE` (RFC 6851; built into IMAP4rev2,
    /// RFC 9051 §6.4.8).
    pub fn supports_move(&self) -> bool {
        has_cap(self.capabilities(), "MOVE") || has_cap(self.capabilities(), "IMAP4rev2")
    }

    /// Whether the server supports `UID EXPUNGE` (RFC 4315 UIDPLUS; built
    /// into IMAP4rev2, RFC 9051 §6.4.9).
    pub fn supports_uid_expunge(&self) -> bool {
        has_cap(self.capabilities(), "UIDPLUS") || has_cap(self.capabilities(), "IMAP4rev2")
    }

    /// `UID STORE <uids> +FLAGS.SILENT (<flag>)`, or `-FLAGS.SILENT` when
    /// `add` is false (RFC 9051 §6.4.6). UIDs that no longer exist are
    /// ignored by the server, so repeating it is harmless.
    pub async fn uid_store_flag(
        &mut self,
        uids: &[u32],
        flag: SystemFlag,
        add: bool,
    ) -> Result<(), ChangeError> {
        if uids.is_empty() {
            return Ok(());
        }
        let set = uid_set(uids);
        let (op, list) = flags_args(flag, add);
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("STORE"),
                    Arg::Built(&set),
                    Arg::Raw(op),
                    Arg::Raw(list),
                ],
            )
            .await?;
        check(&done)
    }

    /// `UID COPY <uids> <folder>` (RFC 9051 §6.4.7). Not idempotent: each
    /// call makes new copies.
    pub async fn uid_copy(&mut self, uids: &[u32], to_raw_name: &str) -> Result<(), ChangeError> {
        if uids.is_empty() {
            return Ok(());
        }
        let set = uid_set(uids);
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("COPY"),
                    Arg::Built(&set),
                    Arg::Str(to_raw_name.as_bytes()),
                ],
            )
            .await?;
        check(&done)
    }

    /// `UID MOVE <uids> <folder>` (RFC 6851 §3.2). Moving UIDs that are no
    /// longer in the folder does nothing, so a repeat after a dropped
    /// connection makes no duplicates. Only call it when
    /// [`Session::supports_move`] is true.
    pub async fn uid_move(&mut self, uids: &[u32], to_raw_name: &str) -> Result<(), ChangeError> {
        if uids.is_empty() {
            return Ok(());
        }
        let set = uid_set(uids);
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("MOVE"),
                    Arg::Built(&set),
                    Arg::Str(to_raw_name.as_bytes()),
                ],
            )
            .await?;
        check(&done)
    }

    /// `UID EXPUNGE <uids>` (RFC 4315 §2.1): removes only these messages,
    /// if they have `\Deleted`. Only call it when
    /// [`Session::supports_uid_expunge`] is true. There is deliberately no
    /// plain `EXPUNGE` here: it would also remove messages other clients
    /// marked `\Deleted`.
    pub async fn uid_expunge(&mut self, uids: &[u32]) -> Result<(), ChangeError> {
        if uids.is_empty() {
            return Ok(());
        }
        let set = uid_set(uids);
        let done = self
            .run_command("UID", &[Arg::Raw("EXPUNGE"), Arg::Built(&set)])
            .await?;
        check(&done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::RespText;

    fn done(status: StatusKind, code: Option<&str>) -> Done {
        Done {
            untagged: Vec::new(),
            status,
            text: RespText {
                code: code.map(|name| Code::Other {
                    name: name.to_string(),
                    args: None,
                }),
                text: String::new(),
            },
        }
    }

    #[test]
    fn refusals_are_permanent_unless_the_code_says_otherwise() {
        assert!(check(&done(StatusKind::Ok, None)).is_ok());
        for (status, code, transient) in [
            (StatusKind::No, None, false),
            (StatusKind::No, Some("TRYCREATE"), false),
            (StatusKind::No, Some("NONEXISTENT"), false),
            (StatusKind::No, Some("UNAVAILABLE"), true),
            (StatusKind::No, Some("inuse"), true),
            (StatusKind::No, Some("LIMIT"), true),
            (StatusKind::Bad, Some("UNAVAILABLE"), false),
            (StatusKind::Bad, None, false),
        ] {
            let error = check(&done(status, code)).err();
            assert!(
                matches!(error, Some(ChangeError::Refused { transient: t }) if t == transient),
                "{status:?} {code:?}"
            );
        }
    }

    #[test]
    fn connection_trouble_is_transient() {
        assert!(ChangeError::Imap(ImapError::ConnectionClosed).is_transient());
        assert!(ChangeError::Imap(ImapError::Timeout).is_transient());
        assert!(!ChangeError::Refused { transient: false }.is_transient());
        assert!(!ChangeError::Imap(ImapError::CommandFailed).is_transient());
    }

    #[test]
    fn flag_arguments_are_fixed_strings() {
        assert_eq!(
            flags_args(SystemFlag::Seen, true),
            ("+FLAGS.SILENT", "(\\Seen)")
        );
        assert_eq!(
            flags_args(SystemFlag::Flagged, false),
            ("-FLAGS.SILENT", "(\\Flagged)")
        );
        assert_eq!(
            flags_args(SystemFlag::Deleted, true),
            ("+FLAGS.SILENT", "(\\Deleted)")
        );
    }
}
