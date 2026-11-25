//! The JMAP session (RFC 8620 §2): where to send requests and which account
//! holds the mail.

use crate::http::{get_following, Auth, ClientConfig};
use crate::{JmapError, CAP_CORE, CAP_MAIL};
use serde_json::Value;

/// What the server says about itself and the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Where method calls go (RFC 8620 §3.1).
    pub api_url: url::Url,
    /// Template for downloading blobs (RFC 8620 §6.2).
    pub download_url: String,
    /// Where blobs are uploaded (RFC 8620 §6.1).
    pub upload_url: String,
    pub username: String,
    /// The account with the mail capability (`primaryAccounts`).
    pub mail_account_id: String,
    /// Changes when the session does.
    pub state: String,
}

/// The session URL for what the user typed: a domain becomes
/// `https://<domain>/.well-known/jmap` (RFC 8620 §2.2), and so does an
/// `https:` URL without a path; an `https:` URL with a path is used as
/// given.
pub fn session_url(input: &str) -> Result<url::Url, JmapError> {
    let t = input.trim();
    if t.to_ascii_lowercase().starts_with("https://") {
        let mut url = url::Url::parse(t).map_err(|_| JmapError::InvalidUrl)?;
        if url.path() == "/" && url.query().is_none() {
            url.set_path("/.well-known/jmap");
        }
        return Ok(url);
    }
    if t.contains("://") || t.is_empty() || t.contains(['/', '?', '#', '@', ' ']) {
        return Err(JmapError::InvalidUrl);
    }
    url::Url::parse(&format!("https://{t}/.well-known/jmap")).map_err(|_| JmapError::InvalidUrl)
}

/// Fetches and checks the session. Redirects from the well-known URL are
/// followed (to `https:` only).
pub async fn discover(
    input: &str,
    auth: &Auth,
    config: &ClientConfig,
) -> Result<Session, JmapError> {
    let url = session_url(input)?;
    let (final_url, body) = get_following(&url, auth, config).await?;
    parse_session(&body, &final_url)
}

/// Parses a session object. Relative URLs are resolved against `base`;
/// every URL must be `https:`.
pub fn parse_session(body: &[u8], base: &url::Url) -> Result<Session, JmapError> {
    let v: Value = serde_json::from_slice(body).map_err(|_| JmapError::Malformed)?;
    let caps = v.get("capabilities").and_then(Value::as_object);
    if !caps.is_some_and(|c| c.contains_key(CAP_CORE) && c.contains_key(CAP_MAIL)) {
        return Err(JmapError::NotJmap);
    }
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let api = text("apiUrl").ok_or(JmapError::Malformed)?;
    let api_url = base.join(&api).map_err(|_| JmapError::InvalidUrl)?;
    if api_url.scheme() != "https" {
        return Err(JmapError::InvalidUrl);
    }
    let mail_account_id = v
        .get("primaryAccounts")
        .and_then(|p| p.get(CAP_MAIL))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(JmapError::NotJmap)?;
    Ok(Session {
        api_url,
        download_url: text("downloadUrl").unwrap_or_default(),
        upload_url: text("uploadUrl").unwrap_or_default(),
        username: text("username").unwrap_or_default(),
        mail_account_id,
        state: text("state").unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_urls() {
        assert_eq!(
            session_url("https://jmap.example.org").map(|u| u.to_string()),
            Ok("https://jmap.example.org/.well-known/jmap".to_owned())
        );
        assert_eq!(
            session_url("fastmail.com").map(|u| u.to_string()),
            Ok("https://fastmail.com/.well-known/jmap".to_owned())
        );
        assert_eq!(
            session_url(" https://api.example.org/jmap/session ").map(|u| u.to_string()),
            Ok("https://api.example.org/jmap/session".to_owned())
        );
        for bad in [
            "",
            "http://example.org",
            "ftp://x",
            "a b",
            "user@example.org",
            "x/y",
        ] {
            assert_eq!(session_url(bad), Err(JmapError::InvalidUrl), "{bad}");
        }
    }

    #[test]
    fn sessions_parse_and_are_checked() {
        let base = url::Url::parse("https://mail.example.org/.well-known/jmap").expect("url");
        let good =
            br#"{"capabilities":{"urn:ietf:params:jmap:core":{},"urn:ietf:params:jmap:mail":{}},
            "apiUrl":"/jmap/api/","downloadUrl":"https://mail.example.org/d/{accountId}/{blobId}",
            "uploadUrl":"/u/","username":"alice@example.org",
            "primaryAccounts":{"urn:ietf:params:jmap:mail":"A1"},"state":"s1"}"#;
        let s = parse_session(good, &base).expect("session");
        assert_eq!(s.api_url.as_str(), "https://mail.example.org/jmap/api/");
        assert_eq!(s.mail_account_id, "A1");
        assert_eq!(s.username, "alice@example.org");

        let no_mail = br#"{"capabilities":{"urn:ietf:params:jmap:core":{}},"apiUrl":"/a"}"#;
        assert_eq!(parse_session(no_mail, &base), Err(JmapError::NotJmap));
        let http_api = br#"{"capabilities":{"urn:ietf:params:jmap:core":{},"urn:ietf:params:jmap:mail":{}},
            "apiUrl":"http://mail.example.org/api","primaryAccounts":{"urn:ietf:params:jmap:mail":"A1"}}"#;
        assert_eq!(parse_session(http_api, &base), Err(JmapError::InvalidUrl));
        assert_eq!(parse_session(b"not json", &base), Err(JmapError::Malformed));
    }
}
