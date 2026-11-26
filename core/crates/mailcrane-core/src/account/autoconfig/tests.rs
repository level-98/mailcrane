//! Lookup order and fallbacks with a fake fetcher and resolver: nothing
//! here touches the network.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use super::dns::build::{mx, response, srv};
use super::*;
use crate::account::setup::Security;

const BASIC: &[u8] = include_bytes!("../../../tests/fixtures/autoconfig/basic.xml");

#[derive(Default)]
struct FakeHttp {
    pages: HashMap<String, Result<Vec<u8>, FetchError>>,
    /// Requests to these URLs never finish.
    hang: Vec<String>,
    seen: Mutex<Vec<String>>,
}

impl FakeHttp {
    fn page(mut self, url: &str, body: &[u8]) -> Self {
        self.pages.insert(url.into(), Ok(body.to_vec()));
        self
    }
    fn hang(mut self, url: &str) -> Self {
        self.hang.push(url.into());
        self
    }
    fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("lock").clone()
    }
}

impl HttpFetcher for FakeHttp {
    fn get(&self, url: &url::Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send {
        let u = url.to_string();
        self.seen.lock().expect("lock").push(u.clone());
        let hang = self.hang.contains(&u);
        let result = if url.scheme() != "https" {
            Err(FetchError::NotHttps)
        } else {
            self.pages
                .get(&u)
                .cloned()
                .unwrap_or(Err(FetchError::NotFound))
        };
        async move {
            if hang {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            result
        }
    }
}

#[derive(Default)]
struct FakeDns {
    answers: HashMap<(String, RecordType), Vec<u8>>,
    seen: Mutex<Vec<String>>,
}

impl FakeDns {
    fn srv(mut self, name: &str, records: &[Vec<u8>]) -> Self {
        self.answers.insert(
            (name.into(), RecordType::Srv),
            response(name, RecordType::Srv, 0, records),
        );
        self
    }
    fn mx(mut self, name: &str, records: &[Vec<u8>]) -> Self {
        self.answers.insert(
            (name.into(), RecordType::Mx),
            response(name, RecordType::Mx, 0, records),
        );
        self
    }
}

impl DnsResolver for FakeDns {
    fn query(&self, name: &str, rtype: RecordType) -> impl Future<Output = Option<Vec<u8>>> + Send {
        self.seen.lock().expect("lock").push(name.to_owned());
        std::future::ready(self.answers.get(&(name.to_owned(), rtype)).cloned())
    }
}

const ADDRESS: &str = "alice.smith@example.org";

fn srv_dns() -> FakeDns {
    FakeDns::default()
        .srv(
            "_imaps._tcp.example.org",
            &[srv(0, 1, 993, "imap.srv.example.org")],
        )
        .srv(
            "_submission._tcp.example.org",
            &[srv(0, 1, 587, "smtp.srv.example.org")],
        )
}

#[tokio::test]
async fn the_autoconfig_file_alone() {
    let http =
        FakeHttp::default().page("https://autoconfig.example.org/mail/config-v1.1.xml", BASIC);
    let found = discover_with(ADDRESS, &http, &FakeDns::default())
        .await
        .expect("found");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, Source::Autoconfig);
    assert_eq!(found[0].imap.host, "imap.example.org");
    assert_eq!(found[0].provider_name.as_deref(), Some("Example Mail"));
    assert_eq!(found[0].username, ADDRESS);
}

#[tokio::test]
async fn the_well_known_path_is_the_second_try() {
    let http = FakeHttp::default().page(
        "https://example.org/.well-known/autoconfig/mail/config-v1.1.xml",
        BASIC,
    );
    let found = discover_with(ADDRESS, &http, &FakeDns::default())
        .await
        .expect("found");
    assert_eq!(found[0].source, Source::Autoconfig);
    assert_eq!(
        http.seen(),
        [
            "https://autoconfig.example.org/mail/config-v1.1.xml",
            "https://example.org/.well-known/autoconfig/mail/config-v1.1.xml"
        ]
    );
}

#[tokio::test]
async fn srv_alone_prefers_implicit_tls_and_requires_starttls_otherwise() {
    let dns = srv_dns().srv(
        "_imap._tcp.example.org",
        &[srv(0, 1, 143, "plain.example.org")],
    );
    let found = discover_with(ADDRESS, &FakeHttp::default(), &dns)
        .await
        .expect("found");
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert_eq!(c.source, Source::Srv);
    assert_eq!(
        (c.imap.host.as_str(), c.imap.port, c.imap.security),
        ("imap.srv.example.org", 993, Security::Tls)
    );
    assert_eq!(
        (c.smtp.host.as_str(), c.smtp.port, c.smtp.security),
        ("smtp.srv.example.org", 587, Security::StartTls)
    );
}

#[tokio::test]
async fn a_dot_target_means_no_service() {
    let dns = FakeDns::default()
        .srv("_imaps._tcp.example.org", &[srv(0, 0, 0, ".")])
        .srv(
            "_submissions._tcp.example.org",
            &[srv(0, 1, 465, "smtp.example.org")],
        );
    assert_eq!(
        discover_with(ADDRESS, &FakeHttp::default(), &dns).await,
        Err(DiscoveryError::NotFound)
    );
}

#[tokio::test]
async fn both_sources_are_offered_autoconfig_first() {
    let http =
        FakeHttp::default().page("https://autoconfig.example.org/mail/config-v1.1.xml", BASIC);
    let found = discover_with(ADDRESS, &http, &srv_dns())
        .await
        .expect("found");
    let sources: Vec<Source> = found.iter().map(|c| c.source).collect();
    assert_eq!(sources, [Source::Autoconfig, Source::Srv]);
}

#[tokio::test]
async fn mx_alone_finds_the_hosting_providers_file() {
    let dns = FakeDns::default().mx(
        "example.org",
        &[
            mx(10, "mx2.mailhost.example.net"),
            mx(5, "mx1.mailhost.example.net"),
        ],
    );
    let http =
        FakeHttp::default().page("https://autoconfig.example.net/mail/config-v1.1.xml", BASIC);
    let found = discover_with(ADDRESS, &http, &dns).await.expect("found");
    assert_eq!(found[0].source, Source::MxAutoconfig);
    assert_eq!(found[0].source_domain, "example.net");
    // Placeholders still use the user's own domain.
    assert_eq!(found[0].imap.host, "imap.example.org");
}

#[tokio::test]
async fn mx_is_skipped_when_something_else_worked() {
    let dns = srv_dns().mx("example.org", &[mx(5, "mx.example.net")]);
    discover_with(ADDRESS, &FakeHttp::default(), &dns)
        .await
        .expect("found");
    assert!(!dns
        .seen
        .lock()
        .expect("lock")
        .contains(&"example.org".to_owned()));
}

#[tokio::test]
async fn everything_failing_is_not_found() {
    let http = FakeHttp::default().page(
        "https://autoconfig.example.org/mail/config-v1.1.xml",
        include_bytes!("../../../tests/fixtures/autoconfig/plain_only.xml"),
    );
    assert_eq!(
        discover_with(ADDRESS, &http, &FakeDns::default()).await,
        Err(DiscoveryError::NotFound)
    );
    assert_eq!(
        discover_with("not an address", &http, &NoDns).await,
        Err(DiscoveryError::InvalidEmail)
    );
}

#[tokio::test(start_paused = true)]
async fn a_hanging_server_hits_the_budget_and_other_results_survive() {
    let http = FakeHttp::default().hang("https://autoconfig.example.org/mail/config-v1.1.xml");
    // Nothing else: a timeout.
    let started = tokio::time::Instant::now();
    assert_eq!(
        discover_with(ADDRESS, &http, &FakeDns::default()).await,
        Err(DiscoveryError::Timeout)
    );
    assert!(started.elapsed() <= TOTAL_BUDGET + Duration::from_secs(1));
    // With SRV answering, its result comes back despite the hang.
    let found = discover_with(ADDRESS, &http, &srv_dns())
        .await
        .expect("found");
    assert_eq!(found[0].source, Source::Srv);
}

#[tokio::test]
async fn no_http_and_the_address_is_never_sent() {
    let dns = FakeDns::default().mx("example.org", &[mx(5, "mx.mailhost.example.net")]);
    let http = FakeHttp::default();
    let _ = discover_with("Alice.Smith+tag@Example.ORG", &http, &dns).await;
    let urls = http.seen();
    assert_eq!(urls.len(), 4, "own domain twice, MX domain twice: {urls:?}");
    for u in &urls {
        assert!(u.starts_with("https://"), "{u}");
        assert!(!u.to_ascii_lowercase().contains("alice"), "{u}");
        assert!(!u.contains('@') && !u.contains("%40"), "{u}");
    }
    for name in dns.seen.lock().expect("lock").iter() {
        assert!(!name.contains("alice"), "{name}");
    }
}

#[test]
fn emails_parse() {
    let e = Email::parse(" Bob@Bücher.Example ").expect("idn");
    assert_eq!(e.domain, "xn--bcher-kva.example");
    assert_eq!(e.local, "Bob");
    for bad in [
        "",
        "bob",
        "@example.org",
        "bob@",
        "bob@localhost",
        "b ob@example.org",
        "bob@exa mple.org",
        "bob@[127.0.0.1]",
    ] {
        assert_eq!(Email::parse(bad), None, "{bad}");
    }
}

#[tokio::test]
async fn the_bundled_ispdb_covers_a_big_provider_with_no_network() {
    let http = FakeHttp::default();
    let found = discover_with("ana@gmail.com", &http, &FakeDns::default())
        .await
        .expect("found");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, Source::Ispdb);
    assert_eq!(found[0].source_domain, "gmail.com");
    assert_eq!(found[0].imap.host, "imap.gmail.com");
    // The provider's own files were still tried first, and nothing else.
    assert_eq!(http.seen().len(), 2);
}

