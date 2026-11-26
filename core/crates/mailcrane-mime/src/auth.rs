//! `Authentication-Results` (RFC 8601): parsing one header value into its
//! SPF, DKIM and DMARC results, and deciding which header to trust.
//!
//! Only the header the user's own provider added means anything; the
//! sender can write any `Authentication-Results` they like.
//! [`provider_results`] picks the trusted one (the same check one-click
//! unsubscribe, M4-10, uses), and [`summarize`] reads it. Neither ever
//! panics, whatever the input.

/// At most this many results are read from one header.
pub const MAX_RESULTS: usize = 64;

/// The longest domain kept from a result's properties.
const MAX_DOMAIN: usize = 253;

/// The result of one authentication method (RFC 8601 §2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResult {
    /// `pass`.
    Pass,
    /// `fail`, and `softfail` for SPF.
    Fail,
    /// `neutral`.
    Neutral,
    /// `none`, or the method isn't in the header.
    None,
    /// `temperror`.
    TempError,
    /// `permerror`.
    PermError,
    /// `policy`, or a value this parser doesn't know.
    Unknown,
}

impl AuthResult {
    fn from_word(method: &str, result: &str) -> Self {
        match result.to_ascii_lowercase().as_str() {
            "pass" => Self::Pass,
            "fail" => Self::Fail,
            "softfail" if method == "spf" => Self::Fail,
            "neutral" => Self::Neutral,
            "none" => Self::None,
            "temperror" => Self::TempError,
            "permerror" => Self::PermError,
            _ => Self::Unknown,
        }
    }
}

/// One `method=result` with its properties (RFC 8601 §2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodResult {
    /// The method, lowercase, without a `/version`: `spf`, `dkim`, `dmarc`…
    pub method: String,
    /// The result.
    pub result: AuthResult,
    /// `ptype.property` (lowercase) and value pairs, in header order, such
    /// as `("header.d", "example.net")`. Also `reason`.
    pub properties: Vec<(String, String)>,
}

impl MethodResult {
    /// The first value of `key` (lowercase, e.g. `header.d`).
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// One parsed `Authentication-Results` header value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthenticationResults {
    /// Who added the header, lowercase; empty if missing.
    pub authserv_id: String,
    /// The results, in header order, at most [`MAX_RESULTS`].
    pub results: Vec<MethodResult>,
}

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Eq,
    Semi,
}

fn flush(word: &mut String, toks: &mut Vec<Tok>) {
    if !word.is_empty() {
        toks.push(Tok::Word(std::mem::take(word)));
    }
}

/// Splits a header value into words, `=` and `;`. Comments are dropped
/// (they separate words), quoted strings become part of a word, and
/// backslash escapes are honoured in both. An unclosed comment or quote
/// runs to the end.
fn lex(text: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut word = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '(' => {
                flush(&mut word, &mut toks);
                let mut depth = 1u32;
                while depth > 0 {
                    match chars.next() {
                        None => break,
                        Some('\\') => {
                            chars.next();
                        }
                        Some('(') => depth = depth.saturating_add(1),
                        Some(')') => depth -= 1,
                        Some(_) => {}
                    }
                }
            }
            '"' => loop {
                match chars.next() {
                    None | Some('"') => break,
                    Some('\\') => {
                        if let Some(e) = chars.next() {
                            word.push(e);
                        }
                    }
                    Some(q) => word.push(q),
                }
            },
            '=' => {
                flush(&mut word, &mut toks);
                toks.push(Tok::Eq);
            }
            ';' => {
                flush(&mut word, &mut toks);
                toks.push(Tok::Semi);
            }
            c if c.is_whitespace() => flush(&mut word, &mut toks),
            c => word.push(c),
        }
    }
    flush(&mut word, &mut toks);
    toks
}

