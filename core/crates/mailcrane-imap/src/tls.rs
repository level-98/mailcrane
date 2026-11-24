//! TLS client configuration.
//!
//! Certificates are checked by `rustls-platform-verifier`, which uses the
//! OS trust store on Android and iOS. Tests can add one extra trusted root
//! (the testkit's); that adds trust and cannot turn checking off. There is
//! no API to skip verification.

use crate::error::ImapError;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::ClientConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) fn client_config(extra_roots: &[Vec<u8>]) -> Result<Arc<ClientConfig>, ImapError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = extra_roots
        .iter()
        .map(|der| CertificateDer::from(der.clone()));
    // The platform verifier checks the chain, the host name and validity.
    // `dangerous()` is only how rustls spells "bring your own verifier".
    #[cfg(not(target_os = "android"))]
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(roots, provider.clone())
            .map_err(|_| ImapError::TlsFailure)?;
    #[cfg(target_os = "android")]
    let verifier = {
        let _ = roots;
        rustls_platform_verifier::Verifier::new(provider.clone())
            .map_err(|_| ImapError::TlsFailure)?
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| ImapError::TlsFailure)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Runs the TLS handshake over `io`.
pub(crate) async fn handshake<S>(
    host: &str,
    extra_roots: &[Vec<u8>],
    limit: Duration,
    io: S,
) -> Result<tokio_rustls::client::TlsStream<S>, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let name = ServerName::try_from(host.to_string()).map_err(|_| ImapError::InvalidInput)?;
    let connector = tokio_rustls::TlsConnector::from(client_config(extra_roots)?);
    tokio::time::timeout(limit, connector.connect(name, io))
        .await
        .map_err(|_| ImapError::Timeout)?
        .map_err(|_| ImapError::TlsFailure)
}
