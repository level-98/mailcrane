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
/// One pass, a few bytes of look-ahead per `<` with no allocation, and no
/// panics on any input.
fn break_html_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len().saturating_add(html.len() / 16));
    let mut rest = html;
    // Whether text has come since the last line break.
    let mut text_since = false;
    let mut in_tag = false;
    while let Some(at) = rest.find('<') {
        let (before, tag) = rest.split_at(at);
        out.push_str(before);
        // `before` starts with the previous tag's name and attributes.
        let text = if in_tag {
            before
                .find('>')
                .map_or("", |i| before.get(i + 1..).unwrap_or_default())
        } else {
            before
        };
        if !text.trim().is_empty() {
            text_since = true;
        }
        let closing = tag.as_bytes().get(1) == Some(&b'/');
        let name_start = usize::from(closing) + 1;
        let after = tag.as_bytes().get(name_start..).unwrap_or_default();
        let len = after
            .iter()
            .take(LONGEST_TAG + 1)
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        let name = after.get(..len).unwrap_or_default();
        let is = |t: &str| t.as_bytes().eq_ignore_ascii_case(name);
        if is("br") || (closing && is("p")) {
            text_since = false;
        } else if text_since && BLOCK_TAGS.iter().any(|t| is(t)) {
            out.push_str("<br>");
            text_since = false;
        }
        out.push('<');
        rest = tag.get(1..).unwrap_or_default();
        in_tag = true;
    }
    out.push_str(rest);
    out
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
