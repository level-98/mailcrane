//! Parses each synthetic fixture in `tests/fixtures/mime/` and checks the
//! fields.

use mailcrane_mime::{parse_message, Disposition, ParsedMessage};

fn load(name: &str) -> ParsedMessage {
    let path = format!(
        "{}/../../../tests/fixtures/mime/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap();
    parse_message(&bytes).unwrap()
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
fn plain_ascii() {
    let m = load("plain-ascii.eml");
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
}

#[test]
fn utf8_body() {
    let m = load("utf8-body.eml");
    assert_eq!(plain(&m).trim(), "Grüße, café ☕");
}

#[test]
fn iso_8859_1() {
    let m = load("iso-8859-1.eml");
    assert_eq!(plain(&m).trim(), "café naïve");
}

#[test]
fn windows_1252() {
    let m = load("windows-1252.eml");
    assert_eq!(plain(&m).trim(), "price € 5 “quoted”");
}

#[test]
fn shift_jis() {
    let m = load("shift-jis.eml");
    assert_eq!(plain(&m).trim(), "こんにちは");
}

#[test]
fn encoded_word_subject_b_and_q() {
    let m = load("encoded-word-subject.eml");
    assert_eq!(m.subject.as_deref(), Some("Hello wörld and café time"));
}

#[test]
fn encoded_display_names() {
    let m = load("encoded-display-names.eml");
    assert_eq!(m.from[0].name.as_deref(), Some("Åsa Åndersson"));
    assert_eq!(m.from[0].email, "asa@example.org");
    assert_eq!(m.to.len(), 2);
    assert_eq!(m.to[0].name.as_deref(), Some("Jürgen Müller"));
    assert_eq!(m.to[1].name.as_deref(), Some("Plain, Name"));
    assert_eq!(m.to[1].email, "pn@example.net");
}

#[test]
fn multipart_alternative() {
    let m = load("multipart-alternative.eml");
    assert_eq!(plain(&m).trim(), "Plain version");
    assert_eq!(html(&m).trim(), "<p>HTML <b>version</b></p>");
    assert!(m.attachments.is_empty());
}

#[test]
fn multipart_mixed_with_pdf() {
    let m = load("multipart-mixed-pdf.eml");
    assert_eq!(plain(&m).trim(), "Please see the attached report.");
    assert_eq!(m.attachments.len(), 1);
    let a = &m.attachments[0];
    assert_eq!(a.filename.as_deref(), Some("report.pdf"));
    assert_eq!(a.mime_type, "application/pdf");
    assert_eq!(a.size, 31);
    assert_eq!(a.disposition, Disposition::Attachment);
    assert!(a.content_id.is_none());
}

#[test]
fn multipart_related_inline_cid() {
    let m = load("multipart-related-cid.eml");
    assert!(html(&m).contains("cid:logo@example.org"));
    assert_eq!(m.attachments.len(), 1);
    let a = &m.attachments[0];
    assert_eq!(a.content_id.as_deref(), Some("logo@example.org"));
    assert_eq!(a.mime_type, "image/png");
    assert_eq!(a.disposition, Disposition::Inline);
    assert_eq!(a.size, 8);
}

#[test]
fn nested_rfc822() {
    let m = load("nested-rfc822.eml");
    assert_eq!(m.subject.as_deref(), Some("Fwd: inner"));
    assert_eq!(plain(&m).trim(), "Forwarding this.");
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].mime_type, "message/rfc822");
    assert_eq!(m.attachments[0].filename.as_deref(), Some("inner.eml"));
    assert!(!plain(&m).contains("Inner body"));
}

#[test]
fn rfc2231_file_name() {
    let m = load("rfc2231-filename.eml");
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(
        m.attachments[0].filename.as_deref(),
        Some("naïve résumé.txt")
    );
    assert_eq!(m.attachments[0].size, 5);
}

#[test]
fn missing_closing_boundary() {
    let m = load("missing-closing-boundary.eml");
    assert!(plain(&m).contains("The closing boundary never arrives."));
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].filename.as_deref(), Some("more.txt"));
}

#[test]
fn no_content_type() {
    let m = load("no-content-type.eml");
    assert_eq!(plain(&m).trim(), "Just a body with no MIME headers.");
    assert!(m.text_html.is_none());
}

#[test]
fn header_line_over_998_characters() {
    let m = load("long-header-line.eml");
    assert_eq!(m.subject.as_deref().map(str::len), Some(1100));
    assert_eq!(m.to.len(), 80);
    assert_eq!(m.to[79].email, "user79@example.com");
}

#[test]
fn header_injection_is_neutralised() {
    let m = load("header-injection.eml");
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
}

#[test]
fn html_only_gets_derived_text() {
    let m = load("html-only.eml");
    assert!(html(&m).contains("<script>"));
    let text = plain(&m);
    assert!(text.contains("HTML here"));
    assert!(!text.contains("alert"));
    assert!(m.snippet.starts_with("Only"));
}

#[test]
fn debug_output_hides_html() {
    let m = load("html-only.eml");
    assert!(!format!("{m:?}").contains("alert"));
}
