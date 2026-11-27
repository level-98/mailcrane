//! Parses each synthetic fixture in `tests/fixtures/mime/` and checks the
//! fields.

use mailcrane_mime::{is_bidi_control, parse_message, strip_bidi, Disposition, ParsedMessage};

type R = Result<(), Box<dyn std::error::Error>>;

fn load(name: &str) -> Result<ParsedMessage, Box<dyn std::error::Error>> {
    let path = format!(
        "{}/../../../tests/fixtures/mime/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path)?;
    Ok(parse_message(&bytes)?)
}

fn plain(m: &ParsedMessage) -> &str {
    m.text_plain.as_deref().unwrap_or_default()
}

fn html(m: &ParsedMessage) -> &str {
    m.text_html
        .as_ref()
        .map(|h| h.as_unsanitised_str())
        .unwrap_or_default()
}

#[test]
fn plain_ascii() -> R {
    let m = load("plain-ascii.eml")?;
    assert_eq!(m.subject.as_deref(), Some("Plain hello"));
    assert_eq!(m.from[0].email, "ann@example.org");
    assert_eq!(m.from[0].name.as_deref(), Some("Ann Example"));
    assert_eq!(m.to[0].email, "bob@example.com");
    assert_eq!(m.message_id.as_deref(), Some("plain@example.org"));
    assert_eq!(m.date, Some(1_772_445_600));
    assert!(plain(&m).starts_with("Hello Bob,"));
    assert!(m.text_html.is_none());
    assert_eq!(m.snippet, "Hello Bob, See you at lunch.");
    assert!(m.attachments.is_empty());
    Ok(())
}

#[test]
fn utf8_body() -> R {
    let m = load("utf8-body.eml")?;
    assert_eq!(plain(&m).trim(), "Grüße, café ☕");
    Ok(())
}

#[test]
fn iso_8859_1() -> R {
    let m = load("iso-8859-1.eml")?;
    assert_eq!(plain(&m).trim(), "café naïve");
    Ok(())
}

#[test]
fn windows_1252() -> R {
    let m = load("windows-1252.eml")?;
    assert_eq!(plain(&m).trim(), "price € 5 “quoted”");
    Ok(())
}

#[test]
fn shift_jis() -> R {
    let m = load("shift-jis.eml")?;
    assert_eq!(plain(&m).trim(), "こんにちは");
    Ok(())
}

#[test]
fn encoded_word_subject_b_and_q() -> R {
    let m = load("encoded-word-subject.eml")?;
    assert_eq!(m.subject.as_deref(), Some("Hello wörld and café time"));
    Ok(())
}

#[test]
fn encoded_display_names() -> R {
    let m = load("encoded-display-names.eml")?;
    assert_eq!(m.from[0].name.as_deref(), Some("Åsa Åndersson"));
    assert_eq!(m.from[0].email, "asa@example.org");
    assert_eq!(m.to.len(), 2);
    assert_eq!(m.to[0].name.as_deref(), Some("Jürgen Müller"));
    assert_eq!(m.to[1].name.as_deref(), Some("Plain, Name"));
    assert_eq!(m.to[1].email, "pn@example.net");
    Ok(())
}

#[test]
fn multipart_alternative() -> R {
    let m = load("multipart-alternative.eml")?;
    assert_eq!(plain(&m).trim(), "Plain version");
    assert_eq!(html(&m).trim(), "<p>HTML <b>version</b></p>");
    assert!(m.attachments.is_empty());
    Ok(())
}

#[test]
fn multipart_mixed_with_pdf() -> R {
    let m = load("multipart-mixed-pdf.eml")?;
    assert_eq!(plain(&m).trim(), "Please see the attached report.");
    assert_eq!(m.attachments.len(), 1);
    let a = &m.attachments[0];
    assert_eq!(a.filename.as_deref(), Some("report.pdf"));
    assert_eq!(a.mime_type, "application/pdf");
    assert_eq!(a.size, 31);
    assert_eq!(a.disposition, Disposition::Attachment);
    assert!(a.content_id.is_none());
    Ok(())
}

#[test]
fn multipart_related_inline_cid() -> R {
    let m = load("multipart-related-cid.eml")?;
    assert!(html(&m).contains("cid:logo@example.org"));
    assert_eq!(m.attachments.len(), 1);
    let a = &m.attachments[0];
    assert_eq!(a.content_id.as_deref(), Some("logo@example.org"));
    assert_eq!(a.mime_type, "image/png");
    assert_eq!(a.disposition, Disposition::Inline);
    assert_eq!(a.size, 8);
    Ok(())
}

#[test]
fn nested_rfc822() -> R {
    let m = load("nested-rfc822.eml")?;
    assert_eq!(m.subject.as_deref(), Some("Fwd: inner"));
    assert_eq!(plain(&m).trim(), "Forwarding this.");
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].mime_type, "message/rfc822");
    assert_eq!(m.attachments[0].filename.as_deref(), Some("inner.eml"));
    assert!(!plain(&m).contains("Inner body"));
    Ok(())
}