/// Parses one `Authentication-Results` header value (RFC 8601 §2.2).
/// Never fails: anything that isn't a `method=result` is skipped, and an
/// empty or garbled value gives no results.
pub fn parse(header: &str) -> AuthenticationResults {
    let toks = lex(header);
    let mut segments = toks.split(|t| *t == Tok::Semi);
    let authserv_id = match segments.next().and_then(|s| s.first()) {
        Some(Tok::Word(w)) => w.to_lowercase(),
        _ => String::new(),
    };
    let mut results = Vec::new();
    for seg in segments {
        if results.len() >= MAX_RESULTS {
            break;
        }
        let [Tok::Word(method), Tok::Eq, Tok::Word(result), rest @ ..] = seg else {
            continue;
        };
        let method = method
            .split('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if method.is_empty() {
            continue;
        }
        let mut properties = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            if let (Some(Tok::Word(k)), Some(Tok::Eq), Some(Tok::Word(v))) =
                (rest.get(i), rest.get(i + 1), rest.get(i + 2))
            {
                properties.push((k.to_ascii_lowercase(), v.clone()));
                i += 3;
            } else {
                i += 1;
            }
        }
        results.push(MethodResult {
            result: AuthResult::from_word(&method, result),
            method,
            properties,
        });
    }
    AuthenticationResults {
        authserv_id,
        results,
    }
}

/// SPF, DKIM and DMARC as read from one (trusted) header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSummary {
    /// The first `spf` result.
    pub spf: AuthResult,
    /// The domain SPF checked (`smtp.mailfrom`, else `smtp.helo`).
    pub spf_domain: Option<String>,
    /// An aligned `dkim=pass` if there is one, else the first `dkim` result.
    pub dkim: AuthResult,
    /// The signing domain of that result (`header.d`, or `header.i`'s).
    pub dkim_domain: Option<String>,
    /// Whether `dkim` is a pass whose domain aligns with the From domain.
    pub dkim_aligned: bool,
    /// The first `dmarc` result.
    pub dmarc: AuthResult,
    /// The From domain DMARC checked (`header.from`).
    pub dmarc_domain: Option<String>,
}

/// Reads SPF, DKIM and DMARC from one header value. `from_domain` is the
/// domain of the message's From address, for DKIM alignment.
pub fn summarize(header: &str, from_domain: &str) -> AuthSummary {
    let parsed = parse(header);
    let first = |m: &str| parsed.results.iter().find(|r| r.method == m);
    let spf = first("spf");
    let dmarc = first("dmarc");
    let mut dkims = parsed.results.iter().filter(|r| r.method == "dkim");
    let first_dkim = dkims.clone().next();
    let aligned = dkims.find(|r| {
        r.result == AuthResult::Pass
            && dkim_domain_of(r).is_some_and(|d| dkim_aligned(&d, from_domain))
    });
    let dkim = aligned.or(first_dkim);
    AuthSummary {
        spf: spf.map_or(AuthResult::None, |r| r.result),
        spf_domain: spf.and_then(|r| {
            r.property("smtp.mailfrom")
                .map(|v| v.rsplit_once('@').map_or(v, |(_, d)| d))
                .or_else(|| r.property("smtp.helo"))
                .and_then(clean_domain)
        }),
        dkim: dkim.map_or(AuthResult::None, |r| r.result),
        dkim_domain: dkim.and_then(dkim_domain_of),
        dkim_aligned: aligned.is_some(),
        dmarc: dmarc.map_or(AuthResult::None, |r| r.result),
        dmarc_domain: dmarc
            .and_then(|r| r.property("header.from"))
            .and_then(clean_domain),
    }
}

fn dkim_domain_of(r: &MethodResult) -> Option<String> {
    r.property("header.d")
        .or_else(|| {
            r.property("header.i")
                .and_then(|i| i.rsplit_once('@').map(|(_, d)| d))
        })
        .and_then(clean_domain)
}

