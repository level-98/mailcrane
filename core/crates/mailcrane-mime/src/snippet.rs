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

/// `html` with a space in front of every block-level start or end tag, so
/// text from adjacent blocks stays apart when the HTML is turned into text
/// (`mail_parser::html_to_text` only separates some of them). Inline tags
/// (`b`, `a`, `span`) are left as they are. One pass, a few bytes of
/// look-ahead per `<` with no allocation, and no panics on any input
/// (#391).
pub(crate) fn space_html_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len().saturating_add(html.len() / 16));
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        let (before, tag) = rest.split_at(at);
        out.push_str(before);
        let name_start = usize::from(tag.as_bytes().get(1) == Some(&b'/')) + 1;
        let after = tag.as_bytes().get(name_start..).unwrap_or_default();
        let len = after
            .iter()
            .take(LONGEST_TAG + 1)
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        let name = after.get(..len).unwrap_or_default();
        if BLOCK_TAGS
            .iter()
            .any(|t| t.as_bytes().eq_ignore_ascii_case(name))
        {
            out.push(' ');
        }
        out.push('<');
        rest = tag.get(1..).unwrap_or_default();
    }
    out.push_str(rest);
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
    fn layout_blocks_are_separated() {
        assert_eq!(
            space_html_blocks("<header>Acme</header><main>Hello</main><center>Sale</center>"),
            " <header>Acme </header> <main>Hello </main> <center>Sale </center>"
        );
    }

    #[test]
    fn block_tags_get_a_space_and_inline_tags_dont() {
        assert_eq!(
            space_html_blocks("<h1>A</h1><p>B<b>c</b></P><BR/>d<li>e"),
            " <h1>A </h1> <p>B<b>c</b> </P> <BR/>d <li>e"
        );
        // Names that only start like a block tag are inline.
        assert_eq!(space_html_blocks("<pre><pa><b><br2>"), " <pre><pa><b><br2>");
        for odd in [
            "", "<", "</", "<<p", "a<", "<\u{e9}>", "<p", "</ p>", "<p<p>",
        ] {
            let _ = space_html_blocks(odd);
        }
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
