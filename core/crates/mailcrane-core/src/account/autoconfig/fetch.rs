//! The real [`HttpFetcher`]: an HTTPS GET over rustls with the platform's
//! certificate checks.
//!
//! Only `https:` URLs, up to three redirects (to `https:` only), at most
//! [`MAX_XML_BYTES`](super::xml::MAX_XML_BYTES) of body and
//! [`REQUEST_TIMEOUT`](super::REQUEST_TIMEOUT) per request. Nothing
//! identifying is sent: no cookies, no `User-Agent`, no `Referer`.

use std::future::Future;
use std::sync::Arc;

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::net::TcpStream;

use super::xml::MAX_XML_BYTES;
use super::{FetchError, HttpFetcher, REQUEST_TIMEOUT};

/// Fetches over HTTPS with the platform verifier. Tests may add one extra
/// trusted root; on Android the system's roots are the only ones.
#[derive(Debug, Clone, Default)]
pub struct HttpsFetcher {
    pub extra_roots: Vec<Vec<u8>>,
}

/// The HTTPS client configuration the core uses: platform certificate
/// checks, which can't be turned off. Also used for OAuth token requests.
pub(crate) fn tls_config(extra_roots: &[Vec<u8>]) -> Result<Arc<rustls::ClientConfig>, FetchError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = extra_roots
        .iter()
        .map(|der| CertificateDer::from(der.clone()));
    #[cfg(not(target_os = "android"))]
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(roots, provider.clone())
            .map_err(|_| FetchError::Tls)?;
    #[cfg(target_os = "android")]
    let verifier = {
        let _ = roots;
        rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|_| FetchError::Tls)?
    };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| FetchError::Tls)?
        // Bring-your-own verifier; the platform verifier checks everything.
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

struct Reply {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

async fn get_once(url: &url::Url, extra_roots: &[Vec<u8>]) -> Result<Reply, FetchError> {
    if url.scheme() != "https" {
        return Err(FetchError::NotHttps);
    }
    let host = url.host_str().ok_or(FetchError::Network)?;
    let port = url.port_or_known_default().ok_or(FetchError::Network)?;
    let bare = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let name = ServerName::try_from(bare.clone()).map_err(|_| FetchError::Network)?;
    let tcp = TcpStream::connect((bare.as_str(), port))
        .await
        .map_err(|_| FetchError::Network)?;
    let tls = tokio_rustls::TlsConnector::from(tls_config(extra_roots)?)
        .connect(name, tcp)
        .await
        .map_err(|_| FetchError::Tls)?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|_| FetchError::Network)?;
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
    let request = Request::get(target)
        .header(hyper::header::HOST, host_header)
        .header(hyper::header::ACCEPT, "application/xml, text/xml")
        .body(Empty::<Bytes>::new())
        .map_err(|_| FetchError::Network)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| FetchError::Network)?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(hyper::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = if (200..300).contains(&status) {
        Limited::new(response.into_body(), MAX_XML_BYTES)
            .collect()
            .await
            .map_err(|e| {
                if e.downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    FetchError::TooLarge
                } else {
                    FetchError::Network
                }
            })?
            .to_bytes()
            .to_vec()
    } else {
        Vec::new()
    };
    driver.abort();
    Ok(Reply {
        status,
        location,
        body,
    })
}

async fn get_following(url: &url::Url, extra_roots: &[Vec<u8>]) -> Result<Vec<u8>, FetchError> {
    let mut current = url.clone();
    for _ in 0..=3 {
        let reply = get_once(&current, extra_roots).await?;
        match reply.status {
            200..=299 => return Ok(reply.body),
            301 | 302 | 303 | 307 | 308 => {
                let next = reply
                    .location
                    .and_then(|l| current.join(&l).ok())
                    .ok_or(FetchError::Network)?;
                if next.scheme() != "https" {
                    return Err(FetchError::NotHttps);
                }
                current = next;
            }
            404 | 410 => return Err(FetchError::NotFound),
            _ => return Err(FetchError::Network),
        }
    }
    Err(FetchError::Network)
}

impl HttpFetcher for HttpsFetcher {
    fn get(&self, url: &url::Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send {
        let url = url.clone();
        let roots = self.extra_roots.clone();
        async move {
            tokio::time::timeout(REQUEST_TIMEOUT, get_following(&url, &roots))
                .await
                .map_err(|_| FetchError::Timeout)?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_testkit::FakeHttps;

    fn trusting(server: &FakeHttps) -> HttpsFetcher {
        HttpsFetcher {
            extra_roots: vec![server.root_cert_der().to_vec()],
        }
    }

    fn url(s: &str) -> url::Url {
        url::Url::parse(s).expect("url")
    }

    #[tokio::test]
    async fn a_document_comes_back_and_nothing_identifying_is_sent() {
        let server = FakeHttps::builder()
            .body(&b"<clientConfig/>"[..])
            .start()
            .await
            .expect("server");
        let body = trusting(&server)
            .get(&url(&server.url("/mail/config-v1.1.xml")))
            .await
            .expect("body");
        assert_eq!(body, b"<clientConfig/>");
        let requests = server.requests().await;
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].target, "/mail/config-v1.1.xml");
        for h in ["user-agent", "cookie", "referer", "authorization"] {
            assert_eq!(requests[0].header(h), None, "{h}");
        }
    }

    #[tokio::test]
    async fn untrusted_http_and_missing_are_errors() {
        let server = FakeHttps::builder().start().await.expect("server");
        let target = url(&server.url("/x"));
        assert_eq!(
            HttpsFetcher::default().get(&target).await,
            Err(FetchError::Tls)
        );
        assert!(server.requests().await.is_empty(), "nothing sent");
        assert_eq!(
            trusting(&server).get(&url("http://localhost/x")).await,
            Err(FetchError::NotHttps)
        );
        let gone = FakeHttps::builder()
            .status(404)
            .start()
            .await
            .expect("server");
        assert_eq!(
            trusting(&gone).get(&url(&gone.url("/x"))).await,
            Err(FetchError::NotFound)
        );
    }

    #[tokio::test]
    async fn a_redirect_to_http_is_refused() {
        let server = FakeHttps::builder()
            .status(302)
            .location("http://localhost/mail/config-v1.1.xml")
            .start()
            .await
            .expect("server");
        assert_eq!(
            trusting(&server).get(&url(&server.url("/x"))).await,
            Err(FetchError::NotHttps)
        );
    }
}
