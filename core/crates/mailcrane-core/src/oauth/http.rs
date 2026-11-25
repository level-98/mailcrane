//! POSTing a form to a token endpoint over HTTPS (RFC 6749 §3.2): the
//! core's own TLS client with platform certificate checks, a size limit and
//! a timeout. No cookies, no `User-Agent`.

use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;

use super::OAuthError;

/// Token responses are small; anything bigger is refused.
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// What came back.
pub(crate) struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Percent-encodes for `application/x-www-form-urlencoded`.
pub(crate) fn form(fields: &[(&str, &str)]) -> String {
    let mut out = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in fields {
        out.append_pair(k, v);
    }
    out.finish()
}

pub(crate) async fn post_form(
    url: &url::Url,
    body: String,
    extra_roots: &[Vec<u8>],
    timeout: Duration,
) -> Result<Reply, OAuthError> {
    tokio::time::timeout(timeout, post_inner(url, body, extra_roots))
        .await
        .map_err(|_| OAuthError::Network)?
}

async fn post_inner(
    url: &url::Url,
    body: String,
    extra_roots: &[Vec<u8>],
) -> Result<Reply, OAuthError> {
    if url.scheme() != "https" {
        return Err(OAuthError::InvalidProvider);
    }
    let host = url.host_str().ok_or(OAuthError::InvalidProvider)?;
    let port = url
        .port_or_known_default()
        .ok_or(OAuthError::InvalidProvider)?;
    let bare = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let name = ServerName::try_from(bare.clone()).map_err(|_| OAuthError::InvalidProvider)?;
    let tcp = TcpStream::connect((bare.as_str(), port))
        .await
        .map_err(|_| OAuthError::Network)?;
    let config =
        crate::account::autoconfig::fetch::tls_config(extra_roots).map_err(|_| OAuthError::Tls)?;
    let tls = tokio_rustls::TlsConnector::from(config)
        .connect(name, tcp)
        .await
        .map_err(|_| OAuthError::Tls)?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|_| OAuthError::Network)?;
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
    let request = Request::post(target)
        .header(hyper::header::HOST, host_header)
        .header(
            hyper::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(hyper::header::ACCEPT, "application/json")
        .body(Full::new(Bytes::from(body)))
        .map_err(|_| OAuthError::Network)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| OAuthError::Network)?;
    let status = response.status().as_u16();
    let body = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
        .collect()
        .await
        .map_err(|_| OAuthError::Malformed)?
        .to_bytes()
        .to_vec();
    driver.abort();
    Ok(Reply { status, body })
}
