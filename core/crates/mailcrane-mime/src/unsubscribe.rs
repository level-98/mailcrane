//! `List-Unsubscribe` (RFC 2369), `List-Unsubscribe-Post` (RFC 8058) and
//! the DKIM results in `Authentication-Results` (RFC 8601) that decide
//! whether one-click unsubscribe may be offered (M4-10). The trust check
//! itself is in [`crate::auth`].

/// Longest URI accepted from the header.
const MAX_URI: usize = 2048;

/// A `mailto:` unsubscribe target (RFC 6068).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailtoTarget {
    /// The one address to write to.
    pub address: String,
    /// `subject=`, if given.
    pub subject: Option<String>,
    /// `body=`, if given.
    pub body: Option<String>,
}

/// What a message's unsubscribe headers offer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListUnsubscribe {
    /// `https:` URIs, in header order. `http:` is ignored.
    pub https: Vec<String>,
    /// `mailto:` targets, in header order.
    pub mailto: Vec<MailtoTarget>,
    /// `List-Unsubscribe-Post: List-Unsubscribe=One-Click` is present and
    /// there is an `https:` URI to post to (RFC 8058 §3.1).
    pub one_click: bool,
}

impl ListUnsubscribe {
    /// Whether there is anything to offer.
    pub fn is_empty(&self) -> bool {
        self.https.is_empty() && self.mailto.is_empty()
    }
}

/// Parses `List-Unsubscribe` and `List-Unsubscribe-Post`. Never fails:
/// anything that isn't a usable `https:` or `mailto:` URI is skipped.
pub fn parse_list_unsubscribe(header: &str, post: Option<&str>) -> ListUnsubscribe {
    let mut out = ListUnsubscribe::default();
    // RFC 2369 §2: angle-bracketed URIs, comma-separated; whitespace
    // (including folding) inside the brackets is ignored.
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else { break };
        let uri: String = after[..end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        rest = &after[end + 1..];
        if uri.is_empty() || uri.len() > MAX_URI || uri.chars().any(char::is_control) {
            continue;
        }
        let lower = uri.to_ascii_lowercase();
        if lower.starts_with("https://") {
            if is_plausible_https(&uri) {
                out.https.push(uri);
            }
        } else if lower.starts_with("mailto:") {
            if let Some(target) = parse_mailto(&uri[7..]) {
                out.mailto.push(target);
            }
        }
    }
    let post_ok = post.is_some_and(|p| {
        p.split_whitespace()
            .collect::<String>()
            .eq_ignore_ascii_case("List-Unsubscribe=One-Click")
    });
    out.one_click = post_ok && !out.https.is_empty();
    out
}

fn is_plausible_https(uri: &str) -> bool {
    let rest = &uri[8..];
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..host_end];
    !authority.is_empty() && !authority.contains('@') && !uri.contains(['"', '<', '>', '\\'])
}

fn parse_mailto(rest: &str) -> Option<MailtoTarget> {
    let (addr, query) = match rest.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (rest, None),
    };
    let address = percent_decode(addr)?;
    let address = address.trim();
    // One plain address only: no lists, no display names.
    let (local, domain) = address.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || address.contains([',', ';', ' ', '<', '>', '"', '(', ')'])
    {
        return None;
    }
    let mut target = MailtoTarget {
        address: address.to_owned(),
        subject: None,
        body: None,
    };
    for pair in query.unwrap_or_default().split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = percent_decode(value).map(|v| clean(&v));
        match key.to_ascii_lowercase().as_str() {
            "subject" => target.subject = value.filter(|v| !v.is_empty()),
            "body" => target.body = value.filter(|v| !v.is_empty()),
            _ => {}
        }
    }
    Some(target)
}

/// Drops control characters other than line breaks, and caps the length.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(1000)
        .collect()
}

/// Percent-decodes (RFC 3986 §2.1). `None` for bytes that aren't UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

// The trust helpers live in `auth` now, shared with the sender check (M5-07).
pub use crate::auth::{dkim_aligned, dkim_pass_domains, org_domain, provider_results, received_by};

#[cfg(test)]
mod tests;
