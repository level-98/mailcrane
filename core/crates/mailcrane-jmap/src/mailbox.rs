//! Mailboxes (RFC 8621 §2): the JMAP equivalent of IMAP folders.

use crate::http::{Auth, ClientConfig};
use crate::request::{call, take, MethodCall};
use crate::session::Session;
use crate::JmapError;
use serde_json::{json, Value};

/// A mailbox role (RFC 8621 §2, from the IANA "IMAP Mailbox Name
/// Attributes" registry), as far as the app uses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailboxRole {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    All,
}

impl MailboxRole {
    fn parse(role: &str) -> Option<Self> {
        Some(match role.to_ascii_lowercase().as_str() {
            "inbox" => Self::Inbox,
            "drafts" => Self::Drafts,
            "sent" => Self::Sent,
            "archive" => Self::Archive,
            "junk" => Self::Junk,
            "trash" => Self::Trash,
            "all" => Self::All,
            _ => return None,
        })
    }
}

/// A mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmapMailbox {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub role: Option<MailboxRole>,
    pub total_emails: u64,
    pub unread_emails: u64,
    pub sort_order: u64,
}

/// Most mailboxes read from one account.
const MAX_MAILBOXES: usize = 10_000;

/// Parses a `Mailbox/get` response's arguments: the state and the list.
/// Entries without an id or name are skipped.
pub fn parse_mailboxes(args: &Value) -> Result<(String, Vec<JmapMailbox>), JmapError> {
    let state = args
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let list = args
        .get("list")
        .and_then(Value::as_array)
        .ok_or(JmapError::Malformed)?;
    if list.len() > MAX_MAILBOXES {
        return Err(JmapError::TooLarge);
    }
    let count = |m: &Value, k: &str| m.get(k).and_then(Value::as_u64).unwrap_or(0);
    let mailboxes = list
        .iter()
        .filter_map(|m| {
            Some(JmapMailbox {
                id: m.get("id")?.as_str()?.to_owned(),
                name: m
                    .get("name")?
                    .as_str()?
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(255)
                    .collect(),
                parent_id: m.get("parentId").and_then(Value::as_str).map(str::to_owned),
                role: m
                    .get("role")
                    .and_then(Value::as_str)
                    .and_then(MailboxRole::parse),
                total_emails: count(m, "totalEmails"),
                unread_emails: count(m, "unreadEmails"),
                sort_order: count(m, "sortOrder"),
            })
        })
        .collect();
    Ok((state, mailboxes))
}

/// Every mailbox of the session's mail account, and the mailbox state for
/// later `Mailbox/changes` (RFC 8621 §2.2, RFC 8620 §5.1).
pub async fn get_mailboxes(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
) -> Result<(String, Vec<JmapMailbox>), JmapError> {
    let calls = [MethodCall {
        name: "Mailbox/get".into(),
        args: json!({
            "accountId": session.mail_account_id,
            "ids": null,
            "properties": ["id", "name", "parentId", "role", "totalEmails", "unreadEmails", "sortOrder"],
        }),
        id: "m0".into(),
    }];
    let responses = call(session, auth, config, &calls).await?;
    parse_mailboxes(&take(responses, "m0")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailboxes_parse_with_roles() {
        let args = json!({"state": "7", "list": [
            {"id": "a", "name": "Inbox", "role": "inbox", "totalEmails": 3, "unreadEmails": 1},
            {"id": "b", "name": "Projects", "parentId": "a"},
            {"id": "c", "name": "Sent Items", "role": "SENT"},
            {"name": "no id"},
            {"id": "d", "name": "Bad\u{0}Name", "role": "custom"}
        ]});
        let (state, list) = parse_mailboxes(&args).expect("parse");
        assert_eq!(state, "7");
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].role, Some(MailboxRole::Inbox));
        assert_eq!(list[0].unread_emails, 1);
        assert_eq!(list[1].parent_id.as_deref(), Some("a"));
        assert_eq!(list[2].role, Some(MailboxRole::Sent));
        assert_eq!(list[3].name, "BadName");
        assert_eq!(list[3].role, None);
        assert_eq!(parse_mailboxes(&json!({})), Err(JmapError::Malformed));
    }
}
