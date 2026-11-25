//! `APPEND` (RFC 9051 §6.3.12) and finding messages by header, for drafts
//! and sent copies.

use crate::command::Arg;
use crate::parser::{Code, StatusKind, Untagged};
use crate::session::Session;
use crate::store_cmds::ChangeError;

/// Flags for an appended message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendFlags {
    /// `\Seen \Draft`: a draft.
    Draft,
    /// `\Seen`: a copy of a sent message.
    Seen,
}

impl AppendFlags {
    fn list(self) -> &'static str {
        match self {
            Self::Draft => "(\\Seen \\Draft)",
            Self::Seen => "(\\Seen)",
        }
    }
}

/// The new message's UID from an `[APPENDUID <uidvalidity> <uid>]` code
/// (RFC 4315 §3), if the server sent one.
fn appenduid(code: Option<&Code>) -> Option<u32> {
    match code {
        Some(Code::Other { name, args }) if name.eq_ignore_ascii_case("APPENDUID") => args
            .as_deref()?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
            .filter(|uid| *uid > 0),
        _ => None,
    }
}

/// One search key (RFC 9051 §6.4.4). Values are user text, always sent as
/// strings, never as protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchKey {
    /// `TEXT`: anywhere in the header or body.
    Text(String),
    /// `FROM`.
    From(String),
    /// `TO`.
    To(String),
    /// `SUBJECT`.
    Subject(String),
    /// `UNSEEN`.
    Unseen,
    /// `FLAGGED`.
    Flagged,
}

/// Adds `keys` to a `SEARCH` command. Values are always sent as strings.
pub(crate) fn push_search_keys<'a>(args: &mut Vec<Arg<'a>>, keys: &'a [SearchKey]) {
    for key in keys {
        match key {
            SearchKey::Text(v) => args.extend([Arg::Raw("TEXT"), Arg::Str(v.as_bytes())]),
            SearchKey::From(v) => args.extend([Arg::Raw("FROM"), Arg::Str(v.as_bytes())]),
            SearchKey::To(v) => args.extend([Arg::Raw("TO"), Arg::Str(v.as_bytes())]),
            SearchKey::Subject(v) => args.extend([Arg::Raw("SUBJECT"), Arg::Str(v.as_bytes())]),
            SearchKey::Unseen => args.push(Arg::Raw("UNSEEN")),
            SearchKey::Flagged => args.push(Arg::Raw("FLAGGED")),
        }
    }
}

impl Session {
    /// Appends `message` (the raw bytes, CRLF line endings) to the folder
    /// named `raw_name`. Returns the new UID when the server reports it
    /// (UIDPLUS), otherwise `None`. Doesn't need a selected folder.
    pub async fn append(
        &mut self,
        raw_name: &str,
        flags: AppendFlags,
        message: &[u8],
    ) -> Result<Option<u32>, ChangeError> {
        let done = self
            .run_command(
                "APPEND",
                &[
                    Arg::Str(raw_name.as_bytes()),
                    Arg::Raw(flags.list()),
                    Arg::Literal(message),
                ],
            )
            .await?;
        if done.status != StatusKind::Ok {
            return Err(ChangeError::Refused { transient: false });
        }
        Ok(appenduid(done.text.code.as_ref()))
    }

    /// `UID SEARCH HEADER Message-ID <id>` in the selected folder (RFC 9051
    /// §6.4.4): the UIDs of messages with that Message-ID, ascending. `id`
    /// is without angle brackets.
    pub async fn uid_search_message_id(&mut self, id: &str) -> Result<Vec<u32>, ChangeError> {
        let value = format!("<{id}>");
        let done = self
            .run_command(
                "UID",
                &[
                    Arg::Raw("SEARCH"),
                    Arg::Raw("HEADER"),
                    Arg::Raw("Message-ID"),
                    Arg::Str(value.as_bytes()),
                ],
            )
            .await?;
        if done.status != StatusKind::Ok {
            return Err(ChangeError::Refused { transient: false });
        }
        let mut uids: Vec<u32> = done
            .untagged
            .iter()
            .filter_map(|u| match u {
                Untagged::Search(list) => Some(list.iter().copied()),
                _ => None,
            })
            .flatten()
            .collect();
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(args: &str) -> Code {
        Code::Other {
            name: "APPENDUID".into(),
            args: Some(args.into()),
        }
    }

    #[test]
    fn appenduid_is_read_only_when_well_formed() {
        assert_eq!(appenduid(Some(&code("38505 3955"))), Some(3955));
        assert_eq!(appenduid(Some(&code("38505"))), None);
        assert_eq!(appenduid(Some(&code("38505 x"))), None);
        assert_eq!(appenduid(Some(&code("38505 0"))), None);
        assert_eq!(appenduid(None), None);
        let other = Code::Other {
            name: "COPYUID".into(),
            args: Some("1 2 3".into()),
        };
        assert_eq!(appenduid(Some(&other)), None);
    }
}
