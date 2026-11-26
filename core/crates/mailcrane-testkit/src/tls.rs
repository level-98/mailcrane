use rcgen::{
    generate_simple_self_signed, BasicConstraints, CertificateParams, CertifiedKey, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
};
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
    TestTls::from_der(cert.der().to_vec(), signing_key.serialize_der())
}

impl TestTls {
    /// A server config for one certificate and its PKCS#8 private key.
    /// `cert_der` is what clients are told to trust (`root_cert_der()`).
    pub fn from_der(cert_der: Vec<u8>, key_der: Vec<u8>) -> Result<Self, String> {
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

    /// A server config that sends the leaf and its CA, so a client that
    /// trusts the CA (for example as an Android user CA) accepts it.
    /// `cert_der` is the CA certificate.
    pub fn from_chain(dev: &DevCertificates) -> Result<Self, String> {
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![
                    CertificateDer::from(dev.leaf_der.clone()),
                    CertificateDer::from(dev.ca_der.clone()),
                ],
                PrivatePkcs8KeyDer::from(dev.leaf_key_der.clone()).into(),
            )
            .map_err(|error| error.to_string())?;
        Ok(TestTls {
            cert_der: dev.ca_der.clone(),
            config: Arc::new(config),
        })
    }
}

/// A local certificate authority and one server certificate it signed, for
/// `cargo xtask dev-mail` (#403). The CA's private key is used once to sign
/// the leaf and then dropped: it is never returned or written anywhere, so
/// nothing can sign more certificates with a CA someone has installed.
#[derive(Clone)]
pub struct DevCertificates {
    /// The CA certificate, DER. This is what an emulator installs.
    pub ca_der: Vec<u8>,
    /// The server certificate, DER, valid for the names given.
    pub leaf_der: Vec<u8>,
    /// The server certificate's private key, PKCS#8 DER.
    pub leaf_key_der: Vec<u8>,
}

impl DevCertificates {
    /// Makes a new CA and a server certificate for `names` (host names or
    /// IP addresses, such as `10.0.2.2`, the host as seen from the Android
    /// emulator).
    pub fn generate(names: &[&str]) -> Result<Self, String> {
        let error = |e: rcgen::Error| e.to_string();
        let ca_key = KeyPair::generate().map_err(error)?;
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).map_err(error)?;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca_params.distinguished_name.push(
            DnType::CommonName,
            "Mailcrane dev mail CA (local testing only)",
        );
        let ca_cert = ca_params.self_signed(&ca_key).map_err(error)?;
        let issuer = Issuer::new(ca_params, ca_key);

        let leaf_key = KeyPair::generate().map_err(error)?;
        let mut leaf_params = CertificateParams::new(
            names
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>(),
        )
        .map_err(error)?;
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "Mailcrane dev mail server");
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf_cert = leaf_params.signed_by(&leaf_key, &issuer).map_err(error)?;

        Ok(Self {
            ca_der: ca_cert.der().to_vec(),
            leaf_der: leaf_cert.der().to_vec(),
            leaf_key_der: leaf_key.serialize_der(),
        })
    }
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

    #[test]
    fn dev_certificates_make_a_server_config_that_presents_the_ca() {
        let dev = DevCertificates::generate(&["10.0.2.2", "localhost"]).expect("certs");
        assert_ne!(dev.ca_der, dev.leaf_der);
        let tls = TestTls::from_chain(&dev).expect("config");
        assert_eq!(tls.cert_der, dev.ca_der);
    }
}
