//! Display-name spoofing (M5-07, rule C): a From name that shows an
//! address, or a bare domain, from another organisation than the address
//! the message really came from, such as `"ceo@example.com"
//! <x@example.net>`.

use mailcrane_mime::auth::org_domain;

/// Only this much of a display name is looked at.
const MAX_NAME_CHARS: usize = 4096;

/// The longest claimed or real address put in a warning.
const MAX_SHOWN_CHARS: usize = 320;

/// A From name that claims to be someone the message isn't from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayNameWarning {
    /// The address or domain the name shows, as normalised for comparison.
    pub claimed: String,
    /// The address the message is really from.
    pub actual: String,
}

/// Rule C for one From address: `name` decoded, `email` the real address.
pub fn display_name_warning(name: &str, email: &str) -> Option<DisplayNameWarning> {
    let (_, real_domain) = email.rsplit_once('@')?;
    let real_org = org_domain(&ascii_domain(real_domain));
    if real_org.is_empty() {
        return None;
    }
    // The name as stored and, when a right-to-left override reverses part of
    // it, as it displays: `"\u{202E}moc.elpmaxe@oec"` shows as
    // `ceo@example.com` (#314). Bidi controls themselves are dropped by
    // `normalise`, so a claimed address never carries one.
    let shown = displayed_order(name);
    let claimed = claimed_elsewhere(&normalise(name), &real_org)
        .or_else(|| shown.and_then(|shown| claimed_elsewhere(&normalise(&shown), &real_org)))?;
    Some(DisplayNameWarning {
        claimed: claimed.chars().take(MAX_SHOWN_CHARS).collect(),
        actual: email.chars().take(MAX_SHOWN_CHARS).collect(),
    })
}

/// The address or bare domain from outside `real_org` that the normalised
/// `name` shows, if any.
fn claimed_elsewhere(name: &str, real_org: &str) -> Option<String> {
    for (local, domain) in address_tokens(name) {
        if org_domain(&ascii_domain(&domain)) != real_org {
            return Some(format!("{local}@{domain}"));
        }
    }
    let bare = name
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']'));
    if is_bare_domain(bare) && org_domain(&ascii_domain(bare)) != real_org {
        return Some(bare.to_owned());
    }
    None
}

/// `name` in the order it displays when it has a right-to-left override
/// (U+202E, RLO): the text under it is shown reversed, up to the matching
/// PDF (U+202C) or the end. Bidi controls are left out. `None` without an
/// override. Isolates and embeddings don't reverse Latin text, so they only
/// need removing (which `normalise` does).
fn displayed_order(name: &str) -> Option<String> {
    const RLO: char = '\u{202E}';
    const LRO: char = '\u{202D}';
    const PDF: char = '\u{202C}';
    if !name.contains(RLO) {
        return None;
    }
    let mut out = String::new();
    let mut run = String::new();
    // The overrides in force, innermost last: true for right-to-left.
    let mut overrides: Vec<bool> = Vec::new();
    let flush = |run: &mut String, out: &mut String, rtl: bool| {
        if rtl {
            out.extend(run.chars().rev());
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in name.chars().take(MAX_NAME_CHARS) {
        let rtl = overrides.last() == Some(&true);
        match c {
            RLO | LRO => {
                flush(&mut run, &mut out, rtl);
                overrides.push(c == RLO);
            }
            PDF => {
                flush(&mut run, &mut out, rtl);
                overrides.pop();
            }
            c if mailcrane_mime::is_bidi_control(c) => {}
            c => run.push(c),
        }
    }
    let rtl = overrides.last() == Some(&true);
    flush(&mut run, &mut out, rtl);
    Some(out)
}

/// NFKC-lite (rule C5): lowercase, zero-width and bidi control characters
/// removed (#314), and the fullwidth and small forms of `@` and `.` mapped
/// to ASCII.
fn normalise(name: &str) -> String {
    name.chars()
        .take(MAX_NAME_CHARS)
        .filter(|c| !matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}'))
        .filter(|c| !mailcrane_mime::is_bidi_control(*c))
        .map(|c| match c {
            '\u{FF20}' | '\u{FE6B}' => '@',
            '\u{FF0E}' | '\u{FE52}' | '\u{3002}' => '.',
            c => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_local_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            '.' | '_'
                | '%'
                | '+'
                | '-'
                | '\''
                | '!'
                | '#'
                | '$'
                | '&'
                | '*'
                | '/'
                | '='
                | '?'
                | '^'
                | '`'
                | '{'
                | '|'
                | '}'
                | '~'
        )
}

fn is_domain_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-')
}

/// Every `local@domain` in `text` (rule C2): around each `@`, the run of
/// address characters before it and domain characters after it. Quotes,
/// brackets, spaces and the like end a token, so an address inside `<…>`,
/// `(…)` or quotes is found. The domain needs a dot; any TLD counts.
fn address_tokens(text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for (at, _) in chars.iter().enumerate().filter(|(_, c)| **c == '@') {
        let before = chars.get(..at).unwrap_or_default();
        let start = before
            .iter()
            .rposition(|c| !is_local_char(*c))
            .map_or(0, |p| p.saturating_add(1));
        let local: String = before.get(start..).unwrap_or_default().iter().collect();
        let after = chars.get(at.saturating_add(1)..).unwrap_or_default();
        let domain: String = after.iter().take_while(|c| is_domain_char(**c)).collect();
        let local = local.trim_matches('.');
        let domain = domain.trim_matches(|c| c == '.' || c == '-');
        if !local.is_empty() && is_bare_domain(domain) {
            out.push((local.to_owned(), domain.to_owned()));
        }
    }
    out
}

/// `label.label…`: domain characters only, at least two labels, none empty,
/// and a last label of two or more characters that isn't all digits (so
/// `1.5` and `j.r` aren't domains).
fn is_bare_domain(text: &str) -> bool {
    let labels: Vec<&str> = text.split('.').collect();
    labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(is_domain_char))
        && labels
            .last()
            .is_some_and(|tld| tld.chars().count() >= 2 && !tld.chars().all(|c| c.is_ascii_digit()))
}

/// The ASCII (punycode) form of a domain, lowercase, so `пример.рф` and
/// `xn--e1afmkfd.xn--p1ai` compare equal. Unchanged (lowercased) if it isn't
/// a valid name.
fn ascii_domain(domain: &str) -> String {
    let d = domain.trim_end_matches('.');
    match url::Host::parse(d) {
        Ok(url::Host::Domain(ascii)) => ascii,
        _ => d.to_lowercase(),
    }
}

#[cfg(test)]
mod tests;