/// A domain to show: lowercase, no trailing dot, only letters, digits,
/// `.`, `-` and `_`, and not too long. `None` otherwise.
fn clean_domain(value: &str) -> Option<String> {
    let d = value.trim_end_matches('.').to_lowercase();
    let ok = !d.is_empty()
        && d.len() <= MAX_DOMAIN
        && d.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'));
    ok.then_some(d)
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

/// `text` without RFC 5322 comments (`(...)`, nested, with `\` escapes).
pub fn strip_comments(text: &str) -> String {
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
pub const TWO_LABEL_SUFFIXES: &[&str] = &[
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

/// How many leading Received headers with an address-literal `by` host are
/// skipped when looking for the provider's first named hop (#398).
const MAX_LITERAL_HOPS: usize = 5;

/// Whether the `by` host of a Received header is an IP address literal:
/// `192.0.2.1`, `2001:db8::1`, `[192.0.2.1]` or `[IPv6:2001:db8::1]` (RFC
/// 5321 §4.1.3). Gmail writes its internal hops this way (#398). Parsed with
/// `std::net`, never by hand.
fn by_is_ip_literal(received: &str) -> bool {
    let text = strip_comments(received);
    let mut words = text.split_whitespace();
    while let Some(w) = words.next() {
        if w.eq_ignore_ascii_case("by") {
            let Some(host) = words.next() else {
                return false;
            };
            let host = host.trim_end_matches([';', '.']);
            let host = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host);
            let host = host
                .get(..5)
                .filter(|p| p.eq_ignore_ascii_case("ipv6:"))
                .and_then(|_| host.get(5..))
                .unwrap_or(host);
            return host.parse::<std::net::IpAddr>().is_ok();
        }
    }
    false
}

/// Of these Authentication-Results headers, the topmost whose authserv-id
/// is in the same organisation as `received_by`. This checks only the
/// authserv-id, which a sender can copy: use [`trusted_results`], which
/// also checks where the header sits.
pub fn provider_results<'a>(results: &'a [String], received_by: Option<&str>) -> Option<&'a str> {
    let provider = org_domain(received_by?);
    if !provider.contains('.') {
        return None;
    }
    results
        .iter()
        .map(String::as_str)
        .find(|r| authserv_in(r, &provider))
}

/// Whether the authserv-id of an Authentication-Results value is in the
/// organisation `provider`.
fn authserv_in(results: &str, provider: &str) -> bool {
    let id = strip_comments(results);
    let id = id.split(';').next().unwrap_or_default();
    let id = id.split_whitespace().next().unwrap_or_default();
    !id.is_empty() && org_domain(id) == provider
}

/// A trace header, as [`crate::ParsedMessage::trace`] lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceHeader {
    /// A `Received` header value (RFC 5321 §4.4).
    Received(String),
    /// An `Authentication-Results` header value (RFC 8601).
    AuthenticationResults(String),
}

/// The Authentication-Results header the user's provider added, if any.
///
/// `trace` is the message's Received and Authentication-Results headers in
/// their original order, topmost first. The provider is the organisation of
/// the `by` host of the topmost Received header. Leading hops whose `by` is
/// an address literal (at most [`MAX_LITERAL_HOPS`], as Gmail writes its
/// internal hops, #398) are skipped only in [`trusted_results_with`], when
/// the user's provider authserv-id names the first named hop's
/// organisation. A header is trusted only
/// if its authserv-id is in that organisation **and** it sits above the
/// first Received header for a hop from outside the provider, where the
/// message entered it (RFC 8601 §5). Everything below that point, the
/// sender could have written, whatever authserv-id it names. When every hop
/// is inside the provider, only headers above the bottom-most Received
/// count. Of the trusted headers, the topmost wins.
pub fn trusted_results(trace: &[TraceHeader]) -> Option<&str> {
    trusted_results_with(trace, None)
}

/// [`trusted_results`], plus the account's own provider authserv-id when the
/// user set one (M5-08). Some servers (Postfix with an OpenDMARC milter)
/// write their header directly *below* their own Received header, where a
/// sender could put one too. So that spot is trusted only for a header
/// whose authserv-id is exactly `provider_authserv_id` (compared without
/// case or a trailing dot), only in the run of Authentication-Results
/// headers right below the Received header where the message entered the
/// provider, and only when nothing above that point was trusted. The user
/// sets it knowing their provider removes such headers from incoming mail;
/// it is never guessed or learned from mail.
pub fn trusted_results_with<'a>(
    trace: &'a [TraceHeader],
    provider_authserv_id: Option<&str>,
) -> Option<&'a str> {
    // Leading hops with an address-literal `by` are the provider's own
    // internal hops (#398); the first named one gives the provider. Past the
    // cap, a literal hop has no name, and there is no provider, as before.
    let mut leading_literal = 0usize;
    let mut provider_host = None;
    for r in trace.iter().filter_map(|h| match h {
        TraceHeader::Received(r) => Some(r),
        TraceHeader::AuthenticationResults(_) => None,
    }) {
        if leading_literal < MAX_LITERAL_HOPS && by_is_ip_literal(r) {
            leading_literal = leading_literal.saturating_add(1);
            continue;
        }
        provider_host = Some(r);
        break;
    }
    let provider = org_domain(&received_by(provider_host?)?);
    if !provider.contains('.') {
        return None;
    }
    // Skipping is anchored to the user's setting (lead's decision on the
    // #442 review): if the user's own server writes every hop with a literal
    // `by`, the first named hop would be the sender's forged one, and the
    // sender's own header would be trusted. So literal hops count only when
    // the provider authserv-id is set and the first named hop is in its
    // organisation (mx.google.com for the setting mx.google.com).
    if leading_literal > 0 {
        let configured = provider_authserv_id?.trim_end_matches('.');
        if !configured.contains('.') || org_domain(configured) != provider {
            return None;
        }
    }
    let mut found = None;
    let mut confirmed = None;
    let mut skipped = 0usize;
    for (i, header) in trace.iter().enumerate() {
        match header {
            TraceHeader::AuthenticationResults(a) => {
                if found.is_none() && authserv_in(a, &provider) {
                    found = Some(a.as_str());
                }
            }
            TraceHeader::Received(r) => {
                confirmed = found;
                // The leading literal hops counted above are internal.
                // Further down, a literal `by` is not in the provider, so
                // the walk stops there: it fails closed.
                if skipped < leading_literal {
                    skipped = skipped.saturating_add(1);
                    continue;
                }
                if !internal_hop(r, &provider) {
                    let provider_wrote_it =
                        received_by(r).is_some_and(|by| org_domain(&by) == provider);
                    return confirmed.or_else(|| {
                        let id = provider_authserv_id.filter(|_| provider_wrote_it)?;
                        right_below(trace.get(i.saturating_add(1)..)?, id)
                    });
                }
            }
        }
    }
    confirmed
}

