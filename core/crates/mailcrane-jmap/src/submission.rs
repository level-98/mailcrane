//! Sending (RFC 8621 §7): upload the message (RFC 8620 §6.1), import it
//! into Drafts, then submit it with `EmailSubmission/set`. On success the
//! server moves it to Sent itself (`onSuccessUpdateEmail`), so there's no
//! separate Sent copy to upload.

use crate::email::{percent_encode, same_org_url};
use crate::http::{send, Auth, ClientConfig};
use crate::request::{call_using, MethodCall, MethodResponse};
use crate::session::Session;
use crate::{JmapError, CAP_CORE, CAP_MAIL};
use hyper::Method;
use serde_json::{json, Value};

/// The submission capability (RFC 8621 §1.3.2).
pub const CAP_SUBMISSION: &str = "urn:ietf:params:jmap:submission";

/// Largest message we upload; providers' limits are around this.
pub const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

/// Most envelope recipients in one submission.
pub const MAX_RECIPIENTS: usize = 500;

/// Who the user may send as (RFC 8621 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// What to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The whole RFC 5322 message, as `mailcrane-mime` builds it.
    pub raw: Vec<u8>,
    pub identity_id: String,
    /// SMTP envelope (RFC 8621 §7.1 `envelope`); Bcc recipients are here
    /// and not in the headers.
    pub mail_from: String,
    pub rcpt_to: Vec<String>,
    pub drafts_mailbox_id: String,
    pub sent_mailbox_id: String,
}

/// What the server made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub email_id: String,
    pub submission_id: String,
}

fn using() -> [&'static str; 3] {
    [CAP_CORE, CAP_MAIL, CAP_SUBMISSION]
}

/// An address fit for the envelope: one `@`, no spaces, controls or angle
/// brackets.
fn envelope_address(a: &str) -> bool {
    a.len() <= 320
        && a.matches('@').count() == 1
        && !a.starts_with('@')
        && !a.ends_with('@')
        && !a
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>,;".contains(c))
}

