//! Push subscriptions (RFC 8620 §7.2): the server sends a small JSON
//! object to a Web Push URL whenever the account's state changes, so the
//! client can sync without a connection of its own (M4-27).
//!
//! A subscription belongs to the user, not to one account, so its calls use
//! only the core capability and take no `accountId`. The server proves that
//! the URL is the client's by posting a `PushVerification` with a code; the
//! client then sets `verificationCode` (§7.2.2). Until then nothing else is
//! pushed.
//!
//! Push bodies come from the network through a third party (the Web Push
//! service), so [`parse_push`] treats them as hostile: a size limit, and
//! only the two object types this client acts on.

use crate::http::{Auth, ClientConfig};
use crate::request::{call_using, take, MethodCall};
use crate::session::Session;
use crate::{JmapError, CAP_CORE};
use serde_json::{json, Map, Value};

/// Largest push body [`parse_push`] reads. A `StateChange` names a few
/// types per account; a verification is under 200 bytes.
pub const MAX_PUSH_BYTES: usize = 16 * 1024;

/// The data types a subscription asks to be told about (RFC 8620 §7.2:
/// `types`). Mailbox and Email cover new mail and moved or read mail.
pub const PUSH_TYPES: [&str; 2] = ["Email", "Mailbox"];

/// Keys for encrypting pushes (RFC 8291, RFC 8620 §7.2 `keys`), both
/// base64url without padding: the client's P-256 public key and its
/// 16-byte authentication secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

/// A subscription the server created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedPush {
    /// The server's id for it.
    pub id: String,
    /// When the server will drop it, as unix seconds, if it said.
    pub expires: Option<i64>,
}

/// What a push body asks of the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushObject {
    /// Prove the subscription: set this code on it (RFC 8620 §7.2.2).
    Verification {
        subscription_id: String,
        code: String,
    },
    /// Something changed in these accounts (RFC 8620 §7.1): sync them.
    StateChange { account_ids: Vec<String> },
}

/// Creates a subscription for `url` (RFC 8620 §7.2.1). `device_client_id`
/// is this install's stable id; `expires` is the unix time asked for (the
/// server may shorten it).
pub async fn create_push_subscription(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    device_client_id: &str,
    url: &str,
    keys: Option<&PushKeys>,
    expires: Option<i64>,
) -> Result<CreatedPush, JmapError> {
    // RFC 8620 §7.2: the URL must be https.
    if !url.starts_with("https://") || device_client_id.is_empty() {
        return Err(JmapError::Malformed);
    }
    let mut sub = Map::new();
    sub.insert("deviceClientId".into(), json!(device_client_id));
    sub.insert("url".into(), json!(url));
    sub.insert("types".into(), json!(PUSH_TYPES));
    if let Some(k) = keys {
        sub.insert("keys".into(), json!({"p256dh": k.p256dh, "auth": k.auth}));
    }
    if let Some(at) = expires {
        sub.insert("expires".into(), json!(utc_date(at)));
    }
    let args = set(session, auth, config, json!({"create": {"push": sub}})).await?;
    let created = args
        .get("created")
        .and_then(|c| c.get("push"))
        .ok_or_else(|| refused(&args, "notCreated", "push"))?;
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or(JmapError::Malformed)?
        .to_owned();
    let expires = created
        .get("expires")
        .and_then(Value::as_str)
        .and_then(crate::email::parse_date)
        // The server echoes nothing when it took the time asked for.
        .or(expires);
    Ok(CreatedPush { id, expires })
}

/// Sets the code from the server's `PushVerification` (RFC 8620 §7.2.2).
pub async fn verify_push_subscription(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    id: &str,
    code: &str,
) -> Result<(), JmapError> {
    update(session, auth, config, id, json!({"verificationCode": code}))
        .await
        .map(|_| ())
}

/// Asks the server to keep the subscription until `expires` (unix
/// seconds). Returns the time the server set, which may be earlier.
pub async fn refresh_push_subscription(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    id: &str,
    expires: i64,
) -> Result<i64, JmapError> {
    let updated = update(
        session,
        auth,
        config,
        id,
        json!({"expires": utc_date(expires)}),
    )
    .await?;
    Ok(updated
        .get("expires")
        .and_then(Value::as_str)
        .and_then(crate::email::parse_date)
        .unwrap_or(expires))
}

/// Destroys the subscription. One the server no longer has counts as done.
pub async fn destroy_push_subscription(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    id: &str,
) -> Result<(), JmapError> {
    let args = set(session, auth, config, json!({"destroy": [id]})).await?;
    let destroyed = args
        .get("destroyed")
        .and_then(Value::as_array)
        .is_some_and(|d| d.iter().any(|v| v.as_str() == Some(id)));
    if destroyed {
        return Ok(());
    }
    match refused(&args, "notDestroyed", id) {
        JmapError::Method(kind) if kind == "notFound" => Ok(()),
        other => Err(other),
    }
}

