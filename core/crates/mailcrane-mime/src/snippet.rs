//! Short preview text for message lists.

/// How many characters the snippet holds.
pub const SNIPPET_CHARS: usize = 200;

/// The first [`SNIPPET_CHARS`] characters of `text`, with every run of
/// whitespace (and control characters) collapsed to one space.
pub fn snippet(text: &str) -> String {
    let mut out = String::new();
    let mut count: usize = 0;
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            // Only with room for a character after it, so the snippet
            // never ends in a space.
            if count.saturating_add(1) >= SNIPPET_CHARS {
                break;
            }
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        if count >= SNIPPET_CHARS {
            break;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Block-level HTML tags whose start or end separates text (#391).
const BLOCK_TAGS: [&str; 40] = [
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "br",
    "li",
    "tr",
    "td",
    "th",
    "blockquote",
    "pre",
    "hr",
    "ul",
    "ol",
    "table",
    "dt",
    "dd",
    "dl",
    "section",
    "center",
    "header",
    "footer",
    "article",
    "aside",
    "nav",
    "main",
    "address",
    "caption",
    "figure",
    "figcaption",
    "details",
    "summary",
    "form",
    "fieldset",
    "legend",
    "option",
];

/// The longest name in [`BLOCK_TAGS`] (a test keeps it right).
const LONGEST_TAG: usize = 10;

/// `html` as plain text with a line break at every block boundary, so
/// text from adjacent blocks stays apart: on its own line in `text_plain`
/// (a draft's body, the reply quote) and one space apart in the snippet,
/// which collapses whitespace (#391, #446). `<br>` and `</p>` keep the
/// line break `mail_parser::html_to_text` gives them, so a blank line made
/// with `<br><br>` stays; runs of blank lines become one, and the text
/// starts and ends with neither.
pub(crate) fn html_to_lines(html: &str) -> String {
    tidy_lines(&mail_parser::html_to_text(&break_html_blocks(html)))
}

/// `html` with a `<br>` in front of a block-level start or end tag that
/// follows text, so `html_to_text` ends the line there. Adjacent blocks
/// (`</div><div>`) get one, and none follows a `<br>` or `</p>`, which end
/// the line already. Inline tags (`b`, `a`, `span`) are left as they are.
///
/// It reads the HTML the way `html_to_text` does, so only text that ends
/// up in the output counts: comments and what is inside `<style>`,
/// `<script>`, `<template>` and `<head>` aren't text (and are left out of
/// the result), and a tag ends at the first `>` outside a quoted attribute
/// value (#456 review). One pass, no panics on any input.
fn break_html_blocks(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len().saturating_add(html.len() / 16));
    // Whether text has come since the last line break.
    let mut text_since = false;
    // Inside an element whose content `html_to_text` drops: its name.
    let mut hidden: Option<&'static str> = None;
    let mut pos = 0;
    while let Some(at) = html.get(pos..).and_then(|rest| rest.find('<')) {
        let lt = pos + at;
        let text = html.get(pos..lt).unwrap_or_default();
        if hidden.is_none() {
            out.push_str(text);
            if !text.trim().is_empty() {
                text_since = true;
            }
        }
        let end = if html.get(lt..).is_some_and(|t| t.starts_with("<!--")) {
            comment_end(bytes, lt + 2)
        } else {
            tag_end(bytes, lt + 1)
        };
        let tag = html.get(lt..end).unwrap_or_default();
        pos = end;
        if tag.starts_with("<!--") {
            continue;
        }
        let closing = bytes.get(lt + 1) == Some(&b'/');
        let name_start = lt + 1 + usize::from(closing);
        let len = bytes
            .get(name_start..end)
            .unwrap_or_default()
            .iter()
            .take(LONGEST_TAG + 1)
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        let name = bytes.get(name_start..name_start + len).unwrap_or_default();
        let is = |t: &str| t.as_bytes().eq_ignore_ascii_case(name);
        // A hidden element is left out whole: `html_to_text` drops its
        // text, but would still break lines at a `<p>` or `<br>` in it.
        if let Some(element) = hidden {
            if closing && is(element) {
                hidden = None;
            }
            continue;
        }
        if let Some(element) = HIDDEN_TAGS.iter().find(|t| is(t)).filter(|_| !closing) {
            hidden = Some(element);
            continue;
        }
        if is("br") || (closing && is("p")) {
            text_since = false;
        } else if text_since && BLOCK_TAGS.iter().any(|t| is(t)) {
            out.push_str("<br>");
            text_since = false;
        }
        out.push_str(tag);
    }
    if hidden.is_none() {
        out.push_str(html.get(pos..).unwrap_or_default());
    }
    out
}

/// Elements whose content `html_to_text` leaves out.
const HIDDEN_TAGS: [&str; 4] = ["style", "script", "template", "head"];

/// Where the tag starting before `from` ends (just past its `>`), skipping
/// quoted attribute values as `html_to_text` does: a quote counts only
/// right after `=` (spaces allowed). The end of `bytes` if it never closes.
fn tag_end(bytes: &[u8], from: usize) -> usize {
    let mut pos = from;
    let mut after_equals = false;
    while let Some(&b) = bytes.get(pos) {
        match b {
            b'>' => return pos + 1,
            b'"' | b'\'' if after_equals => {
                let close = bytes
                    .get(pos + 1..)
                    .and_then(|rest| rest.iter().position(|&c| c == b));
                match close {
                    Some(at) => pos += at + 2,
                    None => return bytes.len(),
                }
                after_equals = false;
                continue;
            }
            b'=' => after_equals = true,
            b' ' | b'\t' | b'\r' | b'\n' => {}
            _ => after_equals = false,
        }
        pos += 1;
    }
    bytes.len()
}

/// Where a comment whose `--` starts at `from` ends (just past its `>`):
/// at the first `>` after `--` or `--!`, as `html_to_text` reads it. The
/// end of `bytes` if it never closes.
fn comment_end(bytes: &[u8], from: usize) -> usize {
    let mut pos = from;
    while let Some(at) = bytes
        .get(pos..)
        .and_then(|rest| rest.iter().position(|&b| b == b'>'))
    {
        let close = pos + at;
        let body = bytes.get(from..close).unwrap_or_default();
        if body.len() >= 2 && (body.ends_with(b"--") || (body.len() >= 3 && body.ends_with(b"--!")))
        {
            return close + 1;
        }
        pos = close + 1;
    }
    bytes.len()
}

/// `text` with trailing spaces cut from each line, no blank lines at the
/// start or end, and each run of blank lines made one.
fn tidy_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = false;
    for line in text.split('\n').map(str::trim_end) {
        if line.is_empty() {
            blank_run = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank_run {
                out.push('\n');
            }
        }
        blank_run = false;
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_tag_covers_every_block_tag() {
        assert!(BLOCK_TAGS.iter().all(|t| t.len() <= LONGEST_TAG));
        assert!(BLOCK_TAGS.iter().any(|t| t.len() == LONGEST_TAG));
    }

    /// #440 review: newsletter layouts use these blocks too.
    #[test]
    fn layout_blocks_are_lines() {
        assert_eq!(
            html_to_lines("<header>Acme</header><main>Hello</main><center>Sale</center>"),
            "Acme\nHello\nSale"
        );
    }

    #[test]
    fn block_tags_end_lines_and_inline_tags_dont() {
        assert_eq!(
            html_to_lines("<h1>A</h1><p>B<b>c</b></P><BR/>d<li>e"),
            "A\nBc\n\nd\ne"
        );
        // Names that only start like a block tag are inline.
        assert_eq!(html_to_lines("a<pa>b<br2>c<b>d"), "abcd");
        for odd in [
            "", "<", "</", "<<p", "a<", "<\u{e9}>", "<p", "</ p>", "<p<p>", "a>b<p>",
        ] {
            let _ = html_to_lines(odd);
        }
    }

    /// #446: adjacent blocks give one line each, not a blank line between;
    /// a blank line the sender made with `<br><br>` stays, once.
    #[test]
    fn adjacent_blocks_are_one_line_each_and_blank_lines_stay() {
        assert_eq!(
            html_to_lines("<div>Line 1</div><div>Line 2</div>"),
            "Line 1\nLine 2"
        );
        assert_eq!(
            html_to_lines("<div>Hi,</div><div><br></div><div>Thanks</div>"),
            "Hi,\n\nThanks"
        );
        assert_eq!(html_to_lines("A<br><br><br><br>B"), "A\n\nB");
        assert_eq!(html_to_lines("<p>A</p><p>B</p>"), "A\nB");
        // Text after a block's end still starts a new line.
        assert_eq!(html_to_lines("<div>A</div>B<div>C</div>"), "A\nB\nC");
        // Attributes aren't text.
        assert_eq!(
            html_to_lines(r#"<div class="x">A</div><div title="y">B</div>"#),
            "A\nB"
        );
    }

    /// #456 review: what `html_to_text` leaves out isn't text, so no blank
    /// line comes from a style block, a script, a template or a comment
    /// between blocks; nor from a `>` or `<` in a quoted attribute value.
    #[test]
    fn hidden_content_and_quoted_values_add_no_blank_line() {
        for between in [
            "<style>p { color: red }</style>",
            "<script>if (a > b) {}</script>",
            "<template><p>x</p></template>",
            "<!-- a > b -->",
            "<!-- <p>x</p> -->",
            r#"<div><a title="1 > 0"><img src=x></a></div>"#,
            r#"<img alt="x>y">"#,
            r#"<img alt='<p>'>"#,
        ] {
            assert_eq!(
                html_to_lines(&format!("<div>A</div>{between}<div>B</div>")),
                "A\nB",
                "{between}"
            );
        }
        // Text after a hidden element still counts.
        assert_eq!(html_to_lines("<style>x</style>A<div>B</div>"), "A\nB");
        // A head before the body is left out, then text counts again.
        assert_eq!(
            html_to_lines(
                "<html><head><title>T</title></head><body><p>A</p><p>B</p></body></html>"
            ),
            "A\nB"
        );
        for odd in ["<!--", "<!-- x", "<a title=\"x", "<style>", "<a title='>"] {
            let _ = html_to_lines(odd);
        }
    }

    #[test]
    fn tidy_lines_trims_and_keeps_one_blank_line() {
        assert_eq!(tidy_lines("\n\n a  \n\n\n\nb \n\n"), " a\n\nb");
        assert_eq!(tidy_lines(""), "");
        assert_eq!(tidy_lines("\n \n"), "");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(snippet("  a \r\n\t b  "), "a b");
    }

    #[test]
    fn never_ends_in_a_space_at_the_cap() {
        let text = format!("{} next", "x".repeat(SNIPPET_CHARS - 1));
        let s = snippet(&text);
        assert_eq!(s, "x".repeat(SNIPPET_CHARS - 1));
    }

    #[test]
    fn caps_length() {
        assert_eq!(snippet(&"x".repeat(500)).chars().count(), SNIPPET_CHARS);
        assert!(snippet(&"x ".repeat(500)).chars().count() <= SNIPPET_CHARS);
    }
}
