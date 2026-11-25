//! Tracking-pixel detection (M5-01) used by the sanitiser, on top of its size
//! and visibility rules.
//!
//! The list of known services is `data/trackers.toml`, written for this
//! project and compiled in. An image matches an entry when its host is one of
//! the entry's domains or a subdomain of one, and, when the entry lists
//! `paths`, its URL path starts with one of them.
//!
//! **Content on a tracker's domain.** Some services serve real images from the
//! same domain as their pixels (Substack's `substackcdn.com`, LinkedIn's
//! profile photos). Those entries list the pixel's `paths`, so only the pixel
//! is removed; a large image from the same domain or a CDN subdomain of it is
//! kept (and still follows the remote-content setting). Domains used only for
//! tracking have no `paths`, so every image from them is removed whatever its
//! size: a tracker can claim any size it likes.

use std::sync::OnceLock;

/// Why an image was treated as a tracker.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrackerKind {
    /// The image comes from a known tracking service (named).
    KnownService(String),
    /// A tiny (1×1-style) image.
    Tiny,
    /// An image hidden with CSS.
    Hidden,
}

/// One tracker found in a message. Duplicates (same domain) are merged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackerHit {
    /// The matched list domain for a known service; otherwise the image's
    /// host. Empty when the image had no web address (a hidden `data:` image).
    pub domain: String,
    /// Why it was removed.
    pub kind: TrackerKind,
}

/// One entry of `data/trackers.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackerEntry {
    pub(crate) service: String,
    pub(crate) domains: Vec<String>,
    pub(crate) paths: Vec<String>,
    pub(crate) source: String,
}

const LIST: &str = include_str!("../data/trackers.toml");

/// The compiled-in list. A parse failure (caught by a test) gives an empty
/// list rather than a panic; the size and visibility rules still apply.
fn list() -> &'static [TrackerEntry] {
    static PARSED: OnceLock<Vec<TrackerEntry>> = OnceLock::new();
    PARSED.get_or_init(|| parse_list(LIST).unwrap_or_default())
}

/// Parses the small subset of TOML the list uses: `[[tracker]]` headers and
/// one-line `key = "string"` or `key = ["string", ...]` values. Strings have
/// no escapes. Errors name the line.
pub(crate) fn parse_list(text: &str) -> Result<Vec<TrackerEntry>, String> {
    let mut entries: Vec<TrackerEntry> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let n = index.saturating_add(1);
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[tracker]]" {
            entries.push(TrackerEntry {
                service: String::new(),
                domains: Vec::new(),
                paths: Vec::new(),
                source: String::new(),
            });
            continue;
        }
        let entry = entries
            .last_mut()
            .ok_or_else(|| format!("line {n}: value before [[tracker]]"))?;
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {n}: expected key = value"))?;
        let value = value.trim();
        match key.trim() {
            "service" => entry.service = string(value).ok_or(format!("line {n}: bad string"))?,
            "source" => entry.source = string(value).ok_or(format!("line {n}: bad string"))?,
            "domains" => entry.domains = array(value).ok_or(format!("line {n}: bad array"))?,
            "paths" => entry.paths = array(value).ok_or(format!("line {n}: bad array"))?,
            other => return Err(format!("line {n}: unknown key {other}")),
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if e.service.is_empty() || e.source.is_empty() || e.domains.is_empty() {
            return Err(format!(
                "entry {}: service, domains and source are required",
                i.saturating_add(1)
            ));
        }
        if e.domains
            .iter()
            .any(|d| d.is_empty() || d != &d.to_ascii_lowercase() || d.contains('/'))
        {
            return Err(format!(
                "entry {}: domains must be lowercase hosts",
                i.saturating_add(1)
            ));
        }
        if e.paths.iter().any(|p| !p.starts_with('/')) {
            return Err(format!(
                "entry {}: paths must start with /",
                i.saturating_add(1)
            ));
        }
    }
    Ok(entries)
}

fn string(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"') && !inner.contains('\\')).then(|| inner.to_owned())
}

fn array(value: &str) -> Option<Vec<String>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    inner.split(',').map(|item| string(item.trim())).collect()
}