/// The first Authentication-Results header with authserv-id `id` in the run
/// of them at the start of `rest`, before any Received header.
fn right_below<'a>(rest: &'a [TraceHeader], id: &str) -> Option<&'a str> {
    let want = id.trim_end_matches('.');
    if want.is_empty() {
        return None;
    }
    rest.iter()
        .map_while(|h| match h {
            TraceHeader::AuthenticationResults(a) => Some(a.as_str()),
            TraceHeader::Received(_) => None,
        })
        .find(|a| {
            let first = strip_comments(a);
            let first = first.split(';').next().unwrap_or_default();
            let authserv = first.split_whitespace().next().unwrap_or_default();
            authserv.trim_end_matches('.').eq_ignore_ascii_case(want)
        })
}

/// Whether a Received header records a hop between the provider's own
/// servers. The `by` host must be in `provider`. Then either there is no
/// `from` clause (a local hand-over), the connection came from a loopback
/// address, or both the name the client gave (HELO) and the name the
/// provider looked up for its address are in `provider`. The HELO name
/// alone is chosen by the client, so it is never enough.
fn internal_hop(received: &str, provider: &str) -> bool {
    let in_org = |host: &str| {
        let h = host
            .trim_matches(|c| c == '[' || c == ']')
            .trim_end_matches('.');
        h.contains('.') && org_domain(h) == provider
    };
    if !received_by(received).is_some_and(|by| in_org(&by)) {
        return false;
    }
    let Some((helo, info)) = from_clause(received) else {
        return true;
    };
    let Some(info) = info else {
        return false;
    };
    let ip = info
        .split('[')
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ip.starts_with("127.") || ip == "ipv6:::1" || ip == "::1" {
        return true;
    }
    let name = info.split(['[', ' ']).next().unwrap_or_default();
    in_org(&helo) && in_org(name)
}

/// The `from` clause of a Received header: the name the client gave, and
/// the provider's comment after it (`(name [address])`), if any. `None`
/// when there is no `from` clause before `by`.
fn from_clause(received: &str) -> Option<(String, Option<String>)> {
    let text = received.trim_start();
    let rest = text
        .get(..4)
        .filter(|w| w.eq_ignore_ascii_case("from"))
        .and_then(|_| text.get(4..))
        .filter(|r| r.starts_with(char::is_whitespace))?
        .trim_start();
    let helo_end = rest
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(rest.len());
    let helo = rest[..helo_end].to_ascii_lowercase();
    if helo.is_empty() || helo.eq_ignore_ascii_case("by") {
        return None;
    }
    let after = rest[helo_end..].trim_start();
    let info = after.strip_prefix('(').map(|inner| {
        let mut depth = 1u32;
        let mut out = String::new();
        for c in inner.chars() {
            match c {
                '(' => depth = depth.saturating_add(1),
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            out.push(c);
        }
        out.trim().to_owned()
    });
    Some((helo, info))
}

#[cfg(test)]
mod tests;
