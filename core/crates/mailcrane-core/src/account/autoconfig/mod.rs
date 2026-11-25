//! Automatic account setup (M1-15): finding server settings from the email
//! address alone, for the user to confirm before [`check_account`] logs in.
//!
//! Sources, most trusted first:
//! 1. the provider's own autoconfig file at its well-known URLs
//!    ([`wellknown`]);
//! 2. DNS SRV records (RFC 6186, RFC 8314) ([`srv`]);
//! 3. if neither finds anything, the bundled snapshot of Mozilla's ISPDB
//!    ([`ispdb`], ADR 0003), looked up on the device;
//! 4. if that has nothing either, the domain that handles the user's mail
//!    according to MX ([`mx`]): its autoconfig file, then the ISPDB, as for
//!    domains hosted by a big provider.
//!
//! Rules: HTTPS only, never `http:`; plain-text servers are never offered;
//! the email address is never sent anywhere (placeholders are filled in
//! locally); each request has [`REQUEST_TIMEOUT`] and the whole lookup
//! [`TOTAL_BUDGET`].
//!
//! [`check_account`]: crate::account::setup::check_account

pub mod dns;
pub mod fetch;
pub mod ispdb;
mod ispdb_index;
pub mod mx;
pub mod srv;
pub mod wellknown;
pub mod xml;

#[cfg(test)]
mod tests;

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::account::setup::ServerConfig;
pub use dns::RecordType;
pub use fetch::HttpsFetcher;

/// Longest one HTTP request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest the whole lookup may take; what was found by then is returned.
pub const TOTAL_BUDGET: Duration = Duration::from_secs(20);

/// Where a configuration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// The provider's autoconfig file on the user's own domain.
    Autoconfig,
    /// DNS SRV records for the user's domain.
    Srv,
    /// The autoconfig file of the domain the MX records point at.
    MxAutoconfig,
    /// The bundled ISPDB snapshot (ADR 0003), for the user's domain or the
    /// MX's domain.
    Ispdb,
}

/// Settings found for the user to confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredConfig {
    pub source: Source,
    /// The domain the settings came from, to show the user.
    pub source_domain: String,
    /// The provider's name, if the file gave one.
    pub provider_name: Option<String>,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// The login name (the same for both servers, as setup expects).
    pub username: String,
}

/// Why nothing could be offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    #[error("not an email address")]
    InvalidEmail,
    #[error("no settings found")]
    NotFound,
    #[error("timed out")]
    Timeout,
}

/// Why one request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("only https is allowed")]
    NotHttps,
    #[error("not found")]
    NotFound,
    #[error("network error")]
    Network,
    #[error("certificate or TLS error")]
    Tls,
    #[error("too large")]
    TooLarge,
    #[error("timed out")]
    Timeout,
}

/// Fetches a URL. Discovery only ever passes `https:` URLs.
pub trait HttpFetcher: Sync {
    fn get(&self, url: &url::Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send;
}

/// Asks the platform's resolver for records and returns the raw DNS
/// response message, or `None` if the lookup failed or isn't available.
pub trait DnsResolver: Sync {
    fn query(&self, name: &str, rtype: RecordType) -> impl Future<Output = Option<Vec<u8>>> + Send;
}

/// A resolver that never answers, for platforms without raw DNS queries.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDns;

impl DnsResolver for NoDns {
    fn query(&self, _: &str, _: RecordType) -> impl Future<Output = Option<Vec<u8>>> + Send {
        std::future::ready(None)
    }
}

/// An address split into its parts, with the domain lowercase and in ASCII
/// (IDNA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    pub address: String,
    pub local: String,
    pub domain: String,
}

impl Email {
    pub fn parse(input: &str) -> Option<Email> {
        let t = input.trim();
        let (local, domain) = t.rsplit_once('@')?;
        if local.is_empty()
            || local.len() > 64
            || local
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '@')
        {
            return None;
        }
        let domain = match url::Host::parse(domain).ok()? {
            url::Host::Domain(d) => d.to_ascii_lowercase(),
            _ => return None,
        };
        if !valid_host(&domain) || !domain.contains('.') {
            return None;
        }
        Some(Email {
            address: format!("{local}@{domain}"),
            local: local.to_owned(),
            domain,
        })
    }
}

/// A DNS host name: letters, digits, `-` and `.`, labels of 1–63, at most
/// 253 in all.
pub fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Time left before `deadline`, or `None` if it has passed.
fn left(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
}

/// Finds settings with the real HTTPS fetcher and the given resolver.
pub async fn discover<R: DnsResolver>(
    email: &str,
    dns: &R,
) -> Result<Vec<DiscoveredConfig>, DiscoveryError> {
    discover_with(email, &HttpsFetcher::default(), dns).await
}

/// Finds settings for `email`, most trusted first, without duplicates.
pub async fn discover_with<F: HttpFetcher, R: DnsResolver>(
    email: &str,
    http: &F,
    dns: &R,
) -> Result<Vec<DiscoveredConfig>, DiscoveryError> {
    let email = Email::parse(email).ok_or(DiscoveryError::InvalidEmail)?;
    let deadline = Instant::now() + TOTAL_BUDGET;
    let mut found: Vec<DiscoveredConfig> = Vec::new();
    let mut timed_out = false;
    let push = |found: &mut Vec<DiscoveredConfig>, c: DiscoveredConfig| {
        let same =
            |a: &DiscoveredConfig| a.imap == c.imap && a.smtp == c.smtp && a.username == c.username;
        if !found.iter().any(same) {
            found.push(c);
        }
    };

    // 1 and 2 run together; each gets what's left of the budget.
    let well_known = wellknown::lookup(&email.domain, &email, Source::Autoconfig, http);
    let srv = srv::lookup(&email, dns);
    let remaining = left(deadline).unwrap_or_default();
    let (a, b) = tokio::join!(
        tokio::time::timeout(remaining, well_known),
        tokio::time::timeout(remaining, srv)
    );
    for result in [a, b] {
        match result {
            Ok(Some(c)) => push(&mut found, c),
            Ok(None) => {}
            Err(_) => timed_out = true,
        }
    }

    // 3, on the device, only when the provider's own sources found nothing.
    if found.is_empty() {
        if let Some(c) = ispdb::lookup(&email.domain, &email) {
            push(&mut found, c);
        }
    }

    // 4 only when nothing else worked.
    if found.is_empty() {
        if let Some(remaining) = left(deadline) {
            match tokio::time::timeout(remaining, mx::lookup(&email, http, dns)).await {
                Ok(Some(c)) => push(&mut found, c),
                Ok(None) => {}
                Err(_) => timed_out = true,
            }
        } else {
            timed_out = true;
        }
    }

    match (found.is_empty(), timed_out) {
        (false, _) => Ok(found),
        (true, true) => Err(DiscoveryError::Timeout),
        (true, false) => Err(DiscoveryError::NotFound),
    }
}
