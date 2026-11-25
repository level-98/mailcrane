//! The HTML sanitiser. Message HTML is never shown without going through
//! [`sanitize_html`].
//!
//! The output is safe to show in a WebView with JavaScript turned off: no
//! scripts, event handlers, forms, frames or dangerous URLs; CSS limited to
//! harmless properties; remote content blocked unless the user asked for it;
//! and tracking pixels removed in both modes. The allow-lists are in
//! `policy.rs`.
//!
//! It works in two stages. First `ammonia` parses the HTML and keeps only the
//! allow-listed tags and attributes, with the URL and CSS checks done in an
//! attribute filter. That output is canonical (every tag well formed, every
//! attribute double-quoted), so the second stage can walk it with a small
//! scanner to drop tracking pixels, block remote images and rewrite `cid:`
//! images, which need a look at a whole `<img>` tag at once.

mod policy;

/// Deepest tag nesting that is sanitised as HTML.
const MAX_NESTING: usize = 1024;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use ammonia::Builder;

use crate::trackers::{host_and_path, known_tracker, push_unique, TrackerHit, TrackerKind};
use crate::UnsanitisedHtml;

/// How to sanitise.
#[derive(Clone, Copy)]
pub struct SanitizeOptions {
    /// Keep remote (`http`/`https`) images. Off by default in the apps: the
    /// user has to ask. Tracking pixels are removed either way.
    pub allow_remote_content: bool,
    /// Maps the content id from a `cid:` reference to the app's internal URL,
    /// for example `mailcrane-cid://<message-id>/<content-id>`. It is called
    /// with the id only (no `cid:`, no angle brackets). Its result must start
    /// with a scheme that is not a web or script scheme; otherwise the image
    /// is dropped. The scheme handler must only serve parts of the message
    /// being shown.
    pub cid_url: fn(&str) -> String,
}

impl std::fmt::Debug for SanitizeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SanitizeOptions")
            .field("allow_remote_content", &self.allow_remote_content)
            .finish_non_exhaustive()
    }
}

/// HTML that went through [`sanitize_html`]. Only this module can build one
/// (the struct is `#[non_exhaustive]`).
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SanitizedHtml {
    /// The safe HTML.
    pub html: String,
    /// Remote images whose `src` was removed because remote content is off.
    pub remote_resources_blocked: u32,
    /// Tracking images removed (every one, duplicates included).
    pub trackers_removed: u32,
    /// The trackers found, one per domain (M5-01), in order of appearance.
    pub trackers: Vec<TrackerHit>,
}

impl std::fmt::Debug for SanitizedHtml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print message content (AGENTS.md §4.4).
        f.debug_struct("SanitizedHtml")
            .field("html_bytes", &self.html.len())
            .field("remote_resources_blocked", &self.remote_resources_blocked)
            .field("trackers_removed", &self.trackers_removed)
            .field("trackers", &self.trackers.len())
            .finish()
    }
}