/// Reads a push body: a verification, or the accounts that changed.
/// Anything else, oversized or malformed is [`JmapError::Malformed`].
pub fn parse_push(body: &[u8]) -> Result<PushObject, JmapError> {
    if body.len() > MAX_PUSH_BYTES {
        return Err(JmapError::TooLarge);
    }
    let v: Value = serde_json::from_slice(body).map_err(|_| JmapError::Malformed)?;
    match v.get("@type").and_then(Value::as_str) {
        Some("PushVerification") => {
            let field = |name: &str| {
                v.get(name)
                    .and_then(Value::as_str)
                    .filter(|s| valid_id(s))
                    .map(str::to_owned)
                    .ok_or(JmapError::Malformed)
            };
            Ok(PushObject::Verification {
                subscription_id: field("pushSubscriptionId")?,
                code: field("verificationCode")?,
            })
        }
        Some("StateChange") => {
            let changed = v
                .get("changed")
                .and_then(Value::as_object)
                .ok_or(JmapError::Malformed)?;
            Ok(PushObject::StateChange {
                account_ids: changed
                    .keys()
                    .filter(|k| valid_id(k))
                    .take(64)
                    .cloned()
                    .collect(),
            })
        }
        _ => Err(JmapError::Malformed),
    }
}

/// An `Id` (RFC 8620 §1.2) or a verification code: 1–255 characters,
/// visible ASCII only, so nothing odd reaches logs or the database.
fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && s.bytes().all(|b| b.is_ascii_graphic())
}

async fn set(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    args: Value,
) -> Result<Value, JmapError> {
    let calls = [MethodCall {
        name: "PushSubscription/set".into(),
        args,
        id: "p0".into(),
    }];
    take(
        call_using(session, auth, config, &[CAP_CORE], &calls).await?,
        "p0",
    )
}

/// Updates `id` with `patch`; returns what the server reported back for it
/// (often nothing, RFC 8620 §5.3).
async fn update(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    id: &str,
    patch: Value,
) -> Result<Value, JmapError> {
    let args = set(session, auth, config, json!({"update": {id: patch}})).await?;
    match args.get("updated").and_then(|u| u.get(id)) {
        Some(v) => Ok(if v.is_object() { v.clone() } else { json!({}) }),
        None => Err(refused(&args, "notUpdated", id)),
    }
}

/// The SetError type for `key` in the `kind` map (RFC 8620 §5.3), as
/// [`JmapError::Method`].
fn refused(args: &Value, kind: &str, key: &str) -> JmapError {
    let reason: String = args
        .get(kind)
        .and_then(|m| m.get(key))
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(64)
        .collect();
    JmapError::Method(reason)
}

/// `2026-11-26T08:00:00Z` for unix seconds (RFC 3339 `UTCDate`, RFC 8620
/// §1.4). Times before 1970 become 1970.
pub fn utc_date(secs: i64) -> String {
    let secs = secs.max(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn verifications_and_state_changes_are_read() {
        let v = br#"{"@type":"PushVerification","pushSubscriptionId":"P7","verificationCode":"da1f097b11ca17f06424e30bf02bfa67"}"#;
        assert_eq!(
            parse_push(v),
            Ok(PushObject::Verification {
                subscription_id: "P7".into(),
                code: "da1f097b11ca17f06424e30bf02bfa67".into()
            })
        );
        let s = br#"{"@type":"StateChange","changed":{"A1":{"Email":"e2","Mailbox":"m9"},"A2":{"Email":"x"}}}"#;
        let Ok(PushObject::StateChange { mut account_ids }) = parse_push(s) else {
            panic!("state change");
        };
        account_ids.sort();
        assert_eq!(account_ids, ["A1", "A2"]);
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            &b""[..],
            b"{}",
            b"[]",
            br#"{"@type":"Mailbox"}"#,
            br#"{"@type":"PushVerification","pushSubscriptionId":"P7"}"#,
            br#"{"@type":"PushVerification","pushSubscriptionId":"","verificationCode":"c"}"#,
            br#"{"@type":"PushVerification","pushSubscriptionId":"P\n7","verificationCode":"c"}"#,
            br#"{"@type":"StateChange","changed":[]}"#,
            b"\xff\xfe",
        ] {
            assert!(
                parse_push(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        let huge = vec![b' '; MAX_PUSH_BYTES + 1];
        assert_eq!(parse_push(&huge), Err(JmapError::TooLarge));
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(utc_date(0), "1970-01-01T00:00:00Z");
        for t in [1_764_144_000, 951_782_400, 4_102_444_799] {
            assert_eq!(crate::email::parse_date(&utc_date(t)), Some(t));
        }
    }

    proptest! {
        #[test]
        fn the_parser_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let _ = parse_push(&bytes);
        }

        #[test]
        fn json_shaped_noise_never_panics(s in r#"\{"@type":"(PushVerification|StateChange|x)"(,"[a-zA-Z]{0,20}":("[ -~]{0,12}"|\{\}|\[\]|1|null))*\}"#) {
            let _ = parse_push(s.as_bytes());
        }
    }
}
