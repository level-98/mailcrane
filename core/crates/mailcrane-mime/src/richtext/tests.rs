use super::*;
use crate::{sanitize_html, SanitizeOptions, UnsanitisedHtml};

fn span(text: &str) -> Span {
    Span {
        text: text.into(),
        ..Span::default()
    }
}

fn link(text: &str, href: &str) -> Span {
    Span {
        text: text.into(),
        link: Some(href.into()),
        ..Span::default()
    }
}

fn sample() -> RichDoc {
    RichDoc {
        blocks: vec![
            Block::Paragraph(vec![
                span("Hello "),
                Span {
                    text: "bold".into(),
                    bold: true,
                    ..Span::default()
                },
                span(" and "),
                Span {
                    text: "both".into(),
                    bold: true,
                    italic: true,
                    ..Span::default()
                },
                span(", see "),
                link("the plan", "https://example.org/plan?a=1&b=2"),
                span("."),
            ]),
            Block::BulletList(vec![vec![span("one")], vec![span("two & <three>")]]),
            Block::NumberedList(vec![
                vec![span("first")],
                vec![link("mail me", "mailto:ana@example.org")],
            ]),
        ],
    }
}

#[test]
fn html_escapes_text_and_urls() {
    let html = to_html(&RichDoc {
        blocks: vec![Block::Paragraph(vec![
            span("<script>alert(\"x\")</script> & 'q'"),
            link("click", "https://example.org/?q=\"><script>"),
        ])],
    });
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#39;q&#39;"));
    assert!(!html.contains("\"><script>"), "{html}");
}

#[test]
fn unsafe_links_become_plain_text() {
    for bad in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "data:text/html,<b>x</b>",
        "vbscript:x",
        "ftp://example.org",
        "https://exa mple.org",
        "mailto:",
        "",
    ] {
        let html = to_html(&RichDoc {
            blocks: vec![Block::Paragraph(vec![link("t", bad)])],
        });
        assert!(!html.contains("<a"), "{bad}: {html}");
        assert!(html.contains(">t<"));
        let plain = to_plain(&RichDoc {
            blocks: vec![Block::Paragraph(vec![link("t", bad)])],
        });
        assert_eq!(plain, "t", "{bad}");
    }
}

#[test]
fn the_sanitiser_leaves_our_html_unchanged() {
    let html = to_html(&sample());
    let clean = sanitize_html(
        &UnsanitisedHtml::new(html.clone()),
        &SanitizeOptions {
            allow_remote_content: false,
            cid_url: |id| format!("mailcrane-cid://part/{id}"),
        },
    );
    assert_eq!(clean.html, html);
}

#[test]
fn plain_text_table() {
    let cases: Vec<(RichDoc, &str)> = vec![
        (
            sample(),
            "Hello bold and both, see the plan <https://example.org/plan?a=1&b=2>.\n\n\
             - one\n- two & <three>\n\n1. first\n2. mail me <mailto:ana@example.org>",
        ),
        // A link that is its own text isn't repeated.
        (
            RichDoc {
                blocks: vec![Block::Paragraph(vec![link(
                    "https://example.org",
                    "https://example.org",
                )])],
            },
            "https://example.org",
        ),
        (
            RichDoc {
                blocks: vec![Block::Paragraph(vec![link(
                    "ana@example.org",
                    "mailto:ana@example.org",
                )])],
            },
            "ana@example.org",
        ),
        (RichDoc::default(), ""),
        (
            RichDoc {
                blocks: vec![
                    Block::Paragraph(vec![span("a")]),
                    Block::Paragraph(vec![span("b")]),
                ],
            },
            "a\n\nb",
        ),
    ];
    for (doc, want) in cases {
        assert_eq!(to_plain(&doc), want);
    }
}

fn rich_message(attachments: Vec<crate::OutgoingAttachment>) -> crate::OutgoingMessage {
    crate::OutgoingMessage {
        from: crate::Address {
            name: Some("Ana".into()),
            email: "ana@example.org".into(),
        },
        to: vec![crate::Address {
            name: None,
            email: "bob@example.net".into(),
        }],
        subject: "Plan".into(),
        body_text: "ignored when rich".into(),
        rich: Some(sample()),
        attachments,
        ..crate::OutgoingMessage::default()
    }
}

#[test]
fn rich_messages_are_alternative_with_text_first_and_round_trip() {
    for attachments in [
        Vec::new(),
        vec![crate::OutgoingAttachment {
            filename: "a.txt".into(),
            mime_type: "text/plain".into(),
            bytes: b"x".to_vec(),
        }],
    ] {
        let with_files = !attachments.is_empty();
        let raw = crate::build_message(&rich_message(attachments)).expect("build");
        let text = String::from_utf8_lossy(&raw);
        let alt = text.find("multipart/alternative").expect("alternative");
        let plain = text[alt..].find("text/plain").expect("plain part");
        let html = text[alt..].find("text/html").expect("html part");
        assert!(plain < html, "plain text first");
        assert_eq!(text.contains("multipart/mixed"), with_files);
        let parsed = crate::parse_message(&raw).expect("parse");
        assert_eq!(
            parsed
                .text_plain
                .as_deref()
                .map(|t| t.trim_end().replace("\r\n", "\n")),
            Some(to_plain(&sample()))
        );
        let html = parsed.text_html.expect("html");
        assert!(html.as_unsanitised_str().contains("<b>bold</b>"));
        assert_eq!(parsed.attachments.len(), usize::from(with_files));
    }
}
