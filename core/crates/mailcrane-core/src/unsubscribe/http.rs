//! The one RFC 8058 request: `POST` with `List-Unsubscribe=One-Click`.
//!
//! Written by hand on purpose, so nothing else goes out: no cookies, no
//! `Referer`, no `User-Agent` or other identifying headers, redirects are
//! not followed, and only the status line is read.

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::ClientConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::UnsubscribeError;

/// The whole request must finish in this time (M4-10).
pub(crate) const TIMEOUT: Duration = Duration::from_secs(15);

/// The body RFC 8058 §3.1 requires.
pub(crate) const BODY: &str = "List-Unsubscribe=One-Click";

/// Most bytes read while looking for the status line.
const MAX_STATUS_LINE: usize = 8 * 1024;

fn client_config(extra_roots: &[Vec<u8>]) -> Result<Arc<ClientConfig>, UnsubscribeError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = extra_roots
        .iter()
        .map(|der| CertificateDer::from(der.clone()));
    #[cfg(not(target_os = "android"))]
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(roots, provider.clone())
            .map_err(|_| UnsubscribeError::Tls)?;
    #[cfg(target_os = "android")]
    let verifier = {
        let _ = roots;
        rustls_platform_verifier::Verifier::new(provider.clone())
            .map_err(|_| UnsubscribeError::Tls)?
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| UnsubscribeError::Tls)?
        // Bring-your-own verifier; the platform verifier checks everything.
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// The request bytes for `url` (already checked to be `https`).
pub(crate) fn request(url: &url::Url) -> Option<String> {
    let host = url.host_str()?;
    let default_port = url.port().is_none();
    let host_header = if default_port {
        host.to_owned()
    } else {
        format!("{host}:{}", url.port()?)
    };
    let mut target = url.path().to_owned();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    // `url` percent-encodes; refuse anything that could break the request.
    if target.is_empty()
        || target.contains(char::is_whitespace)
        || host_header.contains(char::is_whitespace)
    {
        return None;
    }
    Some(format!(
        "POST {target} HTTP/1.1\r\nHost: {host_header}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
        BODY.len()
    ))
}

/// Posts the one-click body to `url` and returns the HTTP status.
pub(crate) async fn post(url: &url::Url, extra_roots: &[Vec<u8>]) -> Result<u16, UnsubscribeError> {
    tokio::time::timeout(TIMEOUT, post_inner(url, extra_roots))
        .await
        .map_err(|_| UnsubscribeError::Timeout)?
}

async fn post_inner(url: &url::Url, extra_roots: &[Vec<u8>]) -> Result<u16, UnsubscribeError> {
    if url.scheme() != "https" {
        return Err(UnsubscribeError::NotOffered);
    }
    let host = url.host_str().ok_or(UnsubscribeError::NotOffered)?;
    let port = url
        .port_or_known_default()
        .ok_or(UnsubscribeError::NotOffered)?;
    let request = request(url).ok_or(UnsubscribeError::NotOffered)?;
    // IPv6 literals come bracketed from `url`; the TLS name and the socket
    // want them bare.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let name = ServerName::try_from(bare.to_owned()).map_err(|_| UnsubscribeError::NotOffered)?;
    let tcp = TcpStream::connect((bare, port))
        .await
        .map_err(|_| UnsubscribeError::Network)?;
    let connector = tokio_rustls::TlsConnector::from(client_config(extra_roots)?);
    let mut tls = connector
        .connect(name, tcp)
        .await
        .map_err(|_| UnsubscribeError::Tls)?;
    tls.write_all(request.as_bytes())
        .await
        .map_err(|_| UnsubscribeError::Network)?;
    tls.flush().await.map_err(|_| UnsubscribeError::Network)?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(2).any(|w| w == b"\r\n") {
        let n = tls
            .read(&mut chunk)
            .await
            .map_err(|_| UnsubscribeError::Network)?;
        if n == 0 || buf.len() > MAX_STATUS_LINE {
            return Err(UnsubscribeError::Network);
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
    }
    parse_status(&buf).ok_or(UnsubscribeError::Network)
}

/// The status code from `HTTP/1.x <code> ...`.
pub(crate) fn parse_status(response: &[u8]) -> Option<u16> {
    let text = std::str::from_utf8(response.get(..response.len().min(MAX_STATUS_LINE))?).ok()?;
    let line = text.split("\r\n").next()?;
    let mut parts = line.split(' ');
    let version = parts.next()?;
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let code = parts.next()?;
    if code.len() != 3 {
        return None;
    }
    code.parse().ok().filter(|c| (100..600).contains(c))
}