/// The identities the user can send as.
pub async fn get_identities(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
) -> Result<Vec<Identity>, JmapError> {
    let calls = [MethodCall {
        name: "Identity/get".into(),
        args: json!({"accountId": session.mail_account_id}),
        id: "i0".into(),
    }];
    let responses = call_using(session, auth, config, &using(), &calls).await?;
    let args = find(responses, "Identity/get", "i0")?;
    let list = args
        .get("list")
        .and_then(Value::as_array)
        .ok_or(JmapError::Malformed)?;
    Ok(list
        .iter()
        .take(1000)
        .filter_map(|i| {
            Some(Identity {
                id: i.get("id")?.as_str()?.to_owned(),
                name: i
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(256)
                    .collect(),
                email: i.get("email")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// Uploads a blob (RFC 8620 §6.1) and returns its id. The upload URL comes
/// from the session, so it must belong to the API's organisation.
pub async fn upload(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    bytes: Vec<u8>,
    content_type: &str,
) -> Result<String, JmapError> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(JmapError::TooLarge);
    }
    let filled = session
        .upload_url
        .replace("{accountId}", &percent_encode(&session.mail_account_id));
    let url = same_org_url(session, &filled)?;
    let r = send(
        Method::POST,
        &url,
        Some(auth),
        Some((bytes, content_type)),
        config,
    )
    .await?;
    match r.status {
        200..=299 => {}
        401 | 403 => return Err(JmapError::Unauthorized),
        413 => return Err(JmapError::TooLarge),
        other => return Err(JmapError::Http(other)),
    }
    let v: Value = serde_json::from_slice(&r.body).map_err(|_| JmapError::Malformed)?;
    v.get("blobId")
        .and_then(Value::as_str)
        .filter(|b| !b.is_empty() && b.len() <= 255)
        .map(str::to_owned)
        .ok_or(JmapError::Malformed)
}

/// The `Email/import` and `EmailSubmission/set` calls for one message
/// whose blob is uploaded. The submission refers to the imported email by
/// creation id (`#draft`, RFC 8620 §5.3), and on success the email leaves
/// Drafts for Sent and loses `$draft` (RFC 8621 §7.5).
pub fn send_calls(
    account_id: &str,
    blob_id: &str,
    out: &Outgoing,
) -> Result<[MethodCall; 2], JmapError> {
    if out.rcpt_to.is_empty() || out.rcpt_to.len() > MAX_RECIPIENTS {
        return Err(JmapError::TooLarge);
    }
    if !envelope_address(&out.mail_from) || !out.rcpt_to.iter().all(|r| envelope_address(r)) {
        return Err(JmapError::Malformed);
    }
    if out.identity_id.is_empty()
        || out.drafts_mailbox_id.is_empty()
        || out.sent_mailbox_id.is_empty()
    {
        return Err(JmapError::Malformed);
    }
    let token = |s: &str| s.replace('~', "~0").replace('/', "~1");
    let mut update = serde_json::Map::new();
    if out.drafts_mailbox_id != out.sent_mailbox_id {
        update.insert(
            format!("mailboxIds/{}", token(&out.drafts_mailbox_id)),
            Value::Null,
        );
    }
    update.insert(
        format!("mailboxIds/{}", token(&out.sent_mailbox_id)),
        Value::Bool(true),
    );
    update.insert("keywords/$draft".into(), Value::Null);
    let rcpt: Vec<Value> = out.rcpt_to.iter().map(|r| json!({"email": r})).collect();
    Ok([
        MethodCall {
            name: "Email/import".into(),
            args: json!({
                "accountId": account_id,
                "emails": {"draft": {
                    "blobId": blob_id,
                    "mailboxIds": {out.drafts_mailbox_id.clone(): true},
                    "keywords": {"$draft": true, "$seen": true},
                }},
            }),
            id: "i0".into(),
        },
        MethodCall {
            name: "EmailSubmission/set".into(),
            args: json!({
                "accountId": account_id,
                "create": {"send": {
                    "identityId": out.identity_id,
                    "emailId": "#draft",
                    "envelope": {"mailFrom": {"email": out.mail_from}, "rcptTo": rcpt},
                }},
                "onSuccessUpdateEmail": {"#send": update},
            }),
            id: "s0".into(),
        },
    ])
}

/// The response named `name` with call id `id`; method errors become
/// errors.
fn find(responses: Vec<MethodResponse>, name: &str, id: &str) -> Result<Value, JmapError> {
    responses
        .into_iter()
        .find(|r| r.id == id && (r.name == name || r.name == "error"))
        .ok_or(JmapError::Malformed)?
        .into_result()
}

/// The id made for creation id `cid`, or the SetError type as an error
/// (RFC 8620 §5.3 `created` / `notCreated`).
fn created_id(args: &Value, cid: &str) -> Result<String, JmapError> {
    if let Some(id) = args
        .pointer(&format!("/created/{cid}/id"))
        .and_then(Value::as_str)
    {
        return Ok(id.to_owned());
    }
    let kind = args
        .pointer(&format!("/notCreated/{cid}/type"))
        .and_then(Value::as_str)
        .ok_or(JmapError::Malformed)?;
    Err(JmapError::Method(kind.chars().take(64).collect()))
}

/// Uploads, imports and submits `out` in two requests.
pub async fn send_message(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    out: &Outgoing,
) -> Result<Sent, JmapError> {
    // Check everything before uploading anything.
    send_calls(&session.mail_account_id, "check", out)?;
    let blob_id = upload(session, auth, config, out.raw.clone(), "message/rfc822").await?;
    let calls = send_calls(&session.mail_account_id, &blob_id, out)?;
    let responses = call_using(session, auth, config, &using(), &calls).await?;
    let email_id = created_id(&find(responses.clone(), "Email/import", "i0")?, "draft")?;
    let submission_id = created_id(&find(responses, "EmailSubmission/set", "s0")?, "send")?;
    Ok(Sent {
        email_id,
        submission_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outgoing() -> Outgoing {
        Outgoing {
            raw: b"Subject: x\r\n\r\nbody\r\n".to_vec(),
            identity_id: "id-1".into(),
            mail_from: "alice@example.org".into(),
            rcpt_to: vec!["bob@example.net".into(), "hidden@example.net".into()],
            drafts_mailbox_id: "d".into(),
            sent_mailbox_id: "s".into(),
        }
    }

    #[test]
    fn calls_chain_import_and_submission() {
        let [import, submit] = send_calls("A1", "B1", &outgoing()).expect("calls");
        assert_eq!(import.args["emails"]["draft"]["blobId"], "B1");
        assert_eq!(import.args["emails"]["draft"]["keywords"]["$draft"], true);
        assert_eq!(submit.args["create"]["send"]["emailId"], "#draft");
        assert_eq!(
            submit.args["create"]["send"]["envelope"]["rcptTo"][1]["email"],
            "hidden@example.net"
        );
        let update = &submit.args["onSuccessUpdateEmail"]["#send"];
        assert_eq!(update["mailboxIds/d"], Value::Null);
        assert_eq!(update["mailboxIds/s"], true);
        assert_eq!(update["keywords/$draft"], Value::Null);
    }

    #[test]
    fn bad_envelopes_are_refused() {
        let mut none = outgoing();
        none.rcpt_to.clear();
        assert_eq!(
            send_calls("A1", "B", &none).map(|_| ()),
            Err(JmapError::TooLarge)
        );
        for bad in [
            "bob",
            "bob@",
            "@x",
            "a b@x",
            "a@b@c",
            "<a@b>",
            "a@b\r\nRCPT TO:<x@y>",
        ] {
            let mut o = outgoing();
            o.rcpt_to = vec![bad.into()];
            assert_eq!(
                send_calls("A1", "B", &o).map(|_| ()),
                Err(JmapError::Malformed),
                "{bad}"
            );
        }
    }

    #[test]
    fn creation_results() {
        let ok = json!({"created": {"draft": {"id": "M9"}}});
        assert_eq!(created_id(&ok, "draft"), Ok("M9".into()));
        let refused = json!({"notCreated": {"draft": {"type": "blobNotFound"}}});
        assert_eq!(
            created_id(&refused, "draft"),
            Err(JmapError::Method("blobNotFound".into()))
        );
        assert_eq!(created_id(&json!({}), "draft"), Err(JmapError::Malformed));
    }
}
