//! Automatic account setup (M1-15), as the apps see it: settings found from
//! the email address, for the user to confirm before `add_account_manual`.
//!
//! DNS lookups are made by the platform's own resolver through
//! [`PlatformDns`], so they go to the user's configured DNS server and
//! nowhere else; the core parses the raw answers. The ISPDB is a bundled
//! snapshot (ADR 0003), so it needs no network at all.

use std::future::Future;
use std::sync::Arc;

use mailcrane_core::account::autoconfig::{
    self as core, DiscoveredConfig, DiscoveryError, DnsResolver, RecordType,
};

use crate::account::{ConnectionSecurity, ServerConfig};

/// The platform's DNS resolver (Android: `DnsResolver.rawQuery`, API 29+).
///
/// `query` may block for a few seconds; the core calls it off the async
/// threads. It returns the whole DNS response message, or `null`/`nil` if
/// the lookup failed or isn't possible on this device.
#[uniffi::export(with_foreign)]
pub trait PlatformDns: Send + Sync {
    /// `record_type` is the DNS type number: 33 for SRV, 15 for MX.
    fn query(&self, name: String, record_type: u16) -> Option<Vec<u8>>;
}

/// Where found settings came from, to show the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SettingsSource {
    /// The provider's autoconfig file on the user's own domain.
    Autoconfig,
    /// DNS SRV records for the user's domain.
    Srv,
    /// The autoconfig file of the provider that receives the domain's mail.
    MxAutoconfig,
    /// Mozilla's ISPDB, bundled in the app (ADR 0003).
    Ispdb,
    /// Built into Mailcrane for a provider that needs an app password (M3-05).
    Preset,
}

/// A provider whose accounts need an app-specific password (M3-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AppPasswordProvider {
    Icloud,
    Yahoo,
    Aol,
}

/// What setup shows for a provider that needs an app password.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppPasswordGuide {
    pub provider: AppPasswordProvider,
    /// The provider's name, such as "iCloud Mail".
    pub provider_name: String,
    /// The provider's page for making an app password, to open in the
    /// system browser. The app never fetches it.
    pub url: String,
}

/// The app-password guide for `email`'s provider, if it needs one: iCloud,
/// Yahoo or AOL, by exact domain. Works offline; nothing is sent anywhere.
#[uniffi::export]
pub fn app_password_guide(email: String) -> Option<AppPasswordGuide> {
    use mailcrane_core::account::presets;
    let preset = presets::preset_for(&email)?;
    Some(AppPasswordGuide {
        provider: match preset.provider {
            presets::AppPasswordProvider::Icloud => AppPasswordProvider::Icloud,
            presets::AppPasswordProvider::Yahoo => AppPasswordProvider::Yahoo,
            presets::AppPasswordProvider::Aol => AppPasswordProvider::Aol,
        },
        provider_name: preset.name.to_owned(),
        url: preset.app_password_url.to_owned(),
    })
}

/// One set of settings for the user to confirm.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiscoveredSettings {
    pub source: SettingsSource,
    /// The domain the settings came from.
    pub source_domain: String,
    pub provider_name: Option<String>,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    pub username: String,
}

/// Why nothing was found. The app then offers manual setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum DiscoverError {
    #[error("not an email address")]
    InvalidEmail,
    #[error("no settings found")]
    NotFound,
    #[error("timed out")]
    Timeout,
}

impl From<DiscoveryError> for DiscoverError {
    fn from(e: DiscoveryError) -> Self {
        match e {
            DiscoveryError::InvalidEmail => Self::InvalidEmail,
            DiscoveryError::NotFound => Self::NotFound,
            DiscoveryError::Timeout => Self::Timeout,
        }
    }
}

fn server(c: mailcrane_core::account::setup::ServerConfig) -> ServerConfig {
    ServerConfig {
        host: c.host,
        port: c.port,
        security: match c.security {
            mailcrane_core::account::setup::Security::Tls => ConnectionSecurity::Tls,
            mailcrane_core::account::setup::Security::StartTls => ConnectionSecurity::StartTls,
        },
    }
}

impl From<DiscoveredConfig> for DiscoveredSettings {
    fn from(c: DiscoveredConfig) -> Self {
        Self {
            source: match c.source {
                core::Source::Autoconfig => SettingsSource::Autoconfig,
                core::Source::Srv => SettingsSource::Srv,
                core::Source::MxAutoconfig => SettingsSource::MxAutoconfig,
                core::Source::Ispdb => SettingsSource::Ispdb,
                core::Source::Preset => SettingsSource::Preset,
            },
            source_domain: c.source_domain,
            provider_name: c.provider_name,
            imap: server(c.imap),
            smtp: server(c.smtp),
            username: c.username,
        }
    }
}

