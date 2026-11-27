//! Emails (RFC 8621 §4): listing a mailbox newest first, reading the list
//! properties, following changes, and downloading the raw message.

use crate::http::{org, send, Auth, ClientConfig};
use crate::request::{call, take, MethodCall};
use crate::session::Session;
use crate::JmapError;
use hyper::Method;
use serde_json::{json, Value};

/// Most ids asked for in one `Email/query` page or `Email/get`.
pub const MAX_PAGE: u32 = 500;

/// An address in an email header (RFC 8621 §4.1.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmapAddress {
    pub name: Option<String>,
    pub email: String,
}

/// The properties the message list needs (RFC 8621 §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmapEmail {
    pub id: String,
    pub blob_id: String,
    pub thread_id: String,
    pub mailbox_ids: Vec<String>,
    /// Keywords such as `$seen`, `$flagged`, `$draft` (RFC 8621 §4.1.1).
    pub keywords: Vec<String>,
    pub size: u64,
    /// `receivedAt`, unix seconds.
    pub received_at: i64,
    /// `sentAt` (the Date header), unix seconds, if given and valid.
    pub sent_at: Option<i64>,
    /// Message-ID without angle brackets.
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from: Vec<JmapAddress>,
    pub to: Vec<JmapAddress>,
    pub cc: Vec<JmapAddress>,
    pub subject: String,
    pub has_attachment: bool,
    pub preview: String,
}

impl JmapEmail {
    pub fn is_seen(&self) -> bool {
        self.keywords
            .iter()
            .any(|k| k.eq_ignore_ascii_case("$seen"))
    }
    pub fn is_flagged(&self) -> bool {
        self.keywords
            .iter()
            .any(|k| k.eq_ignore_ascii_case("$flagged"))
    }
}

/// One page of `Email/query`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailPage {
    pub ids: Vec<String>,
    /// For `Email/queryChanges` later; servers may not provide it.
    pub query_state: String,
    pub position: u64,
    /// The mailbox's total, if the server calculated it.
    pub total: Option<u64>,
}

/// What changed since a state (RFC 8620 §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailChanges {
    pub old_state: String,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

const LIST_PROPERTIES: &[&str] = &[
    "id",
    "blobId",
    "threadId",
    "mailboxIds",
    "keywords",
    "size",
    "receivedAt",
    "sentAt",
    "messageId",
    "inReplyTo",
    "references",
    "from",
    "to",
    "cc",
    "subject",
    "hasAttachment",
    "preview",
];

fn clean(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect()
}

fn strings(v: Option<&Value>, max: usize) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .take(max)
                .map(|s| clean(s.trim_start_matches('<').trim_end_matches('>'), 998))
                .collect()
        })
        .unwrap_or_default()
}