/// The lowercase host and the path of an `http`/`https` URL, or `None`.
pub(crate) fn host_and_path(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest_start = if lower.starts_with("https://") {
        8
    } else if lower.starts_with("http://") {
        7
    } else if lower.starts_with("//") {
        2
    } else {
        return None;
    };
    let rest = trimmed.get(rest_start..)?;
    let authority_end = rest.find(['/', '?', '#', '\\']).unwrap_or(rest.len());
    let authority = rest.get(..authority_end)?;
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = if host_port.starts_with('[') {
        host_port.split(']').next().map(|h| format!("{h}]"))?
    } else {
        host_port.split(':').next().unwrap_or(host_port).to_owned()
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let after = rest.get(authority_end..).unwrap_or_default();
    let path_end = after.find(['?', '#']).unwrap_or(after.len());
    let path = after.get(..path_end).unwrap_or_default().replace('\\', "/");
    Some((
        host,
        if path.is_empty() {
            "/".to_owned()
        } else {
            path
        },
    ))
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// The known service an image URL belongs to, if any.
pub(crate) fn known_tracker(url: &str) -> Option<TrackerHit> {
    known_tracker_in(list(), url)
}

fn known_tracker_in(entries: &[TrackerEntry], url: &str) -> Option<TrackerHit> {
    let (host, path) = host_and_path(url)?;
    let path = path.to_ascii_lowercase();
    entries.iter().find_map(|entry| {
        let domain = entry.domains.iter().find(|d| host_matches(&host, d))?;
        let path_ok = entry.paths.is_empty()
            || entry
                .paths
                .iter()
                .any(|p| path.starts_with(&p.to_ascii_lowercase()));
        path_ok.then(|| TrackerHit {
            domain: domain.clone(),
            kind: TrackerKind::KnownService(entry.service.clone()),
        })
    })
}

/// Adds `hit` unless a hit with the same domain is already there. A known
/// service replaces an earlier size-only hit for the same domain.
pub(crate) fn push_unique(hits: &mut Vec<TrackerHit>, hit: TrackerHit) {
    match hits.iter_mut().find(|h| h.domain == hit.domain) {
        Some(existing) => {
            if matches!(hit.kind, TrackerKind::KnownService(_))
                && !matches!(existing.kind, TrackerKind::KnownService(_))
            {
                *existing = hit;
            }
        }
        None => hits.push(hit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn list_parses_with_sources_and_no_duplicate_domains() {
        let entries = parse_list(LIST).expect("trackers.toml parses");
        assert!(entries.len() >= 30, "at least 30 services");
        let mut seen = HashSet::new();
        for e in &entries {
            assert!(!e.service.is_empty() && !e.source.is_empty());
            for d in &e.domains {
                assert!(seen.insert(d.clone()), "duplicate domain {d}");
            }
        }
        let services: HashSet<_> = entries.iter().map(|e| &e.service).collect();
        assert_eq!(services.len(), entries.len(), "duplicate service");
        assert_eq!(list().len(), entries.len());
    }

    #[test]
    fn parse_errors_name_the_problem() {
        assert!(parse_list("service = \"x\"").is_err());
        assert!(parse_list("[[tracker]]\nservice = \"x\"\nsource = \"y\"").is_err());
        assert!(
            parse_list("[[tracker]]\nservice = \"x\"\nsource = \"y\"\ndomains = [\"A.com\"]")
                .is_err()
        );
        assert!(parse_list("[[tracker]]\nfoo = \"x\"").is_err());
        assert!(parse_list("[[tracker]]\nservice = x").is_err());
    }

    #[test]
    fn hosts_and_paths() {
        let hp = |u: &str| host_and_path(u);
        assert_eq!(
            hp("https://User@Sub.Example.COM.:8080/a/b?x#y"),
            Some(("sub.example.com".into(), "/a/b".into()))
        );
        assert_eq!(
            hp("http://example.com"),
            Some(("example.com".into(), "/".into()))
        );
        assert_eq!(
            hp("//cdn.example.com/x"),
            Some(("cdn.example.com".into(), "/x".into()))
        );
        assert_eq!(hp("cid:abc"), None);
        assert_eq!(hp("data:image/png;base64,AAAA"), None);
        assert_eq!(hp("https:///nohost"), None);
    }

    #[test]
    fn known_services_match_host_suffix_and_paths() {
        let hit = known_tracker("https://us5.list-manage.com/track/open.php?u=1").expect("hit");
        assert_eq!(hit.domain, "list-manage.com");
        assert_eq!(hit.kind, TrackerKind::KnownService("Mailchimp".into()));
        // A look-alike host doesn't match.
        assert!(known_tracker("https://notlist-manage.com/x.gif").is_none());
        assert!(known_tracker("https://list-manage.com.example.org/x.gif").is_none());
        // Path-only entries keep real images from the same domain.
        assert!(known_tracker("https://eotrx.substackcdn.com/open?token=1").is_some());
        assert!(known_tracker("https://substackcdn.com/image/fetch/w_1200/photo.jpg").is_none());
        assert!(known_tracker("https://www.linkedin.com/emimp/abc.gif").is_some());
        assert!(known_tracker("https://media.linkedin.com/photo.jpg").is_none());
    }

    #[test]
    fn duplicates_are_merged_by_domain() {
        let mut hits = Vec::new();
        let tiny = TrackerHit {
            domain: "list-manage.com".into(),
            kind: TrackerKind::Tiny,
        };
        push_unique(&mut hits, tiny.clone());
        push_unique(&mut hits, tiny);
        let known = known_tracker("https://a.list-manage.com/o").expect("hit");
        push_unique(&mut hits, known.clone());
        assert_eq!(hits, vec![known]);
    }
}
