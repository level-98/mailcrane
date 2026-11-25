//! `List-Unsubscribe` (RFC 2369), `List-Unsubscribe-Post` (RFC 8058) and
//! the DKIM results in `Authentication-Results` (RFC 8601) that decide
//! whether one-click unsubscribe may be offered (M4-10).

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

/// The domains that passed DKIM (`dkim=pass`, from `header.d=` or the
/// domain of `header.i=`) in one `Authentication-Results` header value
/// (RFC 8601 §2.2). Comments are ignored. Lowercase.
pub fn dkim_pass_domains(header: &str) -> Vec<String> {
    let text = strip_comments(header);
    let mut parts = text.split(';');
    // The first part is the authserv-id (and an optional version).
    parts.next();
    let mut out = Vec::new();
    for resinfo in parts {
        let mut tokens = resinfo.split_whitespace();
        let Some(method) = tokens.next() else {
            continue;
        };
        let Some((name, result)) = method.split_once('=') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("dkim") || !result.eq_ignore_ascii_case("pass") {
            continue;
        }
        for token in tokens {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            let value = value.trim_matches('"');
            let domain = if key.eq_ignore_ascii_case("header.d") {
                Some(value)
            } else if key.eq_ignore_ascii_case("header.i") {
                value.rsplit_once('@').map(|(_, d)| d)
            } else {
                None
            };
            if let Some(d) = domain {
                let d = d.trim_end_matches('.').to_ascii_lowercase();
                if !d.is_empty() && !out.contains(&d) {
                    out.push(d);
                }
            }
        }
    }
    out
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0u32;
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
            if depth == 0 {
                out.push(c);
            }
            continue;
        }
        match c {
            '\\' => escaped = true,
            '(' => depth = depth.saturating_add(1),
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Public suffixes with two labels that are common in mail. Without a full
/// Public Suffix List, a domain directly under one of these is treated as
/// an organisational domain of its own.
const TWO_LABEL_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "com.au", "net.au", "org.au", "co.nz", "co.jp",
    "ne.jp", "or.jp", "com.br", "com.cn", "com.mx", "co.in", "co.za", "com.tr", "com.sg", "com.hk",
    "co.kr", "com.ar", "com.tw",
];

/// The organisational domain of `domain` (RFC 7489 §3.2), approximated
/// without the full Public Suffix List: the last two labels, or three under
/// a common two-label public suffix. Lowercase, without a trailing dot.
pub fn org_domain(domain: &str) -> String {
    let d = domain.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = d.split('.').collect();
    let n = labels.len();
    let take = if n >= 3 && TWO_LABEL_SUFFIXES.contains(&labels[n - 2..].join(".").as_str()) {
        3
    } else {
        2
    };
    labels[n.saturating_sub(take)..].join(".")
}

/// Whether a DKIM signing domain is aligned with the From domain: the same
/// organisational domain (relaxed alignment, RFC 7489 §3.1.1). A bare
/// public suffix never aligns.
pub fn dkim_aligned(dkim_domain: &str, from_domain: &str) -> bool {
    let (d, f) = (org_domain(dkim_domain), org_domain(from_domain));
    let is_suffix = |o: &str| !o.contains('.') || TWO_LABEL_SUFFIXES.contains(&o);
    !d.is_empty() && d == f && !is_suffix(&d)
}

/// The `by` host of a Received header value (RFC 5321 §4.4), lowercase.
pub fn received_by(received: &str) -> Option<String> {
    let text = strip_comments(received);
    let mut words = text.split_whitespace();
    while let Some(w) = words.next() {
        if w.eq_ignore_ascii_case("by") {
            let host = words
                .next()?
                .trim_end_matches([';', '.'])
                .to_ascii_lowercase();
            let ok = !host.is_empty()
                && host.contains('.')
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            return ok.then_some(host);
        }
    }
    None
}

/// The Authentication-Results header the user's provider added: the
/// topmost one whose authserv-id is in the same organisation as the `by`
/// host of the topmost Received header. Results the sender wrote
/// themselves (lower down, or with another authserv-id) are never trusted.
pub fn provider_results<'a>(results: &'a [String], received_by: Option<&str>) -> Option<&'a str> {
    let provider = org_domain(received_by?);
    if !provider.contains('.') {
        return None;
    }
    results.iter().map(String::as_str).find(|r| {
        let id = strip_comments(r);
        let id = id.split(';').next().unwrap_or_default();
        let id = id.split_whitespace().next().unwrap_or_default();
        !id.is_empty() && org_domain(id) == provider
    })
}

#[cfg(test)]
mod tests;
