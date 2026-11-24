use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::{
    pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    ServerConfig,
};
use std::sync::Arc;

pub struct TestTls {
    pub cert_der: Vec<u8>,
    pub config: Arc<ServerConfig>,
}

pub fn generate() -> Result<TestTls, String> {
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
