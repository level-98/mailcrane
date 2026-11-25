//! Method calls (RFC 8620 §3): one request with several calls, and the
//! responses matched back by call id.

use crate::http::{send, Auth, ClientConfig};
use crate::session::Session;
use crate::{JmapError, CAP_CORE, CAP_MAIL};
use hyper::Method;
use serde_json::{json, Value};

/// One method call: `[name, arguments, callId]`.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodCall {
    pub name: String,
    pub args: Value,
    pub id: String,
}

/// One method response. `name` is `"error"` for a failed call (RFC 8620
/// §3.6.2); use [`MethodResponse::into_result`].
#[derive(Debug, Clone, PartialEq)]
pub struct MethodResponse {
    pub name: String,
    pub args: Value,
    pub id: String,
}

impl MethodResponse {
    /// The arguments, or the call's error type as [`JmapError::Method`].
    pub fn into_result(self) -> Result<Value, JmapError> {
        if self.name == "error" {
            let kind = self
                .args
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(64)
                .collect();
            Err(JmapError::Method(kind))
        } else {
            Ok(self.args)
        }
    }
}

/// Parses a response object's `methodResponses`. Anything that isn't the
/// expected shape is [`JmapError::Malformed`]; nothing panics.
pub fn parse_response(body: &[u8]) -> Result<Vec<MethodResponse>, JmapError> {
    let v: Value = serde_json::from_slice(body).map_err(|_| JmapError::Malformed)?;
    let list = v
        .get("methodResponses")
        .and_then(Value::as_array)
        .ok_or(JmapError::Malformed)?;
    list.iter()
        .map(|item| {
            let parts = item
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or(JmapError::Malformed)?;
            let name = parts[0].as_str().ok_or(JmapError::Malformed)?.to_owned();
            let args = parts[1].clone();
            if !args.is_object() {
                return Err(JmapError::Malformed);
            }
            let id = parts[2].as_str().ok_or(JmapError::Malformed)?.to_owned();
            Ok(MethodResponse { name, args, id })
        })
        .collect()
}

/// Sends `calls` in one request to the session's API URL, using the core
/// and mail capabilities, and returns the responses in order.
pub async fn call(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    calls: &[MethodCall],
) -> Result<Vec<MethodResponse>, JmapError> {
    call_using(session, auth, config, &[CAP_CORE, CAP_MAIL], calls).await
}

/// [`call`] with other capabilities in `using` (RFC 8620 §3.3), for
/// example submission.
pub(crate) async fn call_using(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    using: &[&str],
    calls: &[MethodCall],
) -> Result<Vec<MethodResponse>, JmapError> {
    let body = json!({
        "using": using,
        "methodCalls": calls.iter().map(|c| json!([c.name, c.args, c.id])).collect::<Vec<_>>(),
    });
    let bytes = serde_json::to_vec(&body).map_err(|_| JmapError::Malformed)?;
    let r = send(
        Method::POST,
        &session.api_url,
        Some(auth),
        Some((bytes, "application/json")),
        config,
    )
    .await?;
    match r.status {
        200..=299 => parse_response(&r.body),
        401 | 403 => Err(JmapError::Unauthorized),
        other => Err(JmapError::Http(other)),
    }
}

/// The response to call `id`, as a result.
pub(crate) fn take(responses: Vec<MethodResponse>, id: &str) -> Result<Value, JmapError> {
    responses
        .into_iter()
        .find(|r| r.id == id)
        .ok_or(JmapError::Malformed)?
        .into_result()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn responses_and_errors() {
        let body = br#"{"methodResponses":[["Mailbox/get",{"list":[]},"c0"],
            ["error",{"type":"cannotCalculateChanges"},"c1"]],"sessionState":"s"}"#;
        let r = parse_response(body).expect("parse");
        assert_eq!(r.len(), 2);
        assert_eq!(take(r.clone(), "c0"), Ok(json!({"list": []})));
        assert_eq!(
            take(r.clone(), "c1"),
            Err(JmapError::Method("cannotCalculateChanges".into()))
        );
        assert_eq!(take(r, "c9"), Err(JmapError::Malformed));
        for bad in [
            &b"{}"[..],
            b"[]",
            br#"{"methodResponses":{}}"#,
            br#"{"methodResponses":[["x",{},"c0","extra"]]}"#,
            br#"{"methodResponses":[[1,{},"c0"]]}"#,
            br#"{"methodResponses":[["x",[],"c0"]]}"#,
            b"\xff\xfe",
        ] {
            assert_eq!(parse_response(bad), Err(JmapError::Malformed));
        }
    }

    proptest! {
        #[test]
        fn the_parser_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse_response(&bytes);
        }

        #[test]
        fn json_shaped_noise_never_panics(s in r#"\{"methodResponses":\[(\[("[a-z/]{0,8}"|1|null),(\{\}|\[\]|"x"),("c[0-9]"|2)\],?){0,4}\]\}"#) {
            let _ = parse_response(s.as_bytes());
        }
    }
}