/// Sanitises message HTML. Never panics.
pub fn sanitize_html(html: &UnsanitisedHtml, opts: &SanitizeOptions) -> SanitizedHtml {
    // `ammonia` slows down sharply on very deep nesting, so a hostile message
    // could freeze the app. Show such a message as plain text instead.
    if too_deep(html.as_unsanitised_str()) {
        let text = mail_parser::html_to_text(html.as_unsanitised_str());
        return SanitizedHtml {
            html: format!("<pre>{}</pre>", escape(&text)),
            remote_resources_blocked: 0,
            trackers_removed: 0,
            trackers: Vec::new(),
        };
    }
    // The internal URL prefix `cid:` references become. Used again below so a
    // second pass leaves already-rewritten images alone.
    let cid_prefix = cid_prefix(opts);
    let cid_scheme = cid_prefix.as_deref().and_then(scheme_of);

    let mut schemes: HashSet<&str> = HashSet::new();
    schemes.extend(policy::HREF_SCHEMES.iter().copied());
    schemes.extend(["cid", "data"]);
    if let Some(s) = &cid_scheme {
        schemes.insert(s.as_str());
    }

    let tags: HashSet<&str> = policy::TAGS.iter().copied().collect();
    let clean_content: HashSet<&str> = policy::CLEAN_CONTENT_TAGS.iter().copied().collect();
    let generic: HashSet<&str> = policy::GENERIC_ATTRIBUTES.iter().copied().collect();
    let tag_attrs: HashMap<&str, HashSet<&str>> = policy::TAG_ATTRIBUTES
        .iter()
        .map(|(tag, attrs)| (*tag, attrs.iter().copied().collect()))
        .collect();

    let prefix_for_filter = cid_prefix.clone();
    let cleaned = Builder::empty()
        .tags(tags)
        .clean_content_tags(clean_content)
        .generic_attributes(generic)
        .tag_attributes(tag_attrs)
        .url_schemes(schemes)
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            filter_attribute(element, attribute, value, prefix_for_filter.as_deref())
        })
        .clean(html.as_unsanitised_str())
        .to_string();

    let mut counts = Counts::default();
    let html = process_images(&cleaned, opts, cid_prefix.as_deref(), &mut counts);
    // The HTML parser drops a leading byte-order mark but can leave one behind
    // after removing something in front of it (a character, or a tracking
    // pixel here); drop it so a second pass gives the same result.
    let html = html.trim_start_matches('\u{feff}').to_owned();
    SanitizedHtml {
        html,
        remote_resources_blocked: counts.blocked,
        trackers_removed: counts.trackers,
        trackers: counts.hits,
    }
}

/// Tags that don't stay open, so they don't add to the nesting depth.
const NON_NESTING: &[&str] = &[
    "br", "hr", "img", "col", "wbr", "input", "meta", "link", "base", "p", "li", "dt", "dd", "td",
    "th", "tr", "option", "area", "source", "track",
];

/// Whether tags appear to nest deeper than [`MAX_NESTING`]. A cheap
/// over-estimate: counts opening tags minus closing tags, skipping
/// [`NON_NESTING`] ones.
fn too_deep(html: &str) -> bool {
    let mut depth: usize = 0;
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        rest = rest.get(i.saturating_add(1)..).unwrap_or_default();
        let (closing, name_part) = match rest.strip_prefix('/') {
            Some(r) => (true, r),
            None => (false, rest),
        };
        let name: String = name_part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if name.is_empty() || NON_NESTING.contains(&name.as_str()) {
            continue;
        }
        if closing {
            depth = depth.saturating_sub(1);
        } else {
            depth = depth.saturating_add(1);
            if depth > MAX_NESTING {
                return true;
            }
        }
    }
    false
}

#[derive(Default)]
struct Counts {
    blocked: u32,
    trackers: u32,
    hits: Vec<TrackerHit>,
}

/// The URL prefix `cid_url` produces, if its scheme is acceptable.
fn cid_prefix(opts: &SanitizeOptions) -> Option<String> {
    let prefix = (opts.cid_url)("");
    let probe = (opts.cid_url)("x");
    let scheme = scheme_of(&prefix)?;
    if policy::FORBIDDEN_CID_SCHEMES.contains(&scheme.as_str())
        || !probe.starts_with(&prefix)
        || prefix.chars().any(|c| c.is_control() || c == '"')
    {
        return None;
    }
    Some(prefix)
}

/// The lowercase scheme of a URL, or `None` for a relative URL. Whitespace and
/// control characters are ignored, as browsers do (`java\tscript:`).
fn scheme_of(url: &str) -> Option<String> {
    let mut scheme = String::new();
    for c in url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
    {
        match c {
            ':' => {
                return if scheme.is_empty() {
                    None
                } else {
                    Some(scheme)
                };
            }
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {
                scheme.push(c.to_ascii_lowercase());
            }
            _ => return None,
        }
    }
    None
}

