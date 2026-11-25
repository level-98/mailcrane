//! The fallback for domains whose mail a big provider handles: look up MX,
//! take the most preferred exchange's organisation domain (the last two
//! labels, for example `aspmx.l.google.com` → `google.com`), and try that
//! domain's autoconfig file, then the bundled ISPDB for it.
//!
//! Only the domain is involved, never the address, and only the provider
//! that already receives the user's mail is asked.

use super::dns::{parse_mx, RecordType};
use super::{
    ispdb, valid_host, wellknown, DiscoveredConfig, DnsResolver, Email, HttpFetcher, Source,
};

/// The last two labels of `host`, or three under a country's second-level
/// domain such as `co.uk` or `com.au` (a short label under a two-letter
/// TLD). Without the Public Suffix List this is a heuristic; a wrong guess
/// only means a file that isn't there.
pub fn org_domain(host: &str) -> Option<String> {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    let n = labels.len();
    if n < 2 {
        return None;
    }
    let tld = labels.get(n - 1)?;
    let sld = labels.get(n - 2)?;
    let take = if n >= 3 && tld.len() == 2 && sld.len() <= 3 {
        3
    } else {
        2
    };
    let d = labels.get(n - take..)?.join(".").to_ascii_lowercase();
    valid_host(&d).then_some(d)
}

pub async fn lookup<F: HttpFetcher, R: DnsResolver>(
    email: &Email,
    http: &F,
    dns: &R,
) -> Option<DiscoveredConfig> {
    let answer = dns.query(&email.domain, RecordType::Mx).await?;
    let exchange = parse_mx(&answer)?.into_iter().next()?.exchange;
    let domain = org_domain(&exchange)?;
    if domain == email.domain {
        // Already tried as the user's own domain.
        return None;
    }
    match wellknown::lookup(&domain, email, Source::MxAutoconfig, http).await {
        Some(found) => Some(found),
        None => ispdb::lookup(&domain, email),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organisation_domains() {
        assert_eq!(
            org_domain("aspmx.l.google.com").as_deref(),
            Some("google.com")
        );
        assert_eq!(
            org_domain("in1-smtp.messagingengine.com.").as_deref(),
            Some("messagingengine.com")
        );
        assert_eq!(
            org_domain("mx1.provider.co.uk").as_deref(),
            Some("provider.co.uk")
        );
        assert_eq!(org_domain("mx.example.de").as_deref(), Some("example.de"));
        assert_eq!(org_domain("localhost"), None);
        assert_eq!(org_domain("bad_label.example"), None);
    }
}
