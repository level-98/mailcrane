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
    /// Messages with the `$flagged` keyword (RFC 8621 §4.1.1), a role from
    /// the IANA mailbox roles that RFC 8621 §2 uses: a virtual mailbox,
    /// nothing is moved into it (#461).
    Flagged,
    /// Important messages (RFC 8457 §3). It may be a real folder that mail is
    /// filed into, so it stays an ordinary mailbox (#461 review).
    Important,
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
            "flagged" => Self::Flagged,
            "important" => Self::Important,
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

/// Why a `Mailbox/set` change was refused (RFC 8621 §2.5, RFC 8620 §5.3).
#[derive(Debug, thiserror::Error)]
pub enum MailboxSetError {
    /// A mailbox with that name is already under that parent.
    #[error("a mailbox with that name exists")]
    AlreadyExists,
    /// The mailbox still holds emails (`mailboxHasEmail`).
    #[error("the mailbox has emails")]
    HasEmail,
    /// The mailbox has child mailboxes (`mailboxHasChild`).
    #[error("the mailbox has children")]
    HasChild,
    /// No such mailbox (`notFound`).
    #[error("no such mailbox")]
    NotFound,
    /// Any other set error, such as `forbidden` or `invalidProperties`.
    #[error("refused: {0}")]
    Refused(String),
    /// The request itself failed.
    #[error(transparent)]
    Jmap(#[from] JmapError),
}

impl MailboxSetError {
    fn from_set_error(e: &Value) -> Self {
        let kind = e.get("type").and_then(Value::as_str).unwrap_or("unknown");
        match kind {
            "alreadyExists" => Self::AlreadyExists,
            "mailboxHasEmail" => Self::HasEmail,
            "mailboxHasChild" => Self::HasChild,
            "notFound" => Self::NotFound,
            other => Self::Refused(other.chars().take(64).collect()),
        }
    }
}

/// Runs one `Mailbox/set` with `args` (account id added) and returns the
/// response arguments.
async fn mailbox_set(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    mut args: serde_json::Map<String, Value>,
) -> Result<Value, MailboxSetError> {
    args.insert("accountId".into(), json!(session.mail_account_id));
    let calls = [MethodCall {
        name: "Mailbox/set".into(),
        args: Value::Object(args),
        id: "b0".into(),
    }];
    Ok(take(call(session, auth, config, &calls).await?, "b0")?)
}

/// Creates mailbox `name` under `parent_id` (top level with `None`) and
/// returns its id.
pub async fn create_mailbox(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    name: &str,
    parent_id: Option<&str>,
) -> Result<String, MailboxSetError> {
    let mut args = serde_json::Map::new();
    args.insert(
        "create".into(),
        json!({"new": {"name": name, "parentId": parent_id}}),
    );
    let res = mailbox_set(session, auth, config, args).await?;
    if let Some(e) = res.pointer("/notCreated/new") {
        return Err(MailboxSetError::from_set_error(e));
    }
    res.pointer("/created/new/id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 255)
        .map(str::to_owned)
        .ok_or(MailboxSetError::Jmap(JmapError::Malformed))
}

/// Gives mailbox `id` the name `name` under `parent_id` (top level with
/// `None`): a rename, a move, or both.
pub async fn update_mailbox(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    id: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Result<(), MailboxSetError> {
    let mut args = serde_json::Map::new();
    let mut patch = serde_json::Map::new();
    patch.insert(id.to_owned(), json!({"name": name, "parentId": parent_id}));
    args.insert("update".into(), Value::Object(patch));
    let res = mailbox_set(session, auth, config, args).await?;
    if let Some(e) = res.get("notUpdated").and_then(|n| n.get(id)) {
        return Err(MailboxSetError::from_set_error(e));
    }
    if res.get("updated").and_then(|u| u.get(id)).is_some() {
        Ok(())
    } else {
        Err(MailboxSetError::Jmap(JmapError::Malformed))
    }
}

/// Destroys mailbox `id`, refusing if it still holds emails
/// (`onDestroyRemoveEmails: false`, RFC 8621 §2.5).
pub async fn destroy_mailbox(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    id: &str,
) -> Result<(), MailboxSetError> {
    let mut args = serde_json::Map::new();
    args.insert("destroy".into(), json!([id]));
    args.insert("onDestroyRemoveEmails".into(), json!(false));
    let res = mailbox_set(session, auth, config, args).await?;
    if let Some(e) = res.get("notDestroyed").and_then(|n| n.get(id)) {
        return Err(MailboxSetError::from_set_error(e));
    }
    let destroyed = res
        .get("destroyed")
        .and_then(Value::as_array)
        .is_some_and(|d| d.iter().any(|x| x.as_str() == Some(id)));
    if destroyed {
        Ok(())
    } else {
        Err(MailboxSetError::Jmap(JmapError::Malformed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_set_errors_map() {
        let e = |kind: &str| MailboxSetError::from_set_error(&json!({"type": kind}));
        assert!(matches!(e("alreadyExists"), MailboxSetError::AlreadyExists));
        assert!(matches!(e("mailboxHasEmail"), MailboxSetError::HasEmail));
        assert!(matches!(e("mailboxHasChild"), MailboxSetError::HasChild));
        assert!(matches!(e("notFound"), MailboxSetError::NotFound));
        assert!(matches!(e("forbidden"), MailboxSetError::Refused(k) if k == "forbidden"));
        let long = "x".repeat(500);
        assert!(matches!(e(&long), MailboxSetError::Refused(k) if k.len() == 64));
        assert!(matches!(
            MailboxSetError::from_set_error(&json!({})),
            MailboxSetError::Refused(k) if k == "unknown"
        ));
    }

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