/// Strips whitespace and control characters and lowercases, for checks.
fn squash(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn is_safe_data_image(value: &str) -> bool {
    let v = squash(value);
    let Some(rest) = v.strip_prefix("data:image/") else {
        return false;
    };
    let kind = rest.split([';', ',']).next().unwrap_or_default();
    policy::DATA_IMAGE_TYPES.contains(&kind) && rest.contains(',')
}

fn filter_attribute<'a>(
    element: &str,
    attribute: &str,
    value: &'a str,
    cid_prefix: Option<&str>,
) -> Option<Cow<'a, str>> {
    match (element, attribute) {
        ("a", "href") => {
            let scheme = scheme_of(value)?;
            policy::HREF_SCHEMES
                .contains(&scheme.as_str())
                .then_some(Cow::Borrowed(value))
        }
        ("img", "src") => {
            let scheme = scheme_of(value)?;
            let ok = match scheme.as_str() {
                "http" | "https" | "cid" => true,
                "data" => is_safe_data_image(value),
                // Already rewritten by a previous pass.
                _ => cid_prefix.is_some_and(|p| value.starts_with(p)),
            };
            ok.then_some(Cow::Borrowed(value))
        }
        (_, "style") => {
            let filtered = filter_style(value, element == "img");
            (!filtered.is_empty()).then_some(Cow::Owned(filtered))
        }
        _ => Some(Cow::Borrowed(value)),
    }
}

/// Keeps only allow-listed, harmless declarations. On an `img`, the
/// declarations that hide it are also kept so the second stage can see them
/// (the image is then removed).
fn filter_style(style: &str, is_img: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    for declaration in style.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let allowed = policy::CSS_PROPERTIES.contains(&name.as_str())
            || (is_img && hides_image(&name, value));
        if allowed && css_value_is_safe(value) {
            out.push(format!("{name}:{value}"));
        }
    }
    out.join(";")
}

fn hides_image(name: &str, value: &str) -> bool {
    let v = squash(value);
    match name {
        "display" => v == "none",
        "visibility" => v == "hidden",
        "opacity" => v.parse::<f32>().is_ok_and(|n| n == 0.0),
        _ => false,
    }
}

/// Rejects anything that could load a resource or run code: `url(`,
/// `expression(`, `@import`, escapes, comments, `behavior`, bindings, and
/// any function not in the allow-list.
fn css_value_is_safe(value: &str) -> bool {
    if value
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | '<' | '>' | '&' | '@' | '{' | '}'))
        || value.contains("/*")
    {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if [
        "javascript",
        "vbscript",
        "behavior",
        "binding",
        "import",
        "expression",
        "url",
        "image",
    ]
    .iter()
    .any(|bad| lower.contains(bad))
    {
        return false;
    }
    // Every `(` must follow an allow-listed function name.
    for (i, c) in lower.char_indices() {
        if c == '(' {
            let head = lower.get(..i).unwrap_or_default();
            let name: String = head
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !policy::CSS_FUNCTIONS.contains(&name.as_str()) {
                return false;
            }
        }
    }
    true
}

// ---- Second stage: a scan over ammonia's canonical output -----------------

struct Tag<'a> {
    name: &'a str,
    /// Attribute name and raw (still escaped) value.
    attrs: Vec<(&'a str, &'a str)>,
    /// Byte offset just past the closing `>`.
    end: usize,
}

/// Parses the tag starting at `start` (which holds `<`). Returns `None` if it
/// isn't a tag. Quoted values are skipped whole, so a `>` or `<` inside one
/// can't confuse the scan.
fn parse_tag(s: &str, start: usize) -> Option<Tag<'_>> {
    let bytes = s.as_bytes();
    let mut i = start.checked_add(1)?;
    if !bytes.get(i)?.is_ascii_alphabetic() {
        return None;
    }
    let name_start = i;
    while bytes
        .get(i)
        .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'>' | b'/'))
    {
        i = i.checked_add(1)?;
    }
    let name = s.get(name_start..i)?;
    let mut attrs = Vec::new();
    loop {
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'/')
        {
            i = i.checked_add(1)?;
        }
        match bytes.get(i)? {
            b'>' => {
                return Some(Tag {
                    name,
                    attrs,
                    end: i.checked_add(1)?,
                })
            }
            _ => {
                let a_start = i;
                while bytes
                    .get(i)
                    .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'>' | b'/' | b'='))
                {
                    i = i.checked_add(1)?;
                }
                let a_name = s.get(a_start..i)?;
                let mut value = "";
                if bytes.get(i) == Some(&b'=') {
                    i = i.checked_add(1)?;
                    if let Some(q @ (b'"' | b'\'')) = bytes.get(i).copied() {
                        let v_start = i.checked_add(1)?;
                        let rel = s.get(v_start..)?.find(q as char)?;
                        let v_end = v_start.checked_add(rel)?;
                        value = s.get(v_start..v_end)?;
                        i = v_end.checked_add(1)?;
                    } else {
                        let v_start = i;
                        while bytes
                            .get(i)
                            .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>')
                        {
                            i = i.checked_add(1)?;
                        }
                        value = s.get(v_start..i)?;
                    }
                }
                if a_name.is_empty() {
                    // Can't make progress; give up on this tag.
                    return None;
                }
                attrs.push((a_name, value));
            }
        }
    }
}

