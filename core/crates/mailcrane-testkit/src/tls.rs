use rcgen::{
    generate_simple_self_signed, BasicConstraints, CertificateParams, CertifiedKey, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::{
    client::{danger::ServerCertVerifier, WebPkiServerVerifier},
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    RootCertStore, ServerConfig,
};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

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
        let not_after = rcgen::date_time_ymd(dev_expiry_year(), 1, 1);
        let ca_key = KeyPair::generate().map_err(error)?;
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).map_err(error)?;
        ca_params.not_after = not_after;
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
        leaf_params.not_after = not_after;
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

    /// Whether these certificates still work now for `name`: the key
    /// matches the server certificate, which chains to the CA, covers
    /// `name`, and hasn't expired. Saved files that are cut short, mixed
    /// from two runs or out of date fail here with a reason.
    pub fn check(&self, name: &str) -> Result<(), String> {
        TestTls::from_chain(self)?;
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(self.ca_der.clone()))
            .map_err(|error| format!("CA certificate: {error}"))?;
        let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|error| error.to_string())?;
        let server_name =
            ServerName::try_from(name.to_owned()).map_err(|error| error.to_string())?;
        verifier
            .verify_server_cert(
                &CertificateDer::from(self.leaf_der.clone()),
                &[],
                &server_name,
                &[],
                UnixTime::now(),
            )
            .map_err(|error| format!("server certificate: {error}"))?;
        Ok(())
    }
}

/// Dev certificates are valid until 1 January two years ahead, not rcgen's
/// default of 4096, so a leaked server key stops working (#404 review). The
/// CA's key is gone after signing, so when they lapse `check` fails and
/// `dev-mail` makes a new pair, and the new CA has to be installed again.
fn dev_expiry_year() -> i32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 31,556,952 s is the mean Gregorian year. Being a day out around New
    // Year only moves the expiry by a year.
    let year = 1970_u64.saturating_add(secs / 31_556_952);
    i32::try_from(year).unwrap_or(9997).min(9997) + 2
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

    /// `dev-mail` remakes saved certificates that `check` refuses (#404
    /// review): cut short, mixed from two runs, or for another name.
    #[test]
    fn check_accepts_a_fresh_pair_and_refuses_broken_ones() {
        let dev = DevCertificates::generate(&["10.0.2.2", "localhost"]).expect("certs");
        dev.check("localhost").expect("fresh pair");
        dev.check("10.0.2.2").expect("fresh pair, emulator host");
        assert!(dev.check("example.org").is_err(), "name not covered");

        let other = DevCertificates::generate(&["localhost"]).expect("certs");
        let mixed = DevCertificates {
            ca_der: other.ca_der.clone(),
            ..dev.clone()
        };
        assert!(mixed.check("localhost").is_err(), "leaf from another CA");
        let wrong_key = DevCertificates {
            leaf_key_der: other.leaf_key_der.clone(),
            ..dev.clone()
        };
        assert!(
            wrong_key.check("localhost").is_err(),
            "key from another run"
        );

        let mut cut = dev.clone();
        cut.leaf_der.truncate(cut.leaf_der.len() / 2);
        assert!(cut.check("localhost").is_err(), "half-written leaf");
        let mut empty_ca = dev.clone();
        empty_ca.ca_der.clear();
        assert!(empty_ca.check("localhost").is_err(), "empty CA file");
    }

    #[test]
    fn dev_certificates_expire_within_three_years() {
        let year = dev_expiry_year();
        assert!((2028..=2200).contains(&year), "{year}");
        let now = 1970
            + (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_secs()
                / 31_556_952) as i32;
        assert!(year - now <= 3, "{year} vs {now}");
    }
}