#[test]
fn rfc2231_file_name() -> R {
    let m = load("rfc2231-filename.eml")?;
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(
        m.attachments[0].filename.as_deref(),
        Some("naïve résumé.txt")
    );
    assert_eq!(m.attachments[0].size, 5);
    Ok(())
}

#[test]
fn missing_closing_boundary() -> R {
    let m = load("missing-closing-boundary.eml")?;
    assert!(plain(&m).contains("The closing boundary never arrives."));
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].filename.as_deref(), Some("more.txt"));
    Ok(())
}

#[test]
fn no_content_type() -> R {
    let m = load("no-content-type.eml")?;
    assert_eq!(plain(&m).trim(), "Just a body with no MIME headers.");
    assert!(m.text_html.is_none());
    Ok(())
}

#[test]
fn header_line_over_998_characters() -> R {
    let m = load("long-header-line.eml")?;
    assert_eq!(m.subject.as_deref().map(str::len), Some(1100));
    assert_eq!(m.to.len(), 80);
    assert_eq!(m.to[79].email, "user79@example.com");
    Ok(())
}

#[test]
fn header_injection_is_neutralised() -> R {
    let m = load("header-injection.eml")?;
    let subject = m.subject.unwrap_or_default();
    assert!(subject.starts_with("Hi"));
    assert!(!subject.contains(['\r', '\n', '\0']));
    for a in m.from.iter().chain(&m.to) {
        assert!(!a.email.contains(['\r', '\n', '\0']));
        assert!(!a
            .name
            .clone()
            .unwrap_or_default()
            .contains(['\r', '\n', '\0']));
    }
    Ok(())
}

#[test]
fn html_only_gets_derived_text() -> R {
    let m = load("html-only.eml")?;
    assert!(html(&m).contains("<script>"));
    let text = plain(&m);
    assert!(text.contains("HTML here"));
    assert!(!text.contains("alert"));
    assert!(m.snippet.starts_with("Only"));
    Ok(())
}

#[test]
fn debug_output_hides_html() -> R {
    let m = load("html-only.eml")?;
    assert!(!format!("{m:?}").contains("alert"));
    Ok(())
}

#[test]
fn inline_part_by_content_id() -> R {
    let path = format!(
        "{}/../../../tests/fixtures/mime/multipart-related-cid.eml",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path)?;
    let part = mailcrane_mime::inline_part(&bytes, "logo@example.org")?.ok_or("no part")?;
    assert_eq!(part.mime_type, "image/png");
    // The base64 body decodes to the 8-byte PNG signature.
    assert_eq!(part.bytes, b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        mailcrane_mime::inline_part(&bytes, "other@example.org")?,
        None
    );
    assert_eq!(mailcrane_mime::inline_part(&bytes, "")?, None);
    // Angle brackets are not part of the id.
    assert_eq!(
        mailcrane_mime::inline_part(&bytes, "<logo@example.org>")?,
        None
    );
    Ok(())
}

#[test]
fn inline_part_enforces_the_size_limit() {
    let huge = vec![b'a'; mailcrane_mime::limits::MAX_INPUT_BYTES + 1];
    assert_eq!(
        mailcrane_mime::inline_part(&huge, "x@example.org"),
        Err(mailcrane_mime::MimeError::TooLarge)
    );
}

#[test]
fn bidi_subject_and_attachment_name() -> R {
    // #315: the parser keeps what was sent; strip_bidi gives the logical
    // order the apps show.
    let m = load("bidi-subject-attachment.eml")?;
    let subject = m.subject.as_deref().unwrap_or_default();
    for c in ['\u{202e}', '\u{202d}', '\u{2067}', '\u{2068}', '\u{2069}'] {
        assert!(subject.contains(c), "{:04X} in the fixture", u32::from(c));
    }
    assert_eq!(strip_bidi(subject), "Invoice fdp.exe for March 2026 paid");
    let name = m.attachments[0].filename.as_deref().unwrap_or_default();
    assert!(name.chars().any(is_bidi_control));
    assert_eq!(strip_bidi(name), "invoicefdp.exe");
    Ok(())
}

/// #391: in an HTML-only message, block boundaries (headings, paragraphs,
/// `<br>`, list items, table cells, quotes) become one space in the
/// snippet, even with no whitespace between the tags; inline tags (`b`,
/// `a`, `span`) add none.
#[test]
fn html_blocks_are_separated_in_the_snippet() -> R {
    let m = load("html-blocks.eml")?;
    assert_eq!(
        m.snippet,
        "New people to discover Suggested accounts based on your contacts \
         Line one Line two Line three First Second Cell A Cell B Head C \
         Quoted Examples stay one word Acme Hello Sale Today"
    );
    Ok(())
}