fn unescape(raw: &str) -> String {
    raw.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let decoded = (b == b'%')
            .then(|| {
                let hex = s.get(i.checked_add(1)?..i.checked_add(3)?)?;
                u8::from_str_radix(hex, 16).ok()
            })
            .flatten();
        match decoded {
            Some(d) => {
                out.push(d);
                i = i.saturating_add(3);
            }
            None => {
                out.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses `12`, `12px` (and nothing else) as pixels.
fn parse_px(value: &str) -> Option<u32> {
    let v = value.trim().to_ascii_lowercase();
    let digits = v.strip_suffix("px").unwrap_or(&v).trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Why an `<img>` is a tracker, if it is one: a known service first, then
/// the size and visibility rules.
fn tracker_hit(attrs: &[(&str, &str)]) -> Option<TrackerHit> {
    let src = attrs
        .iter()
        .find(|(name, _)| *name == "src")
        .map(|(_, raw)| unescape(raw))
        .unwrap_or_default();
    if let Some(hit) = known_tracker(&src) {
        return Some(hit);
    }
    let kind = size_or_visibility(attrs)?;
    let domain = host_and_path(&src)
        .map(|(host, _)| host)
        .unwrap_or_default();
    Some(TrackerHit { domain, kind })
}

fn size_or_visibility(attrs: &[(&str, &str)]) -> Option<TrackerKind> {
    let tiny = |v: &str| parse_px(v).is_some_and(|n| n <= policy::TRACKER_MAX_PX);
    for (name, raw) in attrs {
        let value = unescape(raw);
        match *name {
            "width" | "height" if tiny(&value) => return Some(TrackerKind::Tiny),
            "style" => {
                for declaration in value.split(';') {
                    let Some((prop, val)) = declaration.split_once(':') else {
                        continue;
                    };
                    let prop = prop.trim().to_ascii_lowercase();
                    if matches!(prop.as_str(), "width" | "height") && tiny(val) {
                        return Some(TrackerKind::Tiny);
                    }
                    if hides_image(&prop, val) {
                        return Some(TrackerKind::Hidden);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn process_images(
    html: &str,
    opts: &SanitizeOptions,
    cid_prefix: Option<&str>,
    counts: &mut Counts,
) -> String {
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(rel) = html.get(pos..).and_then(|rest| rest.find('<')) {
        let lt = pos.saturating_add(rel);
        out.push_str(html.get(pos..lt).unwrap_or_default());
        let Some(tag) = parse_tag(html, lt) else {
            // Not a tag we understand; keep the `<` and move on.
            out.push('<');
            pos = lt.saturating_add(1);
            continue;
        };
        let whole = html.get(lt..tag.end).unwrap_or_default();
        pos = tag.end;
        if !tag.name.eq_ignore_ascii_case("img") {
            out.push_str(whole);
            continue;
        }
        if let Some(hit) = tracker_hit(&tag.attrs) {
            counts.trackers = counts.trackers.saturating_add(1);
            push_unique(&mut counts.hits, hit);
            continue;
        }
        out.push_str("<img");
        for (name, raw) in &tag.attrs {
            if *name == "src" {
                let src = unescape(raw);
                let scheme = scheme_of(&src);
                match scheme.as_deref() {
                    Some("http" | "https") if !opts.allow_remote_content => {
                        counts.blocked = counts.blocked.saturating_add(1);
                        continue;
                    }
                    Some("cid") => {
                        let id = squash_cid(&src);
                        let mapped = (opts.cid_url)(&id);
                        if cid_prefix.is_some_and(|p| mapped.starts_with(p)) && !id.is_empty() {
                            out.push_str(" src=\"");
                            out.push_str(&escape(&mapped));
                            out.push('"');
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            out.push_str(raw);
            out.push('"');
        }
        out.push('>');
    }
    out.push_str(html.get(pos..).unwrap_or_default());
    out
}

/// The content id inside a `cid:` URL, without `cid:` or angle brackets.
fn squash_cid(src: &str) -> String {
    let trimmed = src.trim();
    let rest = trimmed.get(4..).unwrap_or_default();
    let decoded = percent_decode(rest);
    decoded
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn opts(allow_remote_content: bool) -> SanitizeOptions {
        SanitizeOptions {
            allow_remote_content,
            cid_url: |id| format!("mailcrane-cid://msg-1/{id}"),
        }
    }

    fn run(html: &str, allow: bool) -> SanitizedHtml {
        sanitize_html(&UnsanitisedHtml::new(html.to_owned()), &opts(allow))
    }

    #[test]
    fn sanitize_deep_nesting_falls_back_to_text_quickly() {
        let depth = 50_000;
        let html = format!("<b>{}x{}", "<div>".repeat(depth), "</div>".repeat(depth));
        let out = run(&html, false);
        assert!(out.html.starts_with("<pre>"));
        assert!(!out.html.contains("<div"));
        // And the fallback is stable.
        assert_eq!(run(&out.html, false).html, out.html);
    }

    #[test]
    fn sanitize_moderate_nesting_is_kept() {
        let depth = 500;
        let html = format!("{}x{}", "<div>".repeat(depth), "</div>".repeat(depth));
        let out = run(&html, false);
        assert_eq!(out.html.matches("<div>").count(), depth);
    }

    #[test]
    fn sanitize_leading_byte_order_mark_is_stable() {
        for html in [
            "\0\u{feff}",
            "\u{feff}\u{feff}",
            "<img src=\"data:image/png;base64,AAAA\" style=\"opacity:0\">\u{feff}",
        ] {
            let first = run(html, false);
            assert_eq!(run(&first.html, false).html, first.html);
        }
    }

    fn html_soup() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just("<".to_owned()),
            Just(">".to_owned()),
            Just("\"".to_owned()),
            Just("'".to_owned()),
            Just("=".to_owned()),
            Just("&".to_owned()),
            Just("&#106;".to_owned()),
            Just("<img src=\"cid:a@example.org\">".to_owned()),
            Just("<img src=\"https://example.org/a.png\" width=\"1\">".to_owned()),
            Just("<img src=https://example.org/b.png onerror=x>".to_owned()),
            Just("<img src=\"data:image/png;base64,AAAA\" style=\"opacity:0\">".to_owned()),
            Just("<a href=\"javascript:x\">".to_owned()),
            Just("<a href=\"https://example.org\" target=_blank>".to_owned()),
            Just("<p style=\"color:red;background:url(x)\">".to_owned()),
            Just("<script>".to_owned()),
            Just("<svg><style>".to_owned()),
            Just("<!--".to_owned()),
            Just("-->".to_owned()),
            Just("<table><td>".to_owned()),
            Just("</p>".to_owned()),
            Just("<div>".to_owned()),
            "[a-z ]{0,8}",
            ".{0,12}",
        ];
        prop::collection::vec(piece, 0..24).prop_map(|v| v.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: proptest::test_runner::Config::default().cases.max(512),
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// No panic on arbitrary input, and sanitising twice changes nothing.
        #[test]
        fn sanitize_never_panics_and_is_idempotent(
            html in prop_oneof![html_soup(), ".{0,300}"],
            allow in any::<bool>(),
        ) {
            let first = run(&html, allow);
            let second = run(&first.html, allow);
            prop_assert_eq!(&second.html, &first.html);
            prop_assert_eq!(second.trackers_removed, 0);
            prop_assert_eq!(second.remote_resources_blocked, 0);
            let lower = first.html.to_ascii_lowercase();
            prop_assert!(!lower.contains("<script"));
            prop_assert!(!lower.contains("<iframe"));
        }
    }
}
