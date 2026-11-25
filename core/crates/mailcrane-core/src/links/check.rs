//! Where a link really goes (M5-07, part A). Every result is a hint: the
//! user can always open the link. Nothing here touches the network.

use mailcrane_mime::auth::org_domain;
use url::{Host, Url};

/// Longest link examined in full (bytes).
const MAX_LINK_BYTES: usize = 2048;

/// Why a link deserves a second look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkWarning {
    /// Not an `http` or `https` link; never opened by the preview dialog.
    UnsupportedScheme,
    /// The host mixes scripts inside one label (a lookalike); it is shown
    /// in punycode.
    MixedScript,
    /// A label is written wholly in another alphabet with letters that look
    /// like Latin ones (all-Cyrillic `аррӏе.com`), under a Latin top-level
    /// domain; it is shown in punycode.
    LookalikeHost,
    /// The visible text names a different site than the link goes to.
    TextHostMismatch {
        /// The host the text shows (Unicode form, without `www.`).
        text_host: String,
    },
    /// The host is an IP address.
    IpAddress,
    /// The URL has `user@` before the host.
    UserInfo,
    /// The link is `http`, not `https` ("Not secure").
    Http,
    /// Longer than 2,048 bytes; the text was not compared.
    TooLong,
}

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkCheck {
    /// The host to show: Unicode when every label is in one script,
    /// otherwise the punycode form. Empty when the link has no host.
    pub display_host: String,
    /// The host in ASCII (punycode) form. Empty when the link has no host.
    pub ascii_host: String,
    /// The scheme is `http`.
    pub is_http: bool,
    /// Everything worth a warning, in a fixed order.
    pub warnings: Vec<LinkWarning>,
}

/// Checks a link. `text` is what the reader sees for it, when known.
/// Never panics.
pub fn check_link(href: &str, text: Option<&str>) -> LinkCheck {
    let mut out = LinkCheck::default();
    let too_long = href.len() > MAX_LINK_BYTES;
    if too_long {
        out.warnings.push(LinkWarning::TooLong);
    }
    let Ok(url) = Url::parse(href.trim()) else {
        out.warnings.push(LinkWarning::UnsupportedScheme);
        return out;
    };
    if !matches!(url.scheme(), "http" | "https") {
        out.warnings.push(LinkWarning::UnsupportedScheme);
        return out;
    }
    out.is_http = url.scheme() == "http";
    let host = url.host();
    let mut ip = false;
    if let Some(h) = &host {
        out.ascii_host = h.to_string();
        out.display_host = out.ascii_host.clone();
        ip = !matches!(h, Host::Domain(_));
    }
    if let Some(Host::Domain(d)) = &host {
        let unicode = url::quirks::domain_to_unicode(d);
        if !one_script_per_label(&unicode) {
            out.warnings.push(LinkWarning::MixedScript);
        } else if imitates_latin(&unicode) {
            out.warnings.push(LinkWarning::LookalikeHost);
        } else {
            out.display_host = unicode;
        }
    }
    if ip {
        out.warnings.push(LinkWarning::IpAddress);
    }
    if !url.username().is_empty() || url.password().is_some() {
        out.warnings.push(LinkWarning::UserInfo);
    }
    if out.is_http {
        out.warnings.push(LinkWarning::Http);
    }
    if !too_long {
        if let Some(text_host) = text.and_then(text_site) {
            if org_domain(&text_host.ascii) != org_domain(&out.ascii_host) {
                out.warnings.push(LinkWarning::TextHostMismatch {
                    text_host: text_host.display,
                });
            }
        }
    }
    out
}

struct TextSite {
    ascii: String,
    display: String,
}

/// File extensions that look like a top-level domain in link text.
const FILE_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "zip", "png", "jpg", "jpeg", "gif",
    "html", "htm", "csv", "rtf",
];

/// The text of the link the reader tapped, found by its href. The web view
/// reports a canonical URL (lowercase host, punycode, percent-escapes), so
/// both sides are compared in that form: the WHATWG URL serialisation, which
/// is what Chromium produces for `http` and `https`.
pub fn link_text_for<'a, I>(url: &str, links: I) -> Option<&'a str>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    if url.len() > MAX_LINK_BYTES {
        return None;
    }
    let wanted = canonical_href(url);
    links
        .into_iter()
        .find(|(href, _)| href.len() <= MAX_LINK_BYTES && canonical_href(href) == wanted)
        .map(|(_, text)| text)
}

/// `href` in canonical form when it parses, otherwise as it is.
fn canonical_href(href: &str) -> String {
    match Url::parse(href.trim()) {
        Ok(u) => u.into(),
        Err(_) => href.to_owned(),
    }
}

/// The dots other than `.` that IDNA treats as label separators (UTS #46):
/// ideographic, fullwidth and halfwidth full stops.
const IDNA_DOTS: [char; 3] = ['\u{3002}', '\u{FF0E}', '\u{FF61}'];

