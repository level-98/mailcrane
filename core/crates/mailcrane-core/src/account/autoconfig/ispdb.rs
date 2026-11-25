//! Mozilla's ISPDB, bundled (ADR 0003): settings for providers that publish
//! no autoconfig file or SRV records of their own.
//!
//! The snapshot is compiled in (`data/ispdb/`, imported with
//! `cargo xtask ispdb`), so lookups happen on the device and nothing is sent
//! to Mozilla. Each file is parsed only when its domain is asked for, with
//! the same rules as any other autoconfig file: plain-text servers are
//! dropped and the result is shown for the user to confirm.

use super::ispdb_index::{DOMAINS, FILES};
use super::wellknown::pick;
use super::{xml, DiscoveredConfig, Email, Source};

/// The bundled file that lists `domain`, if any.
pub fn file_for(domain: &str) -> Option<&'static [u8]> {
    let i = DOMAINS.binary_search_by(|(d, _)| (*d).cmp(domain)).ok()?;
    let (_, file) = DOMAINS.get(i)?;
    FILES.get(*file).copied()
}

/// Settings for `domain` from the snapshot, with placeholders filled from
/// `email`.
pub fn lookup(domain: &str, email: &Email) -> Option<DiscoveredConfig> {
    let doc = xml::parse(file_for(domain)?, email)?;
    pick(&doc, Source::Ispdb, domain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::setup::Security;

    fn email(address: &str) -> Email {
        Email::parse(address).expect("email")
    }

    #[test]
    fn the_index_is_sorted_and_every_file_parses() {
        assert!(
            DOMAINS.windows(2).all(|w| w[0].0 < w[1].0),
            "sorted, no duplicates"
        );
        assert!(DOMAINS.iter().all(|(_, f)| *f < FILES.len()));
        let e = email("someone@example.org");
        for (i, file) in FILES.iter().enumerate() {
            assert!(
                xml::parse(file, &e).is_some(),
                "bundled file {i} doesn't parse"
            );
        }
    }

    #[test]
    fn a_big_provider_is_found_without_any_network() {
        let found = lookup("gmail.com", &email("Ana@gmail.com")).expect("gmail");
        assert_eq!(found.source, Source::Ispdb);
        assert_eq!(found.imap.host, "imap.gmail.com");
        assert_eq!(found.imap.port, 993);
        assert_eq!(found.imap.security, Security::Tls);
        assert_eq!(found.smtp.host, "smtp.gmail.com");
        assert_eq!(found.username, "Ana@gmail.com");
        // The MX domain of Google-hosted mail is listed in the same file.
        assert!(file_for("google.com").is_some());
    }

    #[test]
    fn unknown_domains_and_odd_input_find_nothing() {
        assert_eq!(file_for("example.org"), None);
        assert_eq!(file_for(""), None);
        assert_eq!(file_for("GMAIL.COM"), None, "callers pass lowercase");
    }

    #[test]
    fn plain_text_servers_from_the_snapshot_are_never_offered() {
        let e = email("someone@example.org");
        for file in FILES {
            if let Some(doc) = xml::parse(file, &e) {
                assert!(doc
                    .imap
                    .iter()
                    .chain(&doc.smtp)
                    .all(|s| matches!(s.config.security, Security::Tls | Security::StartTls)));
            }
        }
    }
}
