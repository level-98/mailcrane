//! The provider's own autoconfig file: first
//! `https://autoconfig.<domain>/mail/config-v1.1.xml`, then
//! `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`.
//!
//! Thunderbird adds `?emailaddress=` to these; we don't, so the address
//! never leaves the device. Placeholders are filled in locally instead.

use super::xml::{self, ClientConfig};
use super::{DiscoveredConfig, Email, HttpFetcher, Source};

/// The two URLs for `domain`, in order. `None` if `domain` isn't a host.
pub fn urls(domain: &str) -> Option<[url::Url; 2]> {
    if !super::valid_host(domain) {
        return None;
    }
    Some([
        url::Url::parse(&format!("https://autoconfig.{domain}/mail/config-v1.1.xml")).ok()?,
        url::Url::parse(&format!(
            "https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"
        ))
        .ok()?,
    ])
}

/// The first IMAP and first SMTP server a document offers, as one config.
pub fn pick(c: &ClientConfig, source: Source, source_domain: &str) -> Option<DiscoveredConfig> {
    let imap = c.imap.first()?;
    // Setup uses one login for both; prefer an SMTP entry that agrees.
    let smtp = c
        .smtp
        .iter()
        .find(|s| s.username == imap.username)
        .or_else(|| c.smtp.first())?;
    Some(DiscoveredConfig {
        source,
        source_domain: source_domain.to_owned(),
        provider_name: c.provider_name.clone(),
        imap: imap.config.clone(),
        smtp: smtp.config.clone(),
        username: imap.username.clone(),
    })
}

/// Tries both URLs for `domain`; the first usable document wins.
pub async fn lookup<F: HttpFetcher>(
    domain: &str,
    email: &Email,
    source: Source,
    http: &F,
) -> Option<DiscoveredConfig> {
    for url in urls(domain)? {
        let Ok(body) = http.get(&url).await else {
            continue;
        };
        if let Some(found) = xml::parse(&body, email).and_then(|c| pick(&c, source, domain)) {
            return Some(found);
        }
    }
    None
}