/// The site link text names, if it looks like a URL or a bare domain.
fn text_site(text: &str) -> Option<TextSite> {
    let t = text.trim();
    if t.is_empty() || t.len() > MAX_LINK_BYTES || t.contains(char::is_whitespace) {
        return None;
    }
    // `example\u{FF0E}com` reads as `example.com`, so treat it that way.
    let t = t.replace(IDNA_DOTS, ".");
    let t = t.as_str();
    let lower = t.to_lowercase();
    let candidate = if lower.starts_with("http://") || lower.starts_with("https://") {
        t.to_owned()
    } else {
        // `www.…` or `label.tld`, optionally with a path.
        let head = t.split(['/', '?', '#']).next().unwrap_or_default();
        let labels: Vec<&str> = head.split('.').collect();
        let tld = labels.last().copied().unwrap_or_default().to_lowercase();
        let looks_like_host = labels.len() >= 2
            && labels
                .iter()
                .all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
            && tld.chars().count() >= 2
            && tld.chars().all(char::is_alphabetic)
            && !FILE_EXTENSIONS.contains(&tld.as_str());
        if !looks_like_host {
            return None;
        }
        format!("https://{t}")
    };
    let url = Url::parse(&candidate).ok()?;
    let Some(Host::Domain(d)) = url.host() else {
        // An IP in the text: compare it as it is.
        let ip = url.host()?.to_string();
        return Some(TextSite {
            display: ip.clone(),
            ascii: ip,
        });
    };
    let ascii = d.strip_prefix("www.").unwrap_or(d).to_owned();
    let display = url::quirks::domain_to_unicode(&ascii);
    Some(TextSite { ascii, display })
}

/// The scripts we tell apart. Japanese and Korean text mixes several
/// writing systems in one word, so those share a group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Cjk,
    Arabic,
    Hebrew,
    Thai,
    Devanagari,
    Other,
}

fn script_of(c: char) -> Script {
    match u32::from(c) {
        0x41..=0x5A | 0x61..=0x7A | 0xAA | 0xBA | 0xC0..=0x24F | 0x1E00..=0x1EFF => Script::Latin,
        0x370..=0x3FF | 0x1F00..=0x1FFF => Script::Greek,
        0x400..=0x52F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Script::Cyrillic,
        0x590..=0x5FF => Script::Hebrew,
        0x600..=0x6FF | 0x750..=0x77F => Script::Arabic,
        0x900..=0x97F => Script::Devanagari,
        0xE00..=0xE7F => Script::Thai,
        0x3040..=0x30FF
        | 0x3100..=0x312F
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF
        | 0x20000..=0x2A6DF => Script::Cjk,
        _ => Script::Other,
    }
}

/// Cyrillic and Greek letters that look like a Latin letter, after IDNA
/// lowercasing: Cyrillic а е о р с у х ѕ і ј ӏ ԁ һ ԛ ԝ ѵ ү, Greek α ι κ ν ο
/// ρ υ χ ϲ ϳ. Deliberately short: letters that only resemble Latin in some
/// fonts (п, г, м) are left out, so real words such as `пример` still show.
const LATIN_LOOKALIKES: &[char] = &[
    '\u{0430}', '\u{0435}', '\u{043E}', '\u{0440}', '\u{0441}', '\u{0443}', '\u{0445}', '\u{0455}',
    '\u{0456}', '\u{0458}', '\u{04CF}', '\u{0501}', '\u{04BB}', '\u{051B}', '\u{051D}', '\u{0475}',
    '\u{04AF}', '\u{03B1}', '\u{03B9}', '\u{03BA}', '\u{03BD}', '\u{03BF}', '\u{03C1}', '\u{03C5}',
    '\u{03C7}', '\u{03F2}', '\u{03F3}',
];

/// Whether `host` (Unicode form) has a label that imitates a Latin word:
/// non-ASCII, and made only of [`LATIN_LOOKALIKES`] (plus digits and `-`),
/// under an ASCII top-level domain. A host under its own script's domain
/// (`пример.рф`) or in a script without Latin lookalikes (`例え.jp`) is not
/// one.
fn imitates_latin(host: &str) -> bool {
    let mut labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let Some(tld) = labels.pop() else {
        return false;
    };
    if !tld.is_ascii() {
        return false;
    }
    labels.iter().any(|label| {
        !label.is_ascii()
            && label
                .chars()
                .filter(|c| !c.is_ascii_digit() && *c != '-')
                .all(|c| LATIN_LOOKALIKES.contains(&c))
    })
}

/// Whether every label of `host` is written in a single script. ASCII
/// digits and `-` go with any script; an unknown script counts as mixed.
fn one_script_per_label(host: &str) -> bool {
    host.split('.').all(|label| {
        let mut seen: Option<Script> = None;
        for c in label.chars().filter(|c| !c.is_ascii_digit() && *c != '-') {
            let s = script_of(c);
            if s == Script::Other || seen.is_some_and(|prev| prev != s) {
                return false;
            }
            seen = Some(s);
        }
        true
    })
}

#[cfg(test)]
mod tests;
