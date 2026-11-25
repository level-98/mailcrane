//! Sanitiser tests: the XSS fixture suite, remote content, `cid:` images,
//! tracking pixels, and property tests.

#![allow(clippy::expect_used)] // test helpers may panic

use mailcrane_mime::{
    sanitize_html, SanitizeOptions, SanitizedHtml, TrackerHit, TrackerKind, UnsanitisedHtml,
};

type R = Result<(), Box<dyn std::error::Error>>;

fn cid(id: &str) -> String {
    format!("mailcrane-cid://msg-1/{id}")
}

fn opts(allow_remote_content: bool) -> SanitizeOptions {
    SanitizeOptions {
        allow_remote_content,
        cid_url: cid,
    }
}

// `UnsanitisedHtml::new` is crate-private, so build one the way the app does:
// by parsing a message.
fn unsanitised(html: &str) -> UnsanitisedHtml {
    let raw = format!(
        "Subject: t\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
        b64(html.as_bytes())
    );
    mailcrane_mime::parse_message(raw.as_bytes())
        .ok()
        .and_then(|m| m.text_html)
        .unwrap_or_else(unsanitised_fallback)
}

fn unsanitised_fallback() -> UnsanitisedHtml {
    // An empty body parses to no HTML; use a trivial message instead.
    let raw = b"Subject: t\r\nContent-Type: text/html\r\n\r\n<p>x</p>\r\n";
    mailcrane_mime::parse_message(raw)
        .ok()
        .and_then(|m| m.text_html)
        .expect("trivial HTML message parses")
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn run(html: &str, allow: bool) -> SanitizedHtml {
    sanitize_html(&unsanitised(html), &opts(allow))
}

/// Walks the tags in sanitiser output, quote-aware. Returns (tag name,
/// attributes with raw values).
fn tags(html: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut found = Vec::new();
    let b = html.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let start = j;
        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
            j += 1;
        }
        let name = html[start..j].to_ascii_lowercase();
        let mut attrs = Vec::new();
        while j < b.len() && b[j] != b'>' {
            while j < b.len() && (b[j].is_ascii_whitespace() || b[j] == b'/') {
                j += 1;
            }
            let s = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>') {
                j += 1;
            }
            let an = html[s..j].to_ascii_lowercase();
            let mut val = String::new();
            if j < b.len() && b[j] == b'=' {
                j += 1;
                if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    let vs = j + 1;
                    j = vs;
                    while j < b.len() && b[j] != q {
                        j += 1;
                    }
                    val = html[vs..j.min(b.len())].to_owned();
                    j += 1;
                } else {
                    let vs = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    val = html[vs..j].to_owned();
                }
            }
            if !an.is_empty() {
                attrs.push((an, val));
            }
        }
        found.push((name, attrs));
        i = j + 1;
    }
    found
}

fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
            u32::from_str_radix(h, 16).ok().and_then(char::from_u32)
        } else if let Some(d) = ent.strip_prefix('#') {
            d.parse().ok().and_then(char::from_u32)
        } else {
            match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "Tab" => Some('\t'),
                "NewLine" => Some('\n'),
                _ => None,
            }
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

const BAD_TAGS: &[&str] = &[
    "script", "iframe", "object", "embed", "form", "input", "button", "textarea", "select", "meta",
    "link", "base", "svg", "math", "video", "audio", "style", "frame", "frameset", "applet",
];

/// The safety checks every output must pass.
fn assert_safe(name: &str, out: &SanitizedHtml) {
    let lower = out.html.to_ascii_lowercase();
    assert!(!lower.contains("<script"), "{name}: <script in {lower}");
    assert!(!lower.contains("<iframe"), "{name}: <iframe in {lower}");
    assert!(!lower.contains('\0'), "{name}: NUL in output");
    for (tag, attrs) in tags(&out.html) {
        let tag = tag.trim_start_matches('/');
        assert!(!BAD_TAGS.contains(&tag), "{name}: <{tag}> in {lower}");
        for (attr, value) in attrs {
            assert!(
                !attr.starts_with("on"),
                "{name}: attribute {attr} in {lower}"
            );
            assert!(
                !matches!(
                    attr.as_str(),
                    "id" | "name" | "srcset" | "target" | "formaction"
                ),
                "{name}: attribute {attr} in {lower}"
            );
            // Decode entities (twice, for double encoding), drop whitespace
            // and control characters, and lowercase.
            let v: String = decode_entities(&decode_entities(&value))
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_control())
                .collect::<String>()
                .to_ascii_lowercase();
            for bad in [
                "javascript:",
                "vbscript:",
                "data:text",
                "data:image/svg",
                "expression(",
                "url(",
                "@import",
            ] {
                assert!(!v.contains(bad), "{name}: {bad} in {attr}={value}");
            }
            if attr == "href" {
                assert!(
                    v.starts_with("https:") || v.starts_with("http:") || v.starts_with("mailto:"),
                    "{name}: href={value}"
                );
            }
        }
    }
}

