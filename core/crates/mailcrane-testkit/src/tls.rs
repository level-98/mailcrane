use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::{
    pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    ServerConfig,
};
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct TestTls {
    pub cert_der: Vec<u8>,
    pub config: Arc<ServerConfig>,
}

/// The self-signed `localhost` certificate every fake server uses.
///
/// Made once per test process and shared. Generating a key pair is CPU
/// work that can take well over 100 ms on a busy or throttled machine,
/// which used to fail the servers' old start-time check at random when
/// many tests started servers at once. Clients still trust each server
/// only through its `root_cert_der()`, so sharing changes nothing for them.
pub fn generate() -> Result<TestTls, String> {
    static SHARED: OnceLock<Result<TestTls, String>> = OnceLock::new();
    SHARED.get_or_init(make).clone()
}

fn make() -> Result<TestTls, String> {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".into()]).map_err(|error| error.to_string())?;
    let cert_der = cert.der().to_vec();
    let key_der = signing_key.serialize_der();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert_der.clone())],
            PrivatePkcs8KeyDer::from(key_der).into(),
        )
        .map_err(|error| error.to_string())?;
    Ok(TestTls {
        cert_der,
        config: Arc::new(config),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_certificate_is_made_once_and_shared() {
        let first = generate().expect("cert");
        let second = generate().expect("cert");
        assert_eq!(first.cert_der, second.cert_der);
        assert!(Arc::ptr_eq(&first.config, &second.config));
    }
}