/// Adapts the platform's blocking resolver to the core's async trait.
pub(crate) struct ForeignDns(pub(crate) Option<Arc<dyn PlatformDns>>);

impl DnsResolver for ForeignDns {
    fn query(&self, name: &str, rtype: RecordType) -> impl Future<Output = Option<Vec<u8>>> + Send {
        let dns = self.0.clone();
        let name = name.to_owned();
        async move {
            let dns = dns?;
            tokio::task::spawn_blocking(move || dns.query(name, rtype.code()))
                .await
                .ok()
                .flatten()
                // A DNS message over UDP or TCP is at most 64 KiB.
                .filter(|m| m.len() <= 65_535)
        }
    }
}

/// Looks for settings for `email`, most trusted first, within 20 seconds.
/// `dns` is the platform's resolver, or `null`/`nil` where raw DNS queries
/// aren't available (then only the autoconfig files are tried). The
/// address itself is never sent anywhere.
#[uniffi::export(async_runtime = "tokio")]
pub async fn discover_settings(
    email: String,
    dns: Option<Arc<dyn PlatformDns>>,
) -> Result<Vec<DiscoveredSettings>, DiscoverError> {
    crate::worker::on_worker_thread(move || async move {
        let found = core::discover(&email, &ForeignDns(dns)).await?;
        Ok(found.into_iter().map(Into::into).collect())
    })
    .await
    .unwrap_or(Err(DiscoverError::Timeout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_core::account::autoconfig::{discover_with, FetchError, HttpFetcher};
    use std::sync::Mutex;

    struct NoHttp;
    impl HttpFetcher for NoHttp {
        fn get(&self, _: &url::Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send {
            std::future::ready(Err(FetchError::NotFound))
        }
    }

    /// Answers SRV for `_imaps` and `_submissions` with one record each.
    struct SrvDns(Mutex<Vec<(String, u16)>>);

    fn name(out: &mut Vec<u8>, n: &str) {
        for label in n.split('.') {
            out.push(u8::try_from(label.len()).expect("short"));
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
    }

    fn srv_answer(qname: &str, port: u16, target: &str) -> Vec<u8> {
        let mut m = vec![0, 1, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        name(&mut m, qname);
        m.extend_from_slice(&[0, 33, 0, 1, 0xC0, 12, 0, 33, 0, 1, 0, 0, 0, 60]);
        let mut rd = vec![0, 0, 0, 1];
        rd.extend_from_slice(&port.to_be_bytes());
        name(&mut rd, target);
        m.extend_from_slice(&u16::try_from(rd.len()).expect("small").to_be_bytes());
        m.extend_from_slice(&rd);
        m
    }

    impl PlatformDns for SrvDns {
        fn query(&self, name: String, record_type: u16) -> Option<Vec<u8>> {
            self.0
                .lock()
                .expect("lock")
                .push((name.clone(), record_type));
            match name.as_str() {
                "_imaps._tcp.example.org" => Some(srv_answer(&name, 993, "imap.example.org")),
                "_submissions._tcp.example.org" => Some(srv_answer(&name, 465, "smtp.example.org")),
                _ => None,
            }
        }
    }

    #[tokio::test]
    async fn the_platform_resolver_feeds_srv_discovery() {
        let platform = Arc::new(SrvDns(Mutex::new(Vec::new())));
        let dns = ForeignDns(Some(platform.clone()));
        let found = discover_with("ana@example.org", &NoHttp, &dns)
            .await
            .expect("found");
        let settings: DiscoveredSettings = found.into_iter().next().expect("one").into();
        assert_eq!(settings.source, SettingsSource::Srv);
        assert_eq!(settings.imap.host, "imap.example.org");
        assert_eq!(settings.imap.security, ConnectionSecurity::Tls);
        assert_eq!(settings.smtp.port, 465);
        assert_eq!(settings.username, "ana@example.org");
        let asked = platform.0.lock().expect("lock").clone();
        assert!(asked.iter().all(|(_, t)| *t == 33 || *t == 15));
        assert!(asked.iter().all(|(n, _)| !n.contains("ana")), "{asked:?}");
    }

    #[tokio::test]
    async fn no_resolver_and_a_bad_address() {
        assert_eq!(
            discover_settings("not an address".into(), None).await,
            Err(DiscoverError::InvalidEmail)
        );
        let none = ForeignDns(None);
        assert_eq!(none.query("example.org", RecordType::Mx).await, None);
    }
}
