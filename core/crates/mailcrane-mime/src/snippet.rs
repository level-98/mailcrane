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
/// which collapses whitespace (#391, #446). `mail_parser::html_to_text`
/// alone decides what is text; block boundaries are marked before it runs
/// and turned into line breaks after (#456 review). A blank line the
/// sender made with `<br><br>` stays; runs of blank lines become one, and
/// the text starts and ends with neither.
pub(crate) fn html_to_lines(html: &str) -> String {
    let text = mail_parser::html_to_text(&mark_html_blocks(html));
    tidy_lines(&lines_from_marks(&text))
}

/// Stands for a line break between blocks while `html_to_text` runs (a
/// private-use character, so it means nothing in real text).
const MARK: char = '\u{E000}';

/// Stands for a `<br>` or `</p>`, whose line break `html_to_text` writes
/// itself: only a line break right after this mark is the sender's. One
/// without it comes from a `<br>` or `</p>` inside content html_to_text
/// leaves out (a template, a style), and counts as a block boundary.
const MARK_BREAK: char = '\u{E001}';

/// What goes before a block tag: an unknown tag, which `html_to_text`
/// ignores but which also ends any tag it was still reading, then the mark
/// as text. So the mark reaches the output exactly when the block does:
/// inside a comment, a style or script, a template or a quoted attribute
/// value it is dropped with that content (#456 review).
const MARKER: &str = "<mc-block>\u{E000}";

/// The same before a `<br>` or `</p>`.
const MARKER_BREAK: &str = "<mc-block>\u{E001}";

/// `html` with [`MARKER`] in front of every block-level start or end tag,
/// and [`MARKER_BREAK`] in front of each `<br>` and `</p>`. Nothing else
/// is changed: what is text is left to `html_to_text` alone, so no content
/// can be lost here. Names are read as `html_to_text` reads them: after
/// `<`, spaces and `/` are skipped (a `/` makes it an end tag), and the
/// name runs up to `<`, `>`, `/` or a space. Marks already in the input
/// are dropped first. One pass with checked positions; no panics on any
/// input.
fn mark_html_blocks(html: &str) -> String {
    let html = html.replace([MARK, MARK_BREAK], "");
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len().saturating_add(html.len() / 8));
    let mut copied = 0;
    let mut pos = 0;
    while let Some(at) = html.get(pos..).and_then(|rest| rest.find('<')) {
        let lt = pos.saturating_add(at);
        let mut name_start = lt.saturating_add(1);
        while bytes
            .get(name_start)
            .is_some_and(|b| *b == b'/' || b.is_ascii_whitespace())
        {
            name_start = name_start.saturating_add(1);
        }
        let len = bytes
            .get(name_start..)
            .unwrap_or_default()
            .iter()
            .take(LONGEST_TAG.saturating_add(1))
            .take_while(|b| !matches!(b, b'<' | b'>' | b'/') && !b.is_ascii_whitespace())
            .count();
        let name = bytes
            .get(name_start..name_start.saturating_add(len))
            .unwrap_or_default();
        let closing = bytes
            .get(lt.saturating_add(1)..name_start)
            .is_some_and(|between| between.contains(&b'/'));
        let is = |t: &str| t.as_bytes().eq_ignore_ascii_case(name);
        let marker = if is("br") || (closing && is("p")) {
            Some(MARKER_BREAK)
        } else if BLOCK_TAGS.iter().any(|t| is(t)) {
            Some(MARKER)
        } else {
            None
        };
        if let Some(marker) = marker {
            out.push_str(html.get(copied..lt).unwrap_or_default());
            out.push_str(marker);
            copied = lt;
        }
        pos = lt.saturating_add(1);
    }
    out.push_str(html.get(copied..).unwrap_or_default());
    out
}

/// `html_to_text`'s output with each run of block marks made one line
/// break, where text came before it on the line; spaces after a mark are
/// dropped. A line break `html_to_text` wrote for a sender's `<br>` or
/// `</p>` (right after [`MARK_BREAK`]) is kept, after the break of a block
/// mark before it, so a blank line the sender made (`<div><br></div>`,
/// `<br><br>`) stays. Any other line break counts as a block mark.
fn lines_from_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending = false;
    let mut sender_break = false;
    let line_started = |out: &String| !out.is_empty() && !out.ends_with('\n');
    for c in text.chars() {
        match c {
            MARK => pending = true,
            MARK_BREAK => sender_break = true,
            '\n' if sender_break => {
                if pending && line_started(&out) {
                    out.push('\n');
                }
                out.push('\n');
                pending = false;
                sender_break = false;
            }
            '\n' => pending = true,
            ' ' | '\t' if pending || sender_break => {}
            _ => {
                if pending && line_started(&out) {
                    out.push('\n');
                }
                pending = false;
                sender_break = false;
                out.push(c);
            }
        }
    }
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

    /// #456 review: nothing after a stray `<` is lost, wherever it is;
    /// html_to_text alone decides what is text.
    #[test]
    fn a_stray_angle_bracket_loses_nothing() {
        assert_eq!(
            html_to_lines("<div>A</div><script>if (a < b) {}</script><div>B</div>"),
            "A\nB"
        );
        assert_eq!(
            html_to_lines("A<div>B</div><p>C < D</p><div>E</div>"),
            // html_to_text reads `< D</p` as a tag (as it always has); the
            // blocks around it keep their lines.
            "A\nB\nC\nE"
        );
        // html_to_text reads `<3 you` as a tag; the block after it still
        // starts a line.
        assert_eq!(html_to_lines("I <3 you<div>Next</div>"), "I\nNext");
        for name in ["head-x", "style:x", "script-x", "template-x"] {
            assert_eq!(
                html_to_lines(&format!("<div>A</div><{name}>m</{name}><div>B</div>")),
                "A\nm\nB",
                "{name}"
            );
        }
    }

    /// #456 review: a raw no-break space is text to html_to_text, so a block
    /// holding only one is a blank line, as `&nbsp;` is.
    #[test]
    fn a_block_of_a_no_break_space_is_a_blank_line() {
        assert_eq!(
            html_to_lines("<div>A</div><div>\u{a0}</div><div>B</div>"),
            "A\n\nB"
        );
        assert_eq!(
            html_to_lines("<div>A</div><div>&nbsp;</div><div>B</div>"),
            "A\n\nB"
        );
    }

    /// #456 review: tag names as html_to_text reads them.
    #[test]
    fn closing_tags_with_spaces_and_odd_names() {
        assert_eq!(html_to_lines("A</ p><div>B"), "A\nB");
        assert_eq!(html_to_lines("A< br>B"), "A\nB");
        // Not `</p>` to html_to_text: no line break, from it or from us.
        assert_eq!(html_to_lines("<div>A</p:x>B</div>"), "AB");
        assert_eq!(html_to_lines("<div>A</div><!--!>x--><div>B</div>"), "A\nB");
        // A mark in the input is no line break.
        assert_eq!(html_to_lines("A\u{e000}B\u{e001}C"), "ABC");
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
