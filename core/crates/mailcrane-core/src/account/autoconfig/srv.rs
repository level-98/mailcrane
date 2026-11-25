//! DNS SRV records for mail (RFC 6186, updated by RFC 8314):
//! `_imaps._tcp` (implicit TLS) is preferred over `_imap._tcp` (STARTTLS),
//! and `_submissions._tcp` over `_submission._tcp`.
//!
//! The STARTTLS services are only used with STARTTLS required, which setup
//! always does, so nothing is ever sent in clear text. The login name is
//! the whole address (RFC 6186 §5 lets clients try that first).

use super::dns::{parse_srv, RecordType};
use super::{valid_host, DiscoveredConfig, DnsResolver, Email, Source};
use crate::account::setup::{Security, ServerConfig};

/// The first usable target of `_service._tcp.<domain>`.
async fn first<R: DnsResolver>(
    dns: &R,
    service: &str,
    domain: &str,
    security: Security,
) -> Option<ServerConfig> {
    let name = format!("_{service}._tcp.{domain}");
    let answer = dns.query(&name, RecordType::Srv).await?;
    parse_srv(&answer)?
        .into_iter()
        // "." (empty) means the service isn't offered (RFC 2782).
        .find(|s| s.port != 0 && valid_host(&s.target))
        .map(|s| ServerConfig {
            host: s.target,
            port: u32::from(s.port),
            security,
        })
}

/// IMAP and submission servers from SRV, if both are there.
pub async fn lookup<R: DnsResolver>(email: &Email, dns: &R) -> Option<DiscoveredConfig> {
    let d = &email.domain;
    let (imaps, imap, subs, sub) = tokio::join!(
        first(dns, "imaps", d, Security::Tls),
        first(dns, "imap", d, Security::StartTls),
        first(dns, "submissions", d, Security::Tls),
        first(dns, "submission", d, Security::StartTls),
    );
    Some(DiscoveredConfig {
        source: Source::Srv,
        source_domain: d.clone(),
        provider_name: None,
        imap: imaps.or(imap)?,
        smtp: subs.or(sub)?,
        username: email.address.clone(),
    })
}
