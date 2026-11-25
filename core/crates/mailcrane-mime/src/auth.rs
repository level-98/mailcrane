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
