//! HTTPS for JMAP: one request per connection over rustls with the platform
//! verifier, a size limit on every response, a timeout on everything, and
//! redirects followed only for session discovery (RFC 8620 §2.2).

use crate::JmapError;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;

/// Largest response accepted (RFC 8620 §2 lets servers state limits; this
/// is ours for reading).
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Most redirects followed while discovering the session.
const MAX_REDIRECTS: usize = 3;

/// How to authenticate. `Debug` never shows the secret.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    /// HTTP Basic with an app password (RFC 7617).
    Basic { username: String, password: String },
    /// A bearer token (RFC 6750).
    Bearer(String),
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .finish_non_exhaustive(),
            Self::Bearer(_) => f.write_str("Bearer(..)"),
        }
    }
}

impl Auth {
    fn header(&self) -> String {
        match self {
            Self::Basic { username, password } => {
                format!(
                    "Basic {}",
                    base64(format!("{username}:{password}").as_bytes())
                )
            }
            Self::Bearer(token) => format!("Bearer {token}"),
        }
    }
}

/// Connection settings.
#[derive(Clone)]
pub struct ClientConfig {
    /// Limit for each request, connect to last byte.
    pub timeout: Duration,
    /// Extra trusted root certificates (DER), for tests. They add trust;
    /// they can't turn checking off.
    pub extra_roots: Vec<Vec<u8>>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            extra_roots: Vec::new(),
        }
    }
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("timeout", &self.timeout)
            .field("extra_roots", &self.extra_roots.len())
            .finish()
    }
}

fn tls_config(extra_roots: &[Vec<u8>]) -> Result<Arc<rustls::ClientConfig>, JmapError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = extra_roots
        .iter()
        .map(|der| CertificateDer::from(der.clone()));
    #[cfg(not(target_os = "android"))]
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(roots, provider.clone())
            .map_err(|_| JmapError::Tls)?;
    #[cfg(target_os = "android")]
    let verifier = {
        let _ = roots;
        rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|_| JmapError::Tls)?
    };
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| JmapError::Tls)?
        // Bring-your-own verifier; the platform verifier checks everything.
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// A response: status and body.
pub(crate) struct Response {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// One request. Only `https:` URLs; the body (if any) is sent as JSON.
pub(crate) async fn send(
    method: Method,
    url: &url::Url,
    auth: Option<&Auth>,
    body: Option<(Vec<u8>, &str)>,
    config: &ClientConfig,
) -> Result<Response, JmapError> {
    tokio::time::timeout(config.timeout, send_inner(method, url, auth, body, config))
        .await
        .map_err(|_| JmapError::Timeout)?
}

async fn send_inner(
    method: Method,
    url: &url::Url,
    auth: Option<&Auth>,
    body: Option<(Vec<u8>, &str)>,
    config: &ClientConfig,
) -> Result<Response, JmapError> {
    if url.scheme() != "https" {
        return Err(JmapError::InvalidUrl);
    }
    let host = url.host_str().ok_or(JmapError::InvalidUrl)?;
    let port = url.port_or_known_default().ok_or(JmapError::InvalidUrl)?;
    let bare = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let name = ServerName::try_from(bare.clone()).map_err(|_| JmapError::InvalidUrl)?;
    let tcp = TcpStream::connect((bare.as_str(), port))
        .await
        .map_err(|_| JmapError::Network)?;
    let tls = tokio_rustls::TlsConnector::from(tls_config(&config.extra_roots)?)
        .connect(name, tcp)
        .await
        .map_err(|_| JmapError::Tls)?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|_| JmapError::Network)?;
    let driver = tokio::spawn(conn);
    let mut target = url.path().to_owned();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    let host_header = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_owned(),
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(target)
        .header(hyper::header::HOST, host_header)
        .header(hyper::header::ACCEPT, "application/json");
    if let Some(a) = auth {
        builder = builder.header(hyper::header::AUTHORIZATION, a.header());
    }
    let bytes = match body {
        Some((bytes, content_type)) => {
            builder = builder.header(hyper::header::CONTENT_TYPE, content_type);
            bytes
        }
        None => Vec::new(),
    };
    let request = builder
        .body(Full::new(Bytes::from(bytes)))
        .map_err(|_| JmapError::InvalidUrl)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| JmapError::Network)?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(hyper::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let collected = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
        .collect()
        .await
        .map_err(|e| {
            if e.downcast_ref::<http_body_util::LengthLimitError>()
                .is_some()
            {
                JmapError::TooLarge
            } else {
                JmapError::Network
            }
        })?;
    driver.abort();
    Ok(Response {
        status,
        location,
        body: collected.to_bytes().to_vec(),
    })
}

/// A GET that follows up to three redirects to other `https:` URLs. The
/// credentials go along only to the same host or a subdomain of the same
/// organisation's domain (the last two labels), never to an unrelated host.
pub(crate) async fn get_following(
    url: &url::Url,
    auth: &Auth,
    config: &ClientConfig,
) -> Result<(url::Url, Vec<u8>), JmapError> {
    let mut current = url.clone();
    let origin = org(url.host_str().unwrap_or_default());
    for _ in 0..=MAX_REDIRECTS {
        let same_org = org(current.host_str().unwrap_or_default()) == origin;
        let r = send(
            Method::GET,
            &current,
            same_org.then_some(auth),
            None,
            config,
        )
        .await?;
        match r.status {
            200..=299 => return Ok((current, r.body)),
            301 | 302 | 303 | 307 | 308 => {
                let next = r.location.ok_or(JmapError::Malformed)?;
                current = current.join(&next).map_err(|_| JmapError::InvalidUrl)?;
                if current.scheme() != "https" {
                    return Err(JmapError::InvalidUrl);
                }
            }
            401 | 403 => return Err(JmapError::Unauthorized),
            other => return Err(JmapError::Http(other)),
        }
    }
    Err(JmapError::Http(310))
}

/// The last two labels of a host (`api.fastmail.com` → `fastmail.com`).
pub(crate) fn org(host: &str) -> String {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    labels[labels.len().saturating_sub(2)..]
        .join(".")
        .to_ascii_lowercase()
}

/// Standard base64 with padding (RFC 4648 §4).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        // RFC 4648 §10.
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want);
        }
    }

    #[test]
    fn auth_never_shows_secrets() {
        let basic = Auth::Basic {
            username: "alice".into(),
            password: "hunter2".into(),
        };
        assert!(!format!("{basic:?}").contains("hunter2"));
        assert!(!format!("{:?}", Auth::Bearer("tok-123".into())).contains("tok-123"));
        assert_eq!(basic.header(), "Basic YWxpY2U6aHVudGVyMg==");
    }

    #[test]
    fn organisations() {
        assert_eq!(org("api.fastmail.com"), "fastmail.com");
        assert_eq!(org("fastmail.com."), "fastmail.com");
        assert_eq!(org("localhost"), "localhost");
    }
}