#[tokio::test]
async fn the_providers_own_file_beats_the_ispdb() {
    let http = FakeHttp::default().page("https://autoconfig.gmail.com/mail/config-v1.1.xml", BASIC);
    let found = discover_with("ana@gmail.com", &http, &FakeDns::default())
        .await
        .expect("found");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, Source::Autoconfig);
}

#[tokio::test]
async fn a_hosted_domain_is_found_in_the_ispdb_through_mx() {
    let dns = FakeDns::default().mx("example.org", &[mx(1, "aspmx.l.google.com")]);
    let http = FakeHttp::default();
    let found = discover_with(ADDRESS, &http, &dns).await.expect("found");
    assert_eq!(found[0].source, Source::Ispdb);
    assert_eq!(found[0].source_domain, "google.com");
    assert_eq!(found[0].imap.host, "imap.gmail.com");
    assert_eq!(found[0].username, ADDRESS);
}

#[tokio::test]
async fn app_password_providers_are_built_in_and_looked_up_nowhere() {
    // M3-05: iCloud, Yahoo and AOL come from the preset, with no request.
    let http = FakeHttp::default();
    let dns = FakeDns::default();
    let found = discover_with("ann@icloud.com", &http, &dns)
        .await
        .expect("found");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, Source::Preset);
    assert_eq!(found[0].source_domain, "icloud.com");
    assert_eq!(found[0].provider_name.as_deref(), Some("iCloud Mail"));
    assert_eq!(found[0].imap.host, "imap.mail.me.com");
    assert_eq!(found[0].username, "ann@icloud.com");
    assert!(http.seen().is_empty());
    assert!(dns.seen.lock().expect("lock").is_empty());
}