fn addresses(v: Option<&Value>) -> Vec<JmapAddress> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(500)
                .filter_map(|x| {
                    Some(JmapAddress {
                        name: x
                            .get("name")
                            .and_then(Value::as_str)
                            .map(|n| clean(n, 998))
                            .filter(|n| !n.trim().is_empty()),
                        email: clean(x.get("email")?.as_str()?, 998),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn object_keys(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter(|(_, on)| on.as_bool() == Some(true))
                .map(|(k, _)| k.clone())
                .take(500)
                .collect()
        })
        .unwrap_or_default()
}

/// An RFC 3339 `UTCDate`/`Date` (RFC 8620 §1.4) as unix seconds.
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // Fraction, then the zone: Z or ±hh:mm.
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest.as_bytes().first()? {
        b'Z' | b'z' if rest.len() == 1 => 0,
        sign @ (b'+' | b'-') if rest.len() == 6 => {
            let oh: i64 = rest.get(1..3)?.parse().ok()?;
            let om: i64 = rest.get(4..6)?.parse().ok()?;
            let o = oh * 3600 + om * 60;
            if *sign == b'+' {
                o
            } else {
                -o
            }
        }
        _ => return None,
    };
    // Days from civil (Howard Hinnant, public domain).
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Parses an `Email/get` response's list. Entries without an id are
/// skipped.
pub fn parse_emails(args: &Value) -> Result<Vec<JmapEmail>, JmapError> {
    let list = args
        .get("list")
        .and_then(Value::as_array)
        .ok_or(JmapError::Malformed)?;
    if list.len() > MAX_PAGE as usize {
        return Err(JmapError::TooLarge);
    }
    Ok(list
        .iter()
        .filter_map(|e| {
            let text = |k: &str| e.get(k).and_then(Value::as_str);
            Some(JmapEmail {
                id: text("id")?.to_owned(),
                blob_id: text("blobId").unwrap_or_default().to_owned(),
                thread_id: text("threadId").unwrap_or_default().to_owned(),
                mailbox_ids: object_keys(e.get("mailboxIds")),
                keywords: object_keys(e.get("keywords")),
                size: e.get("size").and_then(Value::as_u64).unwrap_or(0),
                received_at: text("receivedAt").and_then(parse_date).unwrap_or(0),
                sent_at: text("sentAt").and_then(parse_date),
                message_id: strings(e.get("messageId"), 1).into_iter().next(),
                in_reply_to: strings(e.get("inReplyTo"), 1).into_iter().next(),
                references: strings(e.get("references"), 100),
                from: addresses(e.get("from")),
                to: addresses(e.get("to")),
                cc: addresses(e.get("cc")),
                subject: clean(text("subject").unwrap_or_default(), 998),
                has_attachment: e
                    .get("hasAttachment")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                preview: clean(text("preview").unwrap_or_default(), 256),
            })
        })
        .collect())
}

/// A page of a mailbox's emails, newest first by `receivedAt` (RFC 8621
/// §4.4, §4.4.2).
pub async fn query_mailbox(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    mailbox_id: &str,
    position: u64,
    limit: u32,
) -> Result<EmailPage, JmapError> {
    let calls = [MethodCall {
        name: "Email/query".into(),
        args: json!({
            "accountId": session.mail_account_id,
            "filter": {"inMailbox": mailbox_id},
            "sort": [{"property": "receivedAt", "isAscending": false}],
            "position": position,
            "limit": limit.min(MAX_PAGE),
            "calculateTotal": true,
        }),
        id: "q0".into(),
    }];
    let args = take(call(session, auth, config, &calls).await?, "q0")?;
    let ids = args
        .get("ids")
        .and_then(Value::as_array)
        .ok_or(JmapError::Malformed)?;
    if ids.len() > MAX_PAGE as usize {
        return Err(JmapError::TooLarge);
    }
    Ok(EmailPage {
        ids: ids
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        query_state: args
            .get("queryState")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        position: args
            .get("position")
            .and_then(Value::as_u64)
            .unwrap_or(position),
        total: args.get("total").and_then(Value::as_u64),
    })
}

/// The list properties of these emails (RFC 8621 §4.2), and the email state.
pub async fn get_emails(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    ids: &[String],
) -> Result<(String, Vec<JmapEmail>), JmapError> {
    if ids.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    if ids.len() > MAX_PAGE as usize {
        return Err(JmapError::TooLarge);
    }
    let calls = [MethodCall {
        name: "Email/get".into(),
        args: json!({
            "accountId": session.mail_account_id,
            "ids": ids,
            "properties": LIST_PROPERTIES,
        }),
        id: "g0".into(),
    }];
    let args = take(call(session, auth, config, &calls).await?, "g0")?;
    let state = args
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok((state, parse_emails(&args)?))
}

/// What changed since `since_state` (RFC 8620 §5.2). A server that can't
/// tell answers `cannotCalculateChanges`, returned as
/// [`JmapError::Method`]; the caller then resyncs.
pub async fn email_changes(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    since_state: &str,
    max_changes: u32,
) -> Result<EmailChanges, JmapError> {
    let calls = [MethodCall {
        name: "Email/changes".into(),
        args: json!({
            "accountId": session.mail_account_id,
            "sinceState": since_state,
            "maxChanges": max_changes.clamp(1, MAX_PAGE),
        }),
        id: "c0".into(),
    }];
    let args = take(call(session, auth, config, &calls).await?, "c0")?;
    let list = |k: &str| strings(args.get(k), MAX_PAGE as usize * 2);
    Ok(EmailChanges {
        old_state: args
            .get("oldState")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        new_state: args
            .get("newState")
            .and_then(Value::as_str)
            .ok_or(JmapError::Malformed)?
            .to_owned(),
        has_more_changes: args
            .get("hasMoreChanges")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        created: list("created"),
        updated: list("updated"),
        destroyed: list("destroyed"),
    })
}

/// Fills the session's download template (RFC 8620 §6.2): `{accountId}`,
/// `{blobId}`, `{name}` and `{type}`, each percent-encoded.
pub fn download_url(session: &Session, blob_id: &str) -> Result<url::Url, JmapError> {
    let enc = percent_encode;
    let filled = session
        .download_url
        .replace("{accountId}", &enc(&session.mail_account_id))
        .replace("{blobId}", &enc(blob_id))
        .replace("{name}", "message.eml")
        .replace("{type}", "message%2Frfc822");
    same_org_url(session, &filled)
}

/// Resolves a server-given URL against the API URL. Templates come from the
/// server, so credentials only go to the API's own organisation, as for
/// redirects, and only over `https:`.
pub(crate) fn same_org_url(session: &Session, given: &str) -> Result<url::Url, JmapError> {
    let url = session
        .api_url
        .join(given)
        .map_err(|_| JmapError::InvalidUrl)?;
    let host = |u: &url::Url| org(u.host_str().unwrap_or_default());
    if url.scheme() != "https" || host(&url) != host(&session.api_url) {
        return Err(JmapError::InvalidUrl);
    }
    Ok(url)
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Downloads a blob: the raw message for parsing by `mailcrane-mime`.
pub async fn download_blob(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    blob_id: &str,
) -> Result<Vec<u8>, JmapError> {
    let url = download_url(session, blob_id)?;
    let r = send(Method::GET, &url, Some(auth), None, config).await?;
    match r.status {
        200..=299 => Ok(r.body),
        401 | 403 => Err(JmapError::Unauthorized),
        other => Err(JmapError::Http(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(parse_date("2026-11-25T08:00:00Z"), Some(1_795_593_600));
        assert_eq!(parse_date("2026-11-25T09:00:00+01:00"), Some(1_795_593_600));
        assert_eq!(parse_date("2026-11-25T08:00:00.123Z"), Some(1_795_593_600));
        assert_eq!(parse_date("2026-11-25T03:00:00-05:00"), Some(1_795_593_600));
        for bad in [
            "",
            "2026-11-25",
            "2026-13-01T00:00:00Z",
            "2026-11-25T08:00:00",
            "nope nope nope nope",
        ] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn emails_parse() {
        let args = json!({"state": "e1", "list": [{
            "id": "M1", "blobId": "B1", "threadId": "T1",
            "mailboxIds": {"inbox": true, "old": false},
            "keywords": {"$seen": true, "$flagged": true},
            "size": 1234, "receivedAt": "2026-11-25T08:00:00Z", "sentAt": "2026-11-25T09:00:00+01:00",
            "messageId": ["<m1@example.org>"], "inReplyTo": null, "references": ["a@x", "b@x"],
            "from": [{"name": "Ana", "email": "ana@example.org"}],
            "to": [{"name": null, "email": "bob@example.net"}],
            "subject": "Hi\r\nthere", "hasAttachment": true, "preview": "Hello"
        }, {"noid": true}]});
        let list = parse_emails(&args).expect("parse");
        assert_eq!(list.len(), 1);
        let e = &list[0];
        assert_eq!(e.mailbox_ids, ["inbox"]);
        assert!(e.is_seen() && e.is_flagged());
        assert_eq!(e.received_at, 1_795_593_600);
        assert_eq!(e.sent_at, Some(1_795_593_600));
        assert_eq!(e.message_id.as_deref(), Some("m1@example.org"));
        assert_eq!(e.references, ["a@x", "b@x"]);
        assert_eq!(e.from[0].name.as_deref(), Some("Ana"));
        assert_eq!(e.to[0].name, None);
        assert_eq!(e.subject, "Hi  there", "no line breaks");
        assert_eq!(parse_emails(&json!({})), Err(JmapError::Malformed));
    }

    #[test]
    fn download_urls_are_filled_and_encoded() {
        let session = Session {
            api_url: url::Url::parse("https://mail.example.org/api/").expect("url"),
            download_url: "https://mail.example.org/d/{accountId}/{blobId}/{name}?type={type}"
                .into(),
            upload_url: String::new(),
            username: String::new(),
            mail_account_id: "A 1".into(),
            state: String::new(),
            max_objects_in_get: None,
        };
        assert_eq!(
            download_url(&session, "b/../x").expect("url").as_str(),
            "https://mail.example.org/d/A%201/b%2F..%2Fx/message.eml?type=message%2Frfc822"
        );
        let elsewhere = Session {
            download_url: "https://evil.example.net/{blobId}".into(),
            ..session.clone()
        };
        assert_eq!(download_url(&elsewhere, "b"), Err(JmapError::InvalidUrl));
        let plain = Session {
            download_url: "http://mail.example.org/{blobId}".into(),
            ..session
        };
        assert_eq!(download_url(&plain, "b"), Err(JmapError::InvalidUrl));
    }
}