fn fixture_dir() -> String {
    format!(
        "{}/../../../tests/fixtures/html",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn read_fixture(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(format!("{}/{name}", fixture_dir()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn sanitize_xss_fixture_suite() -> R {
    let mut count = 0;
    for entry in std::fs::read_dir(fixture_dir())? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("xss-") {
            continue;
        }
        count += 1;
        let html = read_fixture(name)?;
        for allow in [false, true] {
            let out = run(&html, allow);
            assert_safe(name, &out);
            // Idempotent on every attack sample too.
            let again = sanitize_html(&unsanitised(&out.html), &opts(allow));
            assert_eq!(again.html, out.html, "{name}: not idempotent");
        }
    }
    assert!(count >= 40, "only {count} XSS fixtures");
    Ok(())
}

#[test]
fn sanitize_script_and_style_contents_are_removed() {
    let out = run(
        "<p>a</p><script>steal()</script><style>p{color:red}</style><p>b</p>",
        false,
    );
    assert_eq!(out.html, "<p>a</p><p>b</p>");
}

#[test]
fn sanitize_links_get_rel_and_lose_target() {
    let out = run(
        "<a href=\"https://example.org/\" target=\"_blank\" rel=\"opener\">x</a>",
        false,
    );
    assert_eq!(
        out.html,
        "<a href=\"https://example.org/\" rel=\"noopener noreferrer\">x</a>"
    );
}

#[test]
fn sanitize_href_schemes() {
    let out = run(
        "<a href=\"mailto:ann@example.org\">m</a><a href=\"http://example.org/\">h</a><a href=\"/relative\">r</a><a href=\"ftp://example.org/\">f</a><a href=\"cid:x\">c</a>",
        false,
    );
    assert!(out.html.contains("href=\"mailto:ann@example.org\""));
    assert!(out.html.contains("href=\"http://example.org/\""));
    assert!(!out.html.contains("relative"));
    assert!(!out.html.contains("ftp:"));
    assert!(!out.html.contains("cid:"));
}

#[test]
fn sanitize_remote_images_removed_and_counted_by_default() {
    let out = run(
        "<img src=\"https://example.org/a.png\" alt=\"a\" width=\"100\"><img src=\"http://example.org/b.png\">",
        false,
    );
    assert_eq!(out.remote_resources_blocked, 2);
    assert_eq!(out.trackers_removed, 0);
    assert!(!out.html.contains("example.org"));
    assert!(out.html.contains("alt=\"a\""));
}

#[test]
fn sanitize_remote_images_kept_when_allowed() {
    let out = run(
        "<img src=\"https://example.org/a.png\" width=\"100\">",
        true,
    );
    assert_eq!(out.remote_resources_blocked, 0);
    assert!(out.html.contains("src=\"https://example.org/a.png\""));
}

#[test]
fn sanitize_cid_images_are_rewritten() {
    for allow in [false, true] {
        let out = run("<img src=\"cid:logo@example.org\" alt=\"l\">", allow);
        assert_eq!(out.remote_resources_blocked, 0);
        assert!(
            out.html
                .contains("src=\"mailcrane-cid://msg-1/logo@example.org\""),
            "{}",
            out.html
        );
    }
    let out = run("<img src=\"CID:%3Ca%2Bb@example.org%3E\">", false);
    assert!(out
        .html
        .contains("src=\"mailcrane-cid://msg-1/a+b@example.org\""));
}

#[test]
fn sanitize_bad_cid_scheme_drops_the_image_source() {
    let o = SanitizeOptions {
        allow_remote_content: true,
        cid_url: |id| format!("javascript:{id}"),
    };
    let out = sanitize_html(&unsanitised("<img src=\"cid:a@example.org\">"), &o);
    assert!(!out.html.contains("javascript"));
    assert!(!out.html.contains("src"));
}

#[test]
fn sanitize_data_images_allowed_only_for_raster_types() {
    let ok = run("<img src=\"data:image/png;base64,iVBORw0KGgo=\">", false);
    assert!(ok
        .html
        .contains("src=\"data:image/png;base64,iVBORw0KGgo=\""));
    for bad in [
        "data:image/svg+xml;base64,PHN2Zz4=",
        "data:text/html;base64,PHA+",
        "data:image/png",
    ] {
        let out = run(&format!("<img src=\"{bad}\">"), true);
        assert!(!out.html.contains("data:"), "{bad}: {}", out.html);
    }
}

#[test]
fn sanitize_trackers_removed_in_both_modes() {
    let html = concat!(
        "<img src=\"https://example.net/a.gif\" width=\"1\" height=\"1\">",
        "<img src=\"https://example.net/b.gif\" width=\"2px\">",
        "<img src=\"https://example.net/c.gif\" height=\"0\">",
        "<img src=\"https://example.net/d.gif\" style=\"display:none\">",
        "<img src=\"https://example.net/e.gif\" style=\"visibility: hidden\">",
        "<img src=\"https://example.net/f.gif\" style=\"opacity:0\">",
        "<img src=\"https://example.net/g.gif\" style=\"width:1px;height:1px\">",
        "<img src=\"cid:pixel@example.org\" width=\"1\" height=\"1\">",
        "<img src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\" width=\"1\">",
        "<p>text</p>"
    );
    for allow in [false, true] {
        let out = run(html, allow);
        assert_eq!(out.trackers_removed, 9, "allow={allow}: {}", out.html);
        assert_eq!(out.remote_resources_blocked, 0);
        assert_eq!(out.html, "<p>text</p>");
    }
}

#[test]
fn sanitize_normal_sized_images_are_not_trackers() {
    let out = run(
        "<img src=\"https://example.org/a.png\" width=\"3\" height=\"3\"><img src=\"https://example.org/b.png\" width=\"50%\">",
        true,
    );
    assert_eq!(out.trackers_removed, 0);
    assert_eq!(out.html.matches("<img").count(), 2);
}

#[test]
fn sanitize_css_is_filtered() {
    let out = run(
        "<p style=\"color: red; position:fixed; background-color:rgb(1,2,3); background:url(https://example.org/x.png); width:expression(1); font-family:'Helvetica Neue',Arial\">x</p>",
        true,
    );
    assert!(out.html.contains("color:red"));
    assert!(out.html.contains("background-color:rgb(1,2,3)"));
    assert!(out.html.contains("font-family:'Helvetica Neue',Arial"));
    for gone in ["position", "url", "expression", "example.org"] {
        assert!(!out.html.contains(gone), "{gone} in {}", out.html);
    }
}

#[test]
fn sanitize_real_world_newsletter() -> R {
    let html = read_fixture("newsletter.html")?;
    let blocked = run(&html, false);
    assert_safe("newsletter", &blocked);
    assert_eq!(blocked.trackers_removed, 3);
    assert_eq!(blocked.remote_resources_blocked, 1);
    assert!(blocked
        .html
        .contains("mailcrane-cid://msg-1/logo@example.org"));
    assert!(blocked.html.contains("Weekly digest"));
    assert!(blocked.html.contains("href=\"mailto:ann@example.org\""));
    assert!(!blocked.html.contains("header.png"));

    let allowed = run(&html, true);
    assert_safe("newsletter", &allowed);
    assert_eq!(allowed.trackers_removed, 3);
    assert_eq!(allowed.remote_resources_blocked, 0);
    assert!(allowed
        .html
        .contains("https://example.org/images/header.png"));
    Ok(())
}

#[test]
fn sanitize_inline_data_image_fixture() -> R {
    let out = run(&read_fixture("inline-data-image.html")?, false);
    assert!(out.html.contains("data:image/png;base64,iVBORw0KGgo="));
    Ok(())
}

#[test]
fn sanitize_debug_does_not_print_content() {
    let out = run("<p>secret text</p>", false);
    assert!(!format!("{out:?}").contains("secret"));
}

fn tracker_fixture(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    read_fixture(&format!("trackers/{name}"))
}

fn known(service: &str, domain: &str) -> TrackerHit {
    TrackerHit {
        domain: domain.into(),
        kind: TrackerKind::KnownService(service.into()),
    }
}

#[test]
fn sanitize_known_tracking_services_removed_in_both_modes() -> R {
    let html = tracker_fixture("known-services.html")?;
    for allow in [false, true] {
        let out = run(&html, allow);
        assert_eq!(
            out.html.matches("<img").count(),
            0,
            "allow={allow}: {}",
            out.html
        );
        assert_eq!(out.trackers_removed, 6);
        assert_eq!(out.remote_resources_blocked, 0);
        assert_eq!(
            out.trackers,
            vec![
                known("Mailchimp", "list-manage.com"),
                known("SendGrid (Twilio)", "sendgrid.net"),
                known("Amazon SES", "awstrack.me"),
                known("Substack", "substackcdn.com"),
                known("LinkedIn", "linkedin.com"),
            ],
            "allow={allow}"
        );
        // Nothing left to find the second time.
        let again = sanitize_html(&unsanitised(&out.html), &opts(allow));
        assert!(again.trackers.is_empty());
        assert_eq!(again.html, out.html);
    }
    Ok(())
}

#[test]
fn sanitize_tiny_and_hidden_images_reported_by_host() -> R {
    let html = tracker_fixture("size-and-hidden.html")?;
    for allow in [false, true] {
        let out = run(&html, allow);
        assert_eq!(out.trackers_removed, 4, "allow={allow}");
        assert_eq!(
            out.trackers,
            vec![
                TrackerHit {
                    domain: "stats.example.net".into(),
                    kind: TrackerKind::Tiny
                },
                TrackerHit {
                    domain: "img.example.org".into(),
                    kind: TrackerKind::Hidden
                },
                TrackerHit {
                    domain: String::new(),
                    kind: TrackerKind::Tiny
                },
            ]
        );
        // The real photo stays (blocked or shown per the setting).
        assert_eq!(out.html.matches("<img").count(), 1);
        assert_eq!(out.remote_resources_blocked, u32::from(!allow));
    }
    Ok(())
}

#[test]
fn sanitize_real_images_on_a_tracker_domain_are_kept() -> R {
    let html = tracker_fixture("cdn-content.html")?;
    let out = run(&html, true);
    assert_eq!(out.html.matches("<img").count(), 3, "{}", out.html);
    assert!(out.html.contains("substackcdn.com/image/fetch"));
    assert_eq!(out.trackers, vec![known("Substack", "substackcdn.com")]);
    let blocked = run(&html, false);
    assert_eq!(blocked.remote_resources_blocked, 3);
    assert_eq!(blocked.trackers_removed, 1);
    Ok(())
}

#[test]
fn sanitize_collects_link_text() -> R {
    let html = read_fixture("link-mismatch.html")?;
    let out = run(&html, false);
    let find = |href: &str| {
        out.links
            .iter()
            .find(|l| l.href == href)
            .map(|l| l.text.as_str())
    };
    assert_eq!(out.links.len(), 8);
    assert_eq!(
        find("https://example.net/login"),
        Some("https://example.com/login")
    );
    assert_eq!(
        find("https://example.com@example.net/"),
        Some("Open your account")
    );
    // Tags removed and whitespace collapsed.
    assert_eq!(find("http://example.org/news"), Some("Read the news"));
    Ok(())
}

#[test]
fn sanitize_link_collection_is_capped_and_changes_nothing() {
    let many = "<a href=\"https://example.org/\">x</a>".repeat(600);
    let out = run(&many, false);
    assert_eq!(out.links.len(), mailcrane_mime::MAX_LINKS);
    let long = format!("<a href=\"https://example.org/\">{}</a>", "y".repeat(1000));
    let out = run(&long, false);
    assert_eq!(
        out.links[0].text.chars().count(),
        mailcrane_mime::MAX_LINK_TEXT_CHARS
    );
    // Unclosed and href-less anchors don't panic or leak into the list.
    let odd = run(
        "<a href=\"https://example.org/\">open<a>none</a><a href=\"mailto:a@example.org\">m",
        false,
    );
    assert_eq!(odd.links.len(), 2);
}
